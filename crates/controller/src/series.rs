use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use aoe_domain::{ArenaManifest, MatchState};
use aoe_replay::WorldState;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::runner::{RunError, RunOptions, run_match_with_manifest};

const SERIES_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct SeriesOptions {
    pub run: RunOptions,
    /// Number of rounds. `None` runs one round per territory.
    pub rounds: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeriesSummary {
    pub schema_version: u32,
    pub arena_id: String,
    pub rounds_requested: usize,
    pub rounds_completed: usize,
    pub rounds: Vec<SeriesRound>,
    pub standings: Vec<SeriesStanding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeriesRound {
    pub round: usize,
    pub output: PathBuf,
    pub seats: BTreeMap<String, String>,
    pub winner_territory: Option<String>,
    pub winner_agent: Option<String>,
    pub duration_ms: u64,
    pub usage_agents: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeriesStanding {
    pub agent: String,
    pub model: String,
    pub appearances: usize,
    pub wins: usize,
    pub durable_deployments: usize,
    pub median_durable_ms: Option<u64>,
    pub usage_recorded: usize,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost_microusd: Option<u64>,
    pub cost_per_durable_microusd: Option<u64>,
}

#[derive(Debug, Error)]
pub enum SeriesError {
    #[error("manifest failed: {0}")]
    Manifest(#[from] aoe_domain::ManifestError),
    #[error("series must contain at least one round")]
    Empty,
    #[error("existing series checkpoint does not match this run: {0}")]
    ResumeMismatch(String),
    #[error("series port range exceeds 65535")]
    PortRange,
    #[error("round {round} failed: {source}")]
    Round {
        round: usize,
        #[source]
        source: RunError,
    },
    #[error("series I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("could not encode series summary: {0}")]
    Json(#[from] serde_json::Error),
}

/// Run repeated matches while rotating each agent through every territory.
///
/// # Errors
///
/// Returns an error when the manifest is invalid, an existing checkpoint does
/// not describe the same series, a round fails, or the summary cannot be
/// written. A compatible checkpoint is resumed automatically.
pub async fn run_series(options: SeriesOptions) -> Result<SeriesSummary, SeriesError> {
    let original = ArenaManifest::load(&options.run.manifest)?;
    let requested = options.rounds.unwrap_or(original.territories.len());
    if requested == 0 {
        return Err(SeriesError::Empty);
    }
    let summary_path = options.run.output.join("series.json");
    fs::create_dir_all(&options.run.output)?;

    let (mut rounds, mut states) =
        load_checkpoint(&summary_path, &options.run.output, &original, requested)?;
    if states
        .last()
        .is_some_and(|state| state.match_state == MatchState::Aborted)
        || rounds.len() == requested
    {
        return Ok(build_summary(&original, requested, rounds, &states));
    }

    for index in rounds.len()..requested {
        let mut manifest = original.clone();
        rotate_seats(&mut manifest, index);
        let output = options.run.output.join(format!("round-{:03}", index + 1));
        archive_incomplete_round(&output)?;
        let mut run = options.run.clone();
        run.output.clone_from(&output);
        (run.base_port, run.multicast_port) = round_ports(
            options.run.base_port,
            options.run.multicast_port,
            original.territories.len(),
            index,
        )?;
        let state = run_match_with_manifest(run, manifest.clone())
            .await
            .map_err(|source| SeriesError::Round {
                round: index + 1,
                source,
            })?;
        let usage_agents = read_usage_agents(&output.join("events.jsonl"))?;
        rounds.push(round_summary(
            index + 1,
            output,
            &manifest,
            &state,
            usage_agents,
        ));
        states.push(state);
        let summary = build_summary(&original, requested, rounds.clone(), &states);
        write_summary(&summary_path, &summary)?;
        if states
            .last()
            .is_some_and(|state| state.match_state == MatchState::Aborted)
        {
            return Ok(summary);
        }
    }
    Ok(build_summary(&original, requested, rounds, &states))
}

fn load_checkpoint(
    summary_path: &Path,
    output: &Path,
    manifest: &ArenaManifest,
    requested: usize,
) -> Result<(Vec<SeriesRound>, Vec<WorldState>), SeriesError> {
    if !summary_path.exists() {
        return Ok((Vec::with_capacity(requested), Vec::with_capacity(requested)));
    }
    let summary: SeriesSummary = serde_json::from_slice(&fs::read(summary_path)?)?;
    if summary.schema_version != SERIES_SCHEMA_VERSION {
        return Err(SeriesError::ResumeMismatch(format!(
            "schema version is {}, expected {SERIES_SCHEMA_VERSION}",
            summary.schema_version
        )));
    }
    if summary.arena_id != manifest.arena.id {
        return Err(SeriesError::ResumeMismatch(format!(
            "arena is {}, checkpoint is {}",
            manifest.arena.id, summary.arena_id
        )));
    }
    if summary.rounds_requested != requested
        || summary.rounds.len() > requested
        || summary.rounds_completed != summary.rounds.len()
    {
        return Err(SeriesError::ResumeMismatch(format!(
            "requested {requested} rounds, checkpoint requested {}",
            summary.rounds_requested
        )));
    }
    for (index, round) in summary.rounds.iter().enumerate() {
        if round.round != index + 1 {
            return Err(SeriesError::ResumeMismatch(
                "completed rounds are not contiguous".into(),
            ));
        }
    }
    let states = summary
        .rounds
        .iter()
        .map(|round| {
            let path = output
                .join(format!("round-{:03}", round.round))
                .join("world.json");
            serde_json::from_slice(&fs::read(path)?).map_err(SeriesError::from)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((summary.rounds, states))
}

fn archive_incomplete_round(output: &Path) -> Result<(), SeriesError> {
    if !output.exists() {
        return Ok(());
    }
    let mut index = 1;
    loop {
        let archive = output.with_extension(format!("interrupted-{index}"));
        if !archive.exists() {
            fs::rename(output, archive)?;
            return Ok(());
        }
        index += 1;
    }
}

/// Render a compact terminal table for a completed series.
#[must_use]
pub fn render_series(summary: &SeriesSummary) -> String {
    let mut output = format!(
        "\nSERIES {}  rounds={}/{}\n\n",
        summary.arena_id, summary.rounds_completed, summary.rounds_requested
    );
    let _ = writeln!(
        output,
        "{:<28} {:>5} {:>7} {:>7} {:>7} {:>9} {:>12} {:>10} {:>12}",
        "agent", "wins", "durable", "rounds", "usage", "median", "tokens", "cost", "cost/durable"
    );
    let _ = writeln!(output, "{}", "-".repeat(113));
    for standing in &summary.standings {
        let tokens = standing
            .input_tokens
            .zip(standing.output_tokens)
            .map(|(input, output)| input.saturating_add(output));
        let _ = writeln!(
            output,
            "{:<28} {:>5} {:>7} {:>7} {:>7} {:>9} {:>12} {:>10} {:>12}",
            standing.agent,
            standing.wins,
            standing.durable_deployments,
            standing.appearances,
            format!("{}/{}", standing.usage_recorded, standing.appearances),
            standing
                .median_durable_ms
                .map_or_else(|| "n/a".to_owned(), duration),
            tokens.map_or_else(|| "n/a".to_owned(), |value| value.to_string()),
            standing
                .cost_microusd
                .map_or_else(|| "n/a".to_owned(), money),
            standing
                .cost_per_durable_microusd
                .map_or_else(|| "n/a".to_owned(), money),
        );
    }
    output
}

fn rotate_seats(manifest: &mut ArenaManifest, offset: usize) {
    let territories: Vec<_> = manifest
        .territories
        .iter()
        .map(|territory| territory.id.clone())
        .collect();
    if territories.is_empty() {
        return;
    }
    let original: HashMap<_, _> = territories
        .iter()
        .enumerate()
        .map(|(index, territory)| (territory.clone(), index))
        .collect();
    for agent in &mut manifest.agents {
        let index = original
            .get(&agent.territory)
            .copied()
            .expect("validated agent territory");
        agent
            .territory
            .clone_from(&territories[(index + offset) % territories.len()]);
    }
}

fn round_ports(
    base_port: u16,
    multicast_port: u16,
    territories: usize,
    round_index: usize,
) -> Result<(u16, u16), SeriesError> {
    let stride = territories.checked_mul(2).ok_or(SeriesError::PortRange)?;
    let offset = stride
        .checked_mul(round_index)
        .and_then(|value| u16::try_from(value).ok())
        .ok_or(SeriesError::PortRange)?;
    let multicast_offset = u16::try_from(round_index).map_err(|_| SeriesError::PortRange)?;
    Ok((
        base_port
            .checked_add(offset)
            .ok_or(SeriesError::PortRange)?,
        multicast_port
            .checked_add(multicast_offset)
            .ok_or(SeriesError::PortRange)?,
    ))
}

fn round_summary(
    round: usize,
    output: PathBuf,
    manifest: &ArenaManifest,
    state: &WorldState,
    usage_agents: Vec<String>,
) -> SeriesRound {
    let seats = manifest
        .agents
        .iter()
        .map(|agent| (agent.id.clone(), agent.territory.clone()))
        .collect();
    let winner_agent = state.winner.as_ref().and_then(|territory| {
        state
            .territories
            .get(territory)
            .and_then(|view| view.agent.clone())
    });
    SeriesRound {
        round,
        output,
        seats,
        winner_territory: state.winner.clone(),
        winner_agent,
        duration_ms: state.elapsed_ms,
        usage_agents,
    }
}

fn build_summary(
    manifest: &ArenaManifest,
    requested: usize,
    rounds: Vec<SeriesRound>,
    states: &[WorldState],
) -> SeriesSummary {
    let mut standings: Vec<_> = manifest
        .agents
        .iter()
        .map(|agent| {
            let durable_times: Vec<_> = states
                .iter()
                .filter_map(|state| agent_territory(state, &agent.id))
                .filter_map(|territory| territory.durable_at_ms)
                .collect();
            let usage = states
                .iter()
                .filter_map(|state| state.agents.get(&agent.id));
            let (input_tokens, output_tokens, cost_microusd) =
                usage.fold((0_u64, 0_u64, 0_u64), |(input, output, cost), view| {
                    (
                        input.saturating_add(view.input_tokens),
                        output.saturating_add(view.output_tokens),
                        cost.saturating_add(view.cost_microusd),
                    )
                });
            let durable_deployments = durable_times.len();
            let usage_recorded = rounds
                .iter()
                .filter(|round| round.usage_agents.iter().any(|id| id == &agent.id))
                .count();
            let complete_usage = usage_recorded == states.len();
            SeriesStanding {
                agent: agent.id.clone(),
                model: agent.model.clone(),
                appearances: states.len(),
                wins: rounds
                    .iter()
                    .filter(|round| round.winner_agent.as_deref() == Some(agent.id.as_str()))
                    .count(),
                durable_deployments,
                median_durable_ms: median(durable_times),
                usage_recorded,
                input_tokens: complete_usage.then_some(input_tokens),
                output_tokens: complete_usage.then_some(output_tokens),
                cost_microusd: complete_usage.then_some(cost_microusd),
                cost_per_durable_microusd: (complete_usage && durable_deployments > 0)
                    .then(|| cost_microusd / u64::try_from(durable_deployments).unwrap_or(1)),
            }
        })
        .collect();
    standings.sort_by(|left, right| {
        right
            .wins
            .cmp(&left.wins)
            .then_with(|| right.durable_deployments.cmp(&left.durable_deployments))
            .then_with(|| left.median_durable_ms.cmp(&right.median_durable_ms))
            .then_with(|| left.cost_microusd.cmp(&right.cost_microusd))
    });
    SeriesSummary {
        schema_version: SERIES_SCHEMA_VERSION,
        arena_id: manifest.arena.id.clone(),
        rounds_requested: requested,
        rounds_completed: rounds.len(),
        rounds,
        standings,
    }
}

fn agent_territory<'a>(
    state: &'a WorldState,
    agent: &str,
) -> Option<&'a aoe_replay::TerritoryView> {
    state
        .territories
        .values()
        .find(|territory| territory.agent.as_deref() == Some(agent))
}

fn median(mut values: Vec<u64>) -> Option<u64> {
    values.sort_unstable();
    values.get(values.len() / 2).copied()
}

fn write_summary(path: &Path, summary: &SeriesSummary) -> Result<(), SeriesError> {
    let temporary = path.with_extension("json.partial");
    fs::write(&temporary, serde_json::to_vec_pretty(summary)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn read_usage_agents(path: &Path) -> Result<Vec<String>, SeriesError> {
    let source = fs::read_to_string(path)?;
    let mut agents = BTreeSet::new();
    for line in source.lines().filter(|line| !line.trim().is_empty()) {
        let event: serde_json::Value = serde_json::from_str(line)?;
        if event.get("kind").and_then(serde_json::Value::as_str) == Some("usage_charged")
            && ["input_tokens", "output_tokens", "cost_microusd"]
                .iter()
                .any(|field| event.get(field).is_some_and(|value| !value.is_null()))
            && let Some(agent) = event.get("agent").and_then(serde_json::Value::as_str)
        {
            agents.insert(agent.to_owned());
        }
    }
    Ok(agents.into_iter().collect())
}

fn duration(milliseconds: u64) -> String {
    let seconds = milliseconds / 1_000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn money(microusd: u64) -> String {
    let ten_thousandths = microusd.saturating_add(50) / 100;
    format!(
        "${}.{:04}",
        ten_thousandths / 10_000,
        ten_thousandths % 10_000
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use aoe_domain::ArenaManifest;
    use aoe_replay::WorldState;

    use super::{
        archive_incomplete_round, build_summary, load_checkpoint, render_series, rotate_seats,
        round_ports, write_summary,
    };

    const MANIFEST: &str = include_str!("../../runtime/tests/fixture.toml");

    #[test]
    fn rotates_each_agent_through_each_territory() {
        let original = ArenaManifest::parse(MANIFEST).expect("manifest");
        let mut seen = std::collections::HashMap::<String, Vec<String>>::new();
        for offset in 0..original.territories.len() {
            let mut round = original.clone();
            rotate_seats(&mut round, offset);
            for agent in round.agents {
                seen.entry(agent.id).or_default().push(agent.territory);
            }
        }
        for territories in seen.values_mut() {
            territories.sort();
            territories.dedup();
            assert_eq!(territories.len(), original.territories.len());
        }
    }

    #[test]
    fn allocates_a_distinct_port_block_for_each_round() {
        assert_eq!(
            round_ports(26_000, 23_977, 3, 0).expect("round 1"),
            (26_000, 23_977)
        );
        assert_eq!(
            round_ports(26_000, 23_977, 3, 1).expect("round 2"),
            (26_006, 23_978)
        );
        assert_eq!(
            round_ports(26_000, 23_977, 3, 2).expect("round 3"),
            (26_012, 23_979)
        );
        assert!(round_ports(65_530, 65_535, 3, 1).is_err());
    }

    #[test]
    fn summary_sorts_wins_and_computes_cost_per_durable() {
        let manifest = ArenaManifest::parse(MANIFEST).expect("manifest");
        let mut first = WorldState::default();
        for agent in &manifest.agents {
            first.agents.insert(
                agent.id.clone(),
                aoe_replay::AgentView {
                    input_tokens: 100,
                    output_tokens: 10,
                    cost_microusd: 900,
                    ..aoe_replay::AgentView::default()
                },
            );
            first
                .territories
                .entry(agent.territory.clone())
                .or_default()
                .agent = Some(agent.id.clone());
        }
        let winner = manifest.agents[0].territory.clone();
        first.winner = Some(winner.clone());
        first
            .territories
            .get_mut(&winner)
            .expect("winner territory")
            .durable_at_ms = Some(2_000);
        let rounds = vec![super::round_summary(
            1,
            "round-001".into(),
            &manifest,
            &first,
            manifest
                .agents
                .iter()
                .map(|agent| agent.id.clone())
                .collect(),
        )];
        let summary = build_summary(&manifest, 1, rounds, &[first]);
        assert_eq!(summary.standings[0].agent, manifest.agents[0].id);
        assert_eq!(summary.standings[0].wins, 1);
        assert_eq!(summary.standings[0].median_durable_ms, Some(2_000));
        assert_eq!(summary.standings[0].input_tokens, Some(100));
        assert_eq!(summary.standings[0].cost_per_durable_microusd, Some(900));
        assert!(render_series(&summary).contains("$0.0009"));
        assert_eq!(super::money(1_150), "$0.0012");
    }

    #[test]
    fn incomplete_usage_is_not_reported_as_zero() {
        let manifest = ArenaManifest::parse(MANIFEST).expect("manifest");
        let state = WorldState::default();
        let rounds = vec![super::round_summary(
            1,
            "round-001".into(),
            &manifest,
            &state,
            Vec::new(),
        )];
        let summary = build_summary(&manifest, 1, rounds, &[state]);
        assert!(
            summary
                .standings
                .iter()
                .all(|standing| standing.input_tokens.is_none()
                    && standing.output_tokens.is_none()
                    && standing.cost_microusd.is_none())
        );
    }

    #[test]
    fn loads_a_compatible_series_checkpoint() {
        let root = temporary_directory("resume");
        let manifest = ArenaManifest::parse(MANIFEST).expect("manifest");
        let state = WorldState::default();
        let round_output = root.join("round-001");
        fs::create_dir_all(&round_output).expect("round directory");
        fs::write(
            round_output.join("world.json"),
            serde_json::to_vec_pretty(&state).expect("world json"),
        )
        .expect("world");
        let rounds = vec![super::round_summary(
            1,
            round_output,
            &manifest,
            &state,
            Vec::new(),
        )];
        let summary = build_summary(&manifest, 2, rounds, &[state]);
        let summary_path = root.join("series.json");
        write_summary(&summary_path, &summary).expect("summary");

        let (rounds, states) =
            load_checkpoint(&summary_path, &root, &manifest, 2).expect("checkpoint");
        assert_eq!(rounds.len(), 1);
        assert_eq!(states.len(), 1);
        assert!(load_checkpoint(&summary_path, &root, &manifest, 3).is_err());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn archives_an_uncheckpointed_round_before_retrying() {
        let root = temporary_directory("archive");
        let round = root.join("round-002");
        fs::create_dir_all(&round).expect("round directory");
        fs::write(round.join("evidence"), "kept").expect("evidence");

        archive_incomplete_round(&round).expect("archive");
        assert!(!round.exists());
        assert_eq!(
            fs::read_to_string(root.join("round-002.interrupted-1/evidence"))
                .expect("archived evidence"),
            "kept"
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    fn temporary_directory(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "agents-of-empires-series-{label}-{}",
            std::process::id()
        ));
        if path.exists() {
            fs::remove_dir_all(&path).expect("stale test directory");
        }
        fs::create_dir_all(&path).expect("test directory");
        path
    }
}
