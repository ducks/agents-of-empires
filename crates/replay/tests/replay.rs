use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use aoe_domain::{Event, EventEnvelope, FailureSource, MatchState, TerritoryState};
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
