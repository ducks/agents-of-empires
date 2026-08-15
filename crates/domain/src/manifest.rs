use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

const SUPPORTED_SCHEMA_VERSION: u32 = 1;

/// Complete declarative description of one match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArenaManifest {
    pub schema_version: u32,
    pub arena: ArenaConfig,
    pub network: NetworkConfig,
    pub rules: MatchRules,
    pub build: Option<BuildContract>,
    #[serde(default)]
    pub visualization: Option<ArenaVisualization>,
    pub classes: Vec<TerritoryClass>,
    pub territories: Vec<TerritoryConfig>,
    pub agents: Vec<AgentConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArenaVisualization {
    pub nodes: Vec<TopologyNode>,
    #[serde(default)]
    pub links: Vec<TopologyLink>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopologyNode {
    pub id: String,
    pub display_name: String,
    pub kind: TopologyNodeKind,
    pub milestone: Option<String>,
    pub x: u8,
    pub y: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TopologyNodeKind {
    Client,
    Proxy,
    Service,
    Worker,
    Queue,
    Database,
    Storage,
    Host,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopologyLink {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub kind: TopologyLinkKind,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TopologyLinkKind {
    #[default]
    Traffic,
    Queue,
    Replication,
    Storage,
    Lifecycle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArenaConfig {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub mode: MatchMode,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchMode {
    BuildRace,
    #[default]
    Conquest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildContract {
    pub stop_on_first_durable: bool,
    pub completion_milestone: String,
    pub milestones: Vec<MilestoneConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MilestoneConfig {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub verifier: String,
    #[serde(default)]
    pub operation: MilestoneOperation,
    pub timeout_seconds: u64,
    pub points: u64,
    #[serde(default = "default_true")]
    pub required: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MilestoneOperation {
    #[default]
    Observe,
    HostReboot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkConfig {
    pub cidr: String,
    #[serde(default)]
    pub allow_public_internet: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchRules {
    pub duration_seconds: u64,
    pub tick_ms: u64,
    pub healthy_resources_per_tick: u64,
    pub minimum_territories: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerritoryClass {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub strengths: Vec<String>,
    #[serde(default)]
    pub weaknesses: Vec<String>,
    pub resources: ResourceLimits,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceLimits {
    pub vcpus: u16,
    pub memory_mib: u64,
    pub disk_mib: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerritoryConfig {
    pub id: String,
    pub class: String,
    pub nixos_config: String,
    pub service: ServiceCheck,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceCheck {
    pub port: u16,
    pub path: String,
    pub expected_status: u16,
    pub expected_body: String,
    pub health: HealthPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthPolicy {
    pub poll_interval_ms: u64,
    pub consecutive_failures: u32,
    pub recovery_window_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    pub id: String,
    pub territory: String,
    pub adapter: String,
    pub model: String,
    #[serde(default = "default_reasoning_effort")]
    pub reasoning_effort: String,
    pub budget: Budget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    pub resource_units: u64,
    pub max_cost_microusd: Option<u64>,
}

fn default_reasoning_effort() -> String {
    "default".to_owned()
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("could not read arena manifest: {0}")]
    Read(#[from] std::io::Error),
    #[error("could not parse arena manifest: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("arena manifest is invalid")]
    Validation(Vec<ValidationError>),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{path}: {message}")]
pub struct ValidationError {
    pub path: String,
    pub message: String,
}

impl ArenaManifest {
    /// Load and validate a manifest from disk.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read, the TOML cannot be
    /// parsed, or semantic validation fails.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ManifestError> {
        let source = fs::read_to_string(path)?;
        Self::parse(&source)
    }

    /// Parse and validate a TOML manifest.
    ///
    /// # Errors
    ///
    /// Returns an error when the TOML cannot be parsed or semantic validation
    /// fails.
    pub fn parse(source: &str) -> Result<Self, ManifestError> {
        let manifest: Self = toml::from_str(source)?;
        let errors = manifest.validation_errors();
        if errors.is_empty() {
            Ok(manifest)
        } else {
            Err(ManifestError::Validation(errors))
        }
    }

    /// Return every semantic validation error rather than stopping at the first.
    #[must_use]
    pub fn validation_errors(&self) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        if self.schema_version != SUPPORTED_SCHEMA_VERSION {
            push_error(
                &mut errors,
                "schema_version",
                format!("expected {SUPPORTED_SCHEMA_VERSION}"),
            );
        }
        validate_id(&mut errors, "arena.id", &self.arena.id);
        if self.arena.display_name.trim().is_empty() {
            push_error(&mut errors, "arena.display_name", "must not be empty");
        }
        if self.network.allow_public_internet {
            push_error(
                &mut errors,
                "network.allow_public_internet",
                "public internet access is forbidden",
            );
        }
        if !valid_private_cidr(&self.network.cidr) {
            push_error(&mut errors, "network.cidr", "must be an RFC1918 IPv4 CIDR");
        }
        if self.rules.duration_seconds == 0 {
            push_error(&mut errors, "rules.duration_seconds", "must be positive");
        }
        if self.rules.tick_ms == 0 {
            push_error(&mut errors, "rules.tick_ms", "must be positive");
        }
        if self.rules.minimum_territories < 2 {
            push_error(
                &mut errors,
                "rules.minimum_territories",
                "must be at least 2",
            );
        }
        if self.territories.len() < self.rules.minimum_territories {
            push_error(
                &mut errors,
                "territories",
                "does not satisfy rules.minimum_territories",
            );
        }
        validate_build_contract(
            &mut errors,
            self.arena.mode,
            self.build.as_ref(),
            self.rules.duration_seconds,
        );
        validate_visualization(
            &mut errors,
            self.visualization.as_ref(),
            self.build.as_ref(),
        );

        let class_ids = validate_classes(&mut errors, &self.classes);
        let territory_ids = validate_territories(
            &mut errors,
            &self.territories,
            &class_ids,
            self.rules.duration_seconds,
        );
        validate_agents(&mut errors, &self.agents, &territory_ids);
        errors
    }
}

fn validate_visualization(
    errors: &mut Vec<ValidationError>,
    visualization: Option<&ArenaVisualization>,
    build: Option<&BuildContract>,
) {
    let Some(visualization) = visualization else {
        return;
    };
    if visualization.nodes.is_empty() {
        push_error(errors, "visualization.nodes", "must not be empty");
    }
    let milestones: HashSet<_> = build
        .into_iter()
        .flat_map(|contract| &contract.milestones)
        .map(|milestone| milestone.id.as_str())
        .collect();
    let mut node_ids = HashSet::new();
    for (index, node) in visualization.nodes.iter().enumerate() {
        let path = format!("visualization.nodes[{index}]");
        validate_id(errors, &format!("{path}.id"), &node.id);
        if !node_ids.insert(node.id.as_str()) {
            push_error(errors, format!("{path}.id"), "must be unique");
        }
        if node.display_name.trim().is_empty() {
            push_error(errors, format!("{path}.display_name"), "must not be empty");
        }
        if node.x > 100 {
            push_error(errors, format!("{path}.x"), "must be between 0 and 100");
        }
        if node.y > 100 {
            push_error(errors, format!("{path}.y"), "must be between 0 and 100");
        }
        if let Some(milestone) = &node.milestone
            && !milestones.contains(milestone.as_str())
        {
            push_error(
                errors,
                format!("{path}.milestone"),
                format!("references unknown milestone {milestone}"),
            );
        }
    }
    for (index, link) in visualization.links.iter().enumerate() {
        let path = format!("visualization.links[{index}]");
        if !node_ids.contains(link.from.as_str()) {
            push_error(
                errors,
                format!("{path}.from"),
                format!("references unknown node {}", link.from),
            );
        }
        if !node_ids.contains(link.to.as_str()) {
            push_error(
                errors,
                format!("{path}.to"),
                format!("references unknown node {}", link.to),
            );
        }
        if link.from == link.to {
            push_error(errors, path, "must connect two different nodes");
        }
        if link
            .label
            .as_ref()
            .is_some_and(|label| label.trim().is_empty())
        {
            push_error(
                errors,
                format!("visualization.links[{index}].label"),
                "must not be empty",
            );
        }
    }
}

fn validate_build_contract(
    errors: &mut Vec<ValidationError>,
    mode: MatchMode,
    build: Option<&BuildContract>,
    duration_seconds: u64,
) {
    let Some(build) = build else {
        if mode == MatchMode::BuildRace {
            push_error(errors, "build", "is required for build_race mode");
        }
        return;
    };
    if mode != MatchMode::BuildRace {
        push_error(errors, "build", "is only valid for build_race mode");
    }
    let mut ids = HashSet::new();
    for (index, milestone) in build.milestones.iter().enumerate() {
        let path = format!("build.milestones[{index}]");
        validate_id(errors, &format!("{path}.id"), &milestone.id);
        if !ids.insert(milestone.id.clone()) {
            push_error(errors, format!("{path}.id"), "must be unique");
        }
        if milestone.display_name.trim().is_empty() {
            push_error(errors, format!("{path}.display_name"), "must not be empty");
        }
        if milestone.verifier.trim().is_empty() {
            push_error(errors, format!("{path}.verifier"), "must not be empty");
        }
        if milestone.timeout_seconds == 0 || milestone.timeout_seconds > duration_seconds {
            push_error(
                errors,
                format!("{path}.timeout_seconds"),
                "must be positive and no longer than the match",
            );
        }
        if milestone.points == 0 {
            push_error(errors, format!("{path}.points"), "must be positive");
        }
    }
    if build.milestones.is_empty() {
        push_error(errors, "build.milestones", "must not be empty");
    }
    if !ids.contains(&build.completion_milestone) {
        push_error(
            errors,
            "build.completion_milestone",
            "references an unknown milestone",
        );
    }
    let milestones: HashMap<_, _> = build
        .milestones
        .iter()
        .map(|milestone| (milestone.id.as_str(), milestone))
        .collect();
    for (index, milestone) in build.milestones.iter().enumerate() {
        for dependency in &milestone.depends_on {
            if dependency == &milestone.id {
                push_error(
                    errors,
                    format!("build.milestones[{index}].depends_on"),
                    "must not depend on itself",
                );
            } else if !ids.contains(dependency) {
                push_error(
                    errors,
                    format!("build.milestones[{index}].depends_on"),
                    format!("references unknown milestone {dependency}"),
                );
            }
        }
    }
    for milestone in &build.milestones {
        let mut visiting = HashSet::new();
        if milestone_has_cycle(&milestone.id, &milestones, &mut visiting) {
            push_error(
                errors,
                "build.milestones",
                format!("dependency cycle includes {}", milestone.id),
            );
            break;
        }
    }
    if let Some(completion) = milestones.get(build.completion_milestone.as_str())
        && !completion.required
    {
        push_error(
            errors,
            "build.completion_milestone",
            "must reference a required milestone",
        );
    }
}

fn milestone_has_cycle<'a>(
    id: &'a str,
    milestones: &HashMap<&'a str, &'a MilestoneConfig>,
    visiting: &mut HashSet<&'a str>,
) -> bool {
    if !visiting.insert(id) {
        return true;
    }
    let cyclic = milestones.get(id).is_some_and(|milestone| {
        milestone
            .depends_on
            .iter()
            .any(|dependency| milestone_has_cycle(dependency, milestones, visiting))
    });
    visiting.remove(id);
    cyclic
}

fn validate_classes(
    errors: &mut Vec<ValidationError>,
    classes: &[TerritoryClass],
) -> HashSet<String> {
    let mut ids = HashSet::new();
    for (index, class) in classes.iter().enumerate() {
        let path = format!("classes[{index}]");
        validate_id(errors, &format!("{path}.id"), &class.id);
        if !ids.insert(class.id.clone()) {
            push_error(errors, format!("{path}.id"), "must be unique");
        }
        if class.display_name.trim().is_empty() {
            push_error(errors, format!("{path}.display_name"), "must not be empty");
        }
        if class.resources.vcpus == 0 {
            push_error(
                errors,
                format!("{path}.resources.vcpus"),
                "must be positive",
            );
        }
        if class.resources.memory_mib < 128 {
            push_error(
                errors,
                format!("{path}.resources.memory_mib"),
                "must be at least 128",
            );
        }
        if class.resources.disk_mib < 256 {
            push_error(
                errors,
                format!("{path}.resources.disk_mib"),
                "must be at least 256",
            );
        }
    }
    ids
}

fn validate_territories(
    errors: &mut Vec<ValidationError>,
    territories: &[TerritoryConfig],
    class_ids: &HashSet<String>,
    duration_seconds: u64,
) -> HashSet<String> {
    let mut ids = HashSet::new();
    for (index, territory) in territories.iter().enumerate() {
        let path = format!("territories[{index}]");
        validate_id(errors, &format!("{path}.id"), &territory.id);
        if !ids.insert(territory.id.clone()) {
            push_error(errors, format!("{path}.id"), "must be unique");
        }
        if !class_ids.contains(&territory.class) {
            push_error(
                errors,
                format!("{path}.class"),
                "references an unknown class",
            );
        }
        if territory.nixos_config.trim().is_empty() {
            push_error(errors, format!("{path}.nixos_config"), "must not be empty");
        }
        if !territory.service.path.starts_with('/') {
            push_error(errors, format!("{path}.service.path"), "must start with /");
        }
        if !(100..=599).contains(&territory.service.expected_status) {
            push_error(
                errors,
                format!("{path}.service.expected_status"),
                "must be an HTTP status from 100 through 599",
            );
        }
        let health = &territory.service.health;
        if health.poll_interval_ms == 0 {
            push_error(
                errors,
                format!("{path}.service.health.poll_interval_ms"),
                "must be positive",
            );
        }
        if health.consecutive_failures == 0 {
            push_error(
                errors,
                format!("{path}.service.health.consecutive_failures"),
                "must be positive",
            );
        }
        if health.recovery_window_seconds == 0 || health.recovery_window_seconds >= duration_seconds
        {
            push_error(
                errors,
                format!("{path}.service.health.recovery_window_seconds"),
                "must be positive and shorter than the match",
            );
        }
    }
    ids
}

fn validate_agents(
    errors: &mut Vec<ValidationError>,
    agents: &[AgentConfig],
    territory_ids: &HashSet<String>,
) {
    let mut ids = HashSet::new();
    let mut assignments: HashMap<&str, usize> = HashMap::new();
    for (index, agent) in agents.iter().enumerate() {
        let path = format!("agents[{index}]");
        validate_id(errors, &format!("{path}.id"), &agent.id);
        if !ids.insert(agent.id.clone()) {
            push_error(errors, format!("{path}.id"), "must be unique");
        }
        if !territory_ids.contains(&agent.territory) {
            push_error(
                errors,
                format!("{path}.territory"),
                "references an unknown territory",
            );
        }
        *assignments.entry(&agent.territory).or_default() += 1;
        if agent.adapter.trim().is_empty() {
            push_error(errors, format!("{path}.adapter"), "must not be empty");
        }
        if agent.model.trim().is_empty() {
            push_error(errors, format!("{path}.model"), "must not be empty");
        }
        if agent.budget.resource_units == 0 {
            push_error(
                errors,
                format!("{path}.budget.resource_units"),
                "must be positive",
            );
        }
    }
    for territory in territory_ids {
        match assignments
            .get(territory.as_str())
            .copied()
            .unwrap_or_default()
        {
            1 => {}
            0 => push_error(
                errors,
                "agents",
                format!("territory {territory} has no agent"),
            ),
            _ => push_error(
                errors,
                "agents",
                format!("territory {territory} has more than one agent"),
            ),
        }
    }
}

fn validate_id(errors: &mut Vec<ValidationError>, path: &str, value: &str) {
    let valid = !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if !valid {
        push_error(
            errors,
            path,
            "must contain only lowercase ASCII letters, digits, and hyphens",
        );
    }
}

fn valid_private_cidr(value: &str) -> bool {
    value.starts_with("10.")
        || value.starts_with("192.168.")
        || value
            .strip_prefix("172.")
            .and_then(|rest| rest.split('.').next())
            .and_then(|octet| octet.parse::<u8>().ok())
            .is_some_and(|octet| (16..=31).contains(&octet))
}

fn push_error(
    errors: &mut Vec<ValidationError>,
    path: impl Into<String>,
    message: impl Into<String>,
) {
    errors.push(ValidationError {
        path: path.into(),
        message: message.into(),
    });
}
