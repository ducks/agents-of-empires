use serde::{Deserialize, Serialize};

/// Controller-owned lifecycle state for a territory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerritoryState {
    Provisioning,
    Healthy,
    Degraded,
    Recovering,
    Eliminated,
}

/// Controller-owned lifecycle state for a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchState {
    Preparing,
    Running,
    Finished,
    Aborted,
}
