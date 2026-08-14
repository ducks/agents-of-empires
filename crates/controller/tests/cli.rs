use std::path::PathBuf;

use aoe_controller::{ArenaCommand, Cli, Command, ParseError};

#[test]
fn parses_arena_init_with_default_output() {
    let cli = Cli::parse(
        ["arena", "init", "cache-race"]
            .into_iter()
            .map(str::to_owned),
    )
    .expect("parse");
    assert_eq!(
        cli.command,
        Command::Arena {
            command: ArenaCommand::Init {
                name: "cache-race".into(),
                output: PathBuf::from("arenas/cache-race"),
            },
        }
    );
}

#[test]
fn parses_arena_package_validation() {
    let cli = Cli::parse(
        ["arena", "validate", "community/cache-race", "--json"]
            .into_iter()
            .map(str::to_owned),
    )
    .expect("parse");
    assert_eq!(
        cli.command,
        Command::Arena {
            command: ArenaCommand::Validate {
                path: PathBuf::from("community/cache-race"),
                json: true,
            },
        }
    );
}

#[test]
fn parses_run_configuration() {
    let cli = Cli::parse(
        [
            "run",
            "arena.toml",
            "--adapter",
            "claux=/bin/claux-adapter",
            "--credential",
            "gatekeeper=/tmp/key",
            "--base-port",
            "28000",
            "--no-color",
        ]
        .into_iter()
        .map(str::to_owned),
    )
    .expect("parse");
    assert_eq!(
        cli.command,
        Command::Run {
            manifest: PathBuf::from("arena.toml"),
            output: PathBuf::from("matches/latest"),
            adapters: vec!["claux=/bin/claux-adapter".into()],
            credentials: vec!["gatekeeper=/tmp/key".into()],
            base_port: 28000,
            multicast_port: 23977,
            no_color: true,
        }
    );
}

#[test]
fn rejects_unknown_arguments() {
    let error = Cli::parse(
        ["replay", "events.jsonl", "--mystery"]
            .into_iter()
            .map(str::to_owned),
    )
    .expect_err("reject");
    assert_eq!(error, ParseError::Unexpected("--mystery".into()));
}

#[test]
fn parses_inspect_sequence() {
    let cli = Cli::parse(
        ["inspect", "events.jsonl", "42", "--json"]
            .into_iter()
            .map(str::to_owned),
    )
    .expect("parse");
    assert_eq!(
        cli.command,
        Command::Inspect {
            log: PathBuf::from("events.jsonl"),
            sequence: 42,
            json: true,
        }
    );
}

#[test]
fn parses_report_directories() {
    let cli = Cli::parse(
        ["report", "matches", "--output", "docs"]
            .into_iter()
            .map(str::to_owned),
    )
    .expect("parse");
    assert_eq!(
        cli.command,
        Command::Report {
            input: PathBuf::from("matches"),
            output: PathBuf::from("docs"),
            series: Vec::new(),
        }
    );
}

#[test]
fn parses_series_report_inputs() {
    let cli = Cli::parse(
        [
            "report",
            "matches",
            "--series",
            "series/first-build",
            "--series",
            "series/failover",
        ]
        .into_iter()
        .map(str::to_owned),
    )
    .expect("parse");
    assert_eq!(
        cli.command,
        Command::Report {
            input: PathBuf::from("matches"),
            output: PathBuf::from("site"),
            series: vec!["series/first-build".into(), "series/failover".into()],
        }
    );
}

#[test]
fn parses_series_configuration() {
    let cli = Cli::parse(
        [
            "series",
            "arena.toml",
            "--rounds",
            "5",
            "--adapter",
            "claux=/bin/claux-adapter",
            "--credential",
            "gatekeeper=/tmp/key",
            "--output",
            "series/test",
        ]
        .into_iter()
        .map(str::to_owned),
    )
    .expect("parse");
    assert_eq!(
        cli.command,
        Command::Series {
            manifest: PathBuf::from("arena.toml"),
            output: PathBuf::from("series/test"),
            adapters: vec!["claux=/bin/claux-adapter".into()],
            credentials: vec!["gatekeeper=/tmp/key".into()],
            rounds: Some(5),
            base_port: 26000,
            multicast_port: 23977,
            no_color: false,
        }
    );
}

#[test]
fn parses_benchmark_configuration() {
    let cli = Cli::parse(
        [
            "benchmark",
            "suites/infra-core.toml",
            "--adapter",
            "claux=/bin/claux-adapter",
            "--credential",
            "builder-one=/tmp/key",
            "--output",
            "benchmarks/infra-core",
        ]
        .into_iter()
        .map(str::to_owned),
    )
    .expect("parse");
    assert_eq!(
        cli.command,
        Command::Benchmark {
            suite: PathBuf::from("suites/infra-core.toml"),
            output: PathBuf::from("benchmarks/infra-core"),
            adapters: vec!["claux=/bin/claux-adapter".into()],
            credentials: vec!["builder-one=/tmp/key".into()],
            base_port: 26000,
            multicast_port: 23977,
            no_color: false,
        }
    );
}
