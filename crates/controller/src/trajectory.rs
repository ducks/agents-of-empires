use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use aoe_domain::ArenaManifest;
use aoe_replay::WorldState;
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

use crate::provenance::{MatchProvenance, read_provenance};

pub const ATIF_VERSION: &str = "ATIF-v1.7";
pub const INFRA_EVAL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrajectoryExportSummary {
    pub trajectories: usize,
    pub skipped: usize,
    pub output: PathBuf,
}

#[derive(Debug, Error)]
pub enum TrajectoryError {
    #[error("trajectory I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid JSON in {path}: {detail}")]
    Json { path: PathBuf, detail: String },
    #[error("no match artifacts found below {0}")]
    NoMatches(PathBuf),
}

#[derive(Debug, Deserialize)]
struct Transcript {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    outcome: Value,
    #[serde(default)]
    messages: Vec<Value>,
    #[serde(default)]
    tool_trace: Vec<ToolTrace>,
    #[serde(default)]
    usage: Usage,
    #[serde(default)]
    timing: Value,
}

#[derive(Debug, Deserialize)]
struct ToolTrace {
    #[serde(default)]
    id: Option<String>,
    name: String,
    #[serde(default)]
    input: Value,
    #[serde(default)]
    output: Option<String>,
    #[serde(default)]
    is_error: bool,
    #[serde(default)]
    read_only: Option<bool>,
    #[serde(default)]
    started_after_ms: Option<u64>,
    #[serde(default)]
    duration_ms: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct Usage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_tokens: Option<u64>,
    cache_creation_tokens: Option<u64>,
    cost_usd: Option<f64>,
}

/// Export every structured agent transcript below a match path as ATIF.
///
/// One trajectory is written per agent under `OUTPUT/MATCH/AGENT.json`.
/// Private assistant messages are excluded; observable tool calls and outputs
/// remain exact.
///
/// # Errors
///
/// Returns an error for missing matches, malformed retained artifacts, or I/O
/// failures while writing trajectories.
pub fn export_trajectories(
    input: &Path,
    output: &Path,
) -> Result<TrajectoryExportSummary, TrajectoryError> {
    let matches = discover_matches(input)?;
    fs::create_dir_all(output)?;
    let mut trajectories = 0;
    let mut skipped = 0;
    for source in matches {
        let world: WorldState = read_json(&source.join("world.json"))?;
        let arena: Option<ArenaManifest> = optional_json(&source.join("arena.json"))?;
        let provenance = read_provenance(&source.join("match.json")).ok();
        let match_name = source
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("match");
        let destination = output.join(safe_name(match_name));
        fs::create_dir_all(&destination)?;
        for (agent_id, agent) in &world.agents {
            let agent_dir = source.join("agents").join(agent_id);
            let transcript_path = ["transcript.json", "transcript.live.json"]
                .into_iter()
                .map(|name| agent_dir.join(name))
                .find(|path| path.is_file());
            let Some(transcript_path) = transcript_path else {
                skipped += 1;
                continue;
            };
            let transcript: Transcript = match read_json(&transcript_path) {
                Ok(value) => value,
                Err(TrajectoryError::Json { .. }) => {
                    skipped += 1;
                    continue;
                }
                Err(error) => return Err(error),
            };
            if transcript.tool_trace.is_empty() {
                skipped += 1;
                continue;
            }
            let trajectory = render_trajectory(
                match_name,
                agent_id,
                agent,
                &world,
                arena.as_ref(),
                provenance.as_ref(),
                transcript,
            );
            fs::write(
                destination.join(format!("{}.json", safe_name(agent_id))),
                serde_json::to_vec_pretty(&trajectory).map_err(|error| TrajectoryError::Json {
                    path: transcript_path.clone(),
                    detail: error.to_string(),
                })?,
            )?;
            trajectories += 1;
        }
    }
    Ok(TrajectoryExportSummary {
        trajectories,
        skipped,
        output: output.to_owned(),
    })
}

#[allow(clippy::too_many_lines, clippy::cast_precision_loss)]
fn render_trajectory(
    match_name: &str,
    agent_id: &str,
    agent: &aoe_replay::AgentView,
    world: &WorldState,
    arena: Option<&ArenaManifest>,
    provenance: Option<&MatchProvenance>,
    transcript: Transcript,
) -> Value {
    let config = arena.and_then(|manifest| {
        manifest
            .agents
            .iter()
            .find(|candidate| candidate.id == agent_id)
    });
    let adapter = config.map_or("unknown", |value| value.adapter.as_str());
    let adapter_version = provenance
        .and_then(|value| value.adapter_sha256.get(adapter))
        .map_or("unknown", String::as_str);
    let model = transcript.model.as_deref().unwrap_or(&agent.model);
    let mut steps = Vec::new();
    if let Some(prompt) = first_user_message(&transcript.messages) {
        steps.push(json!({"step_id": 1, "source": "user", "message": prompt}));
    }
    for (index, tool) in transcript.tool_trace.into_iter().enumerate() {
        let call_id = tool.id.unwrap_or_else(|| format!("tool-{}", index + 1));
        let mut extra = serde_json::Map::new();
        if let Some(value) = tool.started_after_ms {
            extra.insert("started_after_ms".into(), value.into());
        }
        if let Some(value) = tool.duration_ms {
            extra.insert("duration_ms".into(), value.into());
        }
        if let Some(value) = tool.read_only {
            extra.insert("read_only".into(), value.into());
        }
        steps.push(json!({
            "step_id": steps.len() + 1,
            "source": "agent",
            "model_name": model,
            "message": "",
            "tool_calls": [{
                "tool_call_id": call_id,
                "function_name": tool.name,
                "arguments": tool.input,
            }],
            "observation": {"results": [{
                "source_call_id": call_id,
                "content": tool.output.unwrap_or_default(),
                "extra": {"is_error": tool.is_error},
            }]},
            "extra": extra,
        }));
    }
    let (outcome, final_message) = final_message(&transcript.outcome);
    steps.push(json!({
        "step_id": steps.len() + 1,
        "source": "agent",
        "model_name": model,
        "message": final_message,
        "extra": {"outcome": outcome},
    }));
    let cached_tokens = match (
        transcript.usage.cache_read_tokens,
        transcript.usage.cache_creation_tokens,
    ) {
        (Some(read), Some(created)) => Some(read.saturating_add(created)),
        _ => None,
    };
    let territory = world.territories.get(&agent.territory);
    let verification = territory.map_or_else(BTreeMap::new, |territory| {
        territory
            .milestones
            .iter()
            .map(|(name, milestone)| {
                (
                    name.clone(),
                    json!({
                        "passed": milestone.passed,
                        "evaluating": milestone.evaluating,
                        "points": milestone.points,
                        "failure_category": milestone.failure_category,
                        "failure_detail": milestone.failure_detail,
                    }),
                )
            })
            .collect()
    });
    let failure_category = territory.and_then(|territory| {
        territory
            .milestones
            .values()
            .find_map(|milestone| milestone.failure_category.clone())
    });
    let compatibility_key = provenance.map(|value| value.compatibility_key.as_str());
    let evaluation = json!({
        "schema_version": INFRA_EVAL_VERSION,
        "producer": {
            "name": "agents-of-empires",
            "version": provenance.map(|value| value.controller_version.as_str()),
            "revision": provenance.and_then(|value| value.source_revision.as_deref()),
        },
        "task": {
            "id": provenance.map_or_else(
                || arena.map_or("unknown", |value| value.arena.id.as_str()),
                |value| value.arena_id.as_str(),
            ),
            "version": compatibility_key,
            "pack": Value::Null,
        },
        "execution": {
            "agent": agent_id,
            "model": model,
            "reasoning_effort": config.map(|value| value.reasoning_effort.as_str()),
            "timeout_seconds": Value::Null,
            "duration_seconds": transcript.timing.get("total_duration_ms")
                .and_then(Value::as_u64).map(|value| value as f64 / 1000.0),
            "suite": "agents-of-empires-match",
            "territory": agent.territory,
            "budget_resource_units": config.map(|value| value.budget.resource_units),
        },
        "outcome": {
            "status": agent.terminal_state.map(|value| format!("{value:?}").to_lowercase()),
            "reward": territory.map(|value| value.milestone_points),
            "failure_category": failure_category,
            "verification": verification,
            "durable": territory.is_some_and(|value| value.durable_at_ms.is_some()),
            "durable_at_ms": territory.and_then(|value| value.durable_at_ms),
            "winner": world.winner.as_deref() == Some(agent.territory.as_str()),
            "match_state": format!("{:?}", world.match_state).to_lowercase(),
            "finish_reason": world.finish_reason,
        },
        "provenance": provenance.map(|value| json!({
            "manifest_sha256": value.manifest_sha256,
            "verifier_sha256": value.verifier_sha256,
            "adapter_sha256": value.adapter_sha256,
            "compatibility_key": value.compatibility_key,
        })),
    });
    json!({
        "schema_version": ATIF_VERSION,
        "session_id": format!("{match_name}/{agent_id}"),
        "agent": {
            "name": adapter,
            "version": adapter_version,
            "model_name": model,
            "extra": {
                "reasoning_effort": config.map(|value| value.reasoning_effort.as_str()),
                "territory": agent.territory,
            },
        },
        "steps": steps,
        "notes": "Observable tool calls and exact outputs exported by Agents of Empires. Private assistant reasoning is intentionally excluded.",
        "final_metrics": {
            "total_prompt_tokens": transcript.usage.input_tokens.or(Some(agent.input_tokens)),
            "total_completion_tokens": transcript.usage.output_tokens.or(Some(agent.output_tokens)),
            "total_cached_tokens": cached_tokens,
            "total_cost_usd": transcript.usage.cost_usd.or(Some(agent.cost_microusd as f64 / 1_000_000.0)),
            "total_steps": steps.len(),
            "extra": {
                "cache_read_tokens": transcript.usage.cache_read_tokens,
                "cache_creation_tokens": transcript.usage.cache_creation_tokens,
                "model_rounds": transcript.timing.get("model_rounds"),
                "total_duration_ms": transcript.timing.get("total_duration_ms"),
                "infrastructure_evaluation": evaluation,
            },
        },
    })
}

fn first_user_message(messages: &[Value]) -> Option<&str> {
    messages.iter().find_map(|message| {
        (message.get("role").and_then(Value::as_str) == Some("user"))
            .then(|| message.get("content").and_then(Value::as_str))
            .flatten()
    })
}

fn final_message(outcome: &Value) -> (&str, &str) {
    let status = outcome
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let message = outcome
        .get("result")
        .or_else(|| outcome.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("Agent outcome was not recorded.");
    (status, message)
}

fn discover_matches(input: &Path) -> Result<Vec<PathBuf>, TrajectoryError> {
    if is_match(input) {
        return Ok(vec![input.to_owned()]);
    }
    let mut matches = Vec::new();
    walk_matches(input, &mut matches)?;
    matches.sort();
    matches.dedup();
    if matches.is_empty() {
        Err(TrajectoryError::NoMatches(input.to_owned()))
    } else {
        Ok(matches)
    }
}

fn walk_matches(path: &Path, matches: &mut Vec<PathBuf>) -> Result<(), std::io::Error> {
    if is_match(path) {
        matches.push(path.to_owned());
        return Ok(());
    }
    if !path.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(path)? {
        let child = entry?.path();
        if child.is_dir() {
            walk_matches(&child, matches)?;
        }
    }
    Ok(())
}

fn is_match(path: &Path) -> bool {
    path.join("world.json").is_file() && path.join("agents").is_dir()
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, TrajectoryError> {
    serde_json::from_slice(&fs::read(path)?).map_err(|error| TrajectoryError::Json {
        path: path.to_owned(),
        detail: error.to_string(),
    })
}

fn optional_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Option<T>, TrajectoryError> {
    path.is_file().then(|| read_json(path)).transpose()
}

fn safe_name(value: &str) -> String {
    let rendered: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect();
    rendered.trim_matches(['-', '.']).to_owned()
}
