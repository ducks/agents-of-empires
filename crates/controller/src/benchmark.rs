use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use aoe_domain::{ArenaManifest, FailureSource, MatchMode, MatchState};
use aoe_replay::WorldState;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::provenance::{arena_compatibility_key, read_provenance};
use crate::runner::RunOptions;
use crate::series::{SeriesError, SeriesOptions, SeriesSummary, run_series};

const SUITE_SCHEMA_VERSION: u32 = 1;
const BENCHMARK_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone)]
pub struct BenchmarkOptions {
    pub suite: PathBuf,
    pub output: PathBuf,
    pub adapters: HashMap<String, PathBuf>,
    pub credentials: HashMap<String, PathBuf>,
    pub base_port: u16,
    pub multicast_port: u16,
    pub color: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct SuiteManifest {
    schema_version: u32,
    suite: SuiteConfig,
    arenas: Vec<SuiteArena>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct SuiteConfig {
    id: String,
    rounds: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct SuiteArena {
    manifest: PathBuf,
    rounds: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkPlanEntry {
    pub arena_id: String,
    pub manifest: PathBuf,
    #[serde(default)]
    pub compatibility_key: String,
    pub rounds: usize,
    pub output: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkArenaSummary {
    pub arena_id: String,
    /// Presentation taxonomy copied from the arena manifest.
    #[serde(default)]
    pub category: Option<String>,
    pub output: PathBuf,
    pub rounds_requested: usize,
    pub rounds_completed: usize,
    pub aborted: bool,
    pub standings: Vec<BenchmarkStanding>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkStanding {
    pub model: String,
    #[serde(default)]
    pub adapter: String,
    #[serde(default)]
    pub reasoning_effort: String,
    pub appearances: usize,
    pub wins: usize,
    pub durable_deployments: usize,
    pub durable_times_ms: Vec<u64>,
    pub median_durable_ms: Option<u64>,
    pub milestone_passes: usize,
    pub milestones_available: usize,
    pub usage_recorded: usize,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost_microusd: Option<u64>,
    pub cost_per_durable_microusd: Option<u64>,
    pub failures: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkSummary {
    pub schema_version: u32,
    pub suite_id: String,
    pub arenas_requested: usize,
    pub arenas_completed: usize,
    pub plan: Vec<BenchmarkPlanEntry>,
    pub arenas: Vec<BenchmarkArenaSummary>,
    pub standings: Vec<BenchmarkStanding>,
}

#[derive(Debug, Error)]
pub enum BenchmarkError {
    #[error("benchmark suite I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("could not parse benchmark suite: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("invalid benchmark suite: {0}")]
    Invalid(String),
    #[error("arena manifest {path} failed: {source}")]
    Manifest {
        path: PathBuf,
        #[source]
        source: aoe_domain::ManifestError,
    },
    #[error("arena {arena} failed: {source}")]
    Series {
        arena: String,
        #[source]
        source: SeriesError,
    },
    #[error("existing benchmark checkpoint does not match this run: {0}")]
    ResumeMismatch(String),
    #[error("could not encode benchmark summary: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid match provenance in {path}: {source}")]
    Provenance {
        path: PathBuf,
        #[source]
        source: crate::provenance::ProvenanceError,
    },
}

/// Run every arena in a benchmark suite and checkpoint after each arena.
///
/// # Errors
///
/// Returns an error when the suite is invalid, a match fails, or an existing
/// checkpoint does not describe the same benchmark plan.
pub async fn run_benchmark(options: BenchmarkOptions) -> Result<BenchmarkSummary, BenchmarkError> {
    let (suite_id, plan) = load_plan(&options.suite, &options.output)?;
    fs::create_dir_all(&options.output)?;
    let summary_path = options.output.join("benchmark.json");
    let mut arenas = load_checkpoint(&summary_path, &suite_id, &plan)?;
    if !summary_path.exists() {
        write_summary(
            &summary_path,
            &build_summary(&suite_id, plan.clone(), arenas.clone()),
        )?;
    }
    if arenas
        .last()
        .is_some_and(|arena| arena.aborted || arena.rounds_completed < arena.rounds_requested)
    {
        return Ok(build_summary(&suite_id, plan, arenas));
    }

    for entry in plan.iter().skip(arenas.len()) {
        let series = run_series(SeriesOptions {
            run: RunOptions {
                manifest: entry.manifest.clone(),
                output: entry.output.clone(),
                adapters: options.adapters.clone(),
                credentials: options.credentials.clone(),
                base_port: options.base_port,
                multicast_port: options.multicast_port,
                color: options.color,
            },
            rounds: Some(entry.rounds),
        })
        .await
        .map_err(|source| BenchmarkError::Series {
            arena: entry.arena_id.clone(),
            source,
        })?;
        let arena = summarize_arena(entry, &series)?;
        let stop = arena.aborted || arena.rounds_completed < arena.rounds_requested;
        arenas.push(arena);
        let summary = build_summary(&suite_id, plan.clone(), arenas.clone());
        write_summary(&summary_path, &summary)?;
        if stop {
            return Ok(summary);
        }
    }

    Ok(build_summary(&suite_id, plan, arenas))
}

/// Render an aggregate model leaderboard for a benchmark suite.
#[must_use]
pub fn render_benchmark(summary: &BenchmarkSummary) -> String {
    let mut output = format!(
        "\nBENCHMARK {}  arenas={}/{}\n\n",
        summary.suite_id, summary.arenas_completed, summary.arenas_requested
    );
    let _ = writeln!(
        output,
        "{:<34} {:<18} {:<9} {:>5} {:>7} {:>7} {:>11} {:>9} {:>12} {:>10} {:>12}",
        "model",
        "adapter",
        "reasoning",
        "wins",
        "durable",
        "rounds",
        "milestones",
        "median",
        "tokens",
        "cost",
        "cost/durable"
    );
    let _ = writeln!(output, "{}", "-".repeat(145));
    for standing in &summary.standings {
        let tokens = standing
            .input_tokens
            .zip(standing.output_tokens)
            .map(|(input, output)| input.saturating_add(output));
        let _ = writeln!(
            output,
            "{:<34} {:<18} {:<9} {:>5} {:>7} {:>7} {:>11} {:>9} {:>12} {:>10} {:>12}",
            standing.model,
            standing.adapter,
            standing.reasoning_effort,
            standing.wins,
            standing.durable_deployments,
            standing.appearances,
            ratio(standing.milestone_passes, standing.milestones_available),
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

fn load_plan(
    suite_path: &Path,
    output: &Path,
) -> Result<(String, Vec<BenchmarkPlanEntry>), BenchmarkError> {
    let source = fs::read_to_string(suite_path)?;
    let suite: SuiteManifest = toml::from_str(&source)?;
    if suite.schema_version != SUITE_SCHEMA_VERSION {
        return Err(BenchmarkError::Invalid(format!(
            "unsupported schema version {}",
            suite.schema_version
        )));
    }
    if !safe_id(&suite.suite.id) {
        return Err(BenchmarkError::Invalid(
            "suite id must use only letters, numbers, '-' and '_'".into(),
        ));
    }
    if suite.suite.rounds == 0 {
        return Err(BenchmarkError::Invalid(
            "default round count must be greater than zero".into(),
        ));
    }
    if suite.arenas.is_empty() {
        return Err(BenchmarkError::Invalid(
            "at least one arena is required".into(),
        ));
    }
    let parent = suite_path.parent().unwrap_or_else(|| Path::new("."));
    let mut ids = BTreeSet::new();
    let mut paths = BTreeSet::new();
    let mut expected_fleet = None;
    let mut plan = Vec::with_capacity(suite.arenas.len());
    for (index, config) in suite.arenas.into_iter().enumerate() {
        let manifest_path = if config.manifest.is_absolute() {
            config.manifest
        } else {
            parent.join(config.manifest)
        };
        if !paths.insert(manifest_path.clone()) {
            return Err(BenchmarkError::Invalid(format!(
                "arena manifest {} is listed more than once",
                manifest_path.display()
            )));
        }
        let manifest =
            ArenaManifest::load(&manifest_path).map_err(|source| BenchmarkError::Manifest {
                path: manifest_path.clone(),
                source,
            })?;
        if manifest.arena.mode != MatchMode::BuildRace {
            return Err(BenchmarkError::Invalid(format!(
                "arena {} is not a build race",
                manifest.arena.id
            )));
        }
        if !ids.insert(manifest.arena.id.clone()) {
            return Err(BenchmarkError::Invalid(format!(
                "arena id {} is listed more than once",
                manifest.arena.id
            )));
        }
        let fleet = fleet_identity(&manifest);
        if let Some(expected) = &expected_fleet {
            if expected != &fleet {
                return Err(BenchmarkError::Invalid(format!(
                    "arena {} does not use the suite's model, adapter, and reasoning fleet",
                    manifest.arena.id
                )));
            }
        } else {
            expected_fleet = Some(fleet);
        }
        let rounds = config.rounds.unwrap_or(suite.suite.rounds);
        if rounds == 0 {
            return Err(BenchmarkError::Invalid(format!(
                "arena {} must have at least one round",
                manifest.arena.id
            )));
        }
        plan.push(BenchmarkPlanEntry {
            arena_id: manifest.arena.id.clone(),
            compatibility_key: arena_compatibility_key(&manifest_path, &manifest)?,
            manifest: manifest_path,
            rounds,
            output: output.join(format!("{:02}-{}", index + 1, manifest.arena.id)),
        });
    }
    Ok((suite.suite.id, plan))
}

fn load_checkpoint(
    path: &Path,
    suite_id: &str,
    plan: &[BenchmarkPlanEntry],
) -> Result<Vec<BenchmarkArenaSummary>, BenchmarkError> {
    if !path.exists() {
        return Ok(Vec::with_capacity(plan.len()));
    }
    let summary: BenchmarkSummary = serde_json::from_slice(&fs::read(path)?)?;
    if summary.schema_version != BENCHMARK_SCHEMA_VERSION
        || summary.suite_id != suite_id
        || summary.plan != plan
        || summary.arenas_requested != plan.len()
        || summary.arenas_completed != completed_arenas(&summary.arenas)
        || summary.arenas.len() > plan.len()
    {
        return Err(BenchmarkError::ResumeMismatch(
            "suite id, schema, plan, or completion count changed".into(),
        ));
    }
    for (arena, entry) in summary.arenas.iter().zip(plan) {
        if arena.arena_id != entry.arena_id || arena.output != entry.output {
            return Err(BenchmarkError::ResumeMismatch(
                "completed arenas are not a prefix of the current plan".into(),
            ));
        }
    }
    Ok(summary.arenas)
}

fn summarize_arena(
    entry: &BenchmarkPlanEntry,
    series: &SeriesSummary,
) -> Result<BenchmarkArenaSummary, BenchmarkError> {
    let manifest =
        ArenaManifest::load(&entry.manifest).map_err(|source| BenchmarkError::Manifest {
            path: entry.manifest.clone(),
            source,
        })?;
    let milestone_count = manifest
        .build
        .as_ref()
        .map_or(0, |build| build.milestones.len());
    let model_by_agent: HashMap<_, _> = manifest
        .agents
        .iter()
        .map(|agent| (agent.id.as_str(), agent))
        .collect();
    let mut by_model = BTreeMap::<(String, String, String), BenchmarkStanding>::new();
    let mut aborted = false;

    for round in &series.rounds {
        let provenance_path = entry
            .output
            .join(format!("round-{:03}", round.round))
            .join("match.json");
        let provenance =
            read_provenance(&provenance_path).map_err(|source| BenchmarkError::Provenance {
                path: provenance_path,
                source,
            })?;
        if provenance.compatibility_key != entry.compatibility_key {
            return Err(BenchmarkError::ResumeMismatch(format!(
                "arena {} round {} has compatibility key {}, expected {}",
                entry.arena_id, round.round, provenance.compatibility_key, entry.compatibility_key
            )));
        }
        let world_path = entry
            .output
            .join(format!("round-{:03}", round.round))
            .join("world.json");
        let state: WorldState = serde_json::from_slice(&fs::read(world_path)?)?;
        aborted |= state.match_state == MatchState::Aborted;
        for agent in &manifest.agents {
            let key = competitor_key(agent);
            let standing = by_model.entry(key).or_insert_with(|| BenchmarkStanding {
                model: agent.model.clone(),
                adapter: agent.adapter.clone(),
                reasoning_effort: agent.reasoning_effort.clone(),
                ..BenchmarkStanding::default()
            });
            standing.appearances += 1;
            standing.milestones_available += milestone_count;
            standing.wins += usize::from(round.winner_agent.as_deref() == Some(agent.id.as_str()));
            if let Some(territory) = state
                .territories
                .values()
                .find(|territory| territory.agent.as_deref() == Some(&agent.id))
            {
                standing.milestone_passes += territory
                    .milestones
                    .values()
                    .filter(|milestone| milestone.passed)
                    .count();
                if let Some(time) = territory.durable_at_ms {
                    standing.durable_times_ms.push(time);
                }
            }
            if let Some(source) = state
                .agents
                .get(&agent.id)
                .and_then(|view| view.failure_source)
            {
                *standing
                    .failures
                    .entry(failure_name(source).into())
                    .or_default() += 1;
            }
        }
    }

    for series_standing in &series.standings {
        let agent = model_by_agent
            .get(series_standing.agent.as_str())
            .copied()
            .ok_or_else(|| {
                BenchmarkError::Invalid(format!(
                    "series contains unknown agent {}",
                    series_standing.agent
                ))
            })?;
        let standing = by_model
            .entry(competitor_key(agent))
            .or_insert_with(|| BenchmarkStanding {
                model: agent.model.clone(),
                adapter: agent.adapter.clone(),
                reasoning_effort: agent.reasoning_effort.clone(),
                ..BenchmarkStanding::default()
            });
        standing.usage_recorded += series_standing.usage_recorded;
        add_value(&mut standing.input_tokens, series_standing.input_tokens);
        add_value(&mut standing.output_tokens, series_standing.output_tokens);
        add_value(&mut standing.cost_microusd, series_standing.cost_microusd);
    }
    let mut standings: Vec<_> = by_model.into_values().collect();
    for standing in &mut standings {
        finish_standing(standing);
    }
    sort_standings(&mut standings);
    Ok(BenchmarkArenaSummary {
        arena_id: entry.arena_id.clone(),
        category: manifest.arena.category.clone(),
        output: entry.output.clone(),
        rounds_requested: series.rounds_requested,
        rounds_completed: series.rounds_completed,
        aborted,
        standings,
    })
}

fn build_summary(
    suite_id: &str,
    plan: Vec<BenchmarkPlanEntry>,
    arenas: Vec<BenchmarkArenaSummary>,
) -> BenchmarkSummary {
    let mut by_model = BTreeMap::<(String, String, String), BenchmarkStanding>::new();
    for arena in &arenas {
        for source in &arena.standings {
            let target =
                by_model
                    .entry(standing_key(source))
                    .or_insert_with(|| BenchmarkStanding {
                        model: source.model.clone(),
                        adapter: source.adapter.clone(),
                        reasoning_effort: source.reasoning_effort.clone(),
                        ..BenchmarkStanding::default()
                    });
            target.appearances += source.appearances;
            target.wins += source.wins;
            target.milestone_passes += source.milestone_passes;
            target.milestones_available += source.milestones_available;
            target.usage_recorded += source.usage_recorded;
            target
                .durable_times_ms
                .extend_from_slice(&source.durable_times_ms);
            add_value(&mut target.input_tokens, source.input_tokens);
            add_value(&mut target.output_tokens, source.output_tokens);
            add_value(&mut target.cost_microusd, source.cost_microusd);
            for (failure, count) in &source.failures {
                *target.failures.entry(failure.clone()).or_default() += count;
            }
        }
    }
    let mut standings: Vec<_> = by_model.into_values().collect();
    for standing in &mut standings {
        finish_standing(standing);
    }
    sort_standings(&mut standings);
    BenchmarkSummary {
        schema_version: BENCHMARK_SCHEMA_VERSION,
        suite_id: suite_id.to_owned(),
        arenas_requested: plan.len(),
        arenas_completed: completed_arenas(&arenas),
        plan,
        arenas,
        standings,
    }
}

fn fleet_identity(manifest: &ArenaManifest) -> Vec<(String, String, String)> {
    let mut fleet: Vec<_> = manifest.agents.iter().map(competitor_key).collect();
    fleet.sort();
    fleet
}

fn competitor_key(agent: &aoe_domain::AgentConfig) -> (String, String, String) {
    (
        agent.model.clone(),
        agent.adapter.clone(),
        agent.reasoning_effort.clone(),
    )
}

fn standing_key(standing: &BenchmarkStanding) -> (String, String, String) {
    (
        standing.model.clone(),
        standing.adapter.clone(),
        standing.reasoning_effort.clone(),
    )
}

fn completed_arenas(arenas: &[BenchmarkArenaSummary]) -> usize {
    arenas
        .iter()
        .filter(|arena| !arena.aborted && arena.rounds_completed == arena.rounds_requested)
        .count()
}

fn finish_standing(standing: &mut BenchmarkStanding) {
    standing.durable_times_ms.sort_unstable();
    standing.durable_deployments = standing.durable_times_ms.len();
    standing.median_durable_ms = standing
        .durable_times_ms
        .get(standing.durable_times_ms.len() / 2)
        .copied();
    if standing.usage_recorded != standing.appearances {
        standing.input_tokens = None;
        standing.output_tokens = None;
        standing.cost_microusd = None;
    }
    standing.cost_per_durable_microusd = standing.cost_microusd.and_then(|cost| {
        u64::try_from(standing.durable_deployments)
            .ok()
            .filter(|count| *count > 0)
            .map(|count| cost / count)
    });
}

fn add_value(target: &mut Option<u64>, value: Option<u64>) {
    if let Some(value) = value {
        *target = Some(target.unwrap_or(0).saturating_add(value));
    }
}

fn sort_standings(standings: &mut [BenchmarkStanding]) {
    standings.sort_by(|left, right| {
        right
            .wins
            .cmp(&left.wins)
            .then_with(|| right.durable_deployments.cmp(&left.durable_deployments))
            .then_with(|| coverage_cmp(right, left))
            .then_with(|| left.median_durable_ms.cmp(&right.median_durable_ms))
            .then_with(|| left.cost_microusd.cmp(&right.cost_microusd))
            .then_with(|| left.model.cmp(&right.model))
    });
}

fn coverage_cmp(left: &BenchmarkStanding, right: &BenchmarkStanding) -> Ordering {
    match (left.milestones_available, right.milestones_available) {
        (0, 0) => Ordering::Equal,
        (0, _) => Ordering::Less,
        (_, 0) => Ordering::Greater,
        (left_total, right_total) => left
            .milestone_passes
            .saturating_mul(right_total)
            .cmp(&right.milestone_passes.saturating_mul(left_total)),
    }
}

fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

fn failure_name(source: FailureSource) -> &'static str {
    match source {
        FailureSource::Player => "player",
        FailureSource::Harness => "harness",
        FailureSource::Provider => "provider",
        FailureSource::Arena => "arena",
        FailureSource::Controller => "controller",
        FailureSource::Unknown => "unknown",
    }
}

fn ratio(value: usize, total: usize) -> String {
    if total == 0 {
        "n/a".into()
    } else {
        format!("{value}/{total}")
    }
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

fn write_summary(path: &Path, summary: &BenchmarkSummary) -> Result<(), BenchmarkError> {
    let temporary = path.with_extension("json.partial");
    fs::write(&temporary, serde_json::to_vec_pretty(summary)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;

    use aoe_domain::ArenaManifest;

    use super::{
        BenchmarkArenaSummary, BenchmarkStanding, build_summary, fleet_identity, load_checkpoint,
        load_plan, render_benchmark, write_summary,
    };

    const MANIFEST: &str = include_str!("../../runtime/tests/fixture.toml");

    #[test]
    fn loads_the_built_in_infrastructure_suite() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let (_, plan) = load_plan(
            &root.join("suites/infra-core.toml"),
            &root.join("benchmarks/test"),
        )
        .expect("suite");
        assert_eq!(plan.len(), 4);
        assert!(plan.iter().all(|entry| entry.rounds == 3));
        assert!(plan.iter().all(|entry| !entry.compatibility_key.is_empty()));
        assert_eq!(plan[0].arena_id, "first-build-real");
        assert_eq!(plan[3].arena_id, "primary-failover");
    }

    #[test]
    fn aggregates_models_across_arenas() {
        let arenas = vec![
            arena("first", 1, 1_000, 2, 4, Some(800)),
            arena("second", 0, 3_000, 3, 4, Some(1_200)),
        ];
        let summary = build_summary("infra", Vec::new(), arenas);
        let standing = &summary.standings[0];
        assert_eq!(standing.model, "model-a");
        assert_eq!(standing.appearances, 2);
        assert_eq!(standing.wins, 1);
        assert_eq!(standing.durable_deployments, 2);
        assert_eq!(standing.median_durable_ms, Some(3_000));
        assert_eq!(standing.milestone_passes, 5);
        assert_eq!(standing.cost_microusd, Some(2_000));
        let rendered = render_benchmark(&summary);
        assert!(rendered.contains("5/8"));
        assert!(rendered.contains("high"));
        assert!(rendered.contains("cost/durable"));
        assert!(rendered.contains("$0.0010"));
    }

    #[test]
    fn fleet_identity_includes_adapter_and_reasoning_effort() {
        let manifest = ArenaManifest::parse(MANIFEST).expect("manifest");
        let expected = fleet_identity(&manifest);

        let mut changed_reasoning = manifest.clone();
        changed_reasoning.agents[0].reasoning_effort = "low".into();
        assert_ne!(fleet_identity(&changed_reasoning), expected);

        let mut changed_adapter = manifest;
        changed_adapter.agents[0].adapter = "other-harness".into();
        assert_ne!(fleet_identity(&changed_adapter), expected);
    }

    #[test]
    fn changed_compatibility_key_rejects_a_checkpoint() {
        let root = temporary_directory("compatibility");
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let (suite_id, plan) =
            load_plan(&repository.join("suites/infra-core.toml"), &root).expect("suite");
        let summary = build_summary(&suite_id, plan.clone(), Vec::new());
        let checkpoint = root.join("benchmark.json");
        write_summary(&checkpoint, &summary).expect("checkpoint");

        let mut changed = plan;
        changed[0].compatibility_key = "changed".into();
        assert!(load_checkpoint(&checkpoint, &suite_id, &changed).is_err());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn incomplete_and_aborted_arenas_are_not_counted_as_completed() {
        let mut complete = arena("complete", 1, 1_000, 4, 4, Some(800));
        complete.rounds_requested = 1;
        complete.rounds_completed = 1;
        let mut incomplete = arena("incomplete", 0, 2_000, 1, 4, Some(400));
        incomplete.rounds_requested = 3;
        incomplete.rounds_completed = 1;
        let mut aborted = arena("aborted", 0, 3_000, 0, 4, Some(200));
        aborted.aborted = true;

        let summary = build_summary("infra", Vec::new(), vec![complete, incomplete, aborted]);
        assert_eq!(summary.arenas_completed, 1);
    }

    #[test]
    fn historical_arena_summaries_default_to_no_category() {
        let mut value = serde_json::to_value(arena("historical", 1, 1_000, 4, 4, Some(800)))
            .expect("summary JSON");
        value
            .as_object_mut()
            .expect("summary object")
            .remove("category");
        let decoded: BenchmarkArenaSummary =
            serde_json::from_value(value).expect("historical summary");
        assert_eq!(decoded.category, None);
    }

    fn arena(
        id: &str,
        wins: usize,
        durable_ms: u64,
        passes: usize,
        available: usize,
        cost: Option<u64>,
    ) -> BenchmarkArenaSummary {
        BenchmarkArenaSummary {
            arena_id: id.into(),
            category: None,
            output: id.into(),
            rounds_requested: 1,
            rounds_completed: 1,
            aborted: false,
            standings: vec![BenchmarkStanding {
                model: "model-a".into(),
                adapter: "test".into(),
                reasoning_effort: "high".into(),
                appearances: 1,
                wins,
                durable_deployments: 1,
                durable_times_ms: vec![durable_ms],
                median_durable_ms: Some(durable_ms),
                milestone_passes: passes,
                milestones_available: available,
                usage_recorded: 1,
                input_tokens: Some(10),
                output_tokens: Some(5),
                cost_microusd: cost,
                cost_per_durable_microusd: cost,
                failures: BTreeMap::new(),
            }],
        }
    }

    fn temporary_directory(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "agents-of-empires-benchmark-{label}-{}",
            std::process::id()
        ));
        if path.exists() {
            fs::remove_dir_all(&path).expect("stale test directory");
        }
        fs::create_dir_all(&path).expect("test directory");
        path
    }
}
