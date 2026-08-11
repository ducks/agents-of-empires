use std::sync::Arc;
use std::time::{Duration, Instant};

use aoe_agent::{
    AgentAdapter, AgentController, AgentControllerError, AgentInvocation, AgentResult, AgentStatus,
    AgentUsage,
};
use aoe_domain::{AgentConfig, Budget};
use async_trait::async_trait;

struct FakeAdapter;

#[async_trait]
impl AgentAdapter for FakeAdapter {
    async fn run(
        &self,
        invocation: AgentInvocation,
        _timeout: Duration,
    ) -> Result<AgentResult, AgentControllerError> {
        tokio::time::sleep(Duration::from_millis(50)).await;
        Ok(AgentResult {
            schema_version: 1,
            agent: invocation.config.id,
            territory: invocation.config.territory,
            status: AgentStatus::Completed,
            summary: "done".into(),
            usage: AgentUsage {
                resource_units: 2,
                ..AgentUsage::default()
            },
            transcript: None,
        })
    }
}

fn invocation(id: &str, territory: &str, budget: u64) -> AgentInvocation {
    AgentInvocation {
        config: AgentConfig {
            id: id.into(),
            territory: territory.into(),
            adapter: "fake".into(),
            model: "fake/model".into(),
            reasoning_effort: "default".into(),
            budget: Budget {
                resource_units: budget,
                max_cost_microusd: None,
            },
        },
        territory_host: "127.0.0.1".into(),
        ssh_port: 22000,
        instruction: "survive".into(),
        credential_file: None,
    }
}

#[tokio::test]
async fn runs_agents_concurrently() {
    let mut controller = AgentController::new();
    controller.register("fake", Arc::new(FakeAdapter));
    let start = Instant::now();
    let results = controller
        .run_all(
            vec![
                invocation("one", "gate", 10),
                invocation("two", "archive", 10),
            ],
            Duration::from_secs(1),
        )
        .await;
    assert!(start.elapsed() < Duration::from_millis(90));
    assert!(
        results
            .iter()
            .all(|result| result.status == AgentStatus::Completed)
    );
}

#[tokio::test]
async fn enforces_abstract_budget() {
    let mut controller = AgentController::new();
    controller.register("fake", Arc::new(FakeAdapter));
    let result = controller
        .run_all(vec![invocation("one", "gate", 1)], Duration::from_secs(1))
        .await
        .remove(0);
    assert_eq!(result.status, AgentStatus::BudgetExceeded);
}

#[tokio::test]
async fn unknown_adapter_is_a_harness_error() {
    let mut invocation = invocation("one", "gate", 10);
    invocation.config.adapter = "missing".into();
    let result = AgentController::new()
        .run_all(vec![invocation], Duration::from_secs(1))
        .await
        .remove(0);
    assert_eq!(result.status, AgentStatus::HarnessError);
}
