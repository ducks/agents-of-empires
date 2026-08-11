use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use aoe_domain::TerritoryConfig;
use async_trait::async_trait;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::{NetworkAssignment, RuntimeError, TerritoryDriver, TerritoryHandle};

/// NixOS VM driver using flake installables that resolve to VM runner outputs.
pub struct NixVmDriver {
    state_root: PathBuf,
    children: Mutex<HashMap<String, Child>>,
}

impl NixVmDriver {
    #[must_use]
    pub fn new(state_root: impl Into<PathBuf>) -> Self {
        Self {
            state_root: state_root.into(),
            children: Mutex::new(HashMap::new()),
        }
    }

    async fn build_runner(&self, territory: &TerritoryConfig) -> Result<PathBuf, RuntimeError> {
        let output = Command::new("nix")
            .args([
                "--extra-experimental-features",
                "nix-command flakes",
                "build",
                "--no-link",
                "--print-out-paths",
                &territory.nixos_config,
            ])
            .output()
            .await
            .map_err(|error| RuntimeError::Build {
                territory: territory.id.clone(),
                detail: error.to_string(),
            })?;
        if !output.status.success() {
            return Err(RuntimeError::Build {
                territory: territory.id.clone(),
                detail: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        let store_path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
        find_runner(&store_path).map_err(|detail| RuntimeError::Build {
            territory: territory.id.clone(),
            detail,
        })
    }
}

#[async_trait]
impl TerritoryDriver for NixVmDriver {
    async fn boot(
        &self,
        territory: &TerritoryConfig,
        network: &NetworkAssignment,
    ) -> Result<TerritoryHandle, RuntimeError> {
        let runner = self.build_runner(territory).await?;
        let territory_root = self.state_root.join(&territory.id);
        let vm_state = territory_root.join("vm-state");
        tokio::fs::create_dir_all(&vm_state)
            .await
            .map_err(|error| RuntimeError::Boot {
                territory: territory.id.clone(),
                detail: error.to_string(),
            })?;
        let stdout =
            std::fs::File::create(territory_root.join("console.log")).map_err(|error| {
                RuntimeError::Boot {
                    territory: territory.id.clone(),
                    detail: error.to_string(),
                }
            })?;
        let stderr = stdout.try_clone().map_err(|error| RuntimeError::Boot {
            territory: territory.id.clone(),
            detail: error.to_string(),
        })?;
        let mut command = Command::new(runner);
        command
            .current_dir(vm_state)
            .env("QEMU_NET_OPTS", network.qemu_net_opts())
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .kill_on_drop(true);
        let child = command.spawn().map_err(|error| RuntimeError::Boot {
            territory: territory.id.clone(),
            detail: error.to_string(),
        })?;
        let process_id = child.id();
        self.children
            .lock()
            .await
            .insert(territory.id.clone(), child);
        Ok(TerritoryHandle {
            territory: territory.id.clone(),
            process_id,
            network: network.clone(),
        })
    }

    async fn stop(&self, handle: &TerritoryHandle) -> Result<(), RuntimeError> {
        let Some(mut child) = self.children.lock().await.remove(&handle.territory) else {
            return Ok(());
        };
        child.kill().await.map_err(|error| RuntimeError::Stop {
            territory: handle.territory.clone(),
            detail: error.to_string(),
        })
    }
}

fn find_runner(store_path: &Path) -> Result<PathBuf, String> {
    let bin = store_path.join("bin");
    let entries = std::fs::read_dir(&bin)
        .map_err(|error| format!("could not read {}: {error}", bin.display()))?;
    let mut runners = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("run-") && name.ends_with("-vm"))
        });
    let runner = runners
        .next()
        .ok_or_else(|| format!("{} contains no NixOS VM runner", bin.display()))?;
    if runners.next().is_some() {
        return Err(format!(
            "{} contains multiple NixOS VM runners",
            bin.display()
        ));
    }
    Ok(runner)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::find_runner;

    #[test]
    fn finds_exactly_one_vm_runner() {
        let root = std::env::temp_dir().join(format!("aoe-runtime-{}", std::process::id()));
        let bin = root.join("bin");
        fs::create_dir_all(&bin).expect("create bin");
        fs::write(bin.join("run-test-vm"), "").expect("write runner");
        assert_eq!(find_runner(&root).expect("runner"), bin.join("run-test-vm"));
        fs::remove_dir_all(root).expect("cleanup");
    }
}
