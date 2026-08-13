use std::collections::BTreeMap;

use aoe_domain::{
    CompetitorState, Event, EventEnvelope, FailureSource, MatchState, TerritoryState,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthView {
    pub healthy: bool,
    pub status: Option<u16>,
    pub latency_ms: u64,
    pub detail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentView {
    pub territory: String,
    pub model: String,
    pub running: bool,
    pub successful: Option<bool>,
    pub failure_source: Option<FailureSource>,
    pub resource_units_used: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_microusd: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerritoryView {
    #[serde(default)]
    pub class: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    pub state: TerritoryState,
    pub resources: u64,
    pub last_health: Option<HealthView>,
    pub elimination_source: Option<FailureSource>,
    pub elimination_detail: Option<String>,
    #[serde(default)]
    pub competitor_state: Option<CompetitorState>,
    #[serde(default)]
    pub milestones: BTreeMap<String, MilestoneView>,
    #[serde(default)]
    pub milestone_points: u64,
    #[serde(default)]
    pub durable_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MilestoneView {
    pub evaluating: bool,
    pub passed: bool,
    pub points: u64,
    pub evidence: Option<serde_json::Value>,
    pub failure_category: Option<String>,
    pub failure_detail: Option<String>,
}

impl Default for TerritoryView {
    fn default() -> Self {
        Self {
            class: None,
            agent: None,
            state: TerritoryState::Provisioning,
            resources: 0,
            last_health: None,
            elimination_source: None,
            elimination_detail: None,
            competitor_state: None,
            milestones: BTreeMap::new(),
            milestone_points: 0,
            durable_at_ms: None,
        }
    }
}

/// Complete derived state at one event sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldState {
    pub match_state: MatchState,
    pub territories: BTreeMap<String, TerritoryView>,
    pub agents: BTreeMap<String, AgentView>,
    pub winner: Option<String>,
    pub finish_reason: Option<String>,
    pub infrastructure_failures: u64,
    pub last_sequence: Option<u64>,
    pub elapsed_ms: u64,
}

impl Default for WorldState {
    fn default() -> Self {
        Self {
            match_state: MatchState::Preparing,
            territories: BTreeMap::new(),
            agents: BTreeMap::new(),
            winner: None,
            finish_reason: None,
            infrastructure_failures: 0,
            last_sequence: None,
            elapsed_ms: 0,
        }
    }
}

/// Apply one event to derived state.
pub fn reduce(state: &mut WorldState, envelope: &EventEnvelope) {
    state.last_sequence = Some(envelope.sequence);
    state.elapsed_ms = envelope.elapsed_ms;
    match &envelope.event {
        Event::TerritoryRegistered {
            territory,
            class,
            agent,
        } => register_territory(state, territory, class, agent),
        Event::MatchStateChanged { to, .. } => state.match_state = *to,
        Event::TerritoryStateChanged { territory, to, .. } => {
            state
                .territories
                .entry(territory.clone())
                .or_default()
                .state = *to;
        }
        Event::HealthObserved {
            territory,
            healthy,
            status,
            latency_ms,
            detail,
        } => {
            apply_health(state, territory, *healthy, *status, *latency_ms, detail);
        }
        Event::AgentStarted {
            agent,
            territory,
            model,
        } => {
            state.agents.insert(
                agent.clone(),
                AgentView {
                    territory: territory.clone(),
                    model: model.clone(),
                    running: true,
                    ..AgentView::default()
                },
            );
        }
        Event::AgentFinished {
            agent,
            source,
            success,
            ..
        } => {
            let view = state.agents.entry(agent.clone()).or_default();
            view.running = false;
            view.successful = Some(*success);
            view.failure_source = Some(*source);
        }
        Event::UsageCharged {
            agent,
            resource_units,
            input_tokens,
            output_tokens,
            cost_microusd,
        } => {
            apply_usage(
                state,
                agent,
                *resource_units,
                input_tokens.unwrap_or(0),
                output_tokens.unwrap_or(0),
                cost_microusd.unwrap_or(0),
            );
        }
        Event::PostMatchDrainStarted { .. } | Event::PostMatchDrainFinished { .. } => {}
        Event::CompetitorStateChanged { territory, to, .. } => {
            state
                .territories
                .entry(territory.clone())
                .or_default()
                .competitor_state = Some(*to);
        }
        Event::MilestoneEvaluationStarted {
            territory,
            milestone,
        } => {
            state
                .territories
                .entry(territory.clone())
                .or_default()
                .milestones
                .entry(milestone.clone())
                .or_default()
                .evaluating = true;
        }
        Event::MilestonePassed {
            territory,
            milestone,
            points,
            evidence,
        } => {
            let territory = state.territories.entry(territory.clone()).or_default();
            let milestone = territory.milestones.entry(milestone.clone()).or_default();
            if !milestone.passed {
                territory.milestone_points = territory.milestone_points.saturating_add(*points);
            }
            milestone.evaluating = false;
            milestone.passed = true;
            milestone.points = *points;
            milestone.evidence = Some(evidence.clone());
            milestone.failure_category = None;
            milestone.failure_detail = None;
        }
        Event::MilestoneFailed {
            territory,
            milestone,
            category,
            detail,
            ..
        } => {
            let milestone = state
                .territories
                .entry(territory.clone())
                .or_default()
                .milestones
                .entry(milestone.clone())
                .or_default();
            milestone.evaluating = false;
            milestone.failure_category = Some(category.clone());
            milestone.failure_detail = Some(detail.clone());
        }
        Event::MilestoneRevoked {
            territory,
            milestone,
            ..
        } => {
            let territory = state.territories.entry(territory.clone()).or_default();
            let milestone = territory.milestones.entry(milestone.clone()).or_default();
            if milestone.passed {
                territory.milestone_points =
                    territory.milestone_points.saturating_sub(milestone.points);
            }
            milestone.evaluating = false;
            milestone.passed = false;
            milestone.evidence = None;
        }
        Event::DurableDeploymentCompleted {
            territory,
            elapsed_ms,
        } => {
            let territory = state.territories.entry(territory.clone()).or_default();
            territory.competitor_state = Some(CompetitorState::Durable);
            territory.durable_at_ms = Some(*elapsed_ms);
        }
        Event::ResourcesChanged {
            territory,
            remaining,
            ..
        } => {
            state
                .territories
                .entry(territory.clone())
                .or_default()
                .resources = *remaining;
        }
        Event::TerritoryEliminated {
            territory,
            source,
            detail,
        } => {
            let view = state.territories.entry(territory.clone()).or_default();
            view.state = TerritoryState::Eliminated;
            view.elimination_source = Some(*source);
            view.elimination_detail = Some(detail.clone());
        }
        Event::InfrastructureFailure { .. } => {
            state.infrastructure_failures = state.infrastructure_failures.saturating_add(1);
        }
        Event::MatchFinished { winner, reason } => {
            if state.match_state != MatchState::Aborted {
                state.match_state = MatchState::Finished;
            }
            state.winner.clone_from(winner);
            state.finish_reason = Some(reason.clone());
        }
    }
}

fn register_territory(state: &mut WorldState, territory: &str, class: &str, agent: &str) {
    let view = state.territories.entry(territory.to_owned()).or_default();
    view.class = Some(class.to_owned());
    view.agent = Some(agent.to_owned());
}

fn apply_health(
    state: &mut WorldState,
    territory: &str,
    healthy: bool,
    status: Option<u16>,
    latency_ms: u64,
    detail: &str,
) {
    state
        .territories
        .entry(territory.to_owned())
        .or_default()
        .last_health = Some(HealthView {
        healthy,
        status,
        latency_ms,
        detail: detail.to_owned(),
    });
}

fn apply_usage(
    state: &mut WorldState,
    agent: &str,
    resource_units: u64,
    input_tokens: u64,
    output_tokens: u64,
    cost_microusd: u64,
) {
    let view = state.agents.entry(agent.to_owned()).or_default();
    view.resource_units_used = view.resource_units_used.saturating_add(resource_units);
    view.input_tokens = view.input_tokens.saturating_add(input_tokens);
    view.output_tokens = view.output_tokens.saturating_add(output_tokens);
    view.cost_microusd = view.cost_microusd.saturating_add(cost_microusd);
}

/// Reduce a complete ordered event sequence.
#[must_use]
pub fn replay<'a>(events: impl IntoIterator<Item = &'a EventEnvelope>) -> WorldState {
    let mut state = WorldState::default();
    for event in events {
        reduce(&mut state, event);
    }
    state
}
