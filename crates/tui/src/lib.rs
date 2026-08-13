use std::fmt::Write;

use aoe_domain::{CompetitorState, Event, EventEnvelope, MatchState, TerritoryState};
use aoe_replay::WorldState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderOptions {
    pub width: usize,
    pub color: bool,
    pub event_limit: usize,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            width: 100,
            color: true,
            event_limit: 8,
        }
    }
}

/// Render the same reduced state used by both live matches and replay mode.
#[must_use]
pub fn render_world(
    state: &WorldState,
    events: &[EventEnvelope],
    options: RenderOptions,
) -> String {
    let mut output = String::new();
    let clock = format_clock(state.elapsed_ms);
    let _ = writeln!(
        output,
        "AGENTS OF EMPIRES  match={}  clock={clock}",
        match_label(state.match_state)
    );
    let _ = writeln!(output, "{}", "=".repeat(options.width.clamp(20, 100)));

    if options.width < 60 {
        render_compact_territories(&mut output, state, options.color);
    } else {
        render_territory_table(&mut output, state, options.color);
    }

    let _ = writeln!(output, "\nEVENTS");
    let start = events.len().saturating_sub(options.event_limit);
    for envelope in &events[start..] {
        let _ = writeln!(
            output,
            "{:>6}  #{:<4} {}",
            format_clock(envelope.elapsed_ms),
            envelope.sequence,
            event_summary(&envelope.event)
        );
    }
    if events.is_empty() {
        let _ = writeln!(output, "(no events)");
    }

    if state.match_state == MatchState::Finished {
        let winner = state.winner.as_deref().unwrap_or("none");
        let reason = state.finish_reason.as_deref().unwrap_or("unknown");
        let _ = writeln!(output, "\nWINNER: {winner} ({reason})");
    }
    output
}

fn render_compact_territories(output: &mut String, state: &WorldState, color: bool) {
    for (id, territory) in &state.territories {
        if let Some(competitor) = territory.competitor_state {
            let passed = territory
                .milestones
                .values()
                .filter(|milestone| milestone.passed)
                .count();
            let _ = writeln!(
                output,
                "{} {id} [{}]: milestones={passed}, points={}",
                competitor_marker(competitor, color),
                competitor_label(competitor),
                territory.milestone_points,
            );
            continue;
        }
        let marker = state_marker(territory.state, color);
        let health = territory
            .last_health
            .as_ref()
            .map_or(
                "unobserved",
                |value| if value.healthy { "up" } else { "down" },
            );
        let _ = writeln!(
            output,
            "{marker} {id} [{}]: {health}, resources={}",
            territory.class.as_deref().unwrap_or("unknown"),
            territory.resources,
        );
    }
}

fn render_territory_table(output: &mut String, state: &WorldState, color: bool) {
    if state
        .territories
        .values()
        .any(|territory| territory.competitor_state.is_some())
    {
        render_build_table(output, state, color);
        return;
    }
    let _ = writeln!(
        output,
        "{:<3} {:<16} {:<12} {:<12} {:<10} {:>9}",
        "", "territory", "class", "state", "health", "resources"
    );
    for (id, territory) in &state.territories {
        let marker = state_marker(territory.state, color);
        let health = territory.last_health.as_ref().map_or_else(
            || "unobserved".to_owned(),
            |value| match value.status {
                Some(status) => format!("{} {status}", if value.healthy { "up" } else { "down" }),
                None => if value.healthy { "up" } else { "down" }.to_owned(),
            },
        );
        let _ = writeln!(
            output,
            "{marker:<3} {id:<16} {:<12} {:<12} {health:<10} {:>9}",
            territory.class.as_deref().unwrap_or("unknown"),
            state_label(territory.state),
            territory.resources
        );
    }
}

fn render_build_table(output: &mut String, state: &WorldState, color: bool) {
    let _ = writeln!(
        output,
        "{:<3} {:<16} {:<12} {:<12} {:>10} {:>8}",
        "", "territory", "class", "state", "milestones", "points"
    );
    for (id, territory) in &state.territories {
        let competitor = territory
            .competitor_state
            .unwrap_or(CompetitorState::Preparing);
        let passed = territory
            .milestones
            .values()
            .filter(|milestone| milestone.passed)
            .count();
        let total = territory.milestones.len();
        let _ = writeln!(
            output,
            "{:<3} {id:<16} {:<12} {:<12} {:>5}/{:<4} {:>8}",
            competitor_marker(competitor, color),
            territory.class.as_deref().unwrap_or("unknown"),
            competitor_label(competitor),
            passed,
            total,
            territory.milestone_points,
        );
    }
}

fn competitor_marker(state: CompetitorState, color: bool) -> &'static str {
    match (state, color) {
        (CompetitorState::Durable, true) => "\x1b[32m[+]\x1b[0m",
        (CompetitorState::Incomplete, true) => "\x1b[31m[x]\x1b[0m",
        (CompetitorState::Unavailable, true) => "\x1b[90m[?]\x1b[0m",
        (CompetitorState::Verifying, true) => "\x1b[33m[~]\x1b[0m",
        (CompetitorState::Durable, false) => "[+]",
        (CompetitorState::Incomplete, false) => "[x]",
        (CompetitorState::Unavailable, false) => "[?]",
        (CompetitorState::Verifying, false) => "[~]",
        (CompetitorState::Preparing | CompetitorState::Building, _) => "[>]",
    }
}

fn competitor_label(state: CompetitorState) -> &'static str {
    match state {
        CompetitorState::Preparing => "preparing",
        CompetitorState::Building => "building",
        CompetitorState::Verifying => "verifying",
        CompetitorState::Durable => "durable",
        CompetitorState::Incomplete => "incomplete",
        CompetitorState::Unavailable => "unavailable",
    }
}

fn state_marker(state: TerritoryState, color: bool) -> &'static str {
    match (state, color) {
        (TerritoryState::Healthy, true) => "\x1b[32m[+]\x1b[0m",
        (TerritoryState::Degraded | TerritoryState::Recovering, true) => "\x1b[33m[!]\x1b[0m",
        (TerritoryState::Eliminated, true) => "\x1b[90m[x]\x1b[0m",
        (TerritoryState::Healthy, false) => "[+]",
        (TerritoryState::Degraded, false) => "[!]",
        (TerritoryState::Recovering, false) => "[~]",
        (TerritoryState::Eliminated, false) => "[x]",
        (TerritoryState::Provisioning, _) => "[?]",
    }
}

fn state_label(state: TerritoryState) -> &'static str {
    match state {
        TerritoryState::Provisioning => "provisioning",
        TerritoryState::Healthy => "healthy",
        TerritoryState::Degraded => "degraded",
        TerritoryState::Recovering => "recovering",
        TerritoryState::Eliminated => "eliminated",
    }
}

fn match_label(state: MatchState) -> &'static str {
    match state {
        MatchState::Preparing => "preparing",
        MatchState::Running => "running",
        MatchState::Finished => "finished",
        MatchState::Aborted => "aborted",
    }
}

fn format_clock(elapsed_ms: u64) -> String {
    let seconds = elapsed_ms / 1000;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

#[must_use]
pub fn event_summary(event: &Event) -> String {
    match event {
        Event::TerritoryRegistered {
            territory,
            class,
            agent,
        } => format!("{territory} registered as {class}, owned by {agent}"),
        Event::MatchStateChanged { from, to } => format!("match {from:?} -> {to:?}"),
        Event::TerritoryStateChanged {
            territory,
            from,
            to,
            reason,
        } => format!("{territory} {from:?} -> {to:?}: {reason}"),
        Event::HealthObserved {
            territory,
            healthy,
            status,
            ..
        } => format!(
            "{territory} health={} status={}",
            if *healthy { "up" } else { "down" },
            status.map_or_else(|| "transport".to_owned(), |value| value.to_string())
        ),
        Event::AgentStarted {
            agent, territory, ..
        } => format!("{agent} started in {territory}"),
        Event::AgentFinished {
            agent,
            success,
            detail,
            ..
        } => format!("{agent} finished success={success}: {detail}"),
        Event::AgentInterrupted {
            agent,
            source,
            detail,
        } => format!("{agent} interrupted by {source:?}: {detail}"),
        Event::AgentTerminated { agent, reason } => format!("{agent} terminated: {reason}"),
        Event::UsageCharged {
            agent,
            resource_units,
            ..
        } => format!("{agent} used {resource_units} resources"),
        Event::PostMatchDrainStarted {
            timeout_ms,
            pending_agents,
        } => format!("post-match drain started for {pending_agents} agents ({timeout_ms}ms limit)"),
        Event::PostMatchDrainFinished {
            captured_agents,
            terminated_agents,
        } => format!("post-match drain captured {captured_agents}, terminated {terminated_agents}"),
        Event::CompetitorStateChanged {
            territory,
            from,
            to,
            reason,
        } => format!("{territory} {from:?} -> {to:?}: {reason}"),
        Event::MilestoneEvaluationStarted {
            territory,
            milestone,
        } => format!("{territory} verifying {milestone}"),
        Event::MilestonePassed {
            territory,
            milestone,
            points,
            ..
        } => format!("{territory} passed {milestone} (+{points})"),
        Event::MilestoneFailed {
            territory,
            milestone,
            category,
            ..
        } => format!("{territory} failed {milestone}: {category}"),
        Event::MilestoneRevoked {
            territory,
            milestone,
            reason,
        } => format!("{territory} lost {milestone}: {reason}"),
        Event::DurableDeploymentCompleted {
            territory,
            elapsed_ms,
        } => format!("{territory} completed a durable deployment at {elapsed_ms}ms"),
        Event::ResourcesChanged {
            territory,
            remaining,
            reason,
            ..
        } => format!("{territory} resources={remaining}: {reason}"),
        Event::TerritoryEliminated {
            territory, detail, ..
        } => format!("{territory} eliminated: {detail}"),
        Event::InfrastructureFailure {
            component, detail, ..
        } => format!("infrastructure failure in {component}: {detail}"),
        Event::MatchFinished { winner, reason } => format!(
            "match finished winner={}: {reason}",
            winner.as_deref().unwrap_or("none")
        ),
    }
}
