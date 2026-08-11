use std::path::PathBuf;

use aoe_controller::{Cli, Command, ParseError};

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
