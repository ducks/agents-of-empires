use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use aoe_domain::AgentTerminalState;
use aoe_replay::WorldState;
use serde::Serialize;
use thiserror::Error;

use crate::provenance::{MatchProvenance, read_provenance};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportSummary {
    pub matches: usize,
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
    #[error("no completed match artifacts found below {0}")]
    NoMatches(PathBuf),
}

struct MatchReport {
    name: String,
    state: WorldState,
    events: Vec<ReportEvent>,
    source: PathBuf,
    report_dir: PathBuf,
    provenance: Option<MatchProvenance>,
    current: bool,
}

/// Generate a self-contained static report site from one match directory or a
/// directory containing match directories.
///
/// # Errors
///
/// Returns an error for missing artifacts, malformed event/world data, or file
/// system failures.
pub fn generate_reports(input: &Path, output: &Path) -> Result<ReportSummary, ReportError> {
    let match_dirs = discover_matches(input)?;
    fs::create_dir_all(output.join("matches"))?;
    let mut reports = Vec::with_capacity(match_dirs.len());
    for source in match_dirs {
        let name = source
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("match")
            .to_owned();
        let report_dir = output.join("matches").join(safe_name(&name));
        fs::create_dir_all(&report_dir)?;
        let state = read_world(&source.join("world.json"))?;
        let events = read_report_events(&source.join("events.jsonl"))?;
        let provenance = read_provenance(&source.join("match.json")).ok();
        let report = MatchReport {
            name,
            state,
            events,
            source,
            report_dir,
            provenance,
            current: false,
        };
        copy_public_artifacts(&report)?;
        reports.push(report);
    }
    reports.sort_by(|left, right| right.name.cmp(&left.name));
    mark_current(&mut reports);
    for report in &reports {
        fs::write(report.report_dir.join("index.html"), render_match(report))?;
    }
    let index = output.join("index.html");
    fs::write(&index, render_index(&reports))?;
    Ok(ReportSummary {
        matches: reports.len(),
        index,
    })
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

fn copy_public_artifacts(report: &MatchReport) -> Result<(), ReportError> {
    let raw = report.report_dir.join("artifacts");
    fs::create_dir_all(&raw)?;
    fs::copy(report.source.join("events.jsonl"), raw.join("events.jsonl"))?;
    fs::copy(report.source.join("world.json"), raw.join("world.json"))?;
    if report.source.join("match.json").is_file() {
        fs::copy(report.source.join("match.json"), raw.join("match.json"))?;
    }
    for agent in report.state.agents.keys() {
        let source = report.source.join("agents").join(agent);
        for file in ["transcript.json", "transcript.jsonl", "result.json"] {
            let artifact = source.join(file);
            if artifact.is_file() {
                fs::copy(&artifact, raw.join(format!("{}-{file}", safe_name(agent))))?;
            }
        }
    }
    Ok(())
}

fn render_index(reports: &[MatchReport]) -> String {
    let current = render_cards(reports.iter().filter(|report| report.current));
    let historical = render_cards(reports.iter().filter(|report| !report.current));
    let current = if current.is_empty() {
        "<p class=\"empty\">No matches recorded under the current rules yet.</p>".to_owned()
    } else {
        current
    };
    page(
        "Agents of Empires · Match Archive",
        &format!(
            "<header class=\"hero\"><span class=\"eyebrow\">Match archive</span><h1>Agents of Empires</h1><p>Build races decided by durable deployments, not confident answers.</p></header><main><section class=\"about\"><span class=\"eyebrow\">About the arena</span><h2>What am I looking at?</h2><p>Agents of Empires drops AI infrastructure agents into identical disposable NixOS machines and gives them the same service contract. A referee checks recovered state, fresh work, service restarts, and host reboots. The first agent to produce a durable deployment wins.</p><p><a href=\"https://github.com/ducks/agents-of-empires\">Read how the arena works and view the source →</a></p></section><section><div class=\"section-heading\"><h2>Current</h2><p>Matches sharing the newest manifest and verifier compatibility key for each arena.</p></div><div class=\"match-list\">{current}</div></section><section><div class=\"section-heading\"><h2>Historical</h2><p>Prototype or superseded runs retained for auditability, not direct comparison.</p></div><div class=\"match-list historical\">{historical}</div></section></main>"
        ),
    )
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
            escape(&safe_name(&report.name)),
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
    let timeline = render_timeline(&report.events);
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
        <main>{replay}<section><div class=\"section-heading\"><h2>Territories</h2><p>The referee-owned result frozen when the first deployment became durable.</p></div><div class=\"table-wrap\"><table><thead><tr><th>Territory</th><th>State</th><th>Milestones</th><th>Points</th><th>Durable at</th></tr></thead><tbody>{territories}</tbody></table></div></section>
        <section><div class=\"section-heading\"><h2>Agents</h2><p>Usage includes cumulative checkpoints captured while agents were still running.</p></div><div class=\"table-wrap\"><table><thead><tr><th>Agent</th><th>Model</th><th>Outcome</th><th>Input</th><th>Output</th><th>Cost</th><th>Artifact</th></tr></thead><tbody>{agents}</tbody></table></div></section>
        <section><div class=\"section-heading\"><h2>Event timeline</h2><p>{} immutable events. The match clock remains frozen during post-match collection.</p></div><ol class=\"timeline\">{timeline}</ol></section>
        <footer><a href=\"artifacts/events.jsonl\">events.jsonl</a><a href=\"artifacts/world.json\">world.json</a></footer></main>",
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
    for file in ["transcript.json", "transcript.jsonl", "result.json"] {
        if report
            .source
            .join("agents")
            .join(agent)
            .join(file)
            .is_file()
        {
            return format!(
                "<a href=\"artifacts/{}-{}\">view</a>",
                escape(&safe_name(agent)),
                file
            );
        }
    }
    "—".to_owned()
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
    let payload = serde_json::json!({
        "duration": report.state.elapsed_ms,
        "lanes": lanes,
        "events": report.events.iter().map(|event| &event.value).collect::<Vec<_>>(),
        "stalls": replay_stalls(&report.events, report.state.elapsed_ms),
    });
    let payload = serde_json::to_string(&payload)
        .unwrap_or_else(|_| "{\"duration\":0,\"lanes\":[],\"events\":[]}".to_owned())
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");
    format!(
        r#"<section class="replay" data-match-replay><div class="section-heading"><div><span class="eyebrow">Match replay</span><h2>Watch the race unfold</h2></div><p>Play the referee's event stream or scrub directly to any moment.</p></div>
        <div class="replay-shell"><div class="replay-controls"><button type="button" data-play>Play</button><strong data-clock>0:00</strong><input data-scrubber aria-label="Match clock" type="range" min="0" max="{duration}" value="0" step="100"><select data-speed aria-label="Playback speed"><option value="1">1×</option><option value="5">5×</option><option value="20" selected>20×</option><option value="60">60×</option></select></div><div class="replay-ruler"><span>Start</span><span>{finish}</span></div><div class="replay-lanes" data-lanes></div><aside class="replay-inspector" data-inspector><span class="eyebrow">Selected event</span><strong>Press play or select a marker</strong><p>Milestones, state changes, usage, and terminal outcomes appear on the shared clock.</p></aside></div>
        <script type="application/json" data-replay-data>{payload}</script><script>{REPLAY_SCRIPT}</script></section>"#,
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
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"color-scheme\" content=\"dark\"><title>{}</title><style>{}{}</style></head><body>{}</body></html>",
        escape(title),
        STYLE,
        REPLAY_STYLE,
        content
    )
}

const STYLE: &str = r#"
:root{--bg:#10130f;--panel:#191e18;--line:#333d31;--text:#edf4e9;--muted:#9ca997;--gold:#e7bb55;--green:#85d68b;--red:#ef857c;--orange:#e5a65f}*{box-sizing:border-box}body{margin:0;background:radial-gradient(circle at 75% 0,#253120 0,transparent 32rem),var(--bg);color:var(--text);font:16px/1.5 ui-monospace,SFMono-Regular,Menlo,monospace}a{color:var(--gold)}nav,main,.hero{width:min(1120px,calc(100% - 2rem));margin:auto}nav{display:flex;justify-content:space-between;padding:1.25rem 0;color:var(--muted)}nav a{text-decoration:none}.hero{padding:6rem 0 3rem}.match-hero{padding-top:3.5rem}.eyebrow{color:var(--gold);font-size:.75rem;letter-spacing:.14em;text-transform:uppercase}h1{font-family:Georgia,serif;font-size:clamp(2.7rem,8vw,6.8rem);line-height:.9;margin:.35rem 0 1.2rem;max-width:900px}h2{font-family:Georgia,serif;font-size:2rem;margin:0}.hero>p,.section-heading p{color:var(--muted);max-width:680px}.about{border:1px solid var(--line);background:linear-gradient(135deg,#20271e,var(--panel));padding:2rem}.about h2{margin:.35rem 0 1rem}.about p{color:var(--muted);max-width:850px}.about p:last-child{margin-bottom:0}.about a{text-decoration:none}.hero-stats{display:grid;grid-template-columns:repeat(4,1fr);gap:1px;margin-top:3rem;background:var(--line);border:1px solid var(--line)}.hero-stats div{background:var(--panel);padding:1.25rem}.hero-stats small,td small{display:block;color:var(--muted);margin-bottom:.35rem}.hero-stats strong{font-size:1.25rem}section{margin:1rem 0 4rem}.section-heading{display:flex;align-items:end;justify-content:space-between;gap:2rem;margin-bottom:1rem}.section-heading p{margin:0}.table-wrap{overflow:auto;border:1px solid var(--line)}table{border-collapse:collapse;width:100%;background:var(--panel)}th,td{padding:1rem;text-align:left;border-bottom:1px solid var(--line);vertical-align:top}th{color:var(--muted);font-size:.75rem;text-transform:uppercase;letter-spacing:.08em}.winner-row{background:#252b19}.pill{display:inline-block;border:1px solid var(--line);border-radius:99px;padding:.15rem .55rem;font-size:.8rem}.pill.good{color:var(--green)}.pill.warn{color:var(--orange)}.pill.bad{color:var(--red)}.timeline{list-style:none;padding:0;border-top:1px solid var(--line)}.timeline li{display:grid;grid-template-columns:6rem 1fr;gap:1rem;padding:1rem 0;border-bottom:1px solid var(--line)}.timeline time{color:var(--gold)}.timeline span{color:var(--muted);margin-left:.75rem}.timeline details{margin-top:.4rem;color:var(--muted)}pre{white-space:pre-wrap;word-break:break-word;background:#090b09;padding:1rem;overflow:auto}.match-list{display:grid;gap:1rem}.match-card{display:flex;justify-content:space-between;gap:2rem;padding:1.5rem;border:1px solid var(--line);background:var(--panel);text-decoration:none;color:var(--text)}.match-card:hover{border-color:var(--gold)}.match-card h2{font-size:1.5rem}.match-card p{color:var(--muted);margin:.25rem 0 0}.metrics{text-align:right}.metrics strong,.metrics span{display:block}.metrics span{color:var(--muted)}footer{display:flex;gap:1rem;padding:2rem 0 5rem;border-top:1px solid var(--line)}@media(max-width:720px){.hero{padding-top:3rem}.hero-stats{grid-template-columns:1fr 1fr}.section-heading{display:block}.timeline li{grid-template-columns:4rem 1fr}.match-card{display:block}.metrics{text-align:left;margin-top:1rem}th,td{padding:.75rem}}
"#;

const REPLAY_STYLE: &str = r#"
button,select,input{font:inherit}.replay-shell{border:1px solid var(--line);background:linear-gradient(145deg,#1d241b,var(--panel));padding:1.25rem}.replay-controls{display:grid;grid-template-columns:auto 4rem 1fr auto;gap:1rem;align-items:center}.replay-controls button,.replay-controls select{color:var(--text);background:#0c0f0b;border:1px solid var(--line);padding:.55rem .8rem}.replay-controls button{color:var(--gold);cursor:pointer}.replay-controls input{accent-color:var(--gold);width:100%}.replay-hud,.replay-ruler{display:flex;justify-content:space-between;color:var(--muted);font-size:.75rem}.replay-hud{margin:.75rem 0;border-top:1px solid var(--line);padding-top:.75rem}.replay-ruler{margin:1rem 0 .35rem;padding-left:15.5rem}.replay-lane{display:grid;grid-template-columns:14rem 1fr;gap:1.5rem;align-items:center;padding:1rem 0;border-top:1px solid var(--line)}.lane-label strong,.lane-label small,.lane-label span{display:block}.lane-label small{color:var(--muted);overflow:hidden;text-overflow:ellipsis}.lane-label span{color:var(--gold);font-size:.75rem;margin-top:.3rem}.lane-track{height:2.6rem;position:relative;background:#0d100c;border:1px solid #293126}.lane-progress{position:absolute;inset:0 auto 0 0;width:0;background:linear-gradient(90deg,#39452eaa,#68603388);transition:width .08s linear}.lane-stall{position:absolute;top:.28rem;height:2rem;padding:0 .4rem;border:1px solid #cb6e62;border-radius:.25rem;background:repeating-linear-gradient(135deg,#8f403c88 0,#8f403c88 6px,#6d312d88 6px,#6d312d88 12px);color:#ffe2dc;font-size:.68rem;line-height:1.8rem;text-align:center;overflow:hidden;cursor:pointer;opacity:.18;white-space:nowrap}.lane-stall.visible{opacity:.85}.lane-stall.resolved{border-color:var(--orange);background:repeating-linear-gradient(135deg,#895b3188 0,#895b3188 6px,#60422488 6px,#60422488 12px)}.lane-stall.selected{outline:2px solid var(--gold);z-index:2}.lane-marker{position:absolute;top:50%;translate:-50% -50%;width:.85rem;height:.85rem;padding:0;border:2px solid var(--panel);border-radius:50%;background:var(--muted);cursor:pointer;opacity:.2;transition:opacity .15s,scale .15s}.lane-marker.visible{opacity:1}.lane-marker:hover,.lane-marker.selected{scale:1.5;z-index:3}.lane-marker.milestone_passed,.lane-marker.durable_deployment_completed{background:var(--green)}.lane-marker.competitor_state_changed{background:var(--gold)}.lane-marker.agent_interrupted,.lane-marker.agent_terminated,.lane-marker.agent_failed{background:var(--red)}.lane-marker.usage_charged{background:var(--orange)}.replay-inspector{min-height:7rem;border-top:1px solid var(--line);margin-top:.5rem;padding:1rem 0 0}.replay-inspector strong{display:block;margin:.3rem 0}.replay-inspector p{color:var(--muted);margin:.25rem 0}.replay-inspector pre{max-height:18rem}@media(max-width:720px){.replay-controls{grid-template-columns:auto 3.5rem 1fr}.replay-controls select{grid-column:1/-1}.replay-ruler{padding-left:0}.replay-lane{grid-template-columns:1fr;gap:.5rem}}
"#;

const REPLAY_SCRIPT: &str = r#"
(()=>{const root=document.currentScript.closest('[data-match-replay]');if(!root)return;const data=JSON.parse(root.querySelector('[data-replay-data]').textContent);const lanes=root.querySelector('[data-lanes]'),slider=root.querySelector('[data-scrubber]'),clock=root.querySelector('[data-clock]'),play=root.querySelector('[data-play]'),speed=root.querySelector('[data-speed]'),inspector=root.querySelector('[data-inspector]');const laneMap=new Map(),agentMap=new Map(data.lanes.map(x=>[x.agent,x.territory]));const plotted=new Set(['competitor_state_changed','milestone_passed','durable_deployment_completed','usage_charged','agent_interrupted','agent_terminated','agent_failed']);const fmt=ms=>`${Math.floor(ms/60000)}:${String(Math.floor(ms/1000)%60).padStart(2,'0')}`;const label=e=>e.milestone||e.to||e.reason||e.kind.replaceAll('_',' ');const text=v=>{const span=document.createElement('span');span.textContent=v??'';return span.innerHTML};for(const lane of data.lanes){const el=document.createElement('div');el.className='replay-lane';el.innerHTML=`<div class="lane-label"><strong>${text(lane.territory)}${lane.winner?' ★':''}</strong><small>${text(lane.model)}</small><span data-state>waiting</span></div><div class="lane-track"><div class="lane-progress"></div></div>`;lanes.append(el);laneMap.set(lane.territory,el)}const markers=[];for(const e of data.events){const territory=e.territory||agentMap.get(e.agent);if(!territory||!plotted.has(e.kind)||!laneMap.has(territory))continue;const marker=document.createElement('button');marker.type='button';marker.className=`lane-marker ${e.kind}`;marker.style.left=`${Math.min(100,(e.elapsed_ms||0)/Math.max(1,data.duration)*100)}%`;marker.title=`${fmt(e.elapsed_ms||0)} · ${label(e)}`;marker.addEventListener('click',()=>inspect(e,marker));laneMap.get(territory).querySelector('.lane-track').append(marker);markers.push([marker,e])}function inspect(e,marker){root.querySelectorAll('.lane-marker.selected').forEach(x=>x.classList.remove('selected'));if(marker)marker.classList.add('selected');inspector.innerHTML=`<span class="eyebrow">${fmt(e.elapsed_ms||0)} · event #${e.sequence??'?'}</span><strong>${text(label(e))}</strong><p>${text(e.territory||e.agent||'match')}</p><details><summary>Raw referee event</summary><pre>${text(JSON.stringify(e,null,2))}</pre></details>`}function render(now){slider.value=now;clock.textContent=fmt(now);for(const [territory,lane] of laneMap){lane.querySelector('.lane-progress').style.width=`${Math.min(100,now/Math.max(1,data.duration)*100)}%`;let state='working';for(const e of data.events){if((e.elapsed_ms||0)>now)break;const actor=e.territory||agentMap.get(e.agent);if(actor!==territory)continue;if(e.kind==='milestone_evaluation_started')state=`verifying ${e.milestone}`;else if(e.kind==='milestone_passed')state=`passed ${e.milestone}`;else if(e.kind==='competitor_state_changed')state=e.to;else if(e.kind==='agent_terminated'||e.kind==='agent_interrupted'||e.kind==='agent_failed')state=e.kind.replace('agent_','')}lane.querySelector('[data-state]').textContent=state}for(const [marker,e] of markers)marker.classList.toggle('visible',(e.elapsed_ms||0)<=now)}let running=false,last=0,current=0;function tick(ts){if(!running)return;if(!last)last=ts;current=Math.min(data.duration,current+(ts-last)*Number(speed.value));last=ts;render(current);if(current>=data.duration){running=false;play.textContent='Replay';return}requestAnimationFrame(tick)}play.addEventListener('click',()=>{if(running){running=false;play.textContent='Play';return}if(current>=data.duration)current=0;running=true;last=0;play.textContent='Pause';requestAnimationFrame(tick)});slider.addEventListener('input',()=>{current=Number(slider.value);render(current)});render(0)})();
"#;
