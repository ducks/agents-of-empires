use aoe_domain::{ArenaManifest, Event, EventEnvelope, ManifestError, MatchMode, MatchState};

const VALID: &str = r#"
schema_version = 1

[arena]
id = "first-contact"
display_name = "First Contact"

[network]
cidr = "10.77.0.0/24"
allow_public_internet = false

[rules]
duration_seconds = 900
tick_ms = 1000
healthy_resources_per_tick = 1
minimum_territories = 2

[[classes]]
id = "gatekeeper"
display_name = "Gatekeeper"
strengths = ["routing"]
weaknesses = ["small-state"]
[classes.resources]
vcpus = 1
memory_mib = 512
disk_mib = 1024

[[classes]]
id = "archivist"
display_name = "Archivist"
[classes.resources]
vcpus = 1
memory_mib = 512
disk_mib = 1024

[[territories]]
id = "north-gate"
class = "gatekeeper"
nixos_config = "gatekeeper.nix"
[territories.service]
port = 8080
path = "/health"
expected_status = 200
expected_body = "ok"
[territories.service.health]
poll_interval_ms = 1000
consecutive_failures = 3
recovery_window_seconds = 60

[[territories]]
id = "archive"
class = "archivist"
nixos_config = "archivist.nix"
[territories.service]
port = 8081
path = "/health"
expected_status = 200
expected_body = "ok"
[territories.service.health]
poll_interval_ms = 1000
consecutive_failures = 3
recovery_window_seconds = 60

[[agents]]
id = "agent-gate"
territory = "north-gate"
adapter = "claux"
model = "example/gate"
reasoning_effort = "high"
[agents.budget]
resource_units = 100
max_cost_microusd = 500000

[[agents]]
id = "agent-archive"
territory = "archive"
adapter = "codex"
model = "example/archive"
[agents.budget]
resource_units = 100
"#;

#[test]
fn parses_valid_manifest() {
    let manifest = ArenaManifest::parse(VALID).expect("manifest should be valid");
    assert_eq!(manifest.territories.len(), 2);
    assert_eq!(manifest.agents[1].reasoning_effort, "default");
}

#[test]
fn rejects_unknown_fields() {
    let source = VALID.replace(
        "display_name = \"First Contact\"",
        "display_name = \"First Contact\"\ncheat = true",
    );
    assert!(matches!(
        ArenaManifest::parse(&source),
        Err(ManifestError::Parse(_))
    ));
}

#[test]
fn collects_semantic_errors() {
    let source = VALID
        .replace(
            "allow_public_internet = false",
            "allow_public_internet = true",
        )
        .replace("class = \"archivist\"", "class = \"missing\"")
        .replace("resource_units = 100", "resource_units = 0");
    let Err(ManifestError::Validation(errors)) = ArenaManifest::parse(&source) else {
        panic!("expected validation errors");
    };
    let paths: Vec<_> = errors.iter().map(|error| error.path.as_str()).collect();
    assert!(paths.contains(&"network.allow_public_internet"));
    assert!(paths.contains(&"territories[1].class"));
    assert_eq!(
        paths
            .iter()
            .filter(|path| **path == "agents[0].budget.resource_units")
            .count(),
        1
    );
    assert_eq!(
        paths
            .iter()
            .filter(|path| **path == "agents[1].budget.resource_units")
            .count(),
        1
    );
}

#[test]
fn rejects_duplicate_agent_assignment() {
    let duplicate = r#"
[[agents]]
id = "agent-extra"
territory = "archive"
adapter = "fake"
model = "fake/model"
[agents.budget]
resource_units = 1
"#;
    let source = format!("{VALID}\n{duplicate}");
    let Err(ManifestError::Validation(errors)) = ArenaManifest::parse(&source) else {
        panic!("expected validation errors");
    };
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("more than one agent"))
    );
}

#[test]
fn events_round_trip_as_tagged_json() {
    let envelope = EventEnvelope {
        schema_version: 1,
        sequence: 7,
        elapsed_ms: 42,
        event: Event::MatchStateChanged {
            from: MatchState::Preparing,
            to: MatchState::Running,
        },
    };
    let json = serde_json::to_string(&envelope).expect("serialize");
    assert!(json.contains("match_state_changed"));
    let decoded: EventEnvelope = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded, envelope);
}

#[test]
fn parses_build_race_milestone_dag() {
    let build = r#"
mode = "build_race"

[build]
stop_on_first_durable = true
completion_milestone = "reboot"

[[build.milestones]]
id = "service-up"
display_name = "Service Up"
verifier = "verify/service-up.sh"
timeout_seconds = 30
points = 10

[[build.milestones]]
id = "reboot"
display_name = "Host Reboot"
depends_on = ["service-up"]
verifier = "verify/reboot.sh"
timeout_seconds = 120
points = 50
"#;
    let source = VALID.replace(
        "display_name = \"First Contact\"",
        &format!("display_name = \"First Contact\"\n{build}"),
    );
    let manifest = ArenaManifest::parse(&source).expect("build manifest");
    assert_eq!(manifest.arena.mode, MatchMode::BuildRace);
    assert_eq!(manifest.build.expect("contract").milestones.len(), 2);
}

#[test]
fn parses_and_validates_arena_visualization() {
    let build = r#"
mode = "build_race"

[build]
stop_on_first_durable = true
completion_milestone = "service-up"

[[build.milestones]]
id = "service-up"
display_name = "Service Up"
verifier = "verify/service-up.sh"
timeout_seconds = 30
points = 10

[visualization]

[[visualization.nodes]]
id = "client"
display_name = "Client"
kind = "client"
x = 10
y = 50

[[visualization.nodes]]
id = "app"
display_name = "Application"
kind = "service"
milestone = "service-up"
x = 70
y = 50

[[visualization.links]]
from = "client"
to = "app"
kind = "traffic"
label = "HTTP"
"#;
    let source = VALID.replace(
        "display_name = \"First Contact\"",
        &format!("display_name = \"First Contact\"\n{build}"),
    );
    let manifest = ArenaManifest::parse(&source).expect("visualized build manifest");
    let visualization = manifest.visualization.expect("visualization");
    assert_eq!(visualization.nodes.len(), 2);
    assert_eq!(visualization.links.len(), 1);
}

#[test]
fn rejects_invalid_arena_visualization() {
    let build = r#"
mode = "build_race"

[build]
stop_on_first_durable = true
completion_milestone = "service-up"

[[build.milestones]]
id = "service-up"
display_name = "Service Up"
verifier = "verify/service-up.sh"
timeout_seconds = 30
points = 10

[visualization]

[[visualization.nodes]]
id = "app"
display_name = "Application"
kind = "service"
milestone = "missing"
x = 101
y = 50

[[visualization.links]]
from = "app"
to = "missing"
"#;
    let source = VALID.replace(
        "display_name = \"First Contact\"",
        &format!("display_name = \"First Contact\"\n{build}"),
    );
    let Err(ManifestError::Validation(errors)) = ArenaManifest::parse(&source) else {
        panic!("expected validation errors");
    };
    let paths: Vec<_> = errors.iter().map(|error| error.path.as_str()).collect();
    assert!(paths.contains(&"visualization.nodes[0].milestone"));
    assert!(paths.contains(&"visualization.nodes[0].x"));
    assert!(paths.contains(&"visualization.links[0].to"));
}

#[test]
fn rejects_build_race_without_contract() {
    let source = VALID.replace(
        "display_name = \"First Contact\"",
        "display_name = \"First Contact\"\nmode = \"build_race\"",
    );
    let Err(ManifestError::Validation(errors)) = ArenaManifest::parse(&source) else {
        panic!("expected validation errors");
    };
    assert!(errors.iter().any(|error| error.path == "build"));
}

#[test]
fn rejects_cyclic_milestones() {
    let build = r#"
mode = "build_race"

[build]
stop_on_first_durable = true
completion_milestone = "one"

[[build.milestones]]
id = "one"
display_name = "One"
depends_on = ["two"]
verifier = "verify/one.sh"
timeout_seconds = 30
points = 10

[[build.milestones]]
id = "two"
display_name = "Two"
depends_on = ["one"]
verifier = "verify/two.sh"
timeout_seconds = 30
points = 10
"#;
    let source = VALID.replace(
        "display_name = \"First Contact\"",
        &format!("display_name = \"First Contact\"\n{build}"),
    );
    let Err(ManifestError::Validation(errors)) = ArenaManifest::parse(&source) else {
        panic!("expected validation errors");
    };
    assert!(errors.iter().any(|error| error.message.contains("cycle")));
}
