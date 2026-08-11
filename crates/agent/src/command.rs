use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use tokio::process::Command;

use crate::{AgentAdapter, AgentControllerError, AgentInvocation, AgentResult, AgentStatus};

/// Adapter for an executable implementing the environment-variable protocol.
pub struct CommandAdapter {
    executable: PathBuf,
    state_root: PathBuf,
}

impl CommandAdapter {
    #[must_use]
    pub fn new(executable: impl Into<PathBuf>, state_root: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            state_root: state_root.into(),
        }
    }

    async fn read_result(
        path: &Path,
        invocation: &AgentInvocation,
    ) -> Result<AgentResult, AgentControllerError> {
        let source = tokio::fs::read_to_string(path)
            .await
            .map_err(|error| AgentControllerError::Adapter(error.to_string()))?;
        let result: AgentResult = serde_json::from_str(&source)
            .map_err(|error| AgentControllerError::Adapter(error.to_string()))?;
        if result.schema_version != 1
            || result.agent != invocation.config.id
            || result.territory != invocation.config.territory
        {
            return Err(AgentControllerError::Adapter(
                "adapter result identity does not match invocation".into(),
            ));
        }
        Ok(result)
    }
}

#[async_trait]
impl AgentAdapter for CommandAdapter {
    async fn run(
        &self,
        invocation: AgentInvocation,
        timeout: Duration,
    ) -> Result<AgentResult, AgentControllerError> {
        let run_root = self.state_root.join(&invocation.config.id);
        tokio::fs::create_dir_all(&run_root)
            .await
            .map_err(|error| AgentControllerError::Adapter(error.to_string()))?;
        let instruction_path = run_root.join("instruction.md");
        let result_path = run_root.join("result.json");
        let stdout_path = run_root.join("stdout.log");
        let stderr_path = run_root.join("stderr.log");
        tokio::fs::write(&instruction_path, &invocation.instruction)
            .await
            .map_err(|error| AgentControllerError::Adapter(error.to_string()))?;

        let stdout = std::fs::File::create(&stdout_path)
            .map_err(|error| AgentControllerError::Adapter(error.to_string()))?;
        let stderr = std::fs::File::create(&stderr_path)
            .map_err(|error| AgentControllerError::Adapter(error.to_string()))?;
        let mut command = Command::new(&self.executable);
        command
            .env("AOE_AGENT_ID", &invocation.config.id)
            .env("AOE_TERRITORY_ID", &invocation.config.territory)
            .env("AOE_TERRITORY_HOST", &invocation.territory_host)
            .env("AOE_SSH_PORT", invocation.ssh_port.to_string())
            .env("AOE_MODEL", &invocation.config.model)
            .env("AOE_REASONING_EFFORT", &invocation.config.reasoning_effort)
            .env("AOE_INSTRUCTION_FILE", &instruction_path)
            .env("AOE_RESULT_FILE", &result_path)
            .env("AOE_STDOUT_FILE", &stdout_path)
            .env("AOE_STDERR_FILE", &stderr_path)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .kill_on_drop(true);
        if let Some(credentials) = &invocation.credential_file {
            command.env("AOE_CREDENTIAL_FILE", credentials);
        }
        let mut child = command
            .spawn()
            .map_err(|error| AgentControllerError::Adapter(error.to_string()))?;

        let completed = tokio::time::timeout(timeout, child.wait()).await;
        match completed {
            Ok(Ok(status)) if status.success() => {
                Self::read_result(&result_path, &invocation).await
            }
            Ok(Ok(status)) => {
                if result_path.exists() {
                    Self::read_result(&result_path, &invocation).await
                } else {
                    Ok(AgentResult::controller_result(
                        &invocation,
                        AgentStatus::HarnessError,
                        format!("adapter exited with {status}"),
                    ))
                }
            }
            Ok(Err(error)) => Err(AgentControllerError::Adapter(error.to_string())),
            Err(_) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                if result_path.exists() {
                    let mut partial = Self::read_result(&result_path, &invocation).await?;
                    partial.status = AgentStatus::TimedOut;
                    partial.summary = "agent exceeded its deadline".into();
                    Ok(partial)
                } else {
                    let mut result = AgentResult::controller_result(
                        &invocation,
                        AgentStatus::TimedOut,
                        "agent exceeded its deadline",
                    );
                    result.transcript = Some(stdout_path);
                    Ok(result)
                }
            }
        }
    }
}

/// Write one normalized result from a simple adapter without depending on
/// synchronous filesystem APIs. Intended for adapter implementations.
///
/// # Errors
///
/// Returns an error when serialization or writing fails.
pub async fn write_result(path: &Path, result: &AgentResult) -> Result<(), AgentControllerError> {
    let encoded = serde_json::to_vec(result)
        .map_err(|error| AgentControllerError::Adapter(error.to_string()))?;
    tokio::fs::write(path, encoded)
        .await
        .map_err(|error| AgentControllerError::Adapter(error.to_string()))
}
