use std::collections::BTreeMap;
use std::fs;

use aoe_controller::{MatchProvenance, export_trajectories};
use aoe_domain::{AgentTerminalState, ArenaManifest, CompetitorState, MatchState, TerritoryState};
use aoe_replay::{AgentView, MilestoneView, TerritoryView, WorldState};
use serde_json::json;

const MANIFEST: &str = include_str!("../../../arenas/first-build/arena.toml");

#[test]
#[allow(clippy::too_many_lines)]
fn exports_observable_agent_trace_with_referee_outcome() {
    let root = std::env::temp_dir().join(format!("aoe-atif-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let source = root.join("matches/round-one");
    fs::create_dir_all(source.join("agents/agent-one")).expect("match dirs");

    let mut manifest = ArenaManifest::parse(MANIFEST).expect("manifest");
    manifest.agents[0].id = "agent-one".into();
    manifest.agents[0].territory = "territory-one".into();
    manifest.agents[0].adapter = "claux".into();
    manifest.agents[0].model = "test/model".into();
    manifest.agents[0].reasoning_effort = "high".into();
    fs::write(
        source.join("arena.json"),
        serde_json::to_vec(&manifest).expect("arena JSON"),
    )
    .expect("arena");

    let provenance = MatchProvenance {
        schema_version: 1,
        controller_version: "0.1.0".into(),
        source_revision: Some("revision-one".into()),
        arena_id: "first-build".into(),
        arena_mode: "buildrace".into(),
        manifest_sha256: "manifest-hash".into(),
        player_brief_sha256: None,
        verifier_sha256: "verifier-hash".into(),
        adapter_sha256: BTreeMap::from([("claux".into(), "adapter-hash".into())]),
        compatibility_key: "compatibility-key".into(),
    };
    fs::write(
        source.join("match.json"),
        serde_json::to_vec(&provenance).expect("provenance JSON"),
    )
    .expect("provenance");

    let mut world = WorldState {
        match_state: MatchState::Finished,
        winner: Some("territory-one".into()),
        finish_reason: Some("first durable deployment".into()),
        elapsed_ms: 42_000,
        ..WorldState::default()
    };
    world.agents.insert(
        "agent-one".into(),
        AgentView {
            territory: "territory-one".into(),
            model: "test/model".into(),
            running: false,
            successful: Some(true),
            terminal_state: Some(AgentTerminalState::Completed),
            input_tokens: 100,
            output_tokens: 20,
            cost_microusd: 10_000,
            ..AgentView::default()
        },
    );
    world.territories.insert(
        "territory-one".into(),
        TerritoryView {
            agent: Some("agent-one".into()),
            state: TerritoryState::Healthy,
            competitor_state: Some(CompetitorState::Durable),
            milestone_points: 100,
            durable_at_ms: Some(42_000),
            milestones: BTreeMap::from([(
                "host-reboot".into(),
                MilestoneView {
                    passed: true,
                    points: 40,
                    ..MilestoneView::default()
                },
            )]),
            ..TerritoryView::default()
        },
    );
    fs::write(
        source.join("world.json"),
        serde_json::to_vec(&world).expect("world JSON"),
    )
    .expect("world");
    fs::write(
        source.join("agents/agent-one/transcript.json"),
        serde_json::to_vec(&json!({
            "schema_version": 2,
            "model": "test/model",
            "messages": [
                {"role": "user", "content": "build the service"},
                {"role": "assistant", "content": "private reasoning must not export"}
            ],
            "tool_trace": [{
                "id": "call-one",
                "name": "Bash",
                "input": {"command": "systemctl restart app"},
                "output": "restarted\n",
                "is_error": false,
                "read_only": false,
                "started_after_ms": 100,
                "duration_ms": 25
            }],
            "outcome": {"status": "completed", "result": "service deployed"},
            "usage": {
                "input_tokens": 100,
                "output_tokens": 20,
                "cache_read_tokens": 50,
                "cache_creation_tokens": 5,
                "cost_usd": 0.01
            },
            "timing": {
                "total_duration_ms": 42000,
                "model_rounds": [{"index": 1, "duration_ms": 75}]
            }
        }))
        .expect("transcript JSON"),
    )
    .expect("transcript");

    let output = root.join("trajectories");
    let summary = export_trajectories(&root.join("matches"), &output).expect("export");
    assert_eq!(summary.trajectories, 1);
    assert_eq!(summary.skipped, 0);
    let trajectory =
        fs::read_to_string(output.join("round-one/agent-one.json")).expect("trajectory output");
    let value: serde_json::Value = serde_json::from_str(&trajectory).expect("trajectory JSON");
    assert_eq!(value["schema_version"], "ATIF-v1.7");
    assert_eq!(value["agent"]["name"], "claux");
    assert_eq!(value["agent"]["version"], "adapter-hash");
    assert_eq!(
        value["steps"][1]["tool_calls"][0]["arguments"]["command"],
        "systemctl restart app"
    );
    assert_eq!(
        value["steps"][1]["observation"]["results"][0]["content"],
        "restarted\n"
    );
    assert!(!trajectory.contains("private reasoning"));
    let evaluation = &value["final_metrics"]["extra"]["infrastructure_evaluation"];
    assert_eq!(evaluation["schema_version"], 1);
    assert_eq!(evaluation["producer"]["name"], "agents-of-empires");
    assert_eq!(evaluation["task"]["version"], "compatibility-key");
    assert_eq!(
        evaluation["outcome"]["verification"]["host-reboot"]["passed"],
        true
    );
    assert_eq!(evaluation["outcome"]["durable"], true);
    assert_eq!(evaluation["outcome"]["winner"], true);

    fs::remove_dir_all(root).expect("cleanup");
}
