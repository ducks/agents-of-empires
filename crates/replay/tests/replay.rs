use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use aoe_domain::{
    AgentTerminalState, CompetitorState, Event, EventEnvelope, FailureSource, MatchState,
    TerritoryState,
};
use aoe_replay::{
    EventLog, EventLogError, Snapshot, WorldState, load_events, load_snapshot, reduce, replay,
    write_snapshot,
};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn temp_root() -> PathBuf {
    std::env::temp_dir().join(format!(
        "agents-of-empires-replay-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ))
}

fn event(sequence: u64, elapsed_ms: u64, event: Event) -> EventEnvelope {
    EventEnvelope {
        schema_version: 1,
        sequence,
        elapsed_ms,
        event,
    }
}

fn fixture_events() -> Vec<EventEnvelope> {
    vec![
        event(
            0,
            0,
            Event::MatchStateChanged {
                from: MatchState::Preparing,
                to: MatchState::Running,
            },
        ),
        event(
            1,
            0,
            Event::TerritoryStateChanged {
                territory: "gate".into(),
                from: TerritoryState::Provisioning,
                to: TerritoryState::Healthy,
                reason: "preflight".into(),
            },
        ),
        event(
            2,
            100,
            Event::ResourcesChanged {
                territory: "gate".into(),
                delta: 1,
                remaining: 11,
                reason: "healthy tick".into(),
            },
        ),
        event(
            3,
            200,
            Event::TerritoryEliminated {
                territory: "gate".into(),
                source: FailureSource::Player,
                detail: "offline".into(),
            },
        ),
        event(
            4,
            200,
            Event::MatchFinished {
                winner: Some("archive".into()),
                reason: "last territory standing".into(),
            },
        ),
    ]
}

#[test]
fn live_reduction_matches_disk_replay() {
    let root = temp_root();
    let path = root.join("events.jsonl");
    let events = fixture_events();
    let mut live = WorldState::default();
    let mut log = EventLog::open(&path).expect("open");
    for event in &events {
        reduce(&mut live, event);
        log.append(event).expect("append");
    }
    drop(log);
    let loaded = load_events(&path).expect("load");
    assert_eq!(live, replay(&loaded));
    assert_eq!(live.winner.as_deref(), Some("archive"));
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn ignores_truncated_final_append() {
    let root = temp_root();
    let path = root.join("events.jsonl");
    let first = serde_json::to_string(&fixture_events()[0]).expect("json");
    fs::create_dir_all(&root).expect("root");
    fs::write(&path, format!("{first}\n{{\"schema_version\":")).expect("write");
    let loaded = load_events(&path).expect("recover");
    assert_eq!(loaded.len(), 1);
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn rejects_missing_sequence() {
    let root = temp_root();
    let path = root.join("events.jsonl");
    fs::create_dir_all(&root).expect("root");
    let wrong = event(
        2,
        0,
        Event::InfrastructureFailure {
            component: "vm".into(),
            source: FailureSource::Arena,
            detail: "failed".into(),
        },
    );
    fs::write(
        &path,
        format!("{}\n", serde_json::to_string(&wrong).expect("json")),
    )
    .expect("write");
    assert!(matches!(
        load_events(&path),
        Err(EventLogError::Sequence {
            expected: 0,
            actual: 2
        })
    ));
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn snapshot_round_trip_preserves_reduced_state() {
    let root = temp_root();
    let path = root.join("snapshot.json");
    let state = replay(&fixture_events());
    let snapshot = Snapshot {
        schema_version: 1,
        through_sequence: 4,
        state,
    };
    write_snapshot(&path, &snapshot).expect("write snapshot");
    assert_eq!(load_snapshot(&path).expect("load snapshot"), snapshot);
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn milestone_evidence_reduces_and_revokes() {
    let passed = event(
        0,
        100,
        Event::MilestonePassed {
            territory: "builder".into(),
            milestone: "write-read".into(),
            points: 20,
            evidence: serde_json::json!({"record": "opaque-17"}),
        },
    );
    let durable = event(
        1,
        200,
        Event::DurableDeploymentCompleted {
            territory: "builder".into(),
            elapsed_ms: 200,
        },
    );
    let revoked = event(
        2,
        300,
        Event::MilestoneRevoked {
            territory: "builder".into(),
            milestone: "write-read".into(),
            reason: "record disappeared after reboot".into(),
        },
    );
    let state = replay([&passed, &durable, &revoked]);
    let builder = state.territories.get("builder").expect("builder");
    assert_eq!(builder.competitor_state, Some(CompetitorState::Durable));
    assert_eq!(builder.durable_at_ms, Some(200));
    assert_eq!(builder.milestone_points, 0);
    assert!(!builder.milestones["write-read"].passed);
}

#[test]
fn interrupted_and_terminated_agents_are_terminal_without_becoming_losses() {
    let events = [
        event(
            0,
            0,
            Event::AgentStarted {
                agent: "winner".into(),
                territory: "one".into(),
                model: "model-a".into(),
            },
        ),
        event(
            1,
            10,
            Event::AgentInterrupted {
                agent: "winner".into(),
                source: FailureSource::Arena,
                detail: "referee reboot".into(),
            },
        ),
        event(
            2,
            10,
            Event::AgentStarted {
                agent: "loser".into(),
                territory: "two".into(),
                model: "model-b".into(),
            },
        ),
        event(
            3,
            10,
            Event::AgentTerminated {
                agent: "loser".into(),
                reason: "drain expired".into(),
            },
        ),
    ];
    let state = replay(&events);
    assert_eq!(
        state.agents["winner"].terminal_state,
        Some(AgentTerminalState::Interrupted)
    );
    assert_eq!(state.agents["winner"].successful, None);
    assert_eq!(
        state.agents["loser"].terminal_state,
        Some(AgentTerminalState::Terminated)
    );
    assert!(state.agents.values().all(|agent| !agent.running));
}
