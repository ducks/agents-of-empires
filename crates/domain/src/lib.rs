//! Shared domain types for Agents of Empires.

mod event;
mod manifest;
mod state;

pub use event::{Event, EventEnvelope, FailureSource};
pub use manifest::{
    AgentConfig, ArenaConfig, ArenaManifest, Budget, BuildContract, HealthPolicy, ManifestError,
    MatchMode, MatchRules, MilestoneConfig, NetworkConfig, ResourceLimits, ServiceCheck,
    TerritoryClass, TerritoryConfig, ValidationError,
};
pub use state::{CompetitorState, MatchState, TerritoryState};
