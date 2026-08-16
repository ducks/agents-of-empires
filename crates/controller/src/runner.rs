use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use aoe_agent::{
    AgentController, AgentInvocation, AgentStatus, AgentUsage, AgentUsageCheckpoint, CommandAdapter,
};
use aoe_domain::{
    ArenaManifest, Event, EventEnvelope, FailureSource, MatchMode, MilestoneConfig,
    MilestoneOperation,
};
use aoe_referee::{BuildReferee, HealthProbe, HttpProbe, ProbeTarget, Referee};
use aoe_replay::{EventLog, WorldState, reduce};
use aoe_runtime::{ArenaSupervisor, NetworkPlan, NixVmDriver};
use aoe_tui::{RenderOptions, render_world};
use thiserror::Error;
use tokio::sync::mpsc;

use crate::provenance::write_provenance;

const POST_MATCH_DRAIN: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub manifest: PathBuf,
    pub output: PathBuf,
    pub adapters: HashMap<String, PathBuf>,
    pub credentials: HashMap<String, PathBuf>,
    pub base_port: u16,
    pub multicast_port: u16,
    pub color: bool,
}

#[derive(Debug, Error)]
pub enum RunError {
    #[error("manifest failed: {0}")]
    Manifest(#[from] aoe_domain::ManifestError),
    #[error("network plan failed: {0}")]
    Network(#[from] aoe_runtime::NetworkPlanError),
    #[error("arena runtime failed: {0}")]
    Runtime(#[from] aoe_runtime::RuntimeError),
    #[error("event log failed: {0}")]
    Log(#[from] aoe_replay::EventLogError),
    #[error("referee failed: {0}")]
    Referee(#[from] aoe_referee::RefereeError),
    #[error("could not create health probe: {0}")]
    Probe(#[from] reqwest::Error),
    #[error("I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("agent {agent} uses unregistered adapter {adapter}")]
    UnknownAdapter { agent: String, adapter: String },
    #[error("instruction file missing for {territory}: {path}")]
    MissingInstruction { territory: String, path: String },
    #[error("territories did not pass preflight before deadline: {0}")]
    Preflight(String),
    #[error("agent task failed: {0}")]
    AgentTask(String),
    #[error("could not encode final state: {0}")]
    Json(#[from] serde_json::Error),
    #[error("output already contains an event log: {0}")]
    OutputExists(String),
    #[error("could not record match provenance: {0}")]
    Provenance(String),
    #[error("fog-of-war guest leak in {territory}: {forbidden:?} found at {path}")]
    GuestLeak {
        territory: String,
        forbidden: String,
        path: String,
    },
}

/// Run a complete match, using the same event reducer as replay mode.
///
/// # Errors
///
/// Returns an error for invalid configuration, guest lifecycle failures,
/// failed preflight, event persistence errors, or agent controller failures.
pub async fn run_match(options: RunOptions) -> Result<WorldState, RunError> {
    let manifest = ArenaManifest::load(&options.manifest)?;
    run_match_with_manifest(options, manifest).await
}

pub(crate) async fn run_match_with_manifest(
    options: RunOptions,
    mut manifest: ArenaManifest,
) -> Result<WorldState, RunError> {
    let event_path = options.output.join("events.jsonl");
    if event_path
        .metadata()
        .is_ok_and(|metadata| metadata.len() > 0)
    {
        return Err(RunError::OutputExists(event_path.display().to_string()));
    }
    std::fs::create_dir_all(&options.output)?;
    std::fs::write(
        options.output.join("arena.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    write_provenance(
        &options.manifest,
        &manifest,
        &options.adapters,
        &options.output,
    )
    .map_err(|error| RunError::Provenance(error.to_string()))?;
    resolve_nixos_configs(&options.manifest, &mut manifest);
    validate_adapters(&manifest, &options.adapters)?;
    let plan = NetworkPlan::from_manifest(&manifest, options.base_port, options.multicast_port)?;
    let driver = Arc::new(NixVmDriver::new(options.output.join("territories")));
    let mut supervisor = ArenaSupervisor::new(driver);
    supervisor.boot_all(&manifest, &plan).await?;

    let result = match manifest.arena.mode {
        MatchMode::Conquest => run_booted_match(&manifest, &plan, &options, &mut supervisor).await,
        MatchMode::BuildRace => {
            run_booted_build_match(&manifest, &plan, &options, &mut supervisor).await
        }
    };
    let cleanup_errors = supervisor.stop_all().await;
    if let Err(error) = result {
        return Err(error);
    }
    if !cleanup_errors.is_empty() {
        return Err(RunError::Runtime(aoe_runtime::RuntimeError::Stop {
            territory: "arena".into(),
            detail: cleanup_errors.join("; "),
        }));
    }
    result
}

fn resolve_nixos_configs(manifest_path: &Path, manifest: &mut ArenaManifest) {
    let arena_root = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    for territory in &mut manifest.territories {
        let (flake, attribute) = territory.nixos_config.split_once('#').map_or(
            (territory.nixos_config.as_str(), None),
            |(flake, attribute)| (flake, Some(attribute)),
        );
        let flake_path = Path::new(flake);
        if flake_path.is_absolute() {
            continue;
        }
        let package_relative = arena_root.join(flake_path);
        let legacy_cwd_relative = cwd.join(flake_path);
        let resolved = if package_relative.exists() {
            package_relative
        } else if legacy_cwd_relative.exists() {
            legacy_cwd_relative
        } else {
            continue;
        };
        let resolved = resolved.canonicalize().unwrap_or(resolved);
        let mut reference = format!("path:{}", resolved.to_string_lossy());
        if let Some(attribute) = attribute {
            reference.push('#');
            reference.push_str(attribute);
        }
        territory.nixos_config = reference;
    }
}

async fn run_booted_match(
    manifest: &ArenaManifest,
    plan: &NetworkPlan,
    options: &RunOptions,
    supervisor: &mut ArenaSupervisor<NixVmDriver>,
) -> Result<WorldState, RunError> {
    if manifest.fog_of_war.is_some() {
        wait_for_ssh(plan, &options.credentials, Duration::from_secs(120)).await?;
        audit_fog_of_war(manifest, plan, &options.credentials).await?;
    }
    let probe = HttpProbe::new(Duration::from_secs(3))?;
    let targets = probe_targets(manifest, plan);
    wait_for_preflight(&probe, &targets, Duration::from_secs(120)).await?;

    let mut log = EventLog::open(options.output.join("events.jsonl"))?;
    let mut world = WorldState::default();
    let mut events = Vec::new();
    let mut referee = Referee::from_manifest(manifest);
    append(&mut log, &mut world, &mut events, referee.start()?)?;

    register_territories(manifest, &mut referee, &mut log, &mut world, &mut events)?;

    let invocations = invocations(manifest, plan, options)?;
    let mut agents = AgentController::new();
    for (name, executable) in &options.adapters {
        agents.register(
            name.clone(),
            Arc::new(CommandAdapter::new(
                executable,
                options.output.join("agents"),
            )),
        );
    }
    for invocation in &invocations {
        let event = referee.record(
            Event::AgentStarted {
                agent: invocation.config.id.clone(),
                territory: invocation.config.territory.clone(),
                model: invocation.config.model.clone(),
            },
            0,
        )?;
        append(&mut log, &mut world, &mut events, [event])?;
    }

    let agent_timeout = Duration::from_secs(manifest.rules.duration_seconds);
    let mut agent_task =
        tokio::spawn(async move { agents.run_all(invocations, agent_timeout).await });
    let mut interrupt = Box::pin(tokio::signal::ctrl_c());
    let started = Instant::now();
    let tick = Duration::from_millis(manifest.rules.tick_ms);
    let mut agents_recorded = false;

    while referee.outcome().is_none() {
        tokio::select! {
            signal = &mut interrupt => {
                signal?;
                let elapsed = elapsed_ms(started);
                append(&mut log, &mut world, &mut events, referee.abort("operator interrupt", elapsed)?)?;
                break;
            }
            () = tokio::time::sleep(tick) => {}
        }
        let elapsed = elapsed_ms(started);
        for (territory, target) in &targets {
            if referee.outcome().is_some() {
                break;
            }
            let observed = probe.observe(target).await;
            let emitted = referee.observe(territory, observed, elapsed)?;
            let eliminated = emitted.iter().find_map(|event| match &event.event {
                Event::TerritoryEliminated { territory, .. } => Some(territory.clone()),
                _ => None,
            });
            append(&mut log, &mut world, &mut events, emitted)?;
            if let Some(territory) = eliminated {
                supervisor.stop(&territory).await?;
            }
        }
        if referee.outcome().is_none() {
            append(&mut log, &mut world, &mut events, referee.tick(elapsed)?)?;
        }
        if !agents_recorded && agent_task.is_finished() {
            let results = (&mut agent_task)
                .await
                .map_err(|error| RunError::AgentTask(error.to_string()))?;
            record_agent_results(
                &mut referee,
                &mut log,
                &mut world,
                &mut events,
                results,
                elapsed,
            )?;
            agents_recorded = true;
        }
        render_live(&world, &events, options.color);
    }

    if !agent_task.is_finished() {
        agent_task.abort();
    }
    std::fs::write(
        options.output.join("world.json"),
        serde_json::to_vec_pretty(&world)?,
    )?;
    Ok(world)
}

#[allow(clippy::too_many_lines)]
async fn run_booted_build_match(
    manifest: &ArenaManifest,
    plan: &NetworkPlan,
    options: &RunOptions,
    _supervisor: &mut ArenaSupervisor<NixVmDriver>,
) -> Result<WorldState, RunError> {
    wait_for_ssh(plan, &options.credentials, Duration::from_secs(120)).await?;
    audit_fog_of_war(manifest, plan, &options.credentials).await?;

    let mut log = EventLog::open(options.output.join("events.jsonl"))?;
    let mut world = WorldState::default();
    let mut events = Vec::new();
    let mut referee = BuildReferee::from_manifest(manifest);
    append(&mut log, &mut world, &mut events, referee.start()?)?;
    register_build_competitors(manifest, &mut referee, &mut log, &mut world, &mut events)?;

    let invocations = invocations(manifest, plan, options)?;
    let mut agents = AgentController::new();
    for (name, executable) in &options.adapters {
        agents.register(
            name.clone(),
            Arc::new(CommandAdapter::new(
                executable,
                options.output.join("agents"),
            )),
        );
    }
    for invocation in &invocations {
        let event = referee.record(
            Event::AgentStarted {
                agent: invocation.config.id.clone(),
                territory: invocation.config.territory.clone(),
                model: invocation.config.model.clone(),
            },
            0,
        )?;
        append(&mut log, &mut world, &mut events, [event])?;
    }

    let agent_timeout = Duration::from_secs(manifest.rules.duration_seconds);
    let (agent_sender, mut agent_results) = mpsc::channel(manifest.agents.len().max(1));
    let mut agent_task = tokio::spawn(async move {
        agents
            .run_stream(invocations, agent_timeout, agent_sender)
            .await;
    });
    let mut interrupt = Box::pin(tokio::signal::ctrl_c());
    let started = Instant::now();
    let deadline = Duration::from_secs(manifest.rules.duration_seconds);
    let tick = Duration::from_millis(manifest.rules.tick_ms);
    let build = manifest.build.as_ref().expect("validated build contract");
    let mut usage_seen = HashMap::new();

    while referee.outcome().is_none() && started.elapsed() < deadline {
        tokio::select! {
            signal = &mut interrupt => {
                signal?;
                append(
                    &mut log,
                    &mut world,
                    &mut events,
                    referee.abort("operator interrupt", elapsed_ms(started))?,
                )?;
                break;
            }
            () = tokio::time::sleep(tick) => {}
        }
        for milestone in &build.milestones {
            if referee.outcome().is_some() {
                break;
            }
            let eligible: Vec<_> = manifest
                .territories
                .iter()
                .filter(|territory| milestone_eligible(&world, &territory.id, milestone))
                .cloned()
                .collect();
            if eligible.is_empty() {
                continue;
            }
            for territory in &eligible {
                append(
                    &mut log,
                    &mut world,
                    &mut events,
                    referee.begin_milestone(&territory.id, &milestone.id, elapsed_ms(started))?,
                )?;
            }
            if milestone.operation == MilestoneOperation::HostReboot {
                // Reboot the guest OS, not the QEMU process. Besides being a
                // truer durability test, this keeps every competitor's host
                // forwards stable and lets all reboots begin before any one
                // territory is polled for recovery.
                for territory in &eligible {
                    let agent = manifest
                        .agents
                        .iter()
                        .find(|agent| agent.territory == territory.id)
                        .expect("validated assignment");
                    let marker = options
                        .output
                        .join("agents")
                        .join(&agent.id)
                        .join("referee-reboot");
                    tokio::fs::create_dir_all(marker.parent().expect("agent directory")).await?;
                    tokio::fs::write(&marker, milestone.id.as_bytes()).await?;
                    let assignment = plan
                        .assignments
                        .iter()
                        .find(|assignment| assignment.territory == territory.id)
                        .expect("network assignment");
                    let credential = options
                        .credentials
                        .get(&territory.id)
                        .expect("validated credential");
                    let password = credential_value(credential, "AOE_SSH_PASSWORD")?;
                    let _ = password_ssh(assignment.ssh_port, &password, "systemctl reboot").await;
                }
                futures::future::try_join_all(eligible.iter().map(|territory| {
                    let assignment = plan
                        .assignments
                        .iter()
                        .find(|assignment| assignment.territory == territory.id)
                        .expect("network assignment");
                    let credential = options
                        .credentials
                        .get(&territory.id)
                        .expect("validated credential");
                    wait_for_one_ssh_cycle(
                        assignment,
                        credential,
                        Duration::from_secs(30),
                        Duration::from_secs(120),
                    )
                }))
                .await?;
            }
            let checks = futures::future::join_all(eligible.iter().map(|territory| {
                run_milestone_verifier(options, plan, territory, milestone, &world)
            }))
            .await;
            for (territory, result) in eligible.iter().zip(checks) {
                if referee.outcome().is_some() {
                    break;
                }
                let elapsed = elapsed_ms(started);
                match result {
                    Ok(evidence) => {
                        append(
                            &mut log,
                            &mut world,
                            &mut events,
                            referee.pass_milestone(
                                &territory.id,
                                &milestone.id,
                                milestone.points,
                                evidence,
                                elapsed,
                            )?,
                        )?;
                    }
                    Err(detail) => append(
                        &mut log,
                        &mut world,
                        &mut events,
                        referee.fail_milestone(
                            &territory.id,
                            &milestone.id,
                            "verification_failed",
                            &detail,
                            true,
                            elapsed,
                        )?,
                    )?,
                }
            }
        }
        record_build_usage_checkpoints(
            &options.output,
            &mut usage_seen,
            &mut referee,
            &mut log,
            &mut world,
            &mut events,
            elapsed_ms(started),
        )?;
        render_live(&world, &events, options.color);
    }

    if referee.outcome().is_none() {
        append(
            &mut log,
            &mut world,
            &mut events,
            referee.finish("match timer expired", elapsed_ms(started))?,
        )?;
    }
    let frozen_elapsed_ms = world.elapsed_ms;
    drain_build_agents(
        &mut agent_task,
        &mut agent_results,
        &mut referee,
        &mut log,
        &mut world,
        &mut events,
        &mut usage_seen,
        &options.output,
        frozen_elapsed_ms,
        POST_MATCH_DRAIN,
    )
    .await?;
    std::fs::write(
        options.output.join("world.json"),
        serde_json::to_vec_pretty(&world)?,
    )?;
    Ok(world)
}

#[allow(clippy::too_many_arguments)]
async fn drain_build_agents(
    agent_task: &mut tokio::task::JoinHandle<()>,
    agent_results: &mut mpsc::Receiver<aoe_agent::AgentResult>,
    referee: &mut BuildReferee,
    log: &mut EventLog,
    world: &mut WorldState,
    events: &mut Vec<EventEnvelope>,
    usage_seen: &mut HashMap<String, AgentUsage>,
    usage_root: &Path,
    frozen_elapsed_ms: u64,
    drain: Duration,
) -> Result<(), RunError> {
    let pending_agents = world.agents.values().filter(|agent| agent.running).count() as u64;
    let started = referee.record(
        Event::PostMatchDrainStarted {
            timeout_ms: u64::try_from(drain.as_millis()).unwrap_or(u64::MAX),
            pending_agents,
        },
        frozen_elapsed_ms,
    )?;
    append(log, world, events, [started])?;
    let mut captured_agents = 0_u64;
    let deadline = Instant::now() + drain;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, agent_results.recv()).await {
            Ok(Some(result)) => {
                captured_agents = captured_agents.saturating_add(1);
                record_build_agent_results(
                    referee,
                    log,
                    world,
                    events,
                    vec![result],
                    usage_seen,
                    frozen_elapsed_ms,
                )?;
            }
            Ok(None) | Err(_) => break,
        }
    }
    if !agent_task.is_finished() {
        agent_task.abort();
    }
    let _ = agent_task.await;
    while let Ok(result) = agent_results.try_recv() {
        captured_agents = captured_agents.saturating_add(1);
        record_build_agent_results(
            referee,
            log,
            world,
            events,
            vec![result],
            usage_seen,
            frozen_elapsed_ms,
        )?;
    }
    record_build_usage_checkpoints(
        usage_root,
        usage_seen,
        referee,
        log,
        world,
        events,
        frozen_elapsed_ms,
    )?;
    let terminated: Vec<_> = world
        .agents
        .iter()
        .filter(|(_, agent)| agent.running)
        .map(|(agent, _)| agent.clone())
        .collect();
    for agent in &terminated {
        let event = referee.record(
            Event::AgentTerminated {
                agent: agent.clone(),
                reason: "post-match drain deadline expired".into(),
            },
            frozen_elapsed_ms,
        )?;
        append(log, world, events, [event])?;
    }
    let finished = referee.record(
        Event::PostMatchDrainFinished {
            captured_agents,
            terminated_agents: terminated.len() as u64,
        },
        frozen_elapsed_ms,
    )?;
    append(log, world, events, [finished])?;
    Ok(())
}

fn register_build_competitors(
    manifest: &ArenaManifest,
    referee: &mut BuildReferee,
    log: &mut EventLog,
    world: &mut WorldState,
    events: &mut Vec<EventEnvelope>,
) -> Result<(), RunError> {
    for territory in &manifest.territories {
        let agent = manifest
            .agents
            .iter()
            .find(|agent| agent.territory == territory.id)
            .expect("validated assignment");
        let event = referee.record(
            Event::TerritoryRegistered {
                territory: territory.id.clone(),
                class: territory.class.clone(),
                agent: agent.id.clone(),
            },
            0,
        )?;
        append(log, world, events, [event])?;
    }
    Ok(())
}

async fn wait_for_ssh(
    plan: &NetworkPlan,
    credentials: &HashMap<String, PathBuf>,
    timeout: Duration,
) -> Result<(), RunError> {
    let deadline = Instant::now() + timeout;
    let mut failures = Vec::new();
    while Instant::now() < deadline {
        failures.clear();
        for assignment in &plan.assignments {
            let Some(credential) = credentials.get(&assignment.territory) else {
                failures.push(format!("{}: missing credential", assignment.territory));
                continue;
            };
            let password = credential_value(credential, "AOE_SSH_PASSWORD")?;
            let status = password_ssh(assignment.ssh_port, &password, "true").await;
            if !status.is_ok_and(|status| status.success()) {
                failures.push(format!("{}: SSH unavailable", assignment.territory));
            }
        }
        if failures.is_empty() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(RunError::Preflight(failures.join("; ")))
}

fn milestone_eligible(world: &WorldState, territory: &str, milestone: &MilestoneConfig) -> bool {
    let Some(territory) = world.territories.get(territory) else {
        return false;
    };
    if territory
        .milestones
        .get(&milestone.id)
        .is_some_and(|view| view.passed || view.evaluating)
    {
        return false;
    }
    milestone.depends_on.iter().all(|dependency| {
        territory
            .milestones
            .get(dependency)
            .is_some_and(|view| view.passed)
    })
}

async fn run_milestone_verifier(
    options: &RunOptions,
    plan: &NetworkPlan,
    territory: &aoe_domain::TerritoryConfig,
    milestone: &MilestoneConfig,
    world: &WorldState,
) -> Result<serde_json::Value, String> {
    let assignment = plan
        .assignments
        .iter()
        .find(|assignment| assignment.territory == territory.id)
        .expect("network assignment");
    let arena_root = options.manifest.parent().unwrap_or_else(|| Path::new("."));
    let verifier = arena_root.join(&milestone.verifier);
    let evidence_dir = options.output.join("evidence").join(&territory.id);
    tokio::fs::create_dir_all(&evidence_dir)
        .await
        .map_err(|error| error.to_string())?;
    let evidence_file = evidence_dir.join(format!("{}.json", milestone.id));
    let previous = serde_json::to_vec(
        &world
            .territories
            .get(&territory.id)
            .map(|view| &view.milestones)
            .cloned()
            .unwrap_or_default(),
    )
    .map_err(|error| error.to_string())?;
    let previous_file = evidence_dir.join("previous.json");
    tokio::fs::write(&previous_file, previous)
        .await
        .map_err(|error| error.to_string())?;
    let output = tokio::time::timeout(
        Duration::from_secs(milestone.timeout_seconds),
        tokio::process::Command::new(&verifier)
            .env("AOE_TERRITORY_ID", &territory.id)
            .env("AOE_HOST", "127.0.0.1")
            .env("AOE_SSH_PORT", assignment.ssh_port.to_string())
            .env("AOE_SERVICE_PORT", assignment.service_port.to_string())
            .env(
                "AOE_CREDENTIAL_FILE",
                options.credentials[&territory.id].as_os_str(),
            )
            .env("AOE_PREVIOUS_EVIDENCE", previous_file)
            .env("AOE_EVIDENCE_FILE", &evidence_file)
            .output(),
    )
    .await
    .map_err(|_| "verifier timed out".to_owned())?
    .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    let source = tokio::fs::read_to_string(&evidence_file)
        .await
        .map_err(|error| format!("verifier produced no evidence: {error}"))?;
    serde_json::from_str(&source).map_err(|error| format!("invalid evidence: {error}"))
}

async fn wait_for_one_ssh(
    assignment: &aoe_runtime::NetworkAssignment,
    credential: &Path,
    timeout: Duration,
) -> Result<(), RunError> {
    let password = credential_value(credential, "AOE_SSH_PASSWORD")?;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if password_ssh(assignment.ssh_port, &password, "true")
            .await
            .is_ok_and(|status| status.success())
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(RunError::Preflight(format!(
        "{}: SSH unavailable after reboot",
        assignment.territory
    )))
}

async fn wait_for_one_ssh_cycle(
    assignment: &aoe_runtime::NetworkAssignment,
    credential: &Path,
    stop_timeout: Duration,
    start_timeout: Duration,
) -> Result<(), RunError> {
    let password = credential_value(credential, "AOE_SSH_PASSWORD")?;
    let stopped_by = Instant::now() + stop_timeout;
    while Instant::now() < stopped_by {
        if !password_ssh(assignment.ssh_port, &password, "true")
            .await
            .is_ok_and(|status| status.success())
        {
            return wait_for_one_ssh(assignment, credential, start_timeout).await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(RunError::Preflight(format!(
        "{}: SSH never stopped during reboot",
        assignment.territory
    )))
}

fn record_build_agent_results(
    referee: &mut BuildReferee,
    log: &mut EventLog,
    world: &mut WorldState,
    events: &mut Vec<EventEnvelope>,
    results: Vec<aoe_agent::AgentResult>,
    usage_seen: &mut HashMap<String, AgentUsage>,
    elapsed: u64,
) -> Result<(), RunError> {
    for result in results {
        let source = match result.status {
            AgentStatus::Unavailable => FailureSource::Provider,
            AgentStatus::HarnessError => FailureSource::Harness,
            AgentStatus::Interrupted => FailureSource::Arena,
            _ => FailureSource::Player,
        };
        let terminal = if result.status == AgentStatus::Interrupted {
            Event::AgentInterrupted {
                agent: result.agent.clone(),
                source,
                detail: result.summary.clone(),
            }
        } else {
            Event::AgentFinished {
                agent: result.agent.clone(),
                source,
                success: result.status == AgentStatus::Completed,
                detail: result.summary.clone(),
            }
        };
        let usage = usage_delta(usage_seen, &result.agent, &result.usage);
        for event in [
            terminal,
            Event::UsageCharged {
                agent: result.agent,
                resource_units: usage.resource_units,
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cost_microusd: usage.cost_microusd,
            },
        ] {
            let envelope = referee.record(event, elapsed)?;
            append(log, world, events, [envelope])?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn record_build_usage_checkpoints(
    usage_root: &Path,
    usage_seen: &mut HashMap<String, AgentUsage>,
    referee: &mut BuildReferee,
    log: &mut EventLog,
    world: &mut WorldState,
    events: &mut Vec<EventEnvelope>,
    elapsed: u64,
) -> Result<(), RunError> {
    for agent in world.agents.keys().cloned().collect::<Vec<_>>() {
        let path = usage_root.join("agents").join(&agent).join("usage.json");
        let Ok(source) = std::fs::read(&path) else {
            continue;
        };
        let Ok(checkpoint) = serde_json::from_slice::<AgentUsageCheckpoint>(&source) else {
            continue;
        };
        let expected_territory = world.agents.get(&agent).map(|view| view.territory.as_str());
        if checkpoint.schema_version != 1
            || checkpoint.agent != agent
            || Some(checkpoint.territory.as_str()) != expected_territory
        {
            continue;
        }
        let delta = usage_delta(usage_seen, &agent, &checkpoint.usage);
        if usage_empty(&delta) {
            continue;
        }
        let envelope = referee.record(
            Event::UsageCharged {
                agent,
                resource_units: delta.resource_units,
                input_tokens: delta.input_tokens,
                output_tokens: delta.output_tokens,
                cost_microusd: delta.cost_microusd,
            },
            elapsed,
        )?;
        append(log, world, events, [envelope])?;
    }
    Ok(())
}

fn usage_delta(
    seen: &mut HashMap<String, AgentUsage>,
    agent: &str,
    cumulative: &AgentUsage,
) -> AgentUsage {
    let previous = seen.entry(agent.to_owned()).or_default();
    let cumulative = AgentUsage {
        rounds: option_max(previous.rounds, cumulative.rounds),
        tool_calls: option_max(previous.tool_calls, cumulative.tool_calls),
        input_tokens: option_max(previous.input_tokens, cumulative.input_tokens),
        output_tokens: option_max(previous.output_tokens, cumulative.output_tokens),
        cost_microusd: option_max(previous.cost_microusd, cumulative.cost_microusd),
        resource_units: previous.resource_units.max(cumulative.resource_units),
    };
    let delta = AgentUsage {
        rounds: option_delta(cumulative.rounds, previous.rounds),
        tool_calls: option_delta(cumulative.tool_calls, previous.tool_calls),
        input_tokens: option_delta(cumulative.input_tokens, previous.input_tokens),
        output_tokens: option_delta(cumulative.output_tokens, previous.output_tokens),
        cost_microusd: option_delta(cumulative.cost_microusd, previous.cost_microusd),
        resource_units: cumulative
            .resource_units
            .saturating_sub(previous.resource_units),
    };
    previous.clone_from(&cumulative);
    delta
}

fn option_max(previous: Option<u64>, current: Option<u64>) -> Option<u64> {
    match (previous, current) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (value @ Some(_), None) | (None, value @ Some(_)) => value,
        (None, None) => None,
    }
}

fn option_delta(current: Option<u64>, previous: Option<u64>) -> Option<u64> {
    current.map(|value| value.saturating_sub(previous.unwrap_or(0)))
}

fn usage_empty(usage: &AgentUsage) -> bool {
    usage.resource_units == 0
        && usage.input_tokens.unwrap_or(0) == 0
        && usage.output_tokens.unwrap_or(0) == 0
        && usage.cost_microusd.unwrap_or(0) == 0
}

async fn password_ssh(
    port: u16,
    password: &str,
    command: &str,
) -> Result<std::process::ExitStatus, std::io::Error> {
    Ok(password_ssh_output(port, password, command).await?.status)
}

async fn password_ssh_output(
    port: u16,
    password: &str,
    command: &str,
) -> Result<std::process::Output, std::io::Error> {
    let root = std::env::temp_dir().join(format!("aoe-ssh-{}-{port}", std::process::id()));
    tokio::fs::create_dir_all(&root).await?;
    let askpass = root.join("askpass.sh");
    tokio::fs::write(
        &askpass,
        format!(
            "#!/bin/sh\nprintf '%s\\n' '{}'\n",
            password.replace('"', "")
        ),
    )
    .await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&askpass, std::fs::Permissions::from_mode(0o700)).await?;
    }
    let mut child = tokio::process::Command::new("ssh");
    child
        .args([
            "-p",
            &port.to_string(),
            "-o",
            "BatchMode=no",
            "-o",
            "ConnectTimeout=2",
            "-o",
            "ConnectionAttempts=1",
            "-o",
            "PreferredAuthentications=password",
            "-o",
            "PubkeyAuthentication=no",
            "-o",
            "StrictHostKeyChecking=no",
            "-o",
            "UserKnownHostsFile=/dev/null",
            "root@127.0.0.1",
            command,
        ])
        .env("SSH_ASKPASS", askpass)
        .env("SSH_ASKPASS_REQUIRE", "force")
        .env("DISPLAY", ":0")
        .kill_on_drop(true);
    tokio::time::timeout(Duration::from_secs(15), child.output())
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "SSH probe timed out"))?
}

async fn audit_fog_of_war(
    manifest: &ArenaManifest,
    plan: &NetworkPlan,
    credentials: &HashMap<String, PathBuf>,
) -> Result<(), RunError> {
    let Some(audit) = manifest
        .fog_of_war
        .as_ref()
        .and_then(|fog| fog.guest_leak_audit.as_ref())
    else {
        return Ok(());
    };
    let paths = audit
        .scan_paths
        .iter()
        .map(|path| shell_quote(path))
        .collect::<Vec<_>>()
        .join(" ");
    for assignment in &plan.assignments {
        let credential = credentials.get(&assignment.territory).ok_or_else(|| {
            RunError::Preflight(format!("{}: missing credential", assignment.territory))
        })?;
        let password = credential_value(credential, "AOE_SSH_PASSWORD")?;
        for forbidden in &audit.forbidden_strings {
            let command = format!(
                "match=$(find {paths} -xdev -type f -size -4M -print0 2>/dev/null | \
                 xargs -0 -r grep -IlF -- {} 2>/dev/null | head -n 1); \
                 if [ -n \"$match\" ]; then printf '%s\\n' \"$match\"; exit 42; fi",
                shell_quote(forbidden)
            );
            let output = password_ssh_output(assignment.ssh_port, &password, &command).await?;
            if output.status.code() == Some(42) {
                return Err(RunError::GuestLeak {
                    territory: assignment.territory.clone(),
                    forbidden: forbidden.clone(),
                    path: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
                });
            }
            if !output.status.success() {
                return Err(RunError::Preflight(format!(
                    "{}: fog-of-war leak audit failed: {}",
                    assignment.territory,
                    String::from_utf8_lossy(&output.stderr).trim()
                )));
            }
        }
    }
    Ok(())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn credential_value(path: &Path, key: &str) -> Result<String, RunError> {
    let source = std::fs::read_to_string(path)?;
    source
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{key}=")))
        .map(|value| value.trim_matches(['\'', '"']).to_owned())
        .ok_or_else(|| RunError::Preflight(format!("{} does not define {key}", path.display())))
}

fn validate_adapters(
    manifest: &ArenaManifest,
    adapters: &HashMap<String, PathBuf>,
) -> Result<(), RunError> {
    for agent in &manifest.agents {
        if !adapters.contains_key(&agent.adapter) {
            return Err(RunError::UnknownAdapter {
                agent: agent.id.clone(),
                adapter: agent.adapter.clone(),
            });
        }
    }
    Ok(())
}

fn register_territories(
    manifest: &ArenaManifest,
    referee: &mut Referee,
    log: &mut EventLog,
    world: &mut WorldState,
    events: &mut Vec<EventEnvelope>,
) -> Result<(), RunError> {
    for territory in &manifest.territories {
        let agent = manifest
            .agents
            .iter()
            .find(|agent| agent.territory == territory.id)
            .expect("validated manifest assigns every territory");
        let event = referee.record(
            Event::TerritoryRegistered {
                territory: territory.id.clone(),
                class: territory.class.clone(),
                agent: agent.id.clone(),
            },
            0,
        )?;
        append(log, world, events, [event])?;
    }
    Ok(())
}

fn render_live(world: &WorldState, events: &[EventEnvelope], color: bool) {
    let rendered = render_world(
        world,
        events,
        RenderOptions {
            color,
            ..RenderOptions::default()
        },
    );
    if color {
        print!("\x1b[2J\x1b[H{rendered}");
    } else {
        println!("{rendered}");
    }
}

fn invocations(
    manifest: &ArenaManifest,
    plan: &NetworkPlan,
    options: &RunOptions,
) -> Result<Vec<AgentInvocation>, RunError> {
    let arena_root = options.manifest.parent().unwrap_or_else(|| Path::new("."));
    let player_brief = manifest
        .fog_of_war
        .as_ref()
        .map(|fog| {
            let path = arena_root.join(&fog.player_brief);
            std::fs::read_to_string(&path).map_err(|_| RunError::MissingInstruction {
                territory: "fog-of-war player brief".into(),
                path: path.display().to_string(),
            })
        })
        .transpose()?;
    let build_contract = if manifest.arena.mode == MatchMode::BuildRace && player_brief.is_none() {
        let path = arena_root.join("CONTRACT.md");
        Some(
            std::fs::read_to_string(&path).map_err(|_| RunError::MissingInstruction {
                territory: "build contract".into(),
                path: path.display().to_string(),
            })?,
        )
    } else {
        None
    };
    manifest
        .agents
        .iter()
        .map(|agent| {
            let assignment = plan
                .assignments
                .iter()
                .find(|assignment| assignment.territory == agent.territory)
                .expect("validated manifest and network plan must agree");
            let mut instruction = if let Some(brief) = &player_brief {
                brief.clone()
            } else {
                let instruction_path = arena_root
                    .join("instructions")
                    .join(format!("{}.md", agent.territory));
                std::fs::read_to_string(&instruction_path).map_err(|_| {
                    RunError::MissingInstruction {
                        territory: agent.territory.clone(),
                        path: instruction_path.display().to_string(),
                    }
                })?
            };
            if player_brief.is_some() {
                let _ = write!(
                    instruction,
                    "\n\nThe controller-owned referee is the only authority on completion. Implement and verify the requested outcome on this host, then leave it running. Referee evidence, hidden topology, other competitors, and future events are not available to you. Your hard deadline is {} seconds.\n",
                    manifest.rules.duration_seconds
                );
            }
            if let Some(contract) = &build_contract {
                instruction.push_str(
                    "\n\nThe controller-owned referee is the only authority on completion. Do not stop after merely describing the work. Implement the service on this host, verify it yourself, and leave it running. You have no access to referee evidence or other competitors.\n\n# Service contract\n\n",
                );
                instruction.push_str(contract);
            }
            Ok(AgentInvocation {
                config: agent.clone(),
                territory_host: "127.0.0.1".into(),
                ssh_port: assignment.ssh_port,
                instruction,
                credential_file: options.credentials.get(&agent.territory).cloned(),
            })
        })
        .collect()
}

fn probe_targets(manifest: &ArenaManifest, plan: &NetworkPlan) -> BTreeMap<String, ProbeTarget> {
    manifest
        .territories
        .iter()
        .map(|territory| {
            let assignment = plan
                .assignments
                .iter()
                .find(|assignment| assignment.territory == territory.id)
                .expect("validated manifest and network plan must agree");
            (
                territory.id.clone(),
                ProbeTarget {
                    host: "127.0.0.1".into(),
                    port: assignment.service_port,
                    path: territory.service.path.clone(),
                    expected_status: territory.service.expected_status,
                    expected_body: territory.service.expected_body.clone(),
                },
            )
        })
        .collect()
}

async fn wait_for_preflight(
    probe: &impl HealthProbe,
    targets: &BTreeMap<String, ProbeTarget>,
    timeout: Duration,
) -> Result<(), RunError> {
    let deadline = Instant::now() + timeout;
    let mut unhealthy = Vec::new();
    while Instant::now() < deadline {
        unhealthy.clear();
        for (territory, target) in targets {
            let observation = probe.observe(target).await;
            if !observation.healthy {
                unhealthy.push(format!("{territory}: {}", observation.detail));
            }
        }
        if unhealthy.is_empty() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(RunError::Preflight(unhealthy.join("; ")))
}

fn record_agent_results(
    referee: &mut Referee,
    log: &mut EventLog,
    world: &mut WorldState,
    events: &mut Vec<EventEnvelope>,
    results: Vec<aoe_agent::AgentResult>,
    elapsed: u64,
) -> Result<(), RunError> {
    for result in results {
        let source = match result.status {
            AgentStatus::Unavailable => FailureSource::Provider,
            AgentStatus::HarnessError => FailureSource::Harness,
            AgentStatus::Interrupted => FailureSource::Arena,
            _ => FailureSource::Player,
        };
        let success = result.status == AgentStatus::Completed;
        let finished = referee.record(
            Event::AgentFinished {
                agent: result.agent.clone(),
                source,
                success,
                detail: result.summary,
            },
            elapsed,
        )?;
        append(log, world, events, [finished])?;
        let usage = referee.record(
            Event::UsageCharged {
                agent: result.agent,
                resource_units: result.usage.resource_units,
                input_tokens: result.usage.input_tokens,
                output_tokens: result.usage.output_tokens,
                cost_microusd: result.usage.cost_microusd,
            },
            elapsed,
        )?;
        append(log, world, events, [usage])?;
        if referee.outcome().is_none() {
            let charged =
                referee.charge(&result.territory, result.usage.resource_units, elapsed)?;
            append(log, world, events, [charged])?;
        }
    }
    Ok(())
}

fn append(
    log: &mut EventLog,
    world: &mut WorldState,
    all: &mut Vec<EventEnvelope>,
    emitted: impl IntoIterator<Item = EventEnvelope>,
) -> Result<(), RunError> {
    for event in emitted {
        log.append(&event)?;
        reduce(world, &event);
        all.push(event);
    }
    Ok(())
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use aoe_agent::{AgentResult, AgentStatus, AgentUsage, AgentUsageCheckpoint};
    use aoe_domain::{ArenaManifest, Event, MatchState};
    use aoe_referee::{BuildReferee, Referee};
    use aoe_replay::{EventLog, WorldState};
    use std::collections::HashMap;
    use tokio::sync::mpsc;

    use super::{
        RunOptions, append, drain_build_agents, invocations, record_agent_results,
        record_build_usage_checkpoints, resolve_nixos_configs, shell_quote, usage_delta,
    };

    const MANIFEST: &str = include_str!("../../runtime/tests/fixture.toml");

    #[test]
    fn package_relative_flake_references_become_absolute() {
        let root = std::env::temp_dir().join(format!(
            "aoe-controller-relative-flake-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("arena root");
        let mut manifest = ArenaManifest::parse(MANIFEST).expect("manifest");
        for territory in &mut manifest.territories {
            territory.nixos_config = ".#nixosConfigurations.test".into();
        }

        resolve_nixos_configs(&root.join("arena.toml"), &mut manifest);

        let expected = root.canonicalize().expect("canonical root");
        for territory in &manifest.territories {
            assert_eq!(
                territory.nixos_config,
                format!("path:{}#nixosConfigurations.test", expected.display())
            );
        }
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn cumulative_usage_checkpoints_are_recorded_once() {
        let mut seen = HashMap::new();
        let first = usage_delta(
            &mut seen,
            "agent-a",
            &AgentUsage {
                input_tokens: Some(100),
                output_tokens: Some(10),
                cost_microusd: Some(1_000),
                resource_units: 1,
                ..AgentUsage::default()
            },
        );
        assert_eq!(first.input_tokens, Some(100));
        assert_eq!(first.output_tokens, Some(10));
        assert_eq!(first.cost_microusd, Some(1_000));
        assert_eq!(first.resource_units, 1);

        let second = usage_delta(
            &mut seen,
            "agent-a",
            &AgentUsage {
                input_tokens: Some(175),
                output_tokens: Some(25),
                cost_microusd: Some(1_800),
                resource_units: 1,
                ..AgentUsage::default()
            },
        );
        assert_eq!(second.input_tokens, Some(75));
        assert_eq!(second.output_tokens, Some(15));
        assert_eq!(second.cost_microusd, Some(800));
        assert_eq!(second.resource_units, 0);

        let stale = usage_delta(
            &mut seen,
            "agent-a",
            &AgentUsage {
                input_tokens: Some(150),
                output_tokens: None,
                cost_microusd: None,
                resource_units: 1,
                ..AgentUsage::default()
            },
        );
        assert_eq!(stale.input_tokens, Some(0));
        assert_eq!(stale.output_tokens, Some(0));
        assert_eq!(stale.cost_microusd, Some(0));
        assert_eq!(stale.resource_units, 0);
    }

    #[test]
    fn running_usage_checkpoint_becomes_a_referee_event() {
        let manifest_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../arenas/first-build/agents-real.toml");
        let manifest = ArenaManifest::load(&manifest_path).expect("build manifest");
        let mut referee = BuildReferee::from_manifest(&manifest);
        let mut world = WorldState::default();
        let root = std::env::temp_dir().join(format!(
            "aoe-controller-usage-checkpoint-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let log_path = root.join("events.jsonl");
        std::fs::create_dir_all(root.join("agents/deepseek-builder")).expect("agent dir");
        let mut log = EventLog::open(&log_path).expect("event log");
        let mut events = Vec::new();
        append(
            &mut log,
            &mut world,
            &mut events,
            referee.start().expect("start"),
        )
        .expect("append start");
        for agent in &manifest.agents {
            let event = referee
                .record(
                    Event::AgentStarted {
                        agent: agent.id.clone(),
                        territory: agent.territory.clone(),
                        model: agent.model.clone(),
                    },
                    0,
                )
                .expect("agent started");
            append(&mut log, &mut world, &mut events, [event]).expect("append agent");
        }
        std::fs::write(
            root.join("agents/deepseek-builder/usage.json"),
            serde_json::to_vec(&AgentUsageCheckpoint {
                schema_version: 1,
                agent: "deepseek-builder".into(),
                territory: "builder-one".into(),
                usage: AgentUsage {
                    input_tokens: Some(2_000),
                    output_tokens: Some(50),
                    cost_microusd: Some(9_000),
                    resource_units: 1,
                    ..AgentUsage::default()
                },
            })
            .expect("checkpoint"),
        )
        .expect("checkpoint file");
        record_build_usage_checkpoints(
            &root,
            &mut HashMap::new(),
            &mut referee,
            &mut log,
            &mut world,
            &mut events,
            5_000,
        )
        .expect("record checkpoint");
        let agent = world.agents.get("deepseek-builder").expect("agent view");
        assert!(agent.running);
        assert_eq!(agent.input_tokens, 2_000);
        assert_eq!(agent.output_tokens, 50);
        assert_eq!(agent.cost_microusd, 9_000);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn late_agent_result_is_recorded_without_charging_finished_match() {
        let manifest = ArenaManifest::parse(MANIFEST).expect("manifest");
        let mut referee = Referee::from_manifest(&manifest);
        let mut world = WorldState::default();
        let path = std::env::temp_dir().join(format!(
            "aoe-controller-late-agent-result-{}.jsonl",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let mut log = EventLog::open(&path).expect("event log");
        let mut events = Vec::new();
        append(
            &mut log,
            &mut world,
            &mut events,
            referee.start().expect("start"),
        )
        .expect("append start");
        let finish_at = manifest.rules.duration_seconds * 1000;
        append(
            &mut log,
            &mut world,
            &mut events,
            referee.tick(finish_at).expect("finish on timer"),
        )
        .expect("append finish");
        let result = AgentResult {
            schema_version: 1,
            agent: "gate-agent".into(),
            territory: "gate".into(),
            status: AgentStatus::TimedOut,
            summary: "agent exceeded its deadline".into(),
            usage: AgentUsage {
                resource_units: 1,
                ..AgentUsage::default()
            },
            transcript: None,
        };

        record_agent_results(
            &mut referee,
            &mut log,
            &mut world,
            &mut events,
            vec![result],
            finish_at,
        )
        .expect("late result");

        assert!(events.iter().any(|event| matches!(
            event.event,
            Event::AgentFinished { ref agent, .. } if agent == "gate-agent"
        )));
        assert!(events.iter().any(|event| matches!(
            event.event,
            Event::UsageCharged { ref agent, .. } if agent == "gate-agent"
        )));
        assert!(!events.iter().any(|event| matches!(
            event.event,
            Event::ResourcesChanged { ref reason, .. } if reason == "agent inference"
        )));
        std::fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn build_invocations_embed_the_service_contract() {
        let manifest_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../arenas/first-build/agents-real.toml");
        let manifest = ArenaManifest::load(&manifest_path).expect("build manifest");
        let plan =
            aoe_runtime::NetworkPlan::from_manifest(&manifest, 26000, 23977).expect("network plan");
        let options = RunOptions {
            manifest: manifest_path,
            output: std::path::PathBuf::from("matches/test"),
            adapters: std::collections::HashMap::new(),
            credentials: std::collections::HashMap::new(),
            base_port: 26000,
            multicast_port: 23977,
            color: false,
        };
        let invocations = invocations(&manifest, &plan, &options).expect("invocations");
        assert_eq!(invocations.len(), 3);
        for invocation in invocations {
            assert!(invocation.instruction.contains("GET /health"));
            assert!(invocation.instruction.contains("PUT /records/<id>"));
            assert!(
                invocation
                    .instruction
                    .contains("only authority on completion")
            );
            assert!(!invocation.instruction.contains("builder-one-race"));
        }
    }

    #[test]
    fn fog_invocations_expose_only_the_player_brief() {
        let manifest_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../arenas/durable-job-queue/agents-real.toml");
        let manifest = ArenaManifest::load(&manifest_path).expect("fog manifest");
        let plan =
            aoe_runtime::NetworkPlan::from_manifest(&manifest, 26000, 23977).expect("network plan");
        let options = RunOptions {
            manifest: manifest_path,
            output: std::path::PathBuf::from("matches/test"),
            adapters: std::collections::HashMap::new(),
            credentials: std::collections::HashMap::new(),
            base_port: 26000,
            multicast_port: 23977,
            color: false,
        };
        let invocations = invocations(&manifest, &plan, &options).expect("invocations");
        for invocation in invocations {
            assert!(invocation.instruction.contains("Opaque work accepted"));
            assert!(
                invocation
                    .instruction
                    .contains("hard deadline is 900 seconds")
            );
            assert!(!invocation.instruction.contains("accepted-alpha-7d3"));
            assert!(!invocation.instruction.contains("recover-accepted.sh"));
            assert!(!invocation.instruction.contains("queue-api.service"));
        }
    }

    #[test]
    fn shell_quote_handles_apostrophes() {
        assert_eq!(shell_quote("agent's clue"), "'agent'\\''s clue'");
    }

    #[tokio::test]
    async fn post_match_drain_records_late_results_at_the_frozen_clock() {
        let manifest_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../arenas/first-build/agents-real.toml");
        let manifest = ArenaManifest::load(&manifest_path).expect("build manifest");
        let mut referee = BuildReferee::from_manifest(&manifest);
        let mut world = WorldState::default();
        let path = std::env::temp_dir().join(format!(
            "aoe-controller-build-drain-{}.jsonl",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let mut log = EventLog::open(&path).expect("event log");
        let mut events = Vec::new();
        let output = std::env::temp_dir().join(format!(
            "aoe-controller-build-drain-output-{}",
            std::process::id()
        ));
        append(
            &mut log,
            &mut world,
            &mut events,
            referee.start().expect("start"),
        )
        .expect("append start");
        append(
            &mut log,
            &mut world,
            &mut events,
            referee.finish("test winner", 40_000).expect("finish"),
        )
        .expect("append finish");
        let frozen_winner = world.winner.clone();
        let (sender, mut receiver) = mpsc::channel(2);
        let mut task = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            sender
                .send(AgentResult {
                    schema_version: 1,
                    agent: "luna-builder".into(),
                    territory: "builder-two".into(),
                    status: AgentStatus::Completed,
                    summary: "returned during drain".into(),
                    usage: AgentUsage {
                        input_tokens: Some(12),
                        resource_units: 1,
                        ..AgentUsage::default()
                    },
                    transcript: Some("transcript.json".into()),
                })
                .await
                .expect("send result");
        });
        drain_build_agents(
            &mut task,
            &mut receiver,
            &mut referee,
            &mut log,
            &mut world,
            &mut events,
            &mut HashMap::new(),
            &output,
            40_000,
            std::time::Duration::from_millis(100),
        )
        .await
        .expect("drain");

        assert_eq!(world.match_state, MatchState::Finished);
        assert_eq!(world.winner, frozen_winner);
        let finished = events
            .iter()
            .find(|event| matches!(event.event, Event::AgentFinished { ref agent, .. } if agent == "luna-builder"))
            .expect("late result event");
        assert_eq!(finished.elapsed_ms, 40_000);
        assert_eq!(world.elapsed_ms, 40_000);
        assert!(events.iter().any(|event| matches!(
            event.event,
            Event::PostMatchDrainStarted {
                pending_agents: 0,
                ..
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event.event,
            Event::PostMatchDrainFinished {
                captured_agents: 1,
                terminated_agents: 0,
            }
        )));
        std::fs::remove_file(path).expect("cleanup");
    }

    #[tokio::test]
    async fn post_match_drain_terminates_agents_at_the_deadline() {
        let manifest_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../arenas/first-build/agents-real.toml");
        let manifest = ArenaManifest::load(&manifest_path).expect("build manifest");
        let mut referee = BuildReferee::from_manifest(&manifest);
        let mut world = WorldState::default();
        let path = std::env::temp_dir().join(format!(
            "aoe-controller-build-drain-timeout-{}.jsonl",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let mut log = EventLog::open(&path).expect("event log");
        let mut events = Vec::new();
        let output = std::env::temp_dir().join(format!(
            "aoe-controller-build-timeout-output-{}",
            std::process::id()
        ));
        append(
            &mut log,
            &mut world,
            &mut events,
            referee.start().expect("start"),
        )
        .expect("append start");
        for agent in &manifest.agents {
            let event = referee
                .record(
                    Event::AgentStarted {
                        agent: agent.id.clone(),
                        territory: agent.territory.clone(),
                        model: agent.model.clone(),
                    },
                    0,
                )
                .expect("agent started");
            append(&mut log, &mut world, &mut events, [event]).expect("append agent");
        }
        append(
            &mut log,
            &mut world,
            &mut events,
            referee.finish("test winner", 40_000).expect("finish"),
        )
        .expect("append finish");
        let (sender, mut receiver) = mpsc::channel(1);
        let mut task = tokio::spawn(async move {
            let _sender = sender;
            std::future::pending::<()>().await;
        });
        let started = std::time::Instant::now();
        drain_build_agents(
            &mut task,
            &mut receiver,
            &mut referee,
            &mut log,
            &mut world,
            &mut events,
            &mut HashMap::new(),
            &output,
            40_000,
            std::time::Duration::from_millis(20),
        )
        .await
        .expect("drain");
        assert!(started.elapsed() < std::time::Duration::from_millis(100));
        assert!(events.iter().any(|event| matches!(
            event.event,
            Event::PostMatchDrainFinished {
                captured_agents: 0,
                terminated_agents: 3,
            }
        )));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event.event, Event::AgentTerminated { .. }))
                .count(),
            3
        );
        assert!(world.agents.values().all(|agent| !agent.running));
        std::fs::remove_file(path).expect("cleanup");
    }
}
