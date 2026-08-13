use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use aoe_agent::{AgentController, AgentInvocation, AgentStatus, CommandAdapter};
use aoe_domain::{
    ArenaManifest, Event, EventEnvelope, FailureSource, MatchMode, MilestoneConfig,
    MilestoneOperation,
};
use aoe_referee::{BuildReferee, HealthProbe, HttpProbe, ProbeTarget, Referee};
use aoe_replay::{EventLog, WorldState, reduce};
use aoe_runtime::{ArenaSupervisor, NetworkPlan, NixVmDriver};
use aoe_tui::{RenderOptions, render_world};
use thiserror::Error;

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
}

/// Run a complete match, using the same event reducer as replay mode.
///
/// # Errors
///
/// Returns an error for invalid configuration, guest lifecycle failures,
/// failed preflight, event persistence errors, or agent controller failures.
pub async fn run_match(options: RunOptions) -> Result<WorldState, RunError> {
    let event_path = options.output.join("events.jsonl");
    if event_path
        .metadata()
        .is_ok_and(|metadata| metadata.len() > 0)
    {
        return Err(RunError::OutputExists(event_path.display().to_string()));
    }
    std::fs::create_dir_all(&options.output)?;
    let manifest = ArenaManifest::load(&options.manifest)?;
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

async fn run_booted_match(
    manifest: &ArenaManifest,
    plan: &NetworkPlan,
    options: &RunOptions,
    supervisor: &mut ArenaSupervisor<NixVmDriver>,
) -> Result<WorldState, RunError> {
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

async fn run_booted_build_match(
    manifest: &ArenaManifest,
    plan: &NetworkPlan,
    options: &RunOptions,
    _supervisor: &mut ArenaSupervisor<NixVmDriver>,
) -> Result<WorldState, RunError> {
    wait_for_ssh(plan, &options.credentials, Duration::from_secs(120)).await?;

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
    let mut agent_task =
        tokio::spawn(async move { agents.run_all(invocations, agent_timeout).await });
    let mut interrupt = Box::pin(tokio::signal::ctrl_c());
    let started = Instant::now();
    let deadline = Duration::from_secs(manifest.rules.duration_seconds);
    let tick = Duration::from_millis(manifest.rules.tick_ms);
    let build = manifest.build.as_ref().expect("validated build contract");

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
    if !agent_task.is_finished() {
        agent_task.abort();
    } else {
        let results = (&mut agent_task)
            .await
            .map_err(|error| RunError::AgentTask(error.to_string()))?;
        record_build_agent_results(
            &mut referee,
            &mut log,
            &mut world,
            &mut events,
            results,
            elapsed_ms(started),
        )?;
    }
    std::fs::write(
        options.output.join("world.json"),
        serde_json::to_vec_pretty(&world)?,
    )?;
    Ok(world)
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
    elapsed: u64,
) -> Result<(), RunError> {
    for result in results {
        let source = match result.status {
            AgentStatus::Unavailable => FailureSource::Provider,
            AgentStatus::HarnessError => FailureSource::Harness,
            _ => FailureSource::Player,
        };
        for event in [
            Event::AgentFinished {
                agent: result.agent.clone(),
                source,
                success: result.status == AgentStatus::Completed,
                detail: result.summary,
            },
            Event::UsageCharged {
                agent: result.agent,
                resource_units: result.usage.resource_units,
                input_tokens: result.usage.input_tokens,
                output_tokens: result.usage.output_tokens,
                cost_microusd: result.usage.cost_microusd,
            },
        ] {
            let envelope = referee.record(event, elapsed)?;
            append(log, world, events, [envelope])?;
        }
    }
    Ok(())
}

async fn password_ssh(
    port: u16,
    password: &str,
    command: &str,
) -> Result<std::process::ExitStatus, std::io::Error> {
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
    tokio::process::Command::new("ssh")
        .args([
            "-p",
            &port.to_string(),
            "-o",
            "BatchMode=no",
            "-o",
            "ConnectTimeout=2",
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
        .status()
        .await
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
    let build_contract = if manifest.arena.mode == MatchMode::BuildRace {
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
            let instruction_path = arena_root
                .join("instructions")
                .join(format!("{}.md", agent.territory));
            let mut instruction = std::fs::read_to_string(&instruction_path).map_err(|_| {
                RunError::MissingInstruction {
                    territory: agent.territory.clone(),
                    path: instruction_path.display().to_string(),
                }
            })?;
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
    use aoe_agent::{AgentResult, AgentStatus, AgentUsage};
    use aoe_domain::{ArenaManifest, Event};
    use aoe_referee::Referee;
    use aoe_replay::{EventLog, WorldState};

    use super::{RunOptions, append, invocations, record_agent_results};

    const MANIFEST: &str = include_str!("../../runtime/tests/fixture.toml");

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
}
