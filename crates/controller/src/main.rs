use std::collections::HashMap;
use std::env;
use std::io::IsTerminal;
use std::path::PathBuf;

use aoe_controller::{
    ArenaCommand, BenchmarkOptions, Cli, Command, RunOptions, SeriesOptions, doctor,
    generate_reports_with_benchmarks, init_arena, inspect, render_benchmark, render_series,
    replay_log, run_benchmark, run_match, run_series, validate, validate_arena_package,
};
use aoe_tui::RenderOptions;

const HELP: &str = "Agents of Empires

Usage:
  agents-of-empires arena init NAME [--output DIR]
  agents-of-empires arena validate ARENA_DIR_OR_MANIFEST [--json]
  agents-of-empires validate MANIFEST [--json]
  agents-of-empires run MANIFEST --adapter NAME=PATH [--credential TERRITORY=PATH]
      [--output DIR] [--base-port PORT] [--multicast-port PORT] [--no-color]
  agents-of-empires series MANIFEST --adapter NAME=PATH [--credential TERRITORY=PATH]
      [--rounds N] [--output DIR] [--base-port PORT] [--multicast-port PORT] [--no-color]
  agents-of-empires benchmark SUITE --adapter NAME=PATH [--credential TERRITORY=PATH]
      [--output DIR] [--base-port PORT] [--multicast-port PORT] [--no-color]
  agents-of-empires replay EVENT_LOG [--json] [--no-color] [--width COLUMNS]
  agents-of-empires inspect EVENT_LOG SEQUENCE [--json]
  agents-of-empires report MATCH_OR_MATCHES_DIR [--series SERIES_OR_SERIES_DIR]
      [--benchmark BENCHMARK_OR_BENCHMARKS_DIR] [--output DIR]
  agents-of-empires doctor [--json]
";

#[tokio::main]
async fn main() {
    if let Err(error) = execute().await {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

async fn execute() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse(env::args().skip(1))?;
    match cli.command {
        Command::Help => print!("{HELP}"),
        Command::Arena { command } => match command {
            ArenaCommand::Init { name, output } => {
                let manifest = init_arena(&name, &output)?;
                println!("created arena {name} at {}", output.display());
                println!(
                    "validate it with: agents-of-empires arena validate {}",
                    output.display()
                );
                println!("manifest: {}", manifest.display());
            }
            ArenaCommand::Validate { path, json } => {
                let report = validate_arena_package(&path)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&report)?);
                } else {
                    println!(
                        "{} arena package {} ({})",
                        if report.valid { "valid" } else { "invalid" },
                        report.arena.as_deref().unwrap_or("unknown"),
                        report.root.display()
                    );
                    for warning in &report.warnings {
                        println!("  warning: {warning}");
                    }
                    for error in &report.errors {
                        println!("  error: {error}");
                    }
                }
                if !report.valid {
                    return Err("arena package validation failed".into());
                }
            }
        },
        Command::Validate { manifest, json } => {
            let report = validate(&manifest)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else if report.valid {
                println!(
                    "valid arena {}: {} territories, {} agents",
                    report.arena, report.territories, report.agents
                );
            } else {
                println!("arena {} has local reference warnings:", report.arena);
                for warning in report.warnings {
                    println!("  {warning}");
                }
            }
        }
        Command::Run {
            manifest,
            output,
            adapters,
            credentials,
            base_port,
            multicast_port,
            no_color,
        } => {
            let state = run_match(RunOptions {
                manifest,
                output,
                adapters: mappings(adapters, "--adapter")?,
                credentials: mappings(credentials, "--credential")?,
                base_port,
                multicast_port,
                color: !no_color && std::io::stdout().is_terminal(),
            })
            .await?;
            println!("match ended: {:?}", state.match_state);
        }
        Command::Series {
            manifest,
            output,
            adapters,
            credentials,
            rounds,
            base_port,
            multicast_port,
            no_color,
        } => {
            let summary = run_series(SeriesOptions {
                run: RunOptions {
                    manifest,
                    output,
                    adapters: mappings(adapters, "--adapter")?,
                    credentials: mappings(credentials, "--credential")?,
                    base_port,
                    multicast_port,
                    color: !no_color && std::io::stdout().is_terminal(),
                },
                rounds,
            })
            .await?;
            println!("{}", render_series(&summary));
        }
        Command::Benchmark {
            suite,
            output,
            adapters,
            credentials,
            base_port,
            multicast_port,
            no_color,
        } => {
            let summary = run_benchmark(BenchmarkOptions {
                suite,
                output,
                adapters: mappings(adapters, "--adapter")?,
                credentials: mappings(credentials, "--credential")?,
                base_port,
                multicast_port,
                color: !no_color && std::io::stdout().is_terminal(),
            })
            .await?;
            println!("{}", render_benchmark(&summary));
        }
        Command::Replay {
            log,
            json,
            no_color,
            width,
        } => println!(
            "{}",
            replay_log(
                &log,
                json,
                RenderOptions {
                    width,
                    color: !no_color,
                    ..RenderOptions::default()
                }
            )?
        ),
        Command::Inspect {
            log,
            sequence,
            json,
        } => println!("{}", inspect(&log, sequence, json)?),
        Command::Report {
            input,
            output,
            series,
            benchmarks,
        } => {
            let series: Vec<_> = series.into_iter().map(PathBuf::from).collect();
            let benchmarks: Vec<_> = benchmarks.into_iter().map(PathBuf::from).collect();
            let report = generate_reports_with_benchmarks(&input, &series, &benchmarks, &output)?;
            println!(
                "generated {} match report{}, {} series report{}, and {} benchmark report{} at {}",
                report.matches,
                if report.matches == 1 { "" } else { "s" },
                report.series,
                if report.series == 1 { "" } else { "s" },
                report.benchmarks,
                if report.benchmarks == 1 { "" } else { "s" },
                report.index.display()
            );
        }
        Command::Doctor { json } => {
            let report = doctor();
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                for check in &report.checks {
                    println!(
                        "{} {:<24} {}",
                        if check.available { "ok" } else { "missing" },
                        check.name,
                        check.detail
                    );
                }
            }
            if !report.ready {
                return Err("host is not ready to run an arena".into());
            }
        }
    }
    Ok(())
}

fn mappings(
    values: Vec<String>,
    flag: &str,
) -> Result<HashMap<String, PathBuf>, Box<dyn std::error::Error>> {
    values
        .into_iter()
        .map(|value| {
            let (name, path) = value
                .split_once('=')
                .ok_or_else(|| format!("{flag} expects NAME=PATH"))?;
            Ok((name.to_owned(), PathBuf::from(path)))
        })
        .collect()
}
