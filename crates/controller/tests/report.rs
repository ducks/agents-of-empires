use std::collections::BTreeMap;
use std::fs;

use aoe_controller::{
    BenchmarkArenaSummary, BenchmarkPlanEntry, BenchmarkStanding, BenchmarkSummary, SeriesRound,
    SeriesStanding, SeriesSummary, generate_reports, generate_reports_with_benchmarks,
    generate_reports_with_series,
};
use aoe_domain::{
    AgentTerminalState, ArenaManifest, CompetitorState, Event, EventEnvelope, FailureSource,
    MatchState, TerritoryState,
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
    state.agents.insert(
        "agent-b".into(),
        AgentView {
            territory: "territory-b".into(),
            model: "model/b".into(),
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
    let usage = EventEnvelope {
        schema_version: 1,
        sequence: 1,
        elapsed_ms: 42_000,
        event: Event::UsageCharged {
            agent: "agent-a".into(),
            resource_units: 1,
            input_tokens: Some(12_345),
            output_tokens: Some(678),
            cost_microusd: Some(12_500),
        },
    };
    fs::write(
        source.join("events.jsonl"),
        format!(
            "{}\n{}\n",
            serde_json::to_string(&event).expect("event JSON"),
            serde_json::to_string(&usage).expect("usage JSON")
        ),
    )
    .expect("events");
    fs::write(
        source
            .join("agents")
            .join("agent-a")
            .join("transcript.json"),
        serde_json::to_vec_pretty(&json!({
            "schema_version": 1,
            "messages": [{"role": "assistant", "content": "private reasoning"}],
            "outcome": {"status": "completed"},
            "tool_trace": [
                {
                    "id": "tool-1",
                    "name": "bash",
                    "input": {"command": "systemctl status app.service", "description": "Inspect app"},
                    "output": "ExecStart=/usr/bin/python /srv/app/server.py",
                    "started_after_ms": 1_000,
                    "duration_ms": 100
                },
                {
                    "id": "tool-2",
                    "name": "bash",
                    "input": {"command": "sed -i 's/old/new/' /etc/app.conf", "description": "Repair config"},
                    "output": "",
                    "started_after_ms": 2_000,
                    "duration_ms": 50
                }
            ]
        }))
        .expect("transcript JSON"),
    )
    .expect("transcript");
    fs::create_dir_all(source.join("agents/agent-b")).expect("second agent dir");
    fs::write(
        source.join("agents/agent-b/transcript.live.json"),
        "{ interrupted JSON",
    )
    .expect("interrupted transcript");
    let manifest_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("arenas/first-build/arena.toml");
    let manifest = ArenaManifest::load(manifest_path).expect("arena manifest");
    fs::write(
        source.join("arena.json"),
        serde_json::to_vec_pretty(&manifest).expect("arena JSON"),
    )
    .expect("arena snapshot");

    let output = root.join("site");
    let summary = generate_reports(&root.join("matches"), &output).expect("report");
    assert_eq!(summary.matches, 1);
    let index = fs::read_to_string(output.join("index.html")).expect("index");
    assert!(!index.contains("build-race-001"));
    assert!(index.contains("Browse 1 archived run"));
    assert!(index.contains("What am I looking at?"));
    assert!(index.contains("identical disposable NixOS machines"));
    assert!(index.contains("https://github.com/ducks/agents-of-empires"));
    let archive = fs::read_to_string(output.join("archive/index.html")).expect("archive");
    assert!(archive.contains("build-race-001"));
    assert!(archive.contains("territory-a"));
    assert!(archive.contains("Archived because provenance is unavailable"));
    let match_page =
        fs::read_to_string(output.join("matches/build-race-001/index.html")).expect("match");
    assert!(match_page.contains("model/a"));
    assert!(match_page.contains("12,345"));
    assert!(match_page.contains("Match finished"));
    assert!(match_page.contains("Watch the race unfold"));
    assert!(match_page.contains("data-match-replay"));
    assert!(match_page.contains("data-scrubber"));
    assert!(match_page.contains("match_finished"));
    assert!(match_page.contains("Service map"));
    assert!(match_page.contains("State Store"));
    assert!(match_page.contains("data-topology"));
    assert!(match_page.contains("agent-terminal"));
    assert!(match_page.contains("data-terminal-lines"));
    assert!(match_page.contains("no observable tool activity"));
    assert!(match_page.contains("\"activity\":"));
    assert!(match_page.contains("Repair config"));
    assert!(match_page.contains("How they fought"));
    assert!(match_page.contains("First change"));
    assert!(match_page.contains("Python"));
    assert!(!match_page.contains("private reasoning"));
    assert!(
        output
            .join("matches/build-race-001/artifacts/agent-a-transcript.json")
            .is_file()
    );
    assert!(
        output
            .join("matches/build-race-001/artifacts/agent-b-transcript.live.json")
            .is_file()
    );
    assert!(
        !output
            .join("matches/build-race-001/artifacts/agent-b-analysis.json")
            .exists()
    );
    assert!(
        output
            .join("matches/build-race-001/artifacts/arena.json")
            .is_file()
    );
    let analysis =
        fs::read_to_string(output.join("matches/build-race-001/artifacts/agent-a-analysis.json"))
            .expect("analysis artifact");
    assert!(analysis.contains("first_mutation_after_ms"));
    assert!(!analysis.contains("private reasoning"));

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn missing_usage_is_not_rendered_as_zero() {
    let root = temp_dir("missing-usage");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("match dir");
    let mut state = WorldState::default();
    state.agents.insert(
        "unfinished".into(),
        AgentView {
            territory: "one".into(),
            model: "model/unfinished".into(),
            terminal_state: Some(AgentTerminalState::Terminated),
            ..AgentView::default()
        },
    );
    fs::write(
        root.join("world.json"),
        serde_json::to_vec(&state).expect("world"),
    )
    .expect("world file");
    fs::write(root.join("events.jsonl"), "").expect("events");
    let output = root.join("site");
    generate_reports(&root, &output).expect("report");
    let match_name = root.file_name().expect("match name");
    let page = fs::read_to_string(output.join("matches").join(match_name).join("index.html"))
        .expect("page");
    assert!(page.contains("model/unfinished"));
    assert!(page.contains("<td>n/a</td><td>n/a</td><td>n/a</td>"));
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
fn replay_collapses_repeated_failures_into_stall_spans() {
    let root = temp_dir("stalls");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("match dir");
    let mut state = WorldState {
        elapsed_ms: 12_000,
        ..WorldState::default()
    };
    state.territories.insert(
        "slow-agent".into(),
        TerritoryView {
            agent: Some("agent-a".into()),
            ..TerritoryView::default()
        },
    );
    state.agents.insert(
        "agent-a".into(),
        AgentView {
            territory: "slow-agent".into(),
            model: "model/slow".into(),
            ..AgentView::default()
        },
    );
    fs::write(
        root.join("world.json"),
        serde_json::to_vec(&state).expect("world"),
    )
    .expect("world file");
    fs::write(
        root.join("events.jsonl"),
        [
            json!({"sequence":0,"elapsed_ms":1000,"kind":"milestone_failed","territory":"slow-agent","milestone":"service-up","detail":"connection refused"}),
            json!({"sequence":1,"elapsed_ms":3000,"kind":"milestone_failed","territory":"slow-agent","milestone":"service-up","detail":"connection refused"}),
            json!({"sequence":2,"elapsed_ms":9000,"kind":"milestone_passed","territory":"slow-agent","milestone":"service-up","points":10}),
        ]
        .iter()
        .map(serde_json::Value::to_string)
        .collect::<Vec<_>>()
        .join("\n"),
    )
    .expect("events");

    let output = root.join("site");
    generate_reports(&root, &output).expect("report");
    let match_name = root.file_name().expect("match name");
    let page = fs::read_to_string(output.join("matches").join(match_name).join("index.html"))
        .expect("page");
    assert!(page.contains("\"retries\":2"));
    assert!(page.contains("\"start_ms\":1000"));
    assert!(page.contains("\"end_ms\":9000"));
    assert!(page.contains("lane-stall"));
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
    assert!(index.contains("<h2>Current matches</h2>"));
    assert!(index.contains("race-002"));
    assert!(index.contains("race-003"));
    assert!(!index.contains("race-001"));
    assert!(index.contains("href=\"archive/\""));

    let archive =
        fs::read_to_string(output.join("archive").join("index.html")).expect("archive index");
    assert!(archive.contains("<h2>Historical matches</h2>"));
    assert!(archive.contains("race-001"));
    assert!(!archive.contains("race-002"));
    assert!(!archive.contains("race-003"));
    assert!(archive.contains("Superseded manifest or verifier"));
    assert!(archive.contains("href=\"../matches/race-001/\""));
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn generates_series_battle_card_and_round_links() {
    let root = temp_dir("series-site");
    let _ = fs::remove_dir_all(&root);
    let matches = root.join("matches");
    let archive = matches.join("archive-001");
    fs::create_dir_all(&archive).expect("archive dir");
    fs::write(
        archive.join("world.json"),
        serde_json::to_vec(&WorldState::default()).expect("world"),
    )
    .expect("world file");
    fs::write(archive.join("events.jsonl"), "").expect("events");

    let series = root.join("series/first-build-series");
    let provenance = json!({
        "schema_version": 1,
        "controller_version": "0.1.0",
        "source_revision": null,
        "arena_id": "first-build",
        "arena_mode": "buildrace",
        "manifest_sha256": "manifest",
        "verifier_sha256": "verifier",
        "adapter_sha256": {},
        "compatibility_key": "series-current"
    });
    for round in 1..=2 {
        let source = series.join(format!("round-{round:03}"));
        fs::create_dir_all(&source).expect("round dir");
        fs::write(
            source.join("world.json"),
            serde_json::to_vec(&WorldState::default()).expect("world"),
        )
        .expect("world file");
        fs::write(source.join("events.jsonl"), "").expect("events");
        fs::write(
            source.join("match.json"),
            serde_json::to_vec(&provenance).expect("provenance"),
        )
        .expect("match provenance");
    }
    let summary = SeriesSummary {
        schema_version: 1,
        arena_id: "first-build".into(),
        rounds_requested: 2,
        rounds_completed: 2,
        rounds: vec![
            SeriesRound {
                round: 1,
                output: "round-001".into(),
                seats: [
                    ("deepseek".into(), "builder-one".into()),
                    ("luna".into(), "builder-two".into()),
                ]
                .into(),
                winner_territory: Some("builder-one".into()),
                winner_agent: Some("deepseek".into()),
                duration_ms: 50_000,
                usage_agents: vec!["deepseek".into(), "luna".into()],
            },
            SeriesRound {
                round: 2,
                output: "round-002".into(),
                seats: [
                    ("deepseek".into(), "builder-two".into()),
                    ("luna".into(), "builder-one".into()),
                ]
                .into(),
                winner_territory: Some("builder-two".into()),
                winner_agent: Some("deepseek".into()),
                duration_ms: 55_000,
                usage_agents: vec!["deepseek".into(), "luna".into()],
            },
        ],
        standings: vec![
            SeriesStanding {
                agent: "deepseek".into(),
                model: "deepseek/v4".into(),
                appearances: 2,
                wins: 2,
                durable_deployments: 2,
                median_durable_ms: Some(55_000),
                usage_recorded: 2,
                input_tokens: Some(40_000),
                output_tokens: Some(4_000),
                cost_microusd: Some(2_300),
                cost_per_durable_microusd: Some(1_150),
            },
            SeriesStanding {
                agent: "luna".into(),
                model: "openai/luna".into(),
                appearances: 2,
                wins: 0,
                durable_deployments: 0,
                median_durable_ms: None,
                usage_recorded: 2,
                input_tokens: Some(60_000),
                output_tokens: Some(6_000),
                cost_microusd: Some(9_800),
                cost_per_durable_microusd: None,
            },
        ],
    };
    fs::write(
        series.join("series.json"),
        serde_json::to_vec_pretty(&summary).expect("series"),
    )
    .expect("series file");

    let output = root.join("site");
    let generated = generate_reports_with_series(&matches, &[series], &output).expect("report");
    assert_eq!(generated.matches, 3);
    assert_eq!(generated.series, 1);
    assert_eq!(generated.benchmarks, 0);
    let index = fs::read_to_string(output.join("index.html")).expect("index");
    assert!(index.contains("Seat-rotated races"));
    assert!(index.contains("first-build-series"));
    assert!(index.contains("deepseek"));
    let page = fs::read_to_string(output.join("series/first-build-series/index.html"))
        .expect("series page");
    assert!(page.contains("Battle card"));
    assert!(page.contains("Seat rotation"));
    assert!(page.contains("deepseek/v4"));
    assert!(page.contains("$0.0011"));
    assert!(page.contains("../../matches/first-build-series-round-001/"));
    assert!(
        output
            .join("matches/first-build-series-round-002/index.html")
            .is_file()
    );
    assert!(
        output
            .join("series/first-build-series/artifacts/series.json")
            .is_file()
    );
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
#[allow(clippy::too_many_lines)]
fn generates_benchmark_leaderboard_and_drill_down() {
    let root = temp_dir("benchmark-site");
    let _ = fs::remove_dir_all(&root);
    let benchmark = root.join("benchmarks/infra-core");
    let arena = benchmark.join("01-first-build-real");
    let round = arena.join("round-001");
    fs::create_dir_all(round.join("agents/deepseek-builder")).expect("round dir");
    let mut round_state = WorldState::default();
    round_state.agents.insert(
        "deepseek-builder".into(),
        AgentView {
            territory: "builder-one".into(),
            model: "deepseek/v4".into(),
            ..AgentView::default()
        },
    );
    fs::write(
        round.join("world.json"),
        serde_json::to_vec(&round_state).expect("world"),
    )
    .expect("world file");
    fs::write(round.join("events.jsonl"), "").expect("events");
    fs::write(
        round.join("agents/deepseek-builder/transcript.json"),
        serde_json::to_vec(&json!({
            "tool_trace": [{
                "name": "Bash",
                "input": {"command": "systemctl status app.service"},
                "output": "active",
                "started_after_ms": 100
            }]
        }))
        .expect("transcript"),
    )
    .expect("transcript file");

    let series = SeriesSummary {
        schema_version: 1,
        arena_id: "first-build-real".into(),
        rounds_requested: 1,
        rounds_completed: 1,
        rounds: vec![SeriesRound {
            round: 1,
            output: round.clone(),
            seats: [("deepseek-builder".into(), "builder-one".into())].into(),
            winner_territory: Some("builder-one".into()),
            winner_agent: Some("deepseek-builder".into()),
            duration_ms: 42_000,
            usage_agents: vec!["deepseek-builder".into()],
        }],
        standings: vec![SeriesStanding {
            agent: "deepseek-builder".into(),
            model: "deepseek/v4".into(),
            appearances: 1,
            wins: 1,
            durable_deployments: 1,
            median_durable_ms: Some(42_000),
            usage_recorded: 1,
            input_tokens: Some(10_000),
            output_tokens: Some(1_000),
            cost_microusd: Some(2_000),
            cost_per_durable_microusd: Some(2_000),
        }],
    };
    fs::write(
        arena.join("series.json"),
        serde_json::to_vec_pretty(&series).expect("series"),
    )
    .expect("series file");

    let standing = BenchmarkStanding {
        model: "deepseek/v4".into(),
        adapter: "claux".into(),
        reasoning_effort: "high".into(),
        appearances: 1,
        wins: 1,
        durable_deployments: 1,
        durable_times_ms: vec![42_000],
        median_durable_ms: Some(42_000),
        milestone_passes: 4,
        milestones_available: 4,
        usage_recorded: 1,
        input_tokens: Some(10_000),
        output_tokens: Some(1_000),
        cost_microusd: Some(2_000),
        cost_per_durable_microusd: Some(2_000),
        failures: BTreeMap::new(),
    };
    let summary = BenchmarkSummary {
        schema_version: 2,
        suite_id: "infra-core".into(),
        arenas_requested: 1,
        arenas_completed: 1,
        plan: vec![BenchmarkPlanEntry {
            arena_id: "first-build-real".into(),
            manifest: "arenas/first-build/agents-real.toml".into(),
            compatibility_key: "fixture-key".into(),
            rounds: 1,
            output: arena.clone(),
        }],
        arenas: vec![BenchmarkArenaSummary {
            arena_id: "first-build-real".into(),
            output: arena.clone(),
            rounds_requested: 1,
            rounds_completed: 1,
            aborted: false,
            standings: vec![standing.clone()],
        }],
        standings: vec![standing],
    };
    fs::write(
        benchmark.join("benchmark.json"),
        serde_json::to_vec_pretty(&summary).expect("benchmark"),
    )
    .expect("benchmark file");

    let output = root.join("site");
    let generated = generate_reports_with_benchmarks(
        &benchmark,
        &[],
        std::slice::from_ref(&benchmark),
        &output,
    )
    .expect("report");
    assert_eq!(generated.matches, 1);
    assert_eq!(generated.series, 1);
    assert_eq!(generated.benchmarks, 1);
    let index = fs::read_to_string(output.join("index.html")).expect("index");
    assert!(index.contains("Cross-arena model benchmark"));
    assert!(index.contains("benchmarks/infra-core/"));
    let page = fs::read_to_string(output.join("benchmarks/infra-core/index.html"))
        .expect("benchmark page");
    assert!(page.contains("Model leaderboard"));
    assert!(page.contains("deepseek/v4"));
    assert!(page.contains("claux · high"));
    assert!(page.contains("4/4"));
    assert!(page.contains("../../series/infra-core-first-build-real/"));
    assert!(page.contains("Arenas and match evidence"));
    assert!(page.contains("1 of 1 rounds include strategy analysis"));
    assert!(page.contains("Round 1 · How they fought"));
    assert!(page.contains("../../matches/infra-core-first-build-real-round-001/"));
    assert!(
        output
            .join("matches/infra-core-first-build-real-round-001/index.html")
            .is_file()
    );
    fs::remove_dir_all(root).expect("cleanup");
}
