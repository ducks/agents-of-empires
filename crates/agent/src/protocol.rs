use std::path::PathBuf;

use aoe_domain::AgentConfig;
use serde::{Deserialize, Serialize};

/// Complete controller-owned input to one agent harness.
#[derive(Debug, Clone)]
pub struct AgentInvocation {
    pub config: AgentConfig,
    pub territory_host: String,
    pub ssh_port: u16,
    pub instruction: String,
    pub credential_file: Option<PathBuf>,
}

/// Normalized terminal status from any harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Completed,
    Failed,
    Unavailable,
    TimedOut,
    Interrupted,
    BudgetExceeded,
    HarnessError,
}

/// Comparable usage reported by the harness when available.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentUsage {
    pub rounds: Option<u64>,
    pub tool_calls: Option<u64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost_microusd: Option<u64>,
    pub resource_units: u64,
}

/// Stable result schema written by adapters and consumed by the controller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentResult {
    pub schema_version: u32,
    pub agent: String,
    pub territory: String,
    pub status: AgentStatus,
    pub summary: String,
    #[serde(default)]
    pub usage: AgentUsage,
    pub transcript: Option<PathBuf>,
}

impl AgentResult {
    #[must_use]
    pub fn controller_result(
        invocation: &AgentInvocation,
        status: AgentStatus,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: 1,
            agent: invocation.config.id.clone(),
            territory: invocation.config.territory.clone(),
            status,
            summary: summary.into(),
            usage: AgentUsage::default(),
            transcript: None,
        }
    }
}
