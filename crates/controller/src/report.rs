use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use aoe_domain::{AgentTerminalState, ArenaManifest, ArenaVisualization};
use aoe_replay::WorldState;
use serde::Serialize;
use thiserror::Error;

use crate::analysis::{TranscriptAnalysis, analyze_transcript};
use crate::benchmark::{BenchmarkStanding, BenchmarkSummary};
use crate::provenance::{MatchProvenance, read_provenance};
use crate::series::SeriesSummary;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportSummary {
    pub matches: usize,
    pub series: usize,
    pub benchmarks: usize,
    pub index: PathBuf,
}

#[derive(Debug, Error)]
pub enum ReportError {
    #[error("report I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid event JSON in {path} at line {line}: {detail}")]
    EventJson {
        path: PathBuf,
        line: usize,
        detail: String,
    },
    #[error("invalid world state in {path}: {detail}")]
    World { path: PathBuf, detail: String },
    #[error("invalid arena snapshot in {path}: {detail}")]
    Arena { path: PathBuf, detail: String },
    #[error("invalid series summary in {path}: {detail}")]
    Series { path: PathBuf, detail: String },
    #[error("invalid benchmark summary in {path}: {detail}")]
    Benchmark { path: PathBuf, detail: String },
    #[error("could not analyze transcript {path}: {detail}")]
    Analysis { path: PathBuf, detail: String },
    #[error("no completed match artifacts found below {0}")]
    NoMatches(PathBuf),
    #[error("no completed series artifacts found below {0}")]
    NoSeries(PathBuf),
    #[error("no completed benchmark artifacts found below {0}")]
    NoBenchmarks(PathBuf),
}

struct MatchReport {
    name: String,
    slug: String,
    listed: bool,
    state: WorldState,
    events: Vec<ReportEvent>,
    source: PathBuf,
    report_dir: PathBuf,
    provenance: Option<MatchProvenance>,
    visualization: Option<ArenaVisualization>,
    fog_of_war: bool,
    analyses: BTreeMap<String, TranscriptAnalysis>,
    current: bool,
}

struct SeriesReport {
    name: String,
    slug: String,
    summary: SeriesSummary,
    report_dir: PathBuf,
    round_slugs: Vec<String>,
    provenance: Option<MatchProvenance>,
    current: bool,
}

struct BenchmarkReport {
    name: String,
    slug: String,
    summary: BenchmarkSummary,
    report_dir: PathBuf,
    series_slugs: Vec<String>,
    round_links: Vec<Vec<BenchmarkRoundLink>>,
}

struct BenchmarkRoundLink {
    round: usize,
    slug: String,
    has_analysis: bool,
}

/// Generate a self-contained static report site from one match directory or a
/// directory containing match directories.
///
/// # Errors
///
/// Returns an error for missing artifacts, malformed event/world data, or file
/// system failures.
pub fn generate_reports(input: &Path, output: &Path) -> Result<ReportSummary, ReportError> {
    generate_reports_with_benchmarks(input, &[], &[], output)
}

/// Generate a static archive containing ordinary matches and match series.
///
/// # Errors
///
/// Returns an error for missing artifacts, malformed match or series data, or
/// file system failures.
pub fn generate_reports_with_series(
    input: &Path,
    series_inputs: &[PathBuf],
    output: &Path,
) -> Result<ReportSummary, ReportError> {
    generate_reports_with_benchmarks(input, series_inputs, &[], output)
}

/// Generate a static archive containing matches, series, and benchmark suites.
///
/// # Errors
///
/// Returns an error for missing artifacts, malformed report data, or file
/// system failures.
pub fn generate_reports_with_benchmarks(
    input: &Path,
    series_inputs: &[PathBuf],
    benchmark_inputs: &[PathBuf],
    output: &Path,
) -> Result<ReportSummary, ReportError> {
    let match_dirs = match discover_matches(input) {
        Ok(matches) => matches,
        Err(ReportError::NoMatches(_))
            if !series_inputs.is_empty() || !benchmark_inputs.is_empty() =>
        {
            Vec::new()
        }
        Err(error) => return Err(error),
    };
    fs::create_dir_all(output.join("matches"))?;
    fs::create_dir_all(output.join("series"))?;
    fs::create_dir_all(output.join("benchmarks"))?;
    fs::create_dir_all(output.join("archive"))?;
    let mut reports = Vec::with_capacity(match_dirs.len());
    for source in match_dirs {
        let name = source
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("match")
            .to_owned();
        let slug = safe_name(&name);
        reports.push(load_match_report(source, name, slug, true, output)?);
    }

    let mut series_reports = Vec::new();
    for series_input in series_inputs {
        for source in discover_series(series_input)? {
            let name = source
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("series")
                .to_owned();
            let slug = safe_name(&name);
            append_series_report(
                &source,
                name,
                slug,
                output,
                &mut reports,
                &mut series_reports,
            )?;
        }
    }
    let mut benchmark_reports = Vec::new();
    append_benchmark_reports(
        benchmark_inputs,
        output,
        &mut reports,
        &mut series_reports,
        &mut benchmark_reports,
    )?;
    reports.sort_by(|left, right| right.name.cmp(&left.name));
    series_reports.sort_by(|left, right| right.name.cmp(&left.name));
    benchmark_reports.sort_by(|left, right| right.name.cmp(&left.name));
    mark_current(&mut reports);
    mark_current_series(&mut series_reports);
    for report in &reports {
        fs::write(report.report_dir.join("index.html"), render_match(report))?;
    }
    for report in &series_reports {
        fs::write(report.report_dir.join("index.html"), render_series(report))?;
    }
    for report in &benchmark_reports {
        fs::write(
            report.report_dir.join("index.html"),
            render_benchmark_report(report),
        )?;
    }
    let index = output.join("index.html");
    fs::write(
        &index,
        render_index(&reports, &series_reports, &benchmark_reports),
    )?;
    fs::write(
        output.join("archive").join("index.html"),
        render_archive(&reports, &series_reports),
    )?;
    Ok(ReportSummary {
        matches: reports.len(),
        series: series_reports.len(),
        benchmarks: benchmark_reports.len(),
        index,
    })
}

fn append_benchmark_reports(
    inputs: &[PathBuf],
    output: &Path,
    reports: &mut Vec<MatchReport>,
    series_reports: &mut Vec<SeriesReport>,
    benchmark_reports: &mut Vec<BenchmarkReport>,
) -> Result<(), ReportError> {
    for input in inputs {
        for source in discover_benchmarks(input)? {
            let summary = read_benchmark(&source.join("benchmark.json"))?;
            let name = summary.suite_id.clone();
            let slug = safe_name(&name);
            let mut series_slugs = Vec::with_capacity(summary.arenas.len());
            let mut round_links = Vec::with_capacity(summary.arenas.len());
            for arena in &summary.arenas {
                let arena_source = source.join(
                    arena
                        .output
                        .file_name()
                        .unwrap_or_else(|| std::ffi::OsStr::new(&arena.arena_id)),
                );
                let arena_slug = format!("{slug}-{}", safe_name(&arena.arena_id));
                append_series_report(
                    &arena_source,
                    format!("{} · {}", name, arena.arena_id),
                    arena_slug.clone(),
                    output,
                    reports,
                    series_reports,
                )?;
                let links = series_reports
                    .last()
                    .map(|series| {
                        series
                            .summary
                            .rounds
                            .iter()
                            .zip(&series.round_slugs)
                            .map(|(round, slug)| BenchmarkRoundLink {
                                round: round.round,
                                slug: slug.clone(),
                                has_analysis: reports
                                    .iter()
                                    .find(|report| report.slug == *slug)
                                    .is_some_and(|report| !report.analyses.is_empty()),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                series_slugs.push(arena_slug);
                round_links.push(links);
            }
            let report_dir = output.join("benchmarks").join(&slug);
            fs::create_dir_all(report_dir.join("artifacts"))?;
            fs::copy(
                source.join("benchmark.json"),
                report_dir.join("artifacts/benchmark.json"),
            )?;
            benchmark_reports.push(BenchmarkReport {
                name,
                slug,
                summary,
                report_dir,
                series_slugs,
                round_links,
            });
        }
    }
    Ok(())
}

fn append_series_report(
    source: &Path,
    name: String,
    slug: String,
    output: &Path,
    reports: &mut Vec<MatchReport>,
    series_reports: &mut Vec<SeriesReport>,
) -> Result<(), ReportError> {
    let summary = read_series(&source.join("series.json"))?;
    let mut round_slugs = Vec::with_capacity(summary.rounds.len());
    for round in &summary.rounds {
        let round_name = format!("{} · round {}", name, round.round);
        let round_slug = format!("{}-round-{:03}", slug, round.round);
        let round_source = source.join(format!("round-{:03}", round.round));
        reports.push(load_match_report(
            round_source,
            round_name,
            round_slug.clone(),
            false,
            output,
        )?);
        round_slugs.push(round_slug);
    }
    let report_dir = output.join("series").join(&slug);
    fs::create_dir_all(report_dir.join("artifacts"))?;
    fs::copy(
        source.join("series.json"),
        report_dir.join("artifacts/series.json"),
    )?;
    let provenance = summary.rounds.first().and_then(|round| {
        read_provenance(
            &source
                .join(format!("round-{:03}", round.round))
                .join("match.json"),
        )
        .ok()
    });
    series_reports.push(SeriesReport {
        name,
        slug,
        summary,
        report_dir,
        round_slugs,
        provenance,
        current: false,
    });
    Ok(())
}

fn load_match_report(
    source: PathBuf,
    name: String,
    slug: String,
    listed: bool,
    output: &Path,
) -> Result<MatchReport, ReportError> {
    let report_dir = output.join("matches").join(&slug);
    fs::create_dir_all(&report_dir)?;
    let state = read_world(&source.join("world.json"))?;
    let events = read_report_events(&source.join("events.jsonl"))?;
    let provenance = read_provenance(&source.join("match.json")).ok();
    let (visualization, fog_of_war) = read_arena_presentation(&source.join("arena.json"))?;
    let analyses = analyze_agents(&source, &state)?;
    let report = MatchReport {
        name,
        slug,
        listed,
        state,
        events,
        source,
        report_dir,
        provenance,
        visualization,
        fog_of_war,
        analyses,
        current: false,
    };
    copy_public_artifacts(&report)?;
    Ok(report)
}

fn mark_current(reports: &mut [MatchReport]) {
    let mut current = std::collections::HashMap::new();
    for report in reports.iter() {
        if let Some(provenance) = &report.provenance {
            current
                .entry(provenance.arena_id.clone())
                .or_insert_with(|| provenance.compatibility_key.clone());
        }
    }
    for report in reports {
        report.current = report.provenance.as_ref().is_some_and(|provenance| {
            current.get(&provenance.arena_id) == Some(&provenance.compatibility_key)
        });
    }
}

fn mark_current_series(reports: &mut [SeriesReport]) {
    let mut current = std::collections::HashMap::new();
    for report in reports.iter() {
        if let Some(provenance) = &report.provenance {
            current
                .entry(provenance.arena_id.clone())
                .or_insert_with(|| provenance.compatibility_key.clone());
        }
    }
    for report in reports {
        report.current = report.provenance.as_ref().is_some_and(|provenance| {
            current.get(&provenance.arena_id) == Some(&provenance.compatibility_key)
        });
    }
}

struct ReportEvent {
    sequence: u64,
    elapsed_ms: u64,
    value: serde_json::Value,
}

fn read_report_events(path: &Path) -> Result<Vec<ReportEvent>, ReportError> {
    let source = fs::read_to_string(path)?;
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            let value: serde_json::Value =
                serde_json::from_str(line).map_err(|error| ReportError::EventJson {
                    path: path.to_owned(),
                    line: index + 1,
                    detail: error.to_string(),
                })?;
            Ok(ReportEvent {
                sequence: value
                    .get("sequence")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(u64::try_from(index).unwrap_or(u64::MAX)),
                elapsed_ms: value
                    .get("elapsed_ms")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                value,
            })
        })
        .collect()
}

fn discover_matches(input: &Path) -> Result<Vec<PathBuf>, ReportError> {
    if is_match_dir(input) {
        return Ok(vec![input.to_owned()]);
    }
    let mut matches = Vec::new();
    for entry in fs::read_dir(input)? {
        let path = entry?.path();
        if is_match_dir(&path) {
            matches.push(path);
        }
    }
    matches.sort();
    if matches.is_empty() {
        Err(ReportError::NoMatches(input.to_owned()))
    } else {
        Ok(matches)
    }
}

fn discover_series(input: &Path) -> Result<Vec<PathBuf>, ReportError> {
    if input.join("series.json").is_file() {
        return Ok(vec![input.to_owned()]);
    }
    let mut series = Vec::new();
    for entry in fs::read_dir(input)? {
        let path = entry?.path();
        if path.join("series.json").is_file() {
            series.push(path);
        }
    }
    series.sort();
    if series.is_empty() {
        Err(ReportError::NoSeries(input.to_owned()))
    } else {
        Ok(series)
    }
}

fn discover_benchmarks(input: &Path) -> Result<Vec<PathBuf>, ReportError> {
    if input.join("benchmark.json").is_file() {
        return Ok(vec![input.to_owned()]);
    }
    let mut benchmarks = Vec::new();
    for entry in fs::read_dir(input)? {
        let path = entry?.path();
        if path.join("benchmark.json").is_file() {
            benchmarks.push(path);
        }
    }
    benchmarks.sort();
    if benchmarks.is_empty() {
        Err(ReportError::NoBenchmarks(input.to_owned()))
    } else {
        Ok(benchmarks)
    }
}

fn is_match_dir(path: &Path) -> bool {
    path.join("events.jsonl").is_file() && path.join("world.json").is_file()
}

fn read_world(path: &Path) -> Result<WorldState, ReportError> {
    let source = fs::read(path)?;
    serde_json::from_slice(&source).map_err(|error| ReportError::World {
        path: path.to_owned(),
        detail: error.to_string(),
    })
}

fn read_arena_presentation(path: &Path) -> Result<(Option<ArenaVisualization>, bool), ReportError> {
    if !path.is_file() {
        return Ok((None, false));
    }
    let source = fs::read(path)?;
    let manifest: ArenaManifest =
        serde_json::from_slice(&source).map_err(|error| ReportError::Arena {
            path: path.to_owned(),
            detail: error.to_string(),
        })?;
    let fog = manifest
        .fog_of_war
        .is_some_and(|fog| fog.hide_topology_until_observed);
    Ok((manifest.visualization, fog))
}

fn analyze_agents(
    source: &Path,
    state: &WorldState,
) -> Result<BTreeMap<String, TranscriptAnalysis>, ReportError> {
    let mut analyses = BTreeMap::new();
    for (agent_id, agent) in &state.agents {
        let agent_dir = source.join("agents").join(agent_id);
        for path in ["transcript.json", "transcript.live.json"]
            .into_iter()
            .map(|name| agent_dir.join(name))
            .filter(|path| path.is_file())
        {
            if let Ok(Some(analysis)) =
                analyze_transcript(&path, agent_id, &agent.territory, &agent.model)
            {
                analyses.insert(agent_id.clone(), analysis);
                break;
            }
        }
    }
    Ok(analyses)
}

fn read_series(path: &Path) -> Result<SeriesSummary, ReportError> {
    let source = fs::read(path)?;
    serde_json::from_slice(&source).map_err(|error| ReportError::Series {
        path: path.to_owned(),
        detail: error.to_string(),
    })
}

fn read_benchmark(path: &Path) -> Result<BenchmarkSummary, ReportError> {
    let source = fs::read(path)?;
    serde_json::from_slice(&source).map_err(|error| ReportError::Benchmark {
        path: path.to_owned(),
        detail: error.to_string(),
    })
}

fn copy_public_artifacts(report: &MatchReport) -> Result<(), ReportError> {
    let raw = report.report_dir.join("artifacts");
    fs::create_dir_all(&raw)?;
    fs::copy(report.source.join("events.jsonl"), raw.join("events.jsonl"))?;
    fs::copy(report.source.join("world.json"), raw.join("world.json"))?;
    if report.source.join("match.json").is_file() {
        fs::copy(report.source.join("match.json"), raw.join("match.json"))?;
    }
    if report.source.join("arena.json").is_file() {
        fs::copy(report.source.join("arena.json"), raw.join("arena.json"))?;
    }
    for agent in report.state.agents.keys() {
        let source = report.source.join("agents").join(agent);
        for file in [
            "transcript.json",
            "transcript.live.json",
            "transcript.jsonl",
            "result.json",
        ] {
            let artifact = source.join(file);
            if artifact.is_file() {
                fs::copy(&artifact, raw.join(format!("{}-{file}", safe_name(agent))))?;
            }
        }
        if let Some(analysis) = report.analyses.get(agent) {
            fs::write(
                raw.join(format!("{}-analysis.json", safe_name(agent))),
                serde_json::to_vec_pretty(analysis).map_err(|error| ReportError::Analysis {
                    path: source.join("transcript.json"),
                    detail: error.to_string(),
                })?,
            )?;
        }
    }
    Ok(())
}

fn render_index(
    reports: &[MatchReport],
    series: &[SeriesReport],
    benchmarks: &[BenchmarkReport],
) -> String {
    let benchmark_cards = render_benchmark_cards(benchmarks);
    let benchmark_section = if benchmark_cards.is_empty() {
        String::new()
    } else {
        format!(
            "<section><div class=\"section-heading\"><h2>Benchmarks</h2><p>Model performance across a consistent fleet of infrastructure arenas.</p></div><div class=\"match-list benchmark-list\">{benchmark_cards}</div></section>"
        )
    };
    let current_series = render_series_cards(series.iter().filter(|report| report.current));
    let current_series = if current_series.is_empty() {
        "<p class=\"empty\">No match series recorded yet.</p>".to_owned()
    } else {
        current_series
    };
    let current = render_cards(
        reports
            .iter()
            .filter(|report| report.listed && report.current),
    );
    let archived_matches = reports
        .iter()
        .filter(|report| report.listed && !report.current)
        .count();
    let archived_series = series.iter().filter(|report| !report.current).count();
    let archive_total = archived_matches + archived_series;
    let current = if current.is_empty() {
        "<p class=\"empty\">No matches recorded under the current rules yet.</p>".to_owned()
    } else {
        current
    };
    page(
        "Agents of Empires · Current Season",
        &format!(
            "<header class=\"hero\"><span class=\"eyebrow\">Current season</span><h1>Agents of Empires</h1><p>Build races decided by durable deployments, not confident answers.</p></header><main><section class=\"about\"><span class=\"eyebrow\">About the arena</span><h2>What am I looking at?</h2><p>Agents of Empires drops AI infrastructure agents into identical disposable NixOS machines and gives them the same service contract. A referee checks recovered state, fresh work, service restarts, and host reboots. The first agent to produce a durable deployment wins.</p><p><a href=\"https://github.com/ducks/agents-of-empires\">Read how the arena works and view the source →</a></p></section>{benchmark_section}<section><div class=\"section-heading\"><h2>Series</h2><p>Seat-rotated races that separate agent performance from territory advantage.</p></div><div class=\"match-list series-list\">{current_series}</div></section><section><div class=\"section-heading\"><h2>Current matches</h2><p>Matches sharing the newest manifest and verifier compatibility key for each arena.</p></div><div class=\"match-list\">{current}</div></section><section class=\"archive-callout\"><div><span class=\"eyebrow\">Audit trail</span><h2>Archive</h2><p>Superseded and provenance-free runs remain available without being mixed into current results.</p></div><a class=\"archive-link\" href=\"archive/\">Browse {archive_total} archived run{archive_suffix} →</a></section></main>",
            archive_suffix = if archive_total == 1 { "" } else { "s" },
        ),
    )
}

fn render_archive(reports: &[MatchReport], series: &[SeriesReport]) -> String {
    let historical_series =
        render_archived_series_cards(series.iter().filter(|report| !report.current));
    let historical_matches = render_archived_cards(
        reports
            .iter()
            .filter(|report| report.listed && !report.current),
    );
    let historical_series = if historical_series.is_empty() {
        "<p class=\"empty\">No superseded series recorded.</p>".to_owned()
    } else {
        historical_series
    };
    let historical_matches = if historical_matches.is_empty() {
        "<p class=\"empty\">No historical matches recorded.</p>".to_owned()
    } else {
        historical_matches
    };
    page(
        "Archive · Agents of Empires",
        &format!(
            "<nav><a href=\"../\">← Current season</a><span>Agents of Empires</span></nav><header class=\"hero match-hero\"><span class=\"eyebrow\">Audit trail</span><h1>Archive</h1><p>Prototype, provenance-free, and superseded runs retained for inspection. These results are not directly comparable with the current season.</p></header><main><section><div class=\"section-heading\"><h2>Superseded series</h2><p>Seat-rotated results produced under older arena or verifier compatibility keys.</p></div><div class=\"match-list historical\">{historical_series}</div></section><section><div class=\"section-heading\"><h2>Historical matches</h2><p>Individual runs retained as immutable evidence, not current standings.</p></div><div class=\"match-list historical\">{historical_matches}</div></section></main>"
        ),
    )
}

fn render_benchmark_cards(reports: &[BenchmarkReport]) -> String {
    let mut body = String::new();
    for report in reports {
        let leader = report.summary.standings.first();
        let leader_name = leader.map_or("No leader", |standing| standing.model.as_str());
        let record = leader.map_or_else(
            || "No completed arenas".to_owned(),
            |standing| {
                format!(
                    "{} wins · {}/{} milestones",
                    standing.wins, standing.milestone_passes, standing.milestones_available
                )
            },
        );
        let _ = write!(
            body,
            "<a class=\"match-card benchmark-card\" href=\"benchmarks/{}/\"><div><span class=\"eyebrow\">{} of {} arenas</span><h2>{}</h2><p>Cross-arena model benchmark</p></div><div class=\"metrics\"><strong>{}</strong><span>{}</span></div></a>",
            escape(&report.slug),
            report.summary.arenas_completed,
            report.summary.arenas_requested,
            escape(&report.name),
            escape(leader_name),
            escape(&record),
        );
    }
    body
}

fn render_cards<'a>(reports: impl Iterator<Item = &'a MatchReport>) -> String {
    let mut body = String::new();
    for report in reports {
        let winner = report.state.winner.as_deref().unwrap_or("No winner");
        let total_cost: u64 = report
            .state
            .agents
            .values()
            .map(|agent| agent.cost_microusd)
            .sum();
        let _ = write!(
            body,
            "<a class=\"match-card\" href=\"matches/{}/\"><div><span class=\"eyebrow\">{}</span><h2>{}</h2><p>{}</p></div><div class=\"metrics\"><strong>{}</strong><span>{}</span></div></a>",
            escape(&report.slug),
            escape(&format!("{:?}", report.state.match_state).to_lowercase()),
            escape(&report.name),
            escape(
                report
                    .state
                    .finish_reason
                    .as_deref()
                    .unwrap_or("No finish reason recorded")
            ),
            escape(winner),
            escape(&format!(
                "{} · {}",
                duration(report.state.elapsed_ms),
                money(total_cost)
            ))
        );
    }
    body
}

fn render_archived_cards<'a>(reports: impl Iterator<Item = &'a MatchReport>) -> String {
    let mut body = String::new();
    for report in reports {
        let winner = report.state.winner.as_deref().unwrap_or("No winner");
        let total_cost: u64 = report
            .state
            .agents
            .values()
            .map(|agent| agent.cost_microusd)
            .sum();
        let _ = write!(
            body,
            "<a class=\"match-card\" href=\"../matches/{}/\"><div><span class=\"eyebrow\">{}</span><h2>{}</h2><p>{}</p><small class=\"archive-reason\">{}</small></div><div class=\"metrics\"><strong>{}</strong><span>{}</span></div></a>",
            escape(&report.slug),
            escape(&format!("{:?}", report.state.match_state).to_lowercase()),
            escape(&report.name),
            escape(
                report
                    .state
                    .finish_reason
                    .as_deref()
                    .unwrap_or("No finish reason recorded")
            ),
            escape(&archive_reason(report.provenance.as_ref())),
            escape(winner),
            escape(&format!(
                "{} · {}",
                duration(report.state.elapsed_ms),
                money(total_cost)
            ))
        );
    }
    body
}

fn render_series_cards<'a>(reports: impl Iterator<Item = &'a SeriesReport>) -> String {
    let mut body = String::new();
    for report in reports {
        let leader = report.summary.standings.first();
        let leader_name = leader.map_or("No leader", |standing| standing.agent.as_str());
        let record = leader.map_or_else(
            || "No completed rounds".to_owned(),
            |standing| {
                format!(
                    "{} win{} · {}",
                    standing.wins,
                    if standing.wins == 1 { "" } else { "s" },
                    standing.cost_per_durable_microusd.map_or_else(
                        || "cost unavailable".to_owned(),
                        |cost| { format!("{} per durable", money(cost)) }
                    )
                )
            },
        );
        let _ = write!(
            body,
            "<a class=\"match-card series-card\" href=\"series/{}/\"><div><span class=\"eyebrow\">{} rounds · seat rotated</span><h2>{}</h2><p>{}</p></div><div class=\"metrics\"><strong>{}</strong><span>{}</span></div></a>",
            escape(&report.slug),
            report.summary.rounds_completed,
            escape(&report.name),
            escape(&report.summary.arena_id),
            escape(leader_name),
            escape(&record),
        );
    }
    body
}

fn render_archived_series_cards<'a>(reports: impl Iterator<Item = &'a SeriesReport>) -> String {
    let mut body = String::new();
    for report in reports {
        let leader = report.summary.standings.first();
        let leader_name = leader.map_or("No leader", |standing| standing.agent.as_str());
        let record = leader.map_or_else(
            || "No completed rounds".to_owned(),
            |standing| {
                format!(
                    "{} win{}",
                    standing.wins,
                    if standing.wins == 1 { "" } else { "s" }
                )
            },
        );
        let _ = write!(
            body,
            "<a class=\"match-card series-card\" href=\"../series/{}/\"><div><span class=\"eyebrow\">{} rounds · seat rotated</span><h2>{}</h2><p>{}</p><small class=\"archive-reason\">{}</small></div><div class=\"metrics\"><strong>{}</strong><span>{}</span></div></a>",
            escape(&report.slug),
            report.summary.rounds_completed,
            escape(&report.name),
            escape(&report.summary.arena_id),
            escape(&archive_reason(report.provenance.as_ref())),
            escape(leader_name),
            escape(&record),
        );
    }
    body
}

fn archive_reason(provenance: Option<&MatchProvenance>) -> String {
    provenance.map_or_else(
        || "Archived because provenance is unavailable".to_owned(),
        |value| {
            format!(
                "Superseded manifest or verifier · compatibility {}",
                value
                    .compatibility_key
                    .get(..12)
                    .unwrap_or(&value.compatibility_key)
            )
        },
    )
}

fn render_benchmark_report(report: &BenchmarkReport) -> String {
    let leader = report.summary.standings.first();
    let leader_name = leader.map_or("No leader", |standing| standing.model.as_str());
    let rounds: usize = report
        .summary
        .arenas
        .iter()
        .map(|arena| arena.rounds_completed)
        .sum();
    let all_usage = report
        .summary
        .standings
        .iter()
        .all(|standing| standing.cost_microusd.is_some());
    let total_cost = all_usage.then(|| {
        report
            .summary
            .standings
            .iter()
            .filter_map(|standing| standing.cost_microusd)
            .sum::<u64>()
    });

    let mut standings = String::new();
    for (index, standing) in report.summary.standings.iter().enumerate() {
        let tokens = standing
            .input_tokens
            .zip(standing.output_tokens)
            .map(|(input, output)| input.saturating_add(output));
        let _ = write!(
            standings,
            "<tr class=\"{}\"><td><strong>{}</strong><br><small>{} · {}</small></td><td>{}</td><td>{}/{}</td><td>{}/{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            if index == 0 { "winner-row" } else { "" },
            escape(&standing.model),
            escape(&standing.adapter),
            escape(&standing.reasoning_effort),
            standing.wins,
            standing.durable_deployments,
            standing.appearances,
            standing.milestone_passes,
            standing.milestones_available,
            standing
                .median_durable_ms
                .map_or_else(|| "n/a".to_owned(), duration),
            tokens.map_or_else(|| "n/a".to_owned(), grouped),
            standing
                .cost_microusd
                .map_or_else(|| "n/a".to_owned(), money),
            standing
                .cost_per_durable_microusd
                .map_or_else(|| "n/a".to_owned(), money),
            render_failures(standing),
        );
    }

    let mut arenas = String::new();
    for (index, arena) in report.summary.arenas.iter().enumerate() {
        let leader = arena.standings.first();
        let leader_name = leader.map_or("No leader", |standing| standing.model.as_str());
        let record = leader.map_or_else(
            || "No completed rounds".to_owned(),
            |standing| {
                format!(
                    "{} wins · {}/{} milestones",
                    standing.wins, standing.milestone_passes, standing.milestones_available
                )
            },
        );
        let slug = report.series_slugs.get(index).map_or("", String::as_str);
        let status = if arena.aborted {
            "aborted"
        } else if arena.rounds_completed == arena.rounds_requested {
            "complete"
        } else {
            "in progress"
        };
        let analyzed_rounds = report
            .round_links
            .get(index)
            .into_iter()
            .flatten()
            .filter(|round| round.has_analysis)
            .count();
        let round_links = report.round_links.get(index).map_or_else(String::new, |rounds| {
            rounds
                .iter()
                .map(|round| {
                    format!(
                        "<a class=\"round-analysis-link{}\" href=\"../../matches/{}/\">Round {}{}</a>",
                        if round.has_analysis { " analyzed" } else { "" },
                        escape(&round.slug),
                        round.round,
                        if round.has_analysis { " · How they fought" } else { " · Replay" }
                    )
                })
                .collect::<String>()
        });
        let _ = write!(
            arenas,
            "<article class=\"match-card benchmark-arena-card\"><div class=\"arena-card-head\"><div><span class=\"eyebrow\">{} · {}/{} rounds</span><h2><a href=\"../../series/{}/\">{}</a></h2><p><a href=\"../../series/{}/\">View aggregate seat rotation →</a></p></div><div class=\"metrics\"><strong>{}</strong><span>{}</span></div></div><div class=\"round-analysis-links\"><strong>Match evidence</strong><span>{} of {} rounds include strategy analysis</span><div>{}</div></div></article>",
            status,
            arena.rounds_completed,
            arena.rounds_requested,
            escape(slug),
            escape(&arena.arena_id),
            escape(slug),
            escape(leader_name),
            escape(&record),
            analyzed_rounds,
            arena.rounds_completed,
            round_links,
        );
    }

    let content = format!(
        "<nav><a href=\"../../\">← Archive</a><span>Agents of Empires · benchmark</span></nav>
        <header class=\"hero match-hero\"><span class=\"eyebrow\">Cross-arena benchmark</span><h1>{}</h1><p>One consistent model fleet measured across independent infrastructure contracts.</p>
        <div class=\"hero-stats\"><div><small>Leader</small><strong>{}</strong></div><div><small>Arenas</small><strong>{}/{}</strong></div><div><small>Rounds</small><strong>{}</strong></div><div><small>Recorded cost</small><strong>{}</strong></div></div></header>
        <main><section><div class=\"section-heading\"><h2>Model leaderboard</h2><p>Aggregate results from the verified matches below. The leaderboard is the summary; individual rounds are the evidence.</p></div><div class=\"table-wrap\"><table><thead><tr><th>Model</th><th>Wins</th><th>Durable</th><th>Milestones</th><th>Median</th><th>Tokens</th><th>Cost</th><th>Cost / durable</th><th>Failures</th></tr></thead><tbody>{standings}</tbody></table></div></section>
        <section><div class=\"section-heading\"><h2>Arenas and match evidence</h2><p>Open a round directly for its replay, audit trail, and How they fought strategy analysis.</p></div><div class=\"match-list benchmark-arenas\">{arenas}</div></section>
        <footer><a href=\"artifacts/benchmark.json\">benchmark.json</a></footer></main>",
        escape(&report.name),
        escape(leader_name),
        report.summary.arenas_completed,
        report.summary.arenas_requested,
        rounds,
        total_cost.map_or_else(|| "n/a".to_owned(), money),
    );
    page(&format!("{} · Agents of Empires", report.name), &content)
}

fn render_failures(standing: &BenchmarkStanding) -> String {
    if standing.failures.is_empty() {
        return "—".into();
    }
    standing
        .failures
        .iter()
        .map(|(source, count)| format!("{} {count}", escape(source)))
        .collect::<Vec<_>>()
        .join("<br>")
}

fn render_series(report: &SeriesReport) -> String {
    let leader = report.summary.standings.first();
    let leader_name = leader.map_or("No leader", |standing| standing.agent.as_str());
    let all_usage = report.summary.standings.iter().all(|standing| {
        standing.input_tokens.is_some()
            && standing.output_tokens.is_some()
            && standing.cost_microusd.is_some()
    });
    let total_cost = all_usage.then(|| {
        report
            .summary
            .standings
            .iter()
            .filter_map(|standing| standing.cost_microusd)
            .sum::<u64>()
    });
    let total_tokens = all_usage.then(|| {
        report
            .summary
            .standings
            .iter()
            .filter_map(|standing| {
                standing
                    .input_tokens
                    .zip(standing.output_tokens)
                    .map(|(input, output)| input.saturating_add(output))
            })
            .sum::<u64>()
    });

    let mut standings = String::new();
    for (index, standing) in report.summary.standings.iter().enumerate() {
        let tokens = standing
            .input_tokens
            .zip(standing.output_tokens)
            .map(|(input, output)| input.saturating_add(output));
        let _ = write!(
            standings,
            "<tr class=\"{}\"><td><strong>{}</strong><small>{}</small></td><td>{}</td><td>{}/{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}/{}</td></tr>",
            if index == 0 { "winner-row" } else { "" },
            escape(&standing.agent),
            escape(&standing.model),
            standing.wins,
            standing.durable_deployments,
            standing.appearances,
            standing
                .median_durable_ms
                .map_or_else(|| "n/a".to_owned(), duration),
            tokens.map_or_else(|| "n/a".to_owned(), grouped),
            standing
                .cost_microusd
                .map_or_else(|| "n/a".to_owned(), money),
            standing
                .cost_per_durable_microusd
                .map_or_else(|| "n/a".to_owned(), money),
            standing.usage_recorded,
            standing.appearances,
        );
    }

    let territories: BTreeSet<_> = report
        .summary
        .rounds
        .iter()
        .flat_map(|round| round.seats.values().cloned())
        .collect();
    let territory_headers = territories
        .iter()
        .fold(String::new(), |mut body, territory| {
            let _ = write!(body, "<th>{}</th>", escape(territory));
            body
        });
    let models: std::collections::HashMap<_, _> = report
        .summary
        .standings
        .iter()
        .map(|standing| (standing.agent.as_str(), standing.model.as_str()))
        .collect();
    let mut rounds = String::new();
    for (index, round) in report.summary.rounds.iter().enumerate() {
        let seats: std::collections::HashMap<_, _> = round
            .seats
            .iter()
            .map(|(agent, territory)| (territory.as_str(), agent.as_str()))
            .collect();
        let mut cells = String::new();
        for territory in &territories {
            let agent = seats
                .get(territory.as_str())
                .copied()
                .unwrap_or("unassigned");
            let class = if round.winner_territory.as_deref() == Some(territory.as_str()) {
                "seat-winner"
            } else {
                ""
            };
            let _ = write!(
                cells,
                "<td class=\"{class}\"><strong>{}</strong><small>{}</small></td>",
                escape(agent),
                escape(models.get(agent).copied().unwrap_or("unknown")),
            );
        }
        let round_slug = report.round_slugs.get(index).map_or("", String::as_str);
        let _ = write!(
            rounds,
            "<tr><td><a href=\"../../matches/{}/\">Round {}</a><small>{}</small></td>{cells}<td><strong>{}</strong><small>{}</small></td></tr>",
            escape(round_slug),
            round.round,
            duration(round.duration_ms),
            escape(round.winner_agent.as_deref().unwrap_or("No winner")),
            escape(round.winner_territory.as_deref().unwrap_or("unfinished")),
        );
    }

    let provenance = report.provenance.as_ref().map_or_else(
        || "historical · provenance unavailable".to_owned(),
        |value| {
            format!(
                "{} · {} · {}",
                if report.current {
                    "current"
                } else {
                    "historical"
                },
                value.arena_id,
                value
                    .compatibility_key
                    .get(..12)
                    .unwrap_or(&value.compatibility_key)
            )
        },
    );
    let content = format!(
        "<nav><a href=\"../../\">← Current season</a><span>Agents of Empires · {}</span></nav>
        <header class=\"hero match-hero\"><span class=\"eyebrow\">Seat-rotated series · {}</span><h1>{}</h1><p>Every agent races the same verifier from every territory. Failed attempts remain in total spend.</p>
        <div class=\"hero-stats\"><div><small>Leader</small><strong>{}</strong></div><div><small>Rounds</small><strong>{}/{}</strong></div><div><small>Recorded cost</small><strong>{}</strong></div><div><small>Tokens</small><strong>{}</strong></div></div></header>
        <main><section><div class=\"section-heading\"><h2>Battle card</h2><p>Ranked by wins, then durable deployments, time, and total cost.</p></div><div class=\"table-wrap\"><table><thead><tr><th>Agent</th><th>Wins</th><th>Durable</th><th>Median</th><th>Tokens</th><th>Cost</th><th>Cost / durable</th><th>Usage</th></tr></thead><tbody>{standings}</tbody></table></div></section>
        <section><div class=\"section-heading\"><h2>Seat rotation</h2><p>The highlighted cell won that round. Open any round for its replay and immutable event log.</p></div><div class=\"table-wrap\"><table class=\"seat-matrix\"><thead><tr><th>Round</th>{territory_headers}<th>Winner</th></tr></thead><tbody>{rounds}</tbody></table></div></section>
        <footer><a href=\"artifacts/series.json\">series.json</a></footer></main>",
        escape(&provenance),
        escape(&report.summary.arena_id),
        escape(&report.name),
        escape(leader_name),
        report.summary.rounds_completed,
        report.summary.rounds_requested,
        total_cost.map_or_else(|| "n/a".to_owned(), money),
        total_tokens.map_or_else(|| "n/a".to_owned(), grouped),
    );
    page(&format!("{} · Agents of Empires", report.name), &content)
}

fn render_match(report: &MatchReport) -> String {
    let state = &report.state;
    let winner = state.winner.as_deref().unwrap_or("No winner");
    let mut territories = String::new();
    for (id, territory) in &state.territories {
        let milestones = territory
            .milestones
            .values()
            .filter(|item| item.passed)
            .count();
        let total = territory.milestones.len();
        let winner_class = if state.winner.as_deref() == Some(id) {
            " winner-row"
        } else {
            ""
        };
        let _ = write!(
            territories,
            "<tr class=\"{winner_class}\"><td><strong>{}</strong><small>{}</small></td><td>{}</td><td>{milestones}/{total}</td><td>{}</td><td>{}</td></tr>",
            escape(id),
            escape(territory.agent.as_deref().unwrap_or("unassigned")),
            escape(&territory.competitor_state.map_or_else(
                || "n/a".to_owned(),
                |state| format!("{state:?}").to_lowercase()
            ),),
            territory.milestone_points,
            territory.durable_at_ms.map_or_else(|| "—".into(), duration)
        );
    }

    let mut agents = String::new();
    for (id, agent) in &state.agents {
        let terminal = agent.terminal_state.map_or_else(
            || "running".to_owned(),
            |value| format!("{value:?}").to_lowercase(),
        );
        let artifact = transcript_link(report, id);
        let usage_known = report.events.iter().any(|event| {
            event.value.get("kind").and_then(serde_json::Value::as_str) == Some("usage_charged")
                && event.value.get("agent").and_then(serde_json::Value::as_str) == Some(id)
                && ["input_tokens", "output_tokens", "cost_microusd"]
                    .iter()
                    .any(|field| event.value.get(field).is_some_and(|value| !value.is_null()))
        });
        let _ = write!(
            agents,
            "<tr><td><strong>{}</strong><small>{}</small></td><td>{}</td><td><span class=\"pill {}\">{}</span></td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            escape(id),
            escape(&agent.territory),
            escape(&agent.model),
            terminal_class(agent.terminal_state),
            escape(&terminal),
            if usage_known {
                grouped(agent.input_tokens)
            } else {
                "n/a".into()
            },
            if usage_known {
                grouped(agent.output_tokens)
            } else {
                "n/a".into()
            },
            if usage_known {
                money(agent.cost_microusd)
            } else {
                "n/a".into()
            },
            artifact
        );
    }

    let total_cost: u64 = state.agents.values().map(|agent| agent.cost_microusd).sum();
    let total_tokens: u64 = state
        .agents
        .values()
        .map(|agent| agent.input_tokens.saturating_add(agent.output_tokens))
        .sum();
    let replay = render_replay(report);
    let analysis = render_agent_analysis(report);
    let timeline = render_timeline(&report.events);
    let arena_artifact = if report.source.join("arena.json").is_file() {
        "<a href=\"artifacts/arena.json\">arena.json</a>"
    } else {
        ""
    };
    let provenance = report.provenance.as_ref().map_or_else(
        || "historical · provenance unavailable".to_owned(),
        |value| {
            format!(
                "{} · {} · {}",
                if report.current {
                    "current"
                } else {
                    "historical"
                },
                value.arena_id,
                value
                    .compatibility_key
                    .get(..12)
                    .unwrap_or(&value.compatibility_key)
            )
        },
    );
    let content = format!(
        "<nav><a href=\"../../\">← All matches</a><span>Agents of Empires</span></nav>
        <header class=\"hero match-hero\"><span class=\"eyebrow\">{:?} · {}</span><h1>{}</h1><p>{}</p>
        <div class=\"hero-stats\"><div><small>Winner</small><strong>{}</strong></div><div><small>Durable in</small><strong>{}</strong></div><div><small>Recorded cost</small><strong>{}</strong></div><div><small>Tokens</small><strong>{}</strong></div></div></header>
        <main>{replay}{analysis}<section><div class=\"section-heading\"><h2>Territories</h2><p>The referee-owned result frozen when the first deployment became durable.</p></div><div class=\"table-wrap\"><table><thead><tr><th>Territory</th><th>State</th><th>Milestones</th><th>Points</th><th>Durable at</th></tr></thead><tbody>{territories}</tbody></table></div></section>
        <section><div class=\"section-heading\"><h2>Agents</h2><p>Usage includes cumulative checkpoints captured while agents were still running.</p></div><div class=\"table-wrap\"><table><thead><tr><th>Agent</th><th>Model</th><th>Outcome</th><th>Input</th><th>Output</th><th>Cost</th><th>Artifact</th></tr></thead><tbody>{agents}</tbody></table></div></section>
        <section><div class=\"section-heading\"><h2>Event timeline</h2><p>{} immutable events. The match clock remains frozen during post-match collection.</p></div><ol class=\"timeline\">{timeline}</ol></section>
        <footer><a href=\"artifacts/events.jsonl\">events.jsonl</a><a href=\"artifacts/world.json\">world.json</a>{arena_artifact}</footer></main>",
        state.match_state,
        escape(state.finish_reason.as_deref().unwrap_or("unfinished")),
        escape(&report.name),
        escape(&format!("{} won: {}", winner, state.finish_reason.as_deref().unwrap_or("outcome recorded"))),
        escape(winner),
        duration(state.elapsed_ms),
        money(total_cost),
        grouped(total_tokens),
        report.events.len()
    );
    page(
        &format!("{} · Agents of Empires", report.name),
        &content.replace(
            "Agents of Empires</span>",
            &format!("Agents of Empires · {}</span>", escape(&provenance)),
        ),
    )
}

fn transcript_link(report: &MatchReport, agent: &str) -> String {
    let mut links = Vec::new();
    for file in [
        "transcript.json",
        "transcript.live.json",
        "transcript.jsonl",
        "result.json",
    ] {
        if report
            .source
            .join("agents")
            .join(agent)
            .join(file)
            .is_file()
        {
            links.push(format!(
                "<a href=\"artifacts/{}-{}\">transcript</a>",
                escape(&safe_name(agent)),
                file
            ));
            break;
        }
    }
    if report.analyses.contains_key(agent) {
        links.push(format!(
            "<a href=\"artifacts/{}-analysis.json\">analysis</a>",
            escape(&safe_name(agent))
        ));
    }
    if links.is_empty() {
        "—".to_owned()
    } else {
        links.join(" · ")
    }
}

fn render_agent_analysis(report: &MatchReport) -> String {
    if report.analyses.is_empty() {
        return String::new();
    }
    let mut rows = String::new();
    let mut cards = String::new();
    for (agent_id, analysis) in &report.analyses {
        let architecture = analysis
            .architecture
            .technologies
            .iter()
            .chain(&analysis.architecture.service_units)
            .take(8)
            .map(|value| format!("<span>{}</span>", escape(value)))
            .collect::<String>();
        let first_change = analysis
            .metrics
            .first_mutation_after_ms
            .map_or_else(|| "n/a".to_owned(), duration);
        let _ = write!(
            rows,
            "<tr><td><strong>{}</strong><small>{}</small></td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td><div class=\"architecture-tags\">{}</div></td></tr>",
            escape(agent_id),
            escape(&analysis.model),
            first_change,
            analysis.metrics.discoveries,
            analysis.metrics.mutations,
            analysis.metrics.lifecycle_actions,
            analysis.metrics.validations,
            analysis.metrics.tool_errors,
            architecture
        );

        let mut actions = String::new();
        for action in analysis.actions.iter().take(16) {
            let status = if action.success { "ok" } else { "error" };
            let description = action
                .description
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(&action.command);
            let _ = write!(
                actions,
                "<li class=\"{}\"><time>{}</time><span>{:?}</span><strong>{}</strong><code>{}</code></li>",
                status,
                duration(action.started_after_ms),
                action.kind,
                escape(description),
                escape(&action.command)
            );
        }
        let omitted = analysis.actions.len().saturating_sub(16);
        let omitted = if omitted == 0 {
            String::new()
        } else {
            format!("<p class=\"analysis-omitted\">{omitted} more actions in analysis.json</p>")
        };
        let _ = write!(
            cards,
            "<details class=\"analysis-card\"><summary><strong>{}</strong><span>{} observed tool calls</span></summary><ol>{}</ol>{}</details>",
            escape(agent_id),
            analysis.metrics.tool_calls,
            actions,
            omitted
        );
    }
    format!(
        "<section><div class=\"section-heading\"><h2>How they fought</h2><p>Deterministic analysis of observable tool calls. Private model reasoning is never parsed.</p></div><div class=\"table-wrap\"><table><thead><tr><th>Agent</th><th>First change</th><th>Discovery</th><th>Changes</th><th>Lifecycle</th><th>Checks</th><th>Errors</th><th>Architecture seen</th></tr></thead><tbody>{rows}</tbody></table></div><div class=\"analysis-cards\">{cards}</div></section>"
    )
}

fn render_replay(report: &MatchReport) -> String {
    let lanes: Vec<_> = report
        .state
        .territories
        .iter()
        .map(|(territory, state)| {
            let agent = state.agent.as_deref().unwrap_or("unassigned");
            let model = report
                .state
                .agents
                .get(agent)
                .map_or("unknown", |agent| agent.model.as_str());
            serde_json::json!({
                "territory": territory,
                "agent": agent,
                "model": model,
                "winner": report.state.winner.as_deref() == Some(territory),
            })
        })
        .collect();
    let activity: Vec<_> = report
        .analyses
        .values()
        .flat_map(|analysis| {
            analysis.actions.iter().map(|action| {
                serde_json::json!({
                    "territory": analysis.territory,
                    "agent": analysis.agent,
                    "started_after_ms": action.started_after_ms,
                    "duration_ms": action.duration_ms,
                    "kind": action.kind,
                    "description": action.description,
                    "command": action.command,
                    "success": action.success,
                })
            })
        })
        .collect();
    let payload = serde_json::json!({
        "duration": report.state.elapsed_ms,
        "lanes": lanes,
        "topology": report.visualization,
        "fog_of_war": report.fog_of_war,
        "events": report.events.iter().map(|event| &event.value).collect::<Vec<_>>(),
        "stalls": replay_stalls(&report.events, report.state.elapsed_ms),
        "activity": activity,
    });
    let payload = serde_json::to_string(&payload)
        .unwrap_or_else(|_| "{\"duration\":0,\"lanes\":[],\"events\":[]}".to_owned())
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");
    format!(
        r#"<section class="replay" data-match-replay><div class="section-heading"><div><span class="eyebrow">Match replay</span><h2>Watch the race unfold</h2></div><p>Play the referee's event stream or scrub directly to any moment.</p></div>
        <div class="replay-shell"><div class="replay-controls"><button type="button" data-play>Play</button><strong data-clock>0:00</strong><input data-scrubber aria-label="Match clock" type="range" min="0" max="{duration}" value="0" step="100"><select data-speed aria-label="Playback speed"><option value="1">1×</option><option value="5">5×</option><option value="20" selected>20×</option><option value="60">60×</option></select></div><div class="topology" data-topology hidden></div><div class="replay-ruler"><span>Start</span><span>{finish}</span></div><div class="replay-lanes" data-lanes></div><aside class="replay-inspector" data-inspector><span class="eyebrow">Selected event</span><strong>Press play or select a marker</strong><p>Milestones, state changes, usage, and terminal outcomes appear on the shared clock.</p></aside></div>
        <script type="application/json" data-replay-data>{payload}</script><script>{REPLAY_SCRIPT}</script><script>{FOG_REPLAY_SCRIPT}</script></section>"#,
        duration = report.state.elapsed_ms,
        finish = duration(report.state.elapsed_ms),
    )
}

fn replay_stalls(events: &[ReportEvent], match_end_ms: u64) -> Vec<serde_json::Value> {
    let mut active = std::collections::BTreeMap::<(String, String), (u64, u64, String)>::new();
    let mut stalls = Vec::new();
    for event in events {
        let kind = event.value.get("kind").and_then(serde_json::Value::as_str);
        let territory = event
            .value
            .get("territory")
            .and_then(serde_json::Value::as_str);
        let milestone = event
            .value
            .get("milestone")
            .and_then(serde_json::Value::as_str);
        let (Some(territory), Some(milestone)) = (territory, milestone) else {
            continue;
        };
        let key = (territory.to_owned(), milestone.to_owned());
        match kind {
            Some("milestone_failed") => {
                let detail = event
                    .value
                    .get("detail")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("verification failed")
                    .to_owned();
                active
                    .entry(key)
                    .and_modify(|entry| {
                        entry.1 = entry.1.saturating_add(1);
                        entry.2.clone_from(&detail);
                    })
                    .or_insert((event.elapsed_ms, 1, detail));
            }
            Some("milestone_passed") => {
                if let Some((start_ms, retries, detail)) = active.remove(&key)
                    && retries > 1
                {
                    stalls.push(serde_json::json!({
                        "territory": territory,
                        "milestone": milestone,
                        "start_ms": start_ms,
                        "end_ms": event.elapsed_ms,
                        "retries": retries,
                        "detail": detail,
                        "resolved": true,
                    }));
                }
            }
            _ => {}
        }
    }
    for ((territory, milestone), (start_ms, retries, detail)) in active {
        if retries > 1 {
            stalls.push(serde_json::json!({
                "territory": territory,
                "milestone": milestone,
                "start_ms": start_ms,
                "end_ms": match_end_ms,
                "retries": retries,
                "detail": detail,
                "resolved": false,
            }));
        }
    }
    stalls
}

fn render_timeline(events: &[ReportEvent]) -> String {
    let mut timeline = String::new();
    for envelope in events {
        let kind = envelope
            .value
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("event");
        let actor = ["territory", "agent", "component"]
            .iter()
            .find_map(|key| envelope.value.get(key).and_then(serde_json::Value::as_str))
            .unwrap_or("");
        let detail = compact_event(&envelope.value);
        let _ = write!(
            timeline,
            "<li><time>{}</time><div><strong>{}</strong>{}<details><summary>event #{}</summary><pre>{}</pre></details></div></li>",
            duration(envelope.elapsed_ms),
            escape(&humanize(kind)),
            if actor.is_empty() {
                String::new()
            } else {
                format!("<span>{}</span>", escape(actor))
            },
            envelope.sequence,
            escape(&detail)
        );
    }
    timeline
}

fn compact_event<T: Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_owned())
}

fn terminal_class(state: Option<AgentTerminalState>) -> &'static str {
    match state {
        Some(AgentTerminalState::Completed) => "good",
        Some(AgentTerminalState::Interrupted) => "warn",
        Some(AgentTerminalState::Failed | AgentTerminalState::Terminated) => "bad",
        None => "neutral",
    }
}

fn safe_name(value: &str) -> String {
    let name: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect();
    name.trim_matches('-').to_owned()
}

fn duration(milliseconds: u64) -> String {
    let total_seconds = milliseconds / 1_000;
    format!("{}:{:02}", total_seconds / 60, total_seconds % 60)
}

fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut output = String::new();
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(character);
    }
    output
}

fn money(microusd: u64) -> String {
    format!("${:.4}", microusd as f64 / 1_000_000.0)
}

fn humanize(value: &str) -> String {
    let mut result = value.replace('_', " ");
    if let Some(first) = result.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    result
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn page(title: &str, content: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"color-scheme\" content=\"dark\"><title>{}</title><style>{}{}{}{}{}</style></head><body>{}</body></html>",
        escape(title),
        STYLE,
        REPLAY_STYLE,
        FOG_STYLE,
        ANALYSIS_STYLE,
        ACTIVITY_STYLE,
        content
    )
}

const STYLE: &str = r#"
:root{--bg:#10130f;--panel:#191e18;--line:#333d31;--text:#edf4e9;--muted:#9ca997;--gold:#e7bb55;--green:#85d68b;--red:#ef857c;--orange:#e5a65f}*{box-sizing:border-box}body{margin:0;background:radial-gradient(circle at 75% 0,#253120 0,transparent 32rem),var(--bg);color:var(--text);font:16px/1.5 ui-monospace,SFMono-Regular,Menlo,monospace}a{color:var(--gold)}nav,main,.hero{width:min(1120px,calc(100% - 2rem));margin:auto}nav{display:flex;justify-content:space-between;padding:1.25rem 0;color:var(--muted)}nav a{text-decoration:none}.hero{padding:6rem 0 3rem}.match-hero{padding-top:3.5rem}.eyebrow{color:var(--gold);font-size:.75rem;letter-spacing:.14em;text-transform:uppercase}h1{font-family:Georgia,serif;font-size:clamp(2.7rem,8vw,6.8rem);line-height:.9;margin:.35rem 0 1.2rem;max-width:900px}h2{font-family:Georgia,serif;font-size:2rem;margin:0}.hero>p,.section-heading p{color:var(--muted);max-width:680px}.about{border:1px solid var(--line);background:linear-gradient(135deg,#20271e,var(--panel));padding:2rem}.about h2{margin:.35rem 0 1rem}.about p{color:var(--muted);max-width:850px}.about p:last-child{margin-bottom:0}.about a{text-decoration:none}.hero-stats{display:grid;grid-template-columns:repeat(4,1fr);gap:1px;margin-top:3rem;background:var(--line);border:1px solid var(--line)}.hero-stats div{background:var(--panel);padding:1.25rem}.hero-stats small,td small{display:block;color:var(--muted);margin-bottom:.35rem}.hero-stats strong{font-size:1.25rem}section{margin:1rem 0 4rem}.section-heading{display:flex;align-items:end;justify-content:space-between;gap:2rem;margin-bottom:1rem}.section-heading p{margin:0}.table-wrap{overflow:auto;border:1px solid var(--line)}table{border-collapse:collapse;width:100%;background:var(--panel)}th,td{padding:1rem;text-align:left;border-bottom:1px solid var(--line);vertical-align:top}th{color:var(--muted);font-size:.75rem;text-transform:uppercase;letter-spacing:.08em}.winner-row,.seat-winner{background:#252b19}.seat-winner{box-shadow:inset 0 0 0 1px var(--gold)}.pill{display:inline-block;border:1px solid var(--line);border-radius:99px;padding:.15rem .55rem;font-size:.8rem}.pill.good{color:var(--green)}.pill.warn{color:var(--orange)}.pill.bad{color:var(--red)}.timeline{list-style:none;padding:0;border-top:1px solid var(--line)}.timeline li{display:grid;grid-template-columns:6rem 1fr;gap:1rem;padding:1rem 0;border-bottom:1px solid var(--line)}.timeline time{color:var(--gold)}.timeline span{color:var(--muted);margin-left:.75rem}.timeline details{margin-top:.4rem;color:var(--muted)}pre{white-space:pre-wrap;word-break:break-word;background:#090b09;padding:1rem;overflow:auto}.match-list{display:grid;gap:1rem}.match-card{display:flex;justify-content:space-between;gap:2rem;padding:1.5rem;border:1px solid var(--line);background:var(--panel);text-decoration:none;color:var(--text)}.match-card:hover{border-color:var(--gold)}.match-card h2{font-size:1.5rem}.match-card p{color:var(--muted);margin:.25rem 0 0}.metrics{text-align:right}.metrics strong,.metrics span{display:block}.metrics span{color:var(--muted)}.archive-callout{display:flex;align-items:center;justify-content:space-between;gap:2rem;border:1px solid var(--line);background:linear-gradient(135deg,#20271e,var(--panel));padding:2rem}.archive-callout p{color:var(--muted);margin:.35rem 0 0;max-width:680px}.archive-link{white-space:nowrap;text-decoration:none;border:1px solid var(--gold);padding:.75rem 1rem}.archive-reason{display:block;color:var(--orange);margin-top:.75rem}footer{display:flex;gap:1rem;padding:2rem 0 5rem;border-top:1px solid var(--line)}@media(max-width:720px){.hero{padding-top:3rem}.hero-stats{grid-template-columns:1fr 1fr}.section-heading{display:block}.timeline li{grid-template-columns:4rem 1fr}.match-card,.archive-callout{display:block}.archive-link{display:inline-block;margin-top:1rem}.metrics{text-align:left;margin-top:1rem}th,td{padding:.75rem}}
"#;

const REPLAY_STYLE: &str = r#"
button,select,input{font:inherit}.replay-shell{border:1px solid var(--line);background:linear-gradient(145deg,#1d241b,var(--panel));padding:1.25rem}.replay-controls{display:grid;grid-template-columns:auto 4rem 1fr auto;gap:1rem;align-items:center}.replay-controls button,.replay-controls select{color:var(--text);background:#0c0f0b;border:1px solid var(--line);padding:.55rem .8rem}.replay-controls button{color:var(--gold);cursor:pointer}.replay-controls input{accent-color:var(--gold);width:100%}.replay-hud,.replay-ruler{display:flex;justify-content:space-between;color:var(--muted);font-size:.75rem}.replay-hud{margin:.75rem 0;border-top:1px solid var(--line);padding-top:.75rem}.replay-ruler{margin:1rem 0 .35rem;padding-left:15.5rem}.replay-lane{display:grid;grid-template-columns:14rem 1fr;gap:1.5rem;align-items:center;padding:1rem 0;border-top:1px solid var(--line)}.lane-label strong,.lane-label small,.lane-label span{display:block}.lane-label small{color:var(--muted);overflow:hidden;text-overflow:ellipsis}.lane-label span{color:var(--gold);font-size:.75rem;margin-top:.3rem}.lane-track{height:2.6rem;position:relative;background:#0d100c;border:1px solid #293126}.lane-progress{position:absolute;inset:0 auto 0 0;width:0;background:linear-gradient(90deg,#39452eaa,#68603388);transition:width .08s linear}.lane-stall{position:absolute;top:.28rem;height:2rem;padding:0 .4rem;border:1px solid #cb6e62;border-radius:.25rem;background:repeating-linear-gradient(135deg,#8f403c88 0,#8f403c88 6px,#6d312d88 6px,#6d312d88 12px);color:#ffe2dc;font-size:.68rem;line-height:1.8rem;text-align:center;overflow:hidden;cursor:pointer;opacity:.18;white-space:nowrap}.lane-stall.visible{opacity:.85}.lane-stall.resolved{border-color:var(--orange);background:repeating-linear-gradient(135deg,#895b3188 0,#895b3188 6px,#60422488 6px,#60422488 12px)}.lane-stall.selected{outline:2px solid var(--gold);z-index:2}.lane-marker{position:absolute;top:50%;translate:-50% -50%;width:.85rem;height:.85rem;padding:0;border:2px solid var(--panel);border-radius:50%;background:var(--muted);cursor:pointer;opacity:.2;transition:opacity .15s,scale .15s}.lane-marker.visible{opacity:1}.lane-marker:hover,.lane-marker.selected{scale:1.5;z-index:3}.lane-marker.milestone_passed,.lane-marker.durable_deployment_completed{background:var(--green)}.lane-marker.competitor_state_changed{background:var(--gold)}.lane-marker.agent_interrupted,.lane-marker.agent_terminated,.lane-marker.agent_failed{background:var(--red)}.lane-marker.usage_charged{background:var(--orange)}.replay-inspector{min-height:7rem;border-top:1px solid var(--line);margin-top:.5rem;padding:1rem 0 0}.replay-inspector strong{display:block;margin:.3rem 0}.replay-inspector p{color:var(--muted);margin:.25rem 0}.replay-inspector pre{max-height:18rem}.topology{margin:1.25rem 0 1.75rem}.topology-header{display:flex;justify-content:space-between;align-items:center;margin-bottom:.7rem;color:var(--muted);font-size:.75rem}.topology-legend{display:flex;gap:.75rem;flex-wrap:wrap}.topology-legend span::before{content:"";display:inline-block;width:.55rem;height:.55rem;border-radius:50%;margin-right:.3rem;background:var(--muted)}.topology-legend .verifying::before{background:var(--gold)}.topology-legend .healthy::before{background:var(--green)}.topology-legend .failed::before{background:var(--red)}.topology-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(285px,1fr));gap:.8rem}.topology-card{border:1px solid var(--line);background:#0d100c}.topology-card>header{display:flex;justify-content:space-between;gap:.5rem;padding:.65rem .8rem;border-bottom:1px solid var(--line)}.topology-card>header small{color:var(--muted);overflow:hidden;text-overflow:ellipsis}.topology-board{position:relative;aspect-ratio:16/10;min-height:190px;background:radial-gradient(circle at 50% 50%,#273022 0,transparent 65%)}.topology-links{position:absolute;inset:0;width:100%;height:100%;overflow:visible}.topology-link{stroke:#64705f;stroke-width:1.2;stroke-dasharray:3 2}.topology-link.replication{stroke:var(--gold)}.topology-link.lifecycle{stroke:var(--orange);stroke-dasharray:1 3}.topology-node{position:absolute;translate:-50% -50%;min-width:4.5rem;max-width:6.5rem;padding:.4rem .45rem;border:1px solid #64705f;border-radius:.3rem;background:#20261e;color:var(--text);text-align:center;transition:border-color .15s,background .15s,box-shadow .15s}.topology-node b,.topology-node small{display:block;overflow:hidden;text-overflow:ellipsis}.topology-node b{font-size:.72rem;white-space:nowrap}.topology-node small{color:var(--muted);font-size:.58rem;text-transform:uppercase}.topology-node.verifying{border-color:var(--gold);box-shadow:0 0 0 2px #e7bb5533}.topology-node.healthy{border-color:var(--green);background:#1b3520}.topology-node.failed{border-color:var(--red);background:#3b1d1a}.topology-node.durable{border-color:var(--gold);background:#343019;box-shadow:0 0 0 2px #e7bb5544}@media(max-width:720px){.replay-controls{grid-template-columns:auto 3.5rem 1fr}.replay-controls select{grid-column:1/-1}.replay-ruler{padding-left:0}.replay-lane{grid-template-columns:1fr;gap:.5rem}.topology-grid{grid-template-columns:1fr}.topology-header{display:block}.topology-legend{margin-top:.4rem}}
"#;

const FOG_STYLE: &str = r"
.topology-link.fogged{opacity:0}.topology-node.unknown{border-style:dashed;opacity:.62}
";

const ANALYSIS_STYLE: &str = r"
.architecture-tags{display:flex;flex-wrap:wrap;gap:.3rem;min-width:10rem}.architecture-tags span{border:1px solid var(--line);border-radius:99px;padding:.08rem .4rem;color:var(--muted);font-size:.68rem}.analysis-cards{display:grid;gap:.75rem;margin-top:1rem}.analysis-card{border:1px solid var(--line);background:var(--panel)}.analysis-card summary{display:flex;justify-content:space-between;cursor:pointer;padding:1rem}.analysis-card summary span{color:var(--muted)}.analysis-card ol{list-style:none;padding:0;margin:0;border-top:1px solid var(--line)}.analysis-card li{display:grid;grid-template-columns:4.5rem 6rem minmax(12rem,1fr) minmax(18rem,2fr);gap:.75rem;padding:.7rem 1rem;border-bottom:1px solid var(--line);align-items:start}.analysis-card li.error{border-left:3px solid var(--red)}.analysis-card time{color:var(--gold)}.analysis-card li>span{color:var(--muted);font-size:.75rem;text-transform:lowercase}.analysis-card code{color:var(--muted);white-space:pre-wrap;word-break:break-word;font-size:.75rem}.analysis-omitted{color:var(--muted);padding:0 1rem 1rem}.benchmark-arena-card{display:block;padding:0}.arena-card-head{display:flex;justify-content:space-between;gap:2rem;padding:1.5rem}.arena-card-head h2 a{color:var(--text);text-decoration:none}.arena-card-head p a{text-decoration:none}.round-analysis-links{padding:1rem 1.5rem 1.5rem;border-top:1px solid var(--line)}.round-analysis-links>strong,.round-analysis-links>span{display:block}.round-analysis-links>span{color:var(--muted);font-size:.78rem;margin:.2rem 0 .8rem}.round-analysis-links>div{display:flex;flex-wrap:wrap;gap:.5rem}.round-analysis-link{border:1px solid var(--line);padding:.45rem .65rem;text-decoration:none;color:var(--muted);font-size:.78rem}.round-analysis-link.analyzed{border-color:#53634e;color:var(--green)}.round-analysis-link:hover{border-color:var(--gold);color:var(--gold)}@media(max-width:820px){.analysis-card li{grid-template-columns:4rem 1fr}.analysis-card li strong,.analysis-card li code{grid-column:1/-1}.arena-card-head{display:block}.arena-card-head .metrics{text-align:left;margin-top:1rem}}
";

const ACTIVITY_STYLE: &str = r"
.agent-terminal{height:9.5rem;border-top:1px solid #293126;background:linear-gradient(#070a07ee,#090d09ee),repeating-linear-gradient(0deg,transparent 0,transparent 2px,#5aff7d08 3px);color:#93e89f;font:11px/1.4 ui-monospace,SFMono-Regular,Menlo,monospace;overflow:hidden}.terminal-title{height:1.7rem;display:flex;align-items:center;gap:.3rem;padding:0 .55rem;border-bottom:1px solid #1f2b20;background:#111611;color:#829081}.terminal-title>span{width:.48rem;height:.48rem;border-radius:50%;background:#536052}.terminal-title>span:first-child{background:#a9564e}.terminal-title>span:nth-child(2){background:#ae8a42}.terminal-title strong{margin-left:.25rem;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-weight:500}.terminal-lines{height:7.8rem;padding:.45rem .55rem;overflow:hidden}.terminal-line{display:grid;grid-template-columns:2.8rem 1fr;gap:.45rem;min-height:1.4rem}.terminal-line time{color:#58715c}.terminal-line span{white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.terminal-line b{color:#d5a957;font-weight:500;text-transform:lowercase}.terminal-line.error span{color:#ef857c}.terminal-line.idle span{color:#78927d}.terminal-line.active span{color:#b6e8bc}.terminal-cursor{display:inline-block;width:.45rem;height:.9rem;margin-left:.25rem;vertical-align:-.15rem;background:#80d68b;animation:terminal-blink 1s steps(1) infinite}@keyframes terminal-blink{50%{opacity:0}}@media(prefers-reduced-motion:reduce){.terminal-cursor{animation:none}}
";

const REPLAY_SCRIPT: &str = r#"
(()=>{
const root=document.currentScript.closest('[data-match-replay]');if(!root)return;
const data=JSON.parse(root.querySelector('[data-replay-data]').textContent);
const lanes=root.querySelector('[data-lanes]'),slider=root.querySelector('[data-scrubber]'),clock=root.querySelector('[data-clock]'),play=root.querySelector('[data-play]'),speed=root.querySelector('[data-speed]'),inspector=root.querySelector('[data-inspector]'),topologyRoot=root.querySelector('[data-topology]');
const laneMap=new Map(),topologyMap=new Map(),terminalMap=new Map(),agentMap=new Map(data.lanes.map(x=>[x.agent,x.territory]));
const plotted=new Set(['competitor_state_changed','milestone_passed','durable_deployment_completed','usage_charged','agent_interrupted','agent_terminated','agent_failed']);
const kindLabel={client:'client',proxy:'proxy',service:'service',worker:'worker',queue:'queue',database:'database',storage:'storage',host:'host'};
const fmt=ms=>`${Math.floor(ms/60000)}:${String(Math.floor(ms/1000)%60).padStart(2,'0')}`;
const label=e=>e.milestone||e.to||e.reason||e.kind.replaceAll('_',' ');
const text=v=>{const span=document.createElement('span');span.textContent=v??'';return span.innerHTML};
for(const lane of data.lanes){const el=document.createElement('div');el.className='replay-lane';el.innerHTML=`<div class="lane-label"><strong>${text(lane.territory)}${lane.winner?' ★':''}</strong><small>${text(lane.model)}</small><span data-state>waiting</span></div><div class="lane-track"><div class="lane-progress"></div></div>`;lanes.append(el);laneMap.set(lane.territory,el)}
if(data.topology?.nodes?.length){topologyRoot.hidden=false;topologyRoot.innerHTML='<div class="topology-header"><strong>Service map and agent activity</strong><div class="topology-legend"><span>pending</span><span class="verifying">verifying</span><span class="healthy">healthy</span><span class="failed">failed</span></div></div><div class="topology-grid"></div>';const grid=topologyRoot.querySelector('.topology-grid');for(const lane of data.lanes){const card=document.createElement('article');card.className='topology-card';const links=data.topology.links.map(link=>{const from=data.topology.nodes.find(node=>node.id===link.from),to=data.topology.nodes.find(node=>node.id===link.to);if(!from||!to)return'';return `<line class="topology-link ${text(link.kind)}" x1="${from.x}%" y1="${from.y}%" x2="${to.x}%" y2="${to.y}%"><title>${text(link.label||`${link.from} to ${link.to}`)}</title></line>`}).join('');const nodes=data.topology.nodes.map(node=>`<div class="topology-node pending" data-node="${text(node.id)}" data-milestone="${text(node.milestone||'')}" style="left:${node.x}%;top:${node.y}%"><b>${text(node.display_name)}</b><small>${text(kindLabel[node.kind]||node.kind)}</small></div>`).join('');card.innerHTML=`<header><strong>${text(lane.territory)}${lane.winner?' ★':''}</strong><small>${text(lane.model)}</small></header><div class="topology-board"><svg class="topology-links" aria-hidden="true">${links}</svg>${nodes}</div><div class="agent-terminal" aria-label="${text(lane.agent)} observable activity"><div class="terminal-title"><span></span><span></span><span></span><strong>${text(lane.agent)}</strong></div><div class="terminal-lines" data-terminal-lines></div></div>`;grid.append(card);topologyMap.set(lane.territory,card);terminalMap.set(lane.territory,card.querySelector('[data-terminal-lines]'))}}
const markers=[];for(const e of data.events){const territory=e.territory||agentMap.get(e.agent);if(!territory||!plotted.has(e.kind)||!laneMap.has(territory))continue;const marker=document.createElement('button');marker.type='button';marker.className=`lane-marker ${e.kind}`;marker.style.left=`${Math.min(100,(e.elapsed_ms||0)/Math.max(1,data.duration)*100)}%`;marker.title=`${fmt(e.elapsed_ms||0)} · ${label(e)}`;marker.addEventListener('click',()=>inspect(e,marker));laneMap.get(territory).querySelector('.lane-track').append(marker);markers.push([marker,e])}
function inspect(e,marker){root.querySelectorAll('.lane-marker.selected').forEach(x=>x.classList.remove('selected'));if(marker)marker.classList.add('selected');inspector.innerHTML=`<span class="eyebrow">${fmt(e.elapsed_ms||0)} · event #${e.sequence??'?'}</span><strong>${text(label(e))}</strong><p>${text(e.territory||e.agent||'match')}</p><details><summary>Raw referee event</summary><pre>${text(JSON.stringify(e,null,2))}</pre></details>`}
function renderTopology(territory,now){const card=topologyMap.get(territory);if(!card)return;const states=new Map(data.topology.nodes.filter(node=>node.milestone).map(node=>[node.milestone,'pending']));let durable=false;for(const e of data.events){if((e.elapsed_ms||0)>now)break;if((e.territory||agentMap.get(e.agent))!==territory)continue;if(e.kind==='milestone_evaluation_started')states.set(e.milestone,'verifying');else if(e.kind==='milestone_passed')states.set(e.milestone,'healthy');else if(e.kind==='milestone_failed'||e.kind==='milestone_revoked')states.set(e.milestone,'failed');else if(e.kind==='durable_deployment_completed')durable=true}for(const node of card.querySelectorAll('.topology-node')){node.classList.remove('pending','verifying','healthy','failed','durable');const milestone=node.dataset.milestone;node.classList.add(milestone?(durable&&states.get(milestone)==='healthy'?'durable':states.get(milestone)||'pending'):'pending')}}
function renderTerminal(territory,now){const terminal=terminalMap.get(territory);if(!terminal)return;const visible=(data.activity||[]).filter(item=>item.territory===territory&&(item.started_after_ms||0)<=now);if(!visible.length){terminal.innerHTML='<div class="terminal-line idle"><time>0:00</time><span>waiting for observable agent activity<span class="terminal-cursor"></span></span></div>';return}const recent=visible.slice(-4);terminal.innerHTML=recent.map(item=>`<div class="terminal-line ${item.success?'':'error'}"><time>${fmt(item.started_after_ms||0)}</time><span><b>${text(item.kind)}</b> ${text(item.description||item.command||'tool activity')}</span></div>`).join('');const latest=recent.at(-1),ended=(latest.started_after_ms||0)+(latest.duration_ms||0),quiet=Math.max(0,now-ended);if(quiet>=5000)terminal.insertAdjacentHTML('beforeend',`<div class="terminal-line idle"><time>${fmt(now)}</time><span>… no observable tool activity for ${fmt(quiet)}<span class="terminal-cursor"></span></span></div>`);else terminal.insertAdjacentHTML('beforeend',`<div class="terminal-line active"><time>${fmt(now)}</time><span>working<span class="terminal-cursor"></span></span></div>`);terminal.scrollTop=terminal.scrollHeight}
function render(now){slider.value=now;clock.textContent=fmt(now);for(const [territory,lane] of laneMap){lane.querySelector('.lane-progress').style.width=`${Math.min(100,now/Math.max(1,data.duration)*100)}%`;let state='working';for(const e of data.events){if((e.elapsed_ms||0)>now)break;const actor=e.territory||agentMap.get(e.agent);if(actor!==territory)continue;if(e.kind==='milestone_evaluation_started')state=`verifying ${e.milestone}`;else if(e.kind==='milestone_passed')state=`passed ${e.milestone}`;else if(e.kind==='competitor_state_changed')state=e.to;else if(e.kind==='agent_terminated'||e.kind==='agent_interrupted'||e.kind==='agent_failed')state=e.kind.replace('agent_','')}lane.querySelector('[data-state]').textContent=state;renderTopology(territory,now);renderTerminal(territory,now)}for(const [marker,e] of markers)marker.classList.toggle('visible',(e.elapsed_ms||0)<=now)}
let running=false,last=0,current=0;function tick(ts){if(!running)return;if(!last)last=ts;current=Math.min(data.duration,current+(ts-last)*Number(speed.value));last=ts;render(current);if(current>=data.duration){running=false;play.textContent='Replay';return}requestAnimationFrame(tick)}play.addEventListener('click',()=>{if(running){running=false;play.textContent='Play';return}if(current>=data.duration)current=0;running=true;last=0;play.textContent='Pause';requestAnimationFrame(tick)});slider.addEventListener('input',()=>{current=Number(slider.value);render(current)});render(0)
})();
"#;

const FOG_REPLAY_SCRIPT: &str = r"
(()=>{
const root=document.currentScript.closest('[data-match-replay]');if(!root)return;
const data=JSON.parse(root.querySelector('[data-replay-data]').textContent);if(!data.fog_of_war||!data.topology)return;
const slider=root.querySelector('[data-scrubber]'),agentMap=new Map(data.lanes.map(x=>[x.agent,x.territory]));
function render(){const now=Number(slider.value);for(const card of root.querySelectorAll('.topology-card')){const territory=card.querySelector('header strong').textContent.replace(' ★','');const revealed=new Set(data.topology.nodes.filter(node=>!node.milestone).map(node=>node.id));for(const event of data.events){if((event.elapsed_ms||0)>now)break;if((event.territory||agentMap.get(event.agent))!==territory)continue;if(event.milestone&&['milestone_evaluation_started','milestone_passed','milestone_failed','milestone_revoked'].includes(event.kind)){for(const node of data.topology.nodes)if(node.milestone===event.milestone)revealed.add(node.id)}}for(const node of card.querySelectorAll('.topology-node')){const spec=data.topology.nodes.find(item=>item.id===node.dataset.node),visible=revealed.has(node.dataset.node);node.classList.toggle('unknown',!visible);node.querySelector('b').textContent=visible?spec.display_name:'Unknown';node.querySelector('small').textContent=visible?spec.kind:'unmapped'}card.querySelectorAll('.topology-link').forEach((line,index)=>{const link=data.topology.links[index];line.classList.toggle('fogged',!revealed.has(link.from)||!revealed.has(link.to))})}}
let last='';function watch(){if(slider.value!==last){last=slider.value;render()}requestAnimationFrame(watch)}render();requestAnimationFrame(watch)
})();
";
