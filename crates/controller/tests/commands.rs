use std::fs;

use aoe_controller::{inspect, replay_log};
use aoe_domain::{ArenaManifest, Event, EventEnvelope, MatchState};
use aoe_referee::Referee;
use aoe_replay::{EventLog, load_events, replay};
use aoe_tui::RenderOptions;

const MANIFEST: &str = include_str!("../../runtime/tests/fixture.toml");

fn temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("aoe-controller-{}-{name}", std::process::id()))
}

#[test]
fn live_and_replay_reach_identical_state() {
    let path = temp_path("events.jsonl");
    let _ = fs::remove_file(&path);
    let events = [
        EventEnvelope {
            schema_version: 1,
            sequence: 0,
            elapsed_ms: 0,
            event: Event::MatchStateChanged {
                from: MatchState::Preparing,
                to: MatchState::Running,
            },
        },
        EventEnvelope {
            schema_version: 1,
            sequence: 1,
            elapsed_ms: 10,
            event: Event::MatchFinished {
                winner: Some("gatekeeper".into()),
                reason: "test".into(),
            },
        },
    ];
    let mut log = EventLog::open(&path).expect("log");
    for event in &events {
        log.append(event).expect("append");
    }
    drop(log);

    assert_eq!(replay(&events), replay(&load_events(&path).expect("load")));
    let rendered = replay_log(
        &path,
        false,
        RenderOptions {
            color: false,
            ..RenderOptions::default()
        },
    )
    .expect("replay command");
    assert!(rendered.contains("WINNER: gatekeeper"));
    assert!(
        inspect(&path, 1, false)
            .expect("inspect")
            .contains("winner=gatekeeper")
    );
    fs::remove_file(path).expect("cleanup");
}

#[test]
fn interrupted_match_log_remains_inspectable() {
    let path = temp_path("interrupted.jsonl");
    let _ = fs::remove_file(&path);
    let manifest = ArenaManifest::parse(MANIFEST).expect("manifest");
    let mut referee = Referee::from_manifest(&manifest);
    let mut events = referee.start().expect("start");
    events.extend(referee.abort("operator interrupt", 50).expect("abort"));
    let mut log = EventLog::open(&path).expect("log");
    for event in &events {
        log.append(event).expect("append");
    }
    drop(log);

    let loaded = load_events(&path).expect("load");
    let state = replay(&loaded);
    assert_eq!(state.match_state, MatchState::Aborted);
    assert_eq!(state.finish_reason.as_deref(), Some("operator interrupt"));
    assert!(
        inspect(&path, 4, false)
            .expect("inspect")
            .contains("operator interrupt")
    );
    fs::remove_file(path).expect("cleanup");
}
