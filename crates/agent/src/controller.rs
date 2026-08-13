use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::{StreamExt, stream::FuturesUnordered};
use thiserror::Error;
use tokio::sync::mpsc;

use crate::{AgentInvocation, AgentResult, AgentStatus};

#[derive(Debug, Error)]
pub enum AgentControllerError {
    #[error("adapter failed: {0}")]
    Adapter(String),
}

/// A harness adapter that owns deadline enforcement and partial-result
/// recovery for one invocation.
#[async_trait]
pub trait AgentAdapter: Send + Sync {
    async fn run(
        &self,
        invocation: AgentInvocation,
        timeout: Duration,
    ) -> Result<AgentResult, AgentControllerError>;
}

/// Adapter registry and concurrent dispatcher.
#[derive(Default)]
pub struct AgentController {
    adapters: HashMap<String, Arc<dyn AgentAdapter>>,
}

impl AgentController {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, name: impl Into<String>, adapter: Arc<dyn AgentAdapter>) {
        self.adapters.insert(name.into(), adapter);
    }

    /// Run every invocation concurrently and always return one result per
    /// configured agent.
    pub async fn run_all(
        &self,
        invocations: Vec<AgentInvocation>,
        timeout: Duration,
    ) -> Vec<AgentResult> {
        let mut pending = self.pending(invocations, timeout);
        let mut results = Vec::new();
        while let Some(result) = pending.next().await {
            results.push(result);
        }
        results
    }

    /// Run invocations concurrently and publish each result as soon as that
    /// individual adapter finishes.
    pub async fn run_stream(
        &self,
        invocations: Vec<AgentInvocation>,
        timeout: Duration,
        sender: mpsc::Sender<AgentResult>,
    ) {
        let mut pending = self.pending(invocations, timeout);
        while let Some(result) = pending.next().await {
            if sender.send(result).await.is_err() {
                break;
            }
        }
    }

    fn pending(
        &self,
        invocations: Vec<AgentInvocation>,
        timeout: Duration,
    ) -> FuturesUnordered<impl Future<Output = AgentResult> + '_> {
        invocations
            .into_iter()
            .map(|invocation| async move {
                let Some(adapter) = self.adapters.get(&invocation.config.adapter) else {
                    return AgentResult::controller_result(
                        &invocation,
                        AgentStatus::HarnessError,
                        format!("unknown adapter {}", invocation.config.adapter),
                    );
                };
                let mut result = match adapter.run(invocation.clone(), timeout).await {
                    Ok(result) => result,
                    Err(error) => AgentResult::controller_result(
                        &invocation,
                        AgentStatus::HarnessError,
                        error.to_string(),
                    ),
                };
                apply_budget(&invocation, &mut result);
                result
            })
            .collect()
    }
}

fn apply_budget(invocation: &AgentInvocation, result: &mut AgentResult) {
    let resources_exceeded = result.usage.resource_units > invocation.config.budget.resource_units;
    let cost_exceeded = invocation
        .config
        .budget
        .max_cost_microusd
        .zip(result.usage.cost_microusd)
        .is_some_and(|(limit, actual)| actual > limit);
    if resources_exceeded || cost_exceeded {
        result.status = AgentStatus::BudgetExceeded;
        result.summary = "agent exceeded its configured budget".into();
    }
}
