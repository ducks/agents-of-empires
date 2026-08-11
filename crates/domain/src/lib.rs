//! Shared domain types for Agents of Empires.

mod event;
mod manifest;
mod state;

pub use event::{Event, EventEnvelope, FailureSource};
pub use manifest::{
    AgentConfig, ArenaConfig, ArenaManifest, Budget, HealthPolicy, ManifestError, MatchRules,
    NetworkConfig, ResourceLimits, ServiceCheck, TerritoryClass, TerritoryConfig, ValidationError,
};
pub use state::{MatchState, TerritoryState};
