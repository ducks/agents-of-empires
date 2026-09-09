//! Weekly seasons: a fleet of models, a committed random draw, and a bracket
//! of three-seat heats whose winners advance to a final.
//!
//! A season manifest names the fleet and an arena pool. `draw_week` turns the
//! season id, the week, and an optional salt into a published draw seed, and
//! from that seed derives heat composition, the arena for each round, and seat
//! assignment. The draw is written before any inference runs so it can be
//! audited. A second, secret seed for arena-internal variation is generated
//! randomly; only its commitment is published with the draw, and the seed
//! itself is revealed once the week completes, so verifier-side values stay
//! unpredictable to competitors during the week.
//!
//! `run_week` executes the bracket heat by heat through the ordinary match
//! runner, checkpointing `week.json` after every heat so an interrupted week
//! resumes. A seat whose result is unavailable (provider or harness failure)
//! earns the heat one automatic replay; on a repeat it forfeits and is never
//! counted as a loss. Heats stop at the first durable deployment, as the
//! arenas declare: losing the race is the result.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use aoe_domain::{AgentConfig, ArenaManifest, FailureSource, MatchState};
use aoe_replay::WorldState;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::provenance::arena_compatibility_key;
use crate::runner::{RunError, RunOptions, run_match_with_manifest};

pub const SEASON_SCHEMA_VERSION: u32 = 1;
pub const DRAW_SCHEMA_VERSION: u32 = 1;
pub const WEEK_SCHEMA_VERSION: u32 = 1;
const SECRET_SEED_FILE: &str = "seed.secret";

// ---------------------------------------------------------------------------
// Season manifest
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeasonManifest {
    pub schema_version: u32,
    pub season: SeasonConfig,
    #[serde(default)]
    pub rules: SeasonRules,
    pub fleet: Vec<FleetEntry>,
    pub arenas: Vec<SeasonArena>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeasonConfig {
    pub id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeasonRules {
    /// Automatic replays of a heat when a seat's result is unavailable.
    /// After the last replay the unavailable seat forfeits.
    #[serde(default = "default_unavailable_replays")]
    pub unavailable_replays: usize,
}

impl Default for SeasonRules {
    fn default() -> Self {
        Self {
            unavailable_replays: default_unavailable_replays(),
        }
    }
}

fn default_unavailable_replays() -> usize {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetEntry {
    /// Stable agent id used in manifests, artifacts, and standings.
    pub id: String,
    pub model: String,
    pub adapter: String,
    #[serde(default = "default_reasoning_effort")]
    pub reasoning_effort: String,
}

fn default_reasoning_effort() -> String {
    "high".to_owned()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeasonArena {
    /// Arena manifest whose territories, rules, verifiers, and budgets are
    /// used. Its `[[agents]]` entries are templates only; the draw replaces
    /// them with the heat's fleet members.
    pub manifest: PathBuf,
}

impl SeasonManifest {
    /// Load and validate a season manifest. Relative arena paths resolve
    /// from the manifest's directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read or parsed, or when the
    /// fleet, arena pool, or rules are invalid.
    pub fn load(path: &Path) -> Result<Self, SeasonError> {
        let text = fs::read_to_string(path).map_err(|source| SeasonError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let mut manifest: Self = toml::from_str(&text)?;
        if manifest.schema_version != SEASON_SCHEMA_VERSION {
            return Err(SeasonError::Invalid(format!(
                "schema_version is {}, expected {SEASON_SCHEMA_VERSION}",
                manifest.schema_version
            )));
        }
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        for arena in &mut manifest.arenas {
            if arena.manifest.is_relative() {
                arena.manifest = base.join(&arena.manifest);
            }
        }
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<(), SeasonError> {
        if self.season.id.is_empty() || !safe_id(&self.season.id) {
            return Err(SeasonError::Invalid(
                "season.id must be a non-empty [A-Za-z0-9._-] identifier".into(),
            ));
        }
        if self.fleet.len() < 3 {
            return Err(SeasonError::Invalid(format!(
                "a season needs at least three fleet entries, found {}",
                self.fleet.len()
            )));
        }
        let mut ids = BTreeSet::new();
        for entry in &self.fleet {
            if !safe_id(&entry.id) {
                return Err(SeasonError::Invalid(format!(
                    "fleet id {:?} must be a [A-Za-z0-9._-] identifier",
                    entry.id
                )));
            }
            if !ids.insert(entry.id.clone()) {
                return Err(SeasonError::Invalid(format!(
                    "fleet id {:?} is listed twice",
                    entry.id
                )));
            }
            if entry.model.is_empty() || entry.adapter.is_empty() {
                return Err(SeasonError::Invalid(format!(
                    "fleet entry {:?} needs a model and an adapter",
                    entry.id
                )));
            }
        }
        if self.arenas.is_empty() {
            return Err(SeasonError::Invalid(
                "a season needs at least one arena".into(),
            ));
        }
        Ok(())
    }
}

fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

// ---------------------------------------------------------------------------
// Deterministic randomness
// ---------------------------------------------------------------------------

/// SplitMix64 seeded from a SHA-256 of a published string. Small, portable,
/// and reproducible by anyone with the draw seed; not a cryptographic RNG
/// and never used for secrets.
#[derive(Debug, Clone)]
pub struct DrawRng(u64);

impl DrawRng {
    #[must_use]
    pub fn from_seed(seed: &str) -> Self {
        let digest = Sha256::digest(seed.as_bytes());
        let mut bytes = [0_u8; 8];
        bytes.copy_from_slice(&digest[..8]);
        Self(u64::from_le_bytes(bytes))
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        debug_assert!(bound > 0);
        // Rejection sampling keeps the draw unbiased.
        let bound_u64 = bound as u64;
        let zone = u64::MAX - (u64::MAX % bound_u64);
        loop {
            let value = self.next_u64();
            if value < zone {
                return (value % bound_u64) as usize;
            }
        }
    }

    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i + 1);
            items.swap(i, j);
        }
    }
}

// ---------------------------------------------------------------------------
// Bracket shape
// ---------------------------------------------------------------------------

/// How one round of a bracket is filled, computed from the field size alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoundShape {
    pub round: usize,
    /// Competitors entering the round.
    pub field: usize,
    pub heats: usize,
    /// Competitors who sit out this round and advance unplayed.
    pub byes: usize,
    /// Non-winners promoted from this round to fill the next one.
    pub wildcards: usize,
}

/// The rounds needed to reduce `fleet` competitors to one champion with
/// heats of `heat_size`. Winners and byes advance; when the next field is
/// smaller than a heat or not a whole number of heats, the best non-winners
/// of the round fill it.
///
/// # Errors
///
/// Returns an error when the fleet cannot form a single heat.
pub fn bracket_shape(fleet: usize, heat_size: usize) -> Result<Vec<RoundShape>, SeasonError> {
    if heat_size < 2 {
        return Err(SeasonError::Invalid(
            "heat size must be at least two".into(),
        ));
    }
    if fleet < heat_size {
        return Err(SeasonError::Invalid(format!(
            "a fleet of {fleet} cannot fill a heat of {heat_size}"
        )));
    }
    let mut rounds = Vec::new();
    let mut field = fleet;
    let mut round = 1;
    loop {
        let heats = field / heat_size;
        let byes = field % heat_size;
        if heats == 1 && byes == 0 {
            rounds.push(RoundShape {
                round,
                field,
                heats,
                byes,
                wildcards: 0,
            });
            return Ok(rounds);
        }
        let advancing = heats + byes;
        let wildcards = if advancing < heat_size {
            heat_size - advancing
        } else {
            (heat_size - advancing % heat_size) % heat_size
        };
        let candidates = heats * (heat_size - 1);
        if wildcards > candidates {
            return Err(SeasonError::Invalid(format!(
                "round {round} needs {wildcards} wildcards but only {candidates} non-winners exist"
            )));
        }
        rounds.push(RoundShape {
            round,
            field,
            heats,
            byes,
            wildcards,
        });
        field = advancing + wildcards;
        round += 1;
    }
}

// ---------------------------------------------------------------------------
// Draw
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DrawOptions {
    pub season: PathBuf,
    /// Week label, e.g. `2026-W37`. Part of the published draw seed.
    pub week: String,
    /// Optional extra entropy published with the draw.
    pub salt: Option<String>,
    pub output: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeekDraw {
    pub schema_version: u32,
    pub season_id: String,
    pub season_manifest: PathBuf,
    pub week: String,
    /// The published string every shuffle in this week derives from.
    pub draw_seed: String,
    /// SHA-256 of the secret variation seed handed to verifiers. Revealed in
    /// the week summary once the week completes.
    pub variation_seed_commitment: String,
    pub heat_size: usize,
    pub rules: SeasonRules,
    pub fleet: Vec<FleetEntry>,
    pub arenas: Vec<DrawArena>,
    pub shape: Vec<RoundShape>,
    /// Arena drawn for each round, by index into `arenas`.
    pub round_arenas: Vec<usize>,
    /// Round one is fully drawn; later rounds depend on results.
    pub first_round: RoundDraw,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrawArena {
    pub arena_id: String,
    pub manifest: PathBuf,
    pub compatibility_key: String,
    pub territories: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoundDraw {
    pub round: usize,
    pub heats: Vec<HeatDraw>,
    pub byes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeatDraw {
    pub heat: usize,
    /// Territory id to fleet id.
    pub seats: BTreeMap<String, String>,
}

/// Draw a week: shuffle the fleet into heats, pick an arena per round, and
/// assign seats, all from a published seed. Writes `draw.json` and a
/// private `seed.secret` (mode 0600 on Unix) into the output directory.
///
/// # Errors
///
/// Returns an error when the season or an arena manifest is invalid, the
/// arenas disagree on territory count, the output already holds a draw, or
/// files cannot be written.
pub fn draw_week(options: &DrawOptions) -> Result<WeekDraw, SeasonError> {
    let season = SeasonManifest::load(&options.season)?;
    if options.week.is_empty() || !safe_id(&options.week) {
        return Err(SeasonError::Invalid(
            "week must be a non-empty [A-Za-z0-9._-] label such as 2026-W37".into(),
        ));
    }
    let draw_path = options.output.join("draw.json");
    if draw_path.exists() {
        return Err(SeasonError::DrawExists(draw_path));
    }

    let mut arenas = Vec::with_capacity(season.arenas.len());
    let mut heat_size = None;
    for arena in &season.arenas {
        let manifest =
            ArenaManifest::load(&arena.manifest).map_err(|source| SeasonError::Manifest {
                path: arena.manifest.clone(),
                source,
            })?;
        let territories: Vec<String> = manifest
            .territories
            .iter()
            .map(|territory| territory.id.clone())
            .collect();
        match heat_size {
            None => heat_size = Some(territories.len()),
            Some(size) if size != territories.len() => {
                return Err(SeasonError::Invalid(format!(
                    "arena {} has {} territories but the season's heats are {size} seats",
                    manifest.arena.id,
                    territories.len()
                )));
            }
            Some(_) => {}
        }
        let compatibility_key = arena_compatibility_key(&arena.manifest, &manifest)?;
        arenas.push(DrawArena {
            arena_id: manifest.arena.id.clone(),
            manifest: arena.manifest.clone(),
            compatibility_key,
            territories,
        });
    }
    let heat_size = heat_size.expect("validated non-empty arena pool");
    let shape = bracket_shape(season.fleet.len(), heat_size)?;

    let draw_seed = match &options.salt {
        Some(salt) if !salt.is_empty() => {
            format!("{}/{}/{salt}", season.season.id, options.week)
        }
        _ => format!("{}/{}", season.season.id, options.week),
    };
    let mut rng = DrawRng::from_seed(&draw_seed);

    // Round arenas: one independent pick per round.
    let round_arenas: Vec<usize> = (0..shape.len()).map(|_| rng.below(arenas.len())).collect();

    // Round one: shuffle the fleet, chunk into heats, seat each heat.
    let mut order: Vec<String> = season.fleet.iter().map(|entry| entry.id.clone()).collect();
    rng.shuffle(&mut order);
    let first = &shape[0];
    let mut heats = Vec::with_capacity(first.heats);
    let territories = &arenas[round_arenas[0]].territories;
    for (index, chunk) in order.chunks(heat_size).take(first.heats).enumerate() {
        heats.push(HeatDraw {
            heat: index + 1,
            seats: seat_heat(&mut rng, territories, chunk),
        });
    }
    let byes = order[first.heats * heat_size..].to_vec();

    let secret = random_secret()?;
    let commitment = format!("{:x}", Sha256::digest(secret.as_bytes()));

    let draw = WeekDraw {
        schema_version: DRAW_SCHEMA_VERSION,
        season_id: season.season.id.clone(),
        season_manifest: options.season.clone(),
        week: options.week.clone(),
        draw_seed,
        variation_seed_commitment: commitment,
        heat_size,
        rules: season.rules.clone(),
        fleet: season.fleet.clone(),
        arenas,
        shape,
        round_arenas,
        first_round: RoundDraw {
            round: 1,
            heats,
            byes,
        },
    };

    fs::create_dir_all(&options.output)?;
    write_private(&options.output.join(SECRET_SEED_FILE), secret.as_bytes())?;
    write_json_atomic(&draw_path, &draw)?;
    Ok(draw)
}

/// Assign the given fleet members to territories in a seeded random order.
fn seat_heat(
    rng: &mut DrawRng,
    territories: &[String],
    members: &[String],
) -> BTreeMap<String, String> {
    let mut seats: Vec<&String> = territories.iter().collect();
    rng.shuffle(&mut seats);
    seats
        .into_iter()
        .zip(members)
        .map(|(territory, member)| (territory.clone(), member.clone()))
        .collect()
}

fn random_secret() -> Result<String, SeasonError> {
    let mut bytes = [0_u8; 32];
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), SeasonError> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    use std::io::Write as _;
    options.open(path)?.write_all(bytes)?;
    Ok(())
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), SeasonError> {
    let partial = path.with_extension("json.partial");
    fs::write(&partial, serde_json::to_vec_pretty(value)?)?;
    fs::rename(&partial, path)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Week execution
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct WeekOptions {
    /// Directory holding `draw.json` and `seed.secret`.
    pub week_dir: PathBuf,
    pub adapters: HashMap<String, PathBuf>,
    pub credentials: HashMap<String, PathBuf>,
    pub base_port: u16,
    pub multicast_port: u16,
    pub color: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeekSummary {
    pub schema_version: u32,
    pub season_id: String,
    pub week: String,
    pub draw_seed: String,
    pub variation_seed_commitment: String,
    /// The secret seed, revealed only once every heat has run.
    #[serde(default)]
    pub variation_seed: Option<String>,
    pub completed: bool,
    pub rounds: Vec<RoundResult>,
    #[serde(default)]
    pub champion: Option<String>,
    pub standings: Vec<WeekStanding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoundResult {
    pub round: usize,
    pub arena_id: String,
    pub heats: Vec<HeatResult>,
    pub byes: Vec<String>,
    /// Non-winners promoted to the next round, best first.
    #[serde(default)]
    pub wildcards: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeatResult {
    pub heat: usize,
    pub output: PathBuf,
    /// Attempts run, including replays for unavailable seats.
    pub attempts: usize,
    pub seats: BTreeMap<String, String>,
    pub winner: Option<String>,
    pub standings: Vec<SeatResult>,
    pub aborted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeatOutcome {
    Durable,
    Incomplete,
    Failed,
    /// Provider or harness failure that survived every replay.
    Forfeit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeatResult {
    pub fleet_id: String,
    pub territory: String,
    pub outcome: SeatOutcome,
    pub milestone_points: u64,
    pub durable_at_ms: Option<u64>,
    pub cost_microusd: u64,
    #[serde(default)]
    pub failure_source: Option<FailureSource>,
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeekStanding {
    pub fleet_id: String,
    pub model: String,
    /// Deepest round reached (byes count as reaching the next round).
    pub reached_round: usize,
    pub heats: usize,
    pub wins: usize,
    pub durable_deployments: usize,
    pub milestone_points: u64,
    pub forfeits: usize,
    pub cost_microusd: u64,
}

#[derive(Debug, Error)]
pub enum SeasonError {
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not parse season manifest: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("invalid season: {0}")]
    Invalid(String),
    #[error("arena manifest {path} failed: {source}")]
    Manifest {
        path: PathBuf,
        #[source]
        source: aoe_domain::ManifestError,
    },
    #[error("a draw already exists at {0}; use a new week directory")]
    DrawExists(PathBuf),
    #[error("existing week checkpoint does not match this draw: {0}")]
    ResumeMismatch(String),
    #[error("round {round} heat {heat} failed: {source}")]
    Heat {
        round: usize,
        heat: usize,
        #[source]
        source: RunError,
    },
    #[error("season port range exceeds 65535")]
    PortRange,
    #[error("season I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("could not encode season data: {0}")]
    Json(#[from] serde_json::Error),
}

/// Run every heat of a drawn week, checkpointing after each heat.
///
/// # Errors
///
/// Returns an error when the draw or an arena manifest is invalid, a heat
/// fails to run, an existing checkpoint belongs to another draw, or files
/// cannot be written. An aborted heat ends the week early but keeps the
/// checkpoint inspectable.
pub async fn run_week(options: WeekOptions) -> Result<WeekSummary, SeasonError> {
    let draw: WeekDraw = serde_json::from_slice(&fs::read(options.week_dir.join("draw.json"))?)?;
    if draw.schema_version != DRAW_SCHEMA_VERSION {
        return Err(SeasonError::Invalid(format!(
            "draw schema_version is {}, expected {DRAW_SCHEMA_VERSION}",
            draw.schema_version
        )));
    }
    let secret = fs::read_to_string(options.week_dir.join(SECRET_SEED_FILE))
        .map_err(|source| SeasonError::Read {
            path: options.week_dir.join(SECRET_SEED_FILE),
            source,
        })?
        .trim()
        .to_owned();
    if format!("{:x}", Sha256::digest(secret.as_bytes())) != draw.variation_seed_commitment {
        return Err(SeasonError::Invalid(
            "seed.secret does not match the draw's variation_seed_commitment".into(),
        ));
    }
    let fleet: BTreeMap<&str, &FleetEntry> = draw
        .fleet
        .iter()
        .map(|entry| (entry.id.as_str(), entry))
        .collect();

    let summary_path = options.week_dir.join("week.json");
    let mut summary = load_checkpoint(&summary_path, &draw)?;
    if summary.completed
        || summary
            .rounds
            .last()
            .is_some_and(|round| round.heats.iter().any(|heat| heat.aborted))
    {
        return Ok(summary);
    }

    // Replay the deterministic side of every round so resumed weeks derive
    // the same later-round composition as a fresh run.
    let mut rng = DrawRng::from_seed(&format!("{}/rounds", draw.draw_seed));
    let mut heat_counter = 0_usize;

    for shape in &draw.shape {
        let round_index = shape.round - 1;
        let arena = &draw.arenas[draw.round_arenas[round_index]];
        let manifest =
            ArenaManifest::load(&arena.manifest).map_err(|source| SeasonError::Manifest {
                path: arena.manifest.clone(),
                source,
            })?;

        // Participants for this round.
        let round_draw = if shape.round == 1 {
            draw.first_round.clone()
        } else {
            let previous = &summary.rounds[round_index - 1];
            let mut field: Vec<String> = previous
                .heats
                .iter()
                .filter_map(|heat| heat.winner.clone())
                .collect();
            field.extend(previous.byes.iter().cloned());
            field.extend(previous.wildcards.iter().cloned());
            rng.shuffle(&mut field);
            let heats = field.len() / draw.heat_size;
            let mut drawn = Vec::with_capacity(heats);
            for (index, chunk) in field.chunks(draw.heat_size).take(heats).enumerate() {
                drawn.push(HeatDraw {
                    heat: index + 1,
                    seats: seat_heat(&mut rng, &arena.territories, chunk),
                });
            }
            RoundDraw {
                round: shape.round,
                heats: drawn,
                byes: field[heats * draw.heat_size..].to_vec(),
            }
        };
        // Consume the RNG for round one too, so later rounds' positions are
        // independent of whether the run resumed.
        if shape.round == 1 {
            let _ = rng.next_u64();
        }

        let existing_round = summary.rounds.get(round_index).cloned();
        let mut round_result = existing_round.unwrap_or_else(|| RoundResult {
            round: shape.round,
            arena_id: arena.arena_id.clone(),
            heats: Vec::new(),
            byes: round_draw.byes.clone(),
            wildcards: Vec::new(),
        });
        if round_result.arena_id != arena.arena_id || round_result.byes != round_draw.byes {
            return Err(SeasonError::ResumeMismatch(format!(
                "round {} composition differs from the checkpoint",
                shape.round
            )));
        }

        for heat_draw in &round_draw.heats {
            if round_result
                .heats
                .iter()
                .any(|heat| heat.heat == heat_draw.heat)
            {
                heat_counter += 1;
                continue;
            }
            let heat_dir = options
                .week_dir
                .join(format!("round-{:02}", shape.round))
                .join(format!("heat-{:02}", heat_draw.heat));
            let mut attempts = 0;
            let result = loop {
                attempts += 1;
                let attempt_dir = if attempts == 1 {
                    heat_dir.clone()
                } else {
                    heat_dir.with_extension(format!("replay-{}", attempts - 1))
                };
                archive_incomplete(&attempt_dir)?;
                let mut heat_manifest = manifest.clone();
                seat_manifest(&mut heat_manifest, heat_draw, &fleet)?;
                let (base_port, multicast_port) = heat_ports(
                    options.base_port,
                    options.multicast_port,
                    draw.heat_size,
                    heat_counter,
                )?;
                heat_counter += 1;
                let state = run_match_with_manifest(
                    RunOptions {
                        manifest: arena.manifest.clone(),
                        output: attempt_dir.clone(),
                        adapters: options.adapters.clone(),
                        credentials: options.credentials.clone(),
                        base_port,
                        multicast_port,
                        color: options.color,
                        scenario_seed: Some(secret.clone()),
                    },
                    heat_manifest,
                )
                .await
                .map_err(|source| SeasonError::Heat {
                    round: shape.round,
                    heat: heat_draw.heat,
                    source,
                })?;
                let mut result = heat_result(heat_draw, attempt_dir, attempts, &state);
                let unavailable = result
                    .standings
                    .iter()
                    .any(|seat| seat.outcome == SeatOutcome::Forfeit);
                if result.aborted || !unavailable || attempts > draw.rules.unavailable_replays {
                    if unavailable && !result.aborted {
                        // Replays exhausted: the forfeit stands and the
                        // heat is decided among the evaluated seats.
                        result.winner = decide_winner(&result.standings);
                    }
                    break result;
                }
                // A replay keeps the same seats; the earlier attempt's
                // artifacts stay under their replay-N directory as evidence.
            };
            let aborted = result.aborted;
            round_result.heats.push(result);
            upsert_round(&mut summary.rounds, round_result.clone());
            summary.standings = week_standings(&draw, &summary.rounds);
            write_json_atomic(&summary_path, &summary)?;
            if aborted {
                return Ok(summary);
            }
        }

        // Wildcards: best non-winners by milestones, then earliest durable,
        // then lowest cost. Forfeited seats never advance.
        if shape.wildcards > 0 && round_result.wildcards.is_empty() {
            let mut candidates: Vec<&SeatResult> = round_result
                .heats
                .iter()
                .flat_map(|heat| {
                    heat.standings
                        .iter()
                        .filter(move |seat| Some(&seat.fleet_id) != heat.winner.as_ref())
                })
                .filter(|seat| seat.outcome != SeatOutcome::Forfeit)
                .collect();
            candidates.sort_by(|a, b| seat_rank(b, a));
            round_result.wildcards = candidates
                .into_iter()
                .take(shape.wildcards)
                .map(|seat| seat.fleet_id.clone())
                .collect();
            upsert_round(&mut summary.rounds, round_result.clone());
            write_json_atomic(&summary_path, &summary)?;
        }
    }

    summary.champion = summary
        .rounds
        .last()
        .and_then(|round| round.heats.first())
        .and_then(|heat| heat.winner.clone());
    summary.completed = true;
    summary.variation_seed = Some(secret);
    summary.standings = week_standings(&draw, &summary.rounds);
    write_json_atomic(&summary_path, &summary)?;
    Ok(summary)
}

fn upsert_round(rounds: &mut Vec<RoundResult>, round: RoundResult) {
    if let Some(existing) = rounds.iter_mut().find(|r| r.round == round.round) {
        *existing = round;
    } else {
        rounds.push(round);
    }
}

fn load_checkpoint(path: &Path, draw: &WeekDraw) -> Result<WeekSummary, SeasonError> {
    if !path.exists() {
        return Ok(WeekSummary {
            schema_version: WEEK_SCHEMA_VERSION,
            season_id: draw.season_id.clone(),
            week: draw.week.clone(),
            draw_seed: draw.draw_seed.clone(),
            variation_seed_commitment: draw.variation_seed_commitment.clone(),
            variation_seed: None,
            completed: false,
            rounds: Vec::new(),
            champion: None,
            standings: Vec::new(),
        });
    }
    let summary: WeekSummary = serde_json::from_slice(&fs::read(path)?)?;
    if summary.schema_version != WEEK_SCHEMA_VERSION {
        return Err(SeasonError::ResumeMismatch(format!(
            "schema version is {}, expected {WEEK_SCHEMA_VERSION}",
            summary.schema_version
        )));
    }
    if summary.draw_seed != draw.draw_seed
        || summary.variation_seed_commitment != draw.variation_seed_commitment
    {
        return Err(SeasonError::ResumeMismatch(
            "week.json was produced by a different draw".into(),
        ));
    }
    Ok(summary)
}

fn archive_incomplete(output: &Path) -> Result<(), SeasonError> {
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

/// Replace the template agents with the heat's fleet members, keeping each
/// territory's budget from the template that occupied it.
fn seat_manifest(
    manifest: &mut ArenaManifest,
    heat: &HeatDraw,
    fleet: &BTreeMap<&str, &FleetEntry>,
) -> Result<(), SeasonError> {
    let budgets: HashMap<String, _> = manifest
        .agents
        .iter()
        .map(|agent| (agent.territory.clone(), agent.budget.clone()))
        .collect();
    let fallback = manifest
        .agents
        .first()
        .map(|agent| agent.budget.clone())
        .ok_or_else(|| {
            SeasonError::Invalid(format!(
                "arena {} declares no template agents to take budgets from",
                manifest.arena.id
            ))
        })?;
    let mut agents = Vec::with_capacity(heat.seats.len());
    for (territory, fleet_id) in &heat.seats {
        let entry = fleet.get(fleet_id.as_str()).ok_or_else(|| {
            SeasonError::Invalid(format!("draw names unknown fleet member {fleet_id:?}"))
        })?;
        agents.push(AgentConfig {
            id: entry.id.clone(),
            territory: territory.clone(),
            adapter: entry.adapter.clone(),
            model: entry.model.clone(),
            reasoning_effort: entry.reasoning_effort.clone(),
            budget: budgets
                .get(territory)
                .cloned()
                .unwrap_or_else(|| fallback.clone()),
        });
    }
    manifest.agents = agents;
    Ok(())
}

fn heat_ports(
    base_port: u16,
    multicast_port: u16,
    heat_size: usize,
    heat_index: usize,
) -> Result<(u16, u16), SeasonError> {
    let stride = heat_size.checked_mul(2).ok_or(SeasonError::PortRange)?;
    let offset = stride
        .checked_mul(heat_index)
        .and_then(|value| u16::try_from(value).ok())
        .ok_or(SeasonError::PortRange)?;
    let multicast_offset = u16::try_from(heat_index).map_err(|_| SeasonError::PortRange)?;
    Ok((
        base_port
            .checked_add(offset)
            .ok_or(SeasonError::PortRange)?,
        multicast_port
            .checked_add(multicast_offset)
            .ok_or(SeasonError::PortRange)?,
    ))
}

/// Summarize one heat from its final world state.
#[must_use]
pub fn heat_result(
    heat: &HeatDraw,
    output: PathBuf,
    attempts: usize,
    state: &WorldState,
) -> HeatResult {
    let mut standings = Vec::with_capacity(heat.seats.len());
    for (territory, fleet_id) in &heat.seats {
        let view = state.territories.get(territory);
        let agent = state.agents.get(fleet_id);
        let failure_source = agent.and_then(|agent| agent.failure_source);
        let durable_at_ms = view.and_then(|view| view.durable_at_ms);
        let outcome = if durable_at_ms.is_some() {
            SeatOutcome::Durable
        } else if matches!(
            failure_source,
            Some(FailureSource::Provider | FailureSource::Harness)
        ) {
            SeatOutcome::Forfeit
        } else if agent.is_some_and(|agent| agent.successful == Some(false))
            && failure_source == Some(FailureSource::Player)
            && agent.is_some_and(|agent| {
                agent.terminal_state == Some(aoe_domain::AgentTerminalState::Incomplete)
            })
        {
            SeatOutcome::Incomplete
        } else {
            SeatOutcome::Failed
        };
        standings.push(SeatResult {
            fleet_id: fleet_id.clone(),
            territory: territory.clone(),
            outcome,
            milestone_points: view.map_or(0, |view| view.milestone_points),
            durable_at_ms,
            cost_microusd: agent.map_or(0, |agent| agent.cost_microusd),
            failure_source,
            detail: agent.and_then(|agent| agent.terminal_detail.clone()),
        });
    }
    standings.sort_by(|a, b| seat_rank(b, a));
    let winner = state
        .winner
        .as_ref()
        .and_then(|territory| heat.seats.get(territory).cloned())
        .or_else(|| decide_winner(&standings));
    HeatResult {
        heat: heat.heat,
        output,
        attempts,
        seats: heat.seats.clone(),
        winner,
        standings,
        aborted: state.match_state == MatchState::Aborted,
    }
}

/// Without a durable finisher, the heat goes to the best evaluated seat by
/// milestones, then earliest durable time, then lowest cost. All-forfeit
/// heats have no winner.
fn decide_winner(standings: &[SeatResult]) -> Option<String> {
    let mut evaluated: Vec<&SeatResult> = standings
        .iter()
        .filter(|seat| seat.outcome != SeatOutcome::Forfeit)
        .collect();
    if evaluated.is_empty() {
        return None;
    }
    evaluated.sort_by(|a, b| seat_rank(b, a));
    Some(evaluated[0].fleet_id.clone())
}

/// Ascending rank: greater is better. Milestones first, then an earlier
/// durable time, then lower cost, then id for determinism.
fn seat_rank(a: &SeatResult, b: &SeatResult) -> std::cmp::Ordering {
    a.milestone_points
        .cmp(&b.milestone_points)
        .then_with(|| match (a.durable_at_ms, b.durable_at_ms) {
            (Some(x), Some(y)) => y.cmp(&x),
            (Some(_), None) => std::cmp::Ordering::Greater,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (None, None) => std::cmp::Ordering::Equal,
        })
        .then_with(|| b.cost_microusd.cmp(&a.cost_microusd))
        .then_with(|| b.fleet_id.cmp(&a.fleet_id))
}

fn week_standings(draw: &WeekDraw, rounds: &[RoundResult]) -> Vec<WeekStanding> {
    let mut table: BTreeMap<String, WeekStanding> = draw
        .fleet
        .iter()
        .map(|entry| {
            (
                entry.id.clone(),
                WeekStanding {
                    fleet_id: entry.id.clone(),
                    model: entry.model.clone(),
                    reached_round: 1,
                    heats: 0,
                    wins: 0,
                    durable_deployments: 0,
                    milestone_points: 0,
                    forfeits: 0,
                    cost_microusd: 0,
                },
            )
        })
        .collect();
    for round in rounds {
        for heat in &round.heats {
            for seat in &heat.standings {
                let Some(row) = table.get_mut(&seat.fleet_id) else {
                    continue;
                };
                row.heats += 1;
                row.reached_round = row.reached_round.max(round.round);
                row.milestone_points += seat.milestone_points;
                row.cost_microusd += seat.cost_microusd;
                if seat.outcome == SeatOutcome::Durable {
                    row.durable_deployments += 1;
                }
                if seat.outcome == SeatOutcome::Forfeit {
                    row.forfeits += 1;
                }
                if heat.winner.as_ref() == Some(&seat.fleet_id) {
                    row.wins += 1;
                    row.reached_round = row.reached_round.max(round.round + 1);
                }
            }
        }
        for id in round.byes.iter().chain(round.wildcards.iter()) {
            if let Some(row) = table.get_mut(id) {
                row.reached_round = row.reached_round.max(round.round + 1);
            }
        }
    }
    let last_round = draw.shape.len();
    let mut standings: Vec<WeekStanding> = table.into_values().collect();
    standings.sort_by(|a, b| {
        b.reached_round
            .min(last_round + 1)
            .cmp(&a.reached_round.min(last_round + 1))
            .then_with(|| b.wins.cmp(&a.wins))
            .then_with(|| b.durable_deployments.cmp(&a.durable_deployments))
            .then_with(|| b.milestone_points.cmp(&a.milestone_points))
            .then_with(|| a.cost_microusd.cmp(&b.cost_microusd))
            .then_with(|| a.fleet_id.cmp(&b.fleet_id))
    });
    standings
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Terminal rendering of a draw: the bracket shape and round-one heats.
#[must_use]
pub fn render_draw(draw: &WeekDraw) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Season {} week {}", draw.season_id, draw.week);
    let _ = writeln!(out, "draw seed: {}", draw.draw_seed);
    let _ = writeln!(
        out,
        "variation seed commitment: {}",
        draw.variation_seed_commitment
    );
    for shape in &draw.shape {
        let arena = &draw.arenas[draw.round_arenas[shape.round - 1]];
        let _ = writeln!(
            out,
            "round {}: {} in {} heat{} on {} ({} bye{}, {} wildcard{})",
            shape.round,
            shape.field,
            shape.heats,
            plural(shape.heats),
            arena.arena_id,
            shape.byes,
            plural(shape.byes),
            shape.wildcards,
            plural(shape.wildcards)
        );
    }
    for heat in &draw.first_round.heats {
        let seats: Vec<String> = heat
            .seats
            .iter()
            .map(|(territory, id)| format!("{territory}={id}"))
            .collect();
        let _ = writeln!(out, "  heat {}: {}", heat.heat, seats.join("  "));
    }
    if !draw.first_round.byes.is_empty() {
        let _ = writeln!(out, "  byes: {}", draw.first_round.byes.join(", "));
    }
    out
}

/// Terminal rendering of a week's results.
#[must_use]
pub fn render_week(summary: &WeekSummary) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Season {} week {} ({})",
        summary.season_id,
        summary.week,
        if summary.completed {
            "complete"
        } else {
            "in progress"
        }
    );
    for round in &summary.rounds {
        let _ = writeln!(out, "round {} on {}", round.round, round.arena_id);
        for heat in &round.heats {
            let _ = writeln!(
                out,
                "  heat {} ({} attempt{}): winner {}",
                heat.heat,
                heat.attempts,
                plural(heat.attempts),
                heat.winner.as_deref().unwrap_or("none")
            );
            for seat in &heat.standings {
                let _ = writeln!(
                    out,
                    "    {:<24} {:<10} {:>4} pts  {}  ${:.3}",
                    seat.fleet_id,
                    format!("{:?}", seat.outcome).to_lowercase(),
                    seat.milestone_points,
                    seat.durable_at_ms.map_or_else(
                        || "      -".to_owned(),
                        |ms| format!("{:>5.1}s", ms as f64 / 1000.0)
                    ),
                    seat.cost_microusd as f64 / 1_000_000.0
                );
            }
        }
        if !round.byes.is_empty() {
            let _ = writeln!(out, "  byes: {}", round.byes.join(", "));
        }
        if !round.wildcards.is_empty() {
            let _ = writeln!(out, "  wildcards: {}", round.wildcards.join(", "));
        }
    }
    if let Some(champion) = &summary.champion {
        let _ = writeln!(out, "champion: {champion}");
    }
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "{:<24} {:>5} {:>5} {:>4} {:>7} {:>4} {:>8} {:>9}",
        "fleet", "round", "heats", "wins", "durable", "pts", "forfeits", "cost"
    );
    for row in &summary.standings {
        let _ = writeln!(
            out,
            "{:<24} {:>5} {:>5} {:>4} {:>7} {:>4} {:>8} {:>9.3}",
            row.fleet_id,
            row.reached_round,
            row.heats,
            row.wins,
            row.durable_deployments,
            row.milestone_points,
            row.forfeits,
            row.cost_microusd as f64 / 1_000_000.0
        );
    }
    if let Some(seed) = &summary.variation_seed {
        let _ = writeln!(out, "variation seed revealed: {seed}");
    }
    out
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aoe_domain::AgentTerminalState;
    use aoe_replay::{AgentView, TerritoryView};

    #[test]
    fn bracket_shapes_reduce_any_fleet_to_one_final() {
        for fleet in 3..=12 {
            let shape = bracket_shape(fleet, 3).expect("shape");
            let last = shape.last().expect("at least one round");
            assert_eq!(last.heats, 1, "fleet {fleet}: {shape:?}");
            assert_eq!(last.byes, 0, "fleet {fleet}: {shape:?}");
            assert_eq!(last.wildcards, 0, "fleet {fleet}: {shape:?}");
            for window in shape.windows(2) {
                let (a, b) = (window[0], window[1]);
                assert_eq!(
                    b.field,
                    a.heats + a.byes + a.wildcards,
                    "fleet {fleet}: {shape:?}"
                );
                assert_eq!(b.field % 3, 0, "fleet {fleet}: {shape:?}");
            }
        }
        assert!(bracket_shape(2, 3).is_err());
    }

    #[test]
    fn six_and_nine_model_seasons_have_the_expected_shape() {
        let six = bracket_shape(6, 3).expect("shape");
        assert_eq!(six.len(), 2);
        assert_eq!((six[0].heats, six[0].byes, six[0].wildcards), (2, 0, 1));
        let nine = bracket_shape(9, 3).expect("shape");
        assert_eq!(nine.len(), 2);
        assert_eq!((nine[0].heats, nine[0].byes, nine[0].wildcards), (3, 0, 0));
        let seven = bracket_shape(7, 3).expect("shape");
        assert_eq!(
            (seven[0].heats, seven[0].byes, seven[0].wildcards),
            (2, 1, 0)
        );
        assert_eq!(seven.len(), 2);
    }

    #[test]
    fn draw_rng_is_deterministic_and_unbiased_enough_to_shuffle() {
        let mut a = DrawRng::from_seed("season/2026-W37");
        let mut b = DrawRng::from_seed("season/2026-W37");
        let mut left: Vec<u32> = (0..10).collect();
        let mut right = left.clone();
        a.shuffle(&mut left);
        b.shuffle(&mut right);
        assert_eq!(left, right);
        let mut sorted = left.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..10).collect::<Vec<_>>());
        let mut c = DrawRng::from_seed("season/2026-W38");
        let mut other: Vec<u32> = (0..10).collect();
        c.shuffle(&mut other);
        assert_ne!(other, left, "a different week draws differently");
    }

    fn seat(
        id: &str,
        outcome: SeatOutcome,
        points: u64,
        durable: Option<u64>,
        cost: u64,
    ) -> SeatResult {
        SeatResult {
            fleet_id: id.into(),
            territory: format!("t-{id}"),
            outcome,
            milestone_points: points,
            durable_at_ms: durable,
            cost_microusd: cost,
            failure_source: None,
            detail: None,
        }
    }

    #[test]
    fn heats_without_a_finisher_go_to_milestones_then_time_then_cost() {
        let standings = vec![
            seat("a", SeatOutcome::Incomplete, 30, None, 500),
            seat("b", SeatOutcome::Incomplete, 30, None, 400),
            seat("c", SeatOutcome::Forfeit, 90, None, 1),
        ];
        assert_eq!(decide_winner(&standings).as_deref(), Some("b"));
        let all_forfeit = vec![seat("a", SeatOutcome::Forfeit, 0, None, 0)];
        assert_eq!(decide_winner(&all_forfeit), None);
        let durable_first = vec![
            seat("slow", SeatOutcome::Durable, 100, Some(90_000), 1),
            seat("fast", SeatOutcome::Durable, 100, Some(60_000), 9),
        ];
        assert_eq!(decide_winner(&durable_first).as_deref(), Some("fast"));
    }

    fn world_with(
        seats: &[(
            &str,
            &str,
            u64,
            Option<u64>,
            Option<FailureSource>,
            Option<AgentTerminalState>,
        )],
    ) -> WorldState {
        let mut state = WorldState::default();
        for (territory, fleet_id, points, durable, source, terminal) in seats {
            state.territories.insert(
                (*territory).to_owned(),
                TerritoryView {
                    agent: Some((*fleet_id).to_owned()),
                    milestone_points: *points,
                    durable_at_ms: *durable,
                    ..TerritoryView::default()
                },
            );
            state.agents.insert(
                (*fleet_id).to_owned(),
                AgentView {
                    territory: (*territory).to_owned(),
                    model: "m".into(),
                    successful: terminal.map(|t| t == AgentTerminalState::Completed),
                    failure_source: *source,
                    terminal_state: *terminal,
                    cost_microusd: 10,
                    ..AgentView::default()
                },
            );
        }
        state
    }

    fn draw_for(seats: &[(&str, &str)]) -> HeatDraw {
        HeatDraw {
            heat: 1,
            seats: seats
                .iter()
                .map(|(t, f)| ((*t).to_owned(), (*f).to_owned()))
                .collect(),
        }
    }

    #[test]
    fn heat_result_classifies_seats_and_uses_the_referee_winner() {
        let mut state = world_with(&[
            (
                "one",
                "alpha",
                100,
                Some(70_000),
                None,
                Some(AgentTerminalState::Completed),
            ),
            (
                "two",
                "beta",
                30,
                None,
                Some(FailureSource::Player),
                Some(AgentTerminalState::Incomplete),
            ),
            (
                "three",
                "gamma",
                10,
                None,
                Some(FailureSource::Provider),
                Some(AgentTerminalState::Failed),
            ),
        ]);
        state.winner = Some("one".into());
        let heat = draw_for(&[("one", "alpha"), ("two", "beta"), ("three", "gamma")]);
        let result = heat_result(&heat, PathBuf::from("x"), 1, &state);
        assert_eq!(result.winner.as_deref(), Some("alpha"));
        let outcomes: BTreeMap<_, _> = result
            .standings
            .iter()
            .map(|seat| (seat.fleet_id.clone(), seat.outcome))
            .collect();
        assert_eq!(outcomes["alpha"], SeatOutcome::Durable);
        assert_eq!(outcomes["beta"], SeatOutcome::Incomplete);
        assert_eq!(outcomes["gamma"], SeatOutcome::Forfeit);
        assert_eq!(result.standings[0].fleet_id, "alpha", "ranked best first");
    }

    #[test]
    fn heat_result_without_a_referee_winner_ranks_evaluated_seats() {
        let state = world_with(&[
            (
                "one",
                "alpha",
                30,
                None,
                Some(FailureSource::Player),
                Some(AgentTerminalState::Incomplete),
            ),
            (
                "two",
                "beta",
                60,
                None,
                Some(FailureSource::Player),
                Some(AgentTerminalState::Incomplete),
            ),
            (
                "three",
                "gamma",
                90,
                None,
                Some(FailureSource::Harness),
                Some(AgentTerminalState::Failed),
            ),
        ]);
        let heat = draw_for(&[("one", "alpha"), ("two", "beta"), ("three", "gamma")]);
        let result = heat_result(&heat, PathBuf::from("x"), 2, &state);
        assert_eq!(result.winner.as_deref(), Some("beta"), "forfeits never win");
        assert_eq!(result.attempts, 2);
    }

    fn fleet(n: usize) -> Vec<FleetEntry> {
        (0..n)
            .map(|i| FleetEntry {
                id: format!("m{i}"),
                model: format!("vendor/model-{i}"),
                adapter: "claux".into(),
                reasoning_effort: "high".into(),
            })
            .collect()
    }

    fn draw_with(n: usize) -> WeekDraw {
        WeekDraw {
            schema_version: DRAW_SCHEMA_VERSION,
            season_id: "s".into(),
            season_manifest: PathBuf::from("s.toml"),
            week: "2026-W37".into(),
            draw_seed: "s/2026-W37".into(),
            variation_seed_commitment: String::new(),
            heat_size: 3,
            rules: SeasonRules::default(),
            fleet: fleet(n),
            arenas: vec![],
            shape: bracket_shape(n, 3).expect("shape"),
            round_arenas: vec![],
            first_round: RoundDraw {
                round: 1,
                heats: vec![],
                byes: vec![],
            },
        }
    }

    #[test]
    fn week_standings_rank_by_round_reached_then_wins() {
        let draw = draw_with(6);
        let rounds = vec![
            RoundResult {
                round: 1,
                arena_id: "a".into(),
                heats: vec![
                    HeatResult {
                        heat: 1,
                        output: PathBuf::from("r1h1"),
                        attempts: 1,
                        seats: BTreeMap::new(),
                        winner: Some("m0".into()),
                        standings: vec![
                            seat("m0", SeatOutcome::Durable, 100, Some(1000), 5),
                            seat("m1", SeatOutcome::Incomplete, 60, None, 5),
                            seat("m2", SeatOutcome::Forfeit, 0, None, 0),
                        ],
                        aborted: false,
                    },
                    HeatResult {
                        heat: 2,
                        output: PathBuf::from("r1h2"),
                        attempts: 2,
                        seats: BTreeMap::new(),
                        winner: Some("m3".into()),
                        standings: vec![
                            seat("m3", SeatOutcome::Durable, 100, Some(2000), 5),
                            seat("m4", SeatOutcome::Incomplete, 30, None, 5),
                            seat("m5", SeatOutcome::Incomplete, 10, None, 5),
                        ],
                        aborted: false,
                    },
                ],
                byes: vec![],
                wildcards: vec!["m1".into()],
            },
            RoundResult {
                round: 2,
                arena_id: "b".into(),
                heats: vec![HeatResult {
                    heat: 1,
                    output: PathBuf::from("r2h1"),
                    attempts: 1,
                    seats: BTreeMap::new(),
                    winner: Some("m3".into()),
                    standings: vec![
                        seat("m3", SeatOutcome::Durable, 100, Some(1500), 5),
                        seat("m0", SeatOutcome::Incomplete, 60, None, 5),
                        seat("m1", SeatOutcome::Incomplete, 60, None, 5),
                    ],
                    aborted: false,
                }],
                byes: vec![],
                wildcards: vec![],
            },
        ];
        let standings = week_standings(&draw, &rounds);
        let order: Vec<&str> = standings.iter().map(|row| row.fleet_id.as_str()).collect();
        assert_eq!(order[0], "m3", "champion first");
        assert_eq!(order[1], "m0", "finalist with a win next");
        assert_eq!(order[2], "m1", "wildcard finalist");
        let m2 = standings
            .iter()
            .find(|row| row.fleet_id == "m2")
            .expect("m2");
        assert_eq!(m2.forfeits, 1);
        assert_eq!(m2.reached_round, 1);
        let m3 = standings
            .iter()
            .find(|row| row.fleet_id == "m3")
            .expect("m3");
        assert_eq!(
            (m3.wins, m3.durable_deployments, m3.reached_round),
            (2, 2, 3)
        );
    }
}
