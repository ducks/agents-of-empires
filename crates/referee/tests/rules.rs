use aoe_domain::{ArenaManifest, Event, FailureSource, MatchState, TerritoryState};
use aoe_referee::{HealthObservation, Referee};

const MANIFEST: &str = include_str!("../../runtime/tests/fixture.toml");

fn observation(healthy: bool) -> HealthObservation {
    HealthObservation {
        healthy,
        status: Some(if healthy { 200 } else { 502 }),
        latency_ms: 2,
        detail: if healthy { "ok" } else { "bad gateway" }.into(),
    }
}

fn referee() -> Referee {
    let manifest = ArenaManifest::parse(MANIFEST).expect("manifest");
    let mut referee = Referee::from_manifest(&manifest);
    referee.start().expect("start");
    referee
}

#[test]
fn transient_failure_does_not_degrade() {
    let mut referee = referee();
    referee
        .observe("gate", observation(false), 1_000)
        .expect("observe");
    referee
        .observe("gate", observation(true), 2_000)
        .expect("observe");
    assert_eq!(
        referee.territory_state("gate"),
        Some(TerritoryState::Healthy)
    );
}

#[test]
fn sustained_failure_requires_durable_recovery() {
    let mut referee = referee();
    for time in [1_000, 2_000, 3_000] {
        referee
            .observe("gate", observation(false), time)
            .expect("observe");
    }
    assert_eq!(
        referee.territory_state("gate"),
        Some(TerritoryState::Degraded)
    );
    referee
        .observe("gate", observation(true), 4_000)
        .expect("recovering");
    assert_eq!(
        referee.territory_state("gate"),
        Some(TerritoryState::Recovering)
    );
    referee
        .observe("gate", observation(false), 5_000)
        .expect("regression");
    assert_eq!(
        referee.territory_state("gate"),
        Some(TerritoryState::Degraded)
    );
    for time in [6_000, 7_000, 8_000] {
        referee
            .observe("gate", observation(true), time)
            .expect("durable recovery");
    }
    assert_eq!(
        referee.territory_state("gate"),
        Some(TerritoryState::Healthy)
    );
}

#[test]
fn expired_recovery_window_eliminates_and_finishes_match() {
    let mut referee = referee();
    for time in [1_000, 2_000, 3_000] {
        referee
            .observe("gate", observation(false), time)
            .expect("observe");
    }
    let events = referee.tick(63_000).expect("deadline tick");
    assert_eq!(
        referee.territory_state("gate"),
        Some(TerritoryState::Eliminated)
    );
    assert_eq!(
        referee.outcome().expect("outcome").winner.as_deref(),
        Some("archive")
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event.event, Event::TerritoryEliminated { .. }))
    );
}

#[test]
fn healthy_ticks_generate_resources_and_charges_spend_them() {
    let mut referee = referee();
    let generated = referee.tick(1_000).expect("tick");
    assert!(generated.iter().any(|event| matches!(
        event.event,
        Event::ResourcesChanged {
            ref territory,
            delta: 1,
            remaining: 11,
            ..
        } if territory == "gate"
    )));
    let charged = referee.charge("gate", 4, 1_000).expect("charge");
    assert!(matches!(
        charged.event,
        Event::ResourcesChanged {
            delta: -4,
            remaining: 7,
            ..
        }
    ));
}

#[test]
fn provider_failure_is_not_a_player_loss() {
    let mut referee = referee();
    let event = referee
        .infrastructure_failure("agent-gate", FailureSource::Provider, "HTTP 429", 1_000)
        .expect("provider failure");
    assert!(matches!(
        event.event,
        Event::InfrastructureFailure {
            source: FailureSource::Provider,
            ..
        }
    ));
    assert_eq!(
        referee.territory_state("gate"),
        Some(TerritoryState::Healthy)
    );
}

#[test]
fn controller_events_share_the_referee_sequence() {
    let manifest = ArenaManifest::parse(MANIFEST).expect("manifest");
    let mut referee = Referee::from_manifest(&manifest);
    let started = referee.start().expect("start");
    let next = referee
        .record(
            Event::AgentStarted {
                agent: "agent-gate".into(),
                territory: "gate".into(),
                model: "test".into(),
            },
            5,
        )
        .expect("record");
    assert_eq!(next.sequence, started.len() as u64);

    let aborted = referee.abort("operator interrupt", 10).expect("abort");
    assert_eq!(aborted[0].sequence, next.sequence + 1);
    assert!(matches!(
        aborted[0].event,
        Event::MatchStateChanged {
            to: MatchState::Aborted,
            ..
        }
    ));
}
