use std::fs;

use aoe_controller::generate_reports;
use aoe_domain::{
    AgentTerminalState, CompetitorState, Event, EventEnvelope, FailureSource, MatchState,
    TerritoryState,
};
use aoe_replay::{AgentView, MilestoneView, TerritoryView, WorldState};
use serde_json::json;

fn temp_dir(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("aoe-report-{}-{name}", std::process::id()))
}

#[test]
fn generates_archive_and_match_artifacts() {
    let root = temp_dir("site");
    let _ = fs::remove_dir_all(&root);
    let source = root.join("matches").join("build-race-001");
    fs::create_dir_all(source.join("agents").join("agent-a")).expect("source dirs");

    let mut state = WorldState {
        match_state: MatchState::Finished,
        winner: Some("territory-a".into()),
        finish_reason: Some("first durable deployment".into()),
        elapsed_ms: 42_000,
        ..WorldState::default()
    };
    state.territories.insert(
        "territory-a".into(),
        TerritoryView {
            class: Some("builder".into()),
            agent: Some("agent-a".into()),
            state: TerritoryState::Healthy,
            competitor_state: Some(CompetitorState::Durable),
            milestone_points: 100,
            durable_at_ms: Some(42_000),
            milestones: [(
                "host-reboot".into(),
                MilestoneView {
                    passed: true,
                    points: 40,
                    ..MilestoneView::default()
                },
            )]
            .into(),
            ..TerritoryView::default()
        },
    );
    state.agents.insert(
        "agent-a".into(),
        AgentView {
            territory: "territory-a".into(),
            model: "model/a".into(),
            running: false,
            successful: Some(true),
            failure_source: Some(FailureSource::Player),
            terminal_state: Some(AgentTerminalState::Completed),
            terminal_detail: Some("done".into()),
            input_tokens: 12_345,
            output_tokens: 678,
            cost_microusd: 12_500,
            ..AgentView::default()
        },
    );
    fs::write(
        source.join("world.json"),
        serde_json::to_vec_pretty(&state).expect("state JSON"),
    )
    .expect("world");
    let event = EventEnvelope {
        schema_version: 1,
        sequence: 0,
        elapsed_ms: 42_000,
        event: Event::MatchFinished {
            winner: Some("territory-a".into()),
            reason: "first durable deployment".into(),
        },
    };
    fs::write(
        source.join("events.jsonl"),
        format!("{}\n", serde_json::to_string(&event).expect("event JSON")),
    )
    .expect("events");
    fs::write(
        source
            .join("agents")
            .join("agent-a")
            .join("transcript.json"),
        "[]",
    )
    .expect("transcript");

    let output = root.join("site");
    let summary = generate_reports(&root.join("matches"), &output).expect("report");
    assert_eq!(summary.matches, 1);
    let index = fs::read_to_string(output.join("index.html")).expect("index");
    assert!(index.contains("build-race-001"));
    assert!(index.contains("territory-a"));
    assert!(index.contains("What am I looking at?"));
    assert!(index.contains("identical disposable NixOS machines"));
    assert!(index.contains("https://github.com/ducks/agents-of-empires"));
    let match_page =
        fs::read_to_string(output.join("matches/build-race-001/index.html")).expect("match");
    assert!(match_page.contains("model/a"));
    assert!(match_page.contains("12,345"));
    assert!(match_page.contains("Match finished"));
    assert!(
        output
            .join("matches/build-race-001/artifacts/agent-a-transcript.json")
            .is_file()
    );

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn accepts_historical_events_with_duplicate_elapsed_time() {
    let root = temp_dir("historical");
    let _ = fs::remove_dir_all(&root);
    let source = root.join("old-match");
    fs::create_dir_all(&source).expect("source dir");
    fs::write(
        source.join("world.json"),
        serde_json::to_vec(&WorldState::default()).expect("world JSON"),
    )
    .expect("world");
    fs::write(
        source.join("events.jsonl"),
        "{\"schema_version\":1,\"sequence\":0,\"elapsed_ms\":10,\"kind\":\"durable_deployment_completed\",\"territory\":\"one\",\"elapsed_ms\":10}\n",
    )
    .expect("events");

    let output = root.join("site");
    generate_reports(&source, &output).expect("historical report");
    let page = fs::read_to_string(output.join("matches/old-match/index.html")).expect("page");
    assert!(page.contains("Durable deployment completed"));
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn separates_current_compatibility_key_from_history() {
    let root = temp_dir("seasons");
    let _ = fs::remove_dir_all(&root);
    for (name, key) in [
        ("race-001", "old"),
        ("race-002", "new"),
        ("race-003", "new"),
    ] {
        let source = root.join("matches").join(name);
        fs::create_dir_all(&source).expect("match dir");
        fs::write(
            source.join("world.json"),
            serde_json::to_vec(&WorldState::default()).expect("world"),
        )
        .expect("world file");
        fs::write(source.join("events.jsonl"), "").expect("events");
        fs::write(
            source.join("match.json"),
            serde_json::to_vec(&json!({
                "schema_version": 1,
                "controller_version": "0.1.0",
                "source_revision": null,
                "arena_id": "build",
                "arena_mode": "buildrace",
                "manifest_sha256": key,
                "verifier_sha256": key,
                "adapter_sha256": {},
                "compatibility_key": key
            }))
            .expect("provenance"),
        )
        .expect("provenance file");
    }
    let output = root.join("site");
    generate_reports(&root.join("matches"), &output).expect("reports");
    let index = fs::read_to_string(output.join("index.html")).expect("index");
    let current = index.find("<h2>Current</h2>").expect("current");
    let historical = index.find("<h2>Historical</h2>").expect("historical");
    assert!(index[current..historical].contains("race-002"));
    assert!(index[current..historical].contains("race-003"));
    assert!(!index[current..historical].contains("race-001"));
    assert!(index[historical..].contains("race-001"));
    fs::remove_dir_all(root).expect("cleanup");
}
