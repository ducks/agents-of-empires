use std::collections::HashMap;
use std::sync::Arc;

use aoe_domain::{ArenaManifest, TerritoryConfig};
use async_trait::async_trait;
use futures::future::join_all;
use thiserror::Error;

use crate::{NetworkAssignment, NetworkPlan};

/// Controller-side reference to one running territory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerritoryHandle {
    pub territory: String,
    pub process_id: Option<u32>,
    pub network: NetworkAssignment,
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("could not build territory {territory}: {detail}")]
    Build { territory: String, detail: String },
    #[error("could not boot territory {territory}: {detail}")]
    Boot { territory: String, detail: String },
    #[error("could not stop territory {territory}: {detail}")]
    Stop { territory: String, detail: String },
    #[error("territory {0} is not running")]
    NotRunning(String),
    #[error("network plan does not contain territory {0}")]
    MissingNetwork(String),
    #[error("one or more territories failed to boot: {0}")]
    PartialBoot(String),
}

/// Backend responsible for building and controlling one kind of guest.
#[async_trait]
pub trait TerritoryDriver: Send + Sync {
    async fn boot(
        &self,
        territory: &TerritoryConfig,
        network: &NetworkAssignment,
    ) -> Result<TerritoryHandle, RuntimeError>;

    async fn stop(&self, handle: &TerritoryHandle) -> Result<(), RuntimeError>;

    async fn reboot(
        &self,
        territory: &TerritoryConfig,
        handle: &TerritoryHandle,
    ) -> Result<TerritoryHandle, RuntimeError> {
        self.stop(handle).await?;
        self.boot(territory, &handle.network).await
    }
}

/// Coordinates multiple guests and cleans up successful boots after a partial
/// failure.
pub struct ArenaSupervisor<D: TerritoryDriver> {
    driver: Arc<D>,
    running: HashMap<String, TerritoryHandle>,
}

impl<D: TerritoryDriver> ArenaSupervisor<D> {
    #[must_use]
    pub fn new(driver: Arc<D>) -> Self {
        Self {
            driver,
            running: HashMap::new(),
        }
    }

    /// Boot every territory concurrently.
    ///
    /// # Errors
    ///
    /// Returns a partial-boot error after stopping every guest that did start.
    pub async fn boot_all(
        &mut self,
        manifest: &ArenaManifest,
        network: &NetworkPlan,
    ) -> Result<(), RuntimeError> {
        let assignments: HashMap<_, _> = network
            .assignments
            .iter()
            .map(|assignment| (assignment.territory.as_str(), assignment))
            .collect();
        let driver = Arc::clone(&self.driver);
        let futures = manifest.territories.iter().map(|territory| {
            let driver = Arc::clone(&driver);
            let assignment = assignments.get(territory.id.as_str()).copied();
            async move {
                let Some(assignment) = assignment else {
                    return (
                        territory.id.clone(),
                        Err(RuntimeError::MissingNetwork(territory.id.clone())),
                    );
                };
                (
                    territory.id.clone(),
                    driver.boot(territory, assignment).await,
                )
            }
        });
        let results = join_all(futures).await;
        let mut failures = Vec::new();
        for (territory, result) in results {
            match result {
                Ok(handle) => {
                    self.running.insert(territory, handle);
                }
                Err(error) => failures.push(error.to_string()),
            }
        }
        if failures.is_empty() {
            return Ok(());
        }
        let cleanup_errors = self.stop_all().await;
        if !cleanup_errors.is_empty() {
            failures.extend(cleanup_errors);
        }
        Err(RuntimeError::PartialBoot(failures.join("; ")))
    }

    /// Reboot one territory while preserving its assigned network coordinates.
    ///
    /// # Errors
    ///
    /// Returns an error when the territory is unknown or its driver fails.
    pub async fn reboot(&mut self, territory: &TerritoryConfig) -> Result<(), RuntimeError> {
        let handle = self
            .running
            .get(&territory.id)
            .cloned()
            .ok_or_else(|| RuntimeError::NotRunning(territory.id.clone()))?;
        let replacement = self.driver.reboot(territory, &handle).await?;
        self.running.insert(territory.id.clone(), replacement);
        Ok(())
    }

    /// Stop one running territory.
    ///
    /// # Errors
    ///
    /// Returns an error when the territory is unknown or the driver cannot
    /// stop it.
    pub async fn stop(&mut self, territory: &str) -> Result<(), RuntimeError> {
        let handle = self
            .running
            .remove(territory)
            .ok_or_else(|| RuntimeError::NotRunning(territory.to_owned()))?;
        self.driver.stop(&handle).await
    }

    /// Stop all running guests, returning every cleanup error.
    pub async fn stop_all(&mut self) -> Vec<String> {
        let handles: Vec<_> = self.running.drain().map(|(_, handle)| handle).collect();
        join_all(handles.iter().map(|handle| self.driver.stop(handle)))
            .await
            .into_iter()
            .filter_map(Result::err)
            .map(|error| error.to_string())
            .collect()
    }

    #[must_use]
    pub fn handles(&self) -> &HashMap<String, TerritoryHandle> {
        &self.running
    }
}
