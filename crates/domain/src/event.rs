use serde::{Deserialize, Serialize};

use crate::{CompetitorState, MatchState, TerritoryState};

/// The authority responsible for a failed operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureSource {
    Player,
    Harness,
    Provider,
    Arena,
    Controller,
    Unknown,
}

/// One immutable fact emitted by the controller.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    TerritoryRegistered {
        territory: String,
        class: String,
        agent: String,
    },
    MatchStateChanged {
        from: MatchState,
        to: MatchState,
    },
    TerritoryStateChanged {
        territory: String,
        from: TerritoryState,
        to: TerritoryState,
        reason: String,
    },
    HealthObserved {
        territory: String,
        healthy: bool,
        status: Option<u16>,
        latency_ms: u64,
        detail: String,
    },
    AgentStarted {
        agent: String,
        territory: String,
        model: String,
    },
    AgentFinished {
        agent: String,
        source: FailureSource,
        success: bool,
        detail: String,
    },
    UsageCharged {
        agent: String,
        resource_units: u64,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        cost_microusd: Option<u64>,
    },
    PostMatchDrainStarted {
        timeout_ms: u64,
        pending_agents: u64,
    },
    PostMatchDrainFinished {
        captured_agents: u64,
        terminated_agents: u64,
    },
    CompetitorStateChanged {
        territory: String,
        from: CompetitorState,
        to: CompetitorState,
        reason: String,
    },
    MilestoneEvaluationStarted {
        territory: String,
        milestone: String,
    },
    MilestonePassed {
        territory: String,
        milestone: String,
        points: u64,
        evidence: serde_json::Value,
    },
    MilestoneFailed {
        territory: String,
        milestone: String,
        category: String,
        detail: String,
        retryable: bool,
    },
    MilestoneRevoked {
        territory: String,
        milestone: String,
        reason: String,
    },
    DurableDeploymentCompleted {
        territory: String,
        elapsed_ms: u64,
    },
    ResourcesChanged {
        territory: String,
        delta: i64,
        remaining: u64,
        reason: String,
    },
    TerritoryEliminated {
        territory: String,
        source: FailureSource,
        detail: String,
    },
    InfrastructureFailure {
        component: String,
        source: FailureSource,
        detail: String,
    },
    MatchFinished {
        winner: Option<String>,
        reason: String,
    },
}

/// Sequence and time metadata surrounding an event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub schema_version: u32,
    pub sequence: u64,
    pub elapsed_ms: u64,
    #[serde(flatten)]
    pub event: Event,
}
