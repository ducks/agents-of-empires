use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const ANALYSIS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptAnalysis {
    pub schema_version: u32,
    pub analyzer: String,
    pub source_sha256: String,
    pub agent: String,
    pub territory: String,
    pub model: String,
    pub outcome: Option<String>,
    pub metrics: AnalysisMetrics,
    pub architecture: ArchitectureEvidence,
    pub actions: Vec<ObservedAction>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisMetrics {
    pub tool_calls: usize,
    pub tool_errors: usize,
    pub discoveries: usize,
    pub mutations: usize,
    pub lifecycle_actions: usize,
    pub validations: usize,
    pub first_mutation_after_ms: Option<u64>,
    pub last_observed_after_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitectureEvidence {
    pub technologies: Vec<String>,
    pub service_units: Vec<String>,
    pub persistent_paths: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Discovery,
    Mutation,
    Lifecycle,
    Validation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedAction {
    pub index: usize,
    pub tool_id: Option<String>,
    pub kind: ActionKind,
    pub started_after_ms: u64,
    pub duration_ms: u64,
    pub description: Option<String>,
    pub command: String,
    pub success: bool,
    pub output_excerpt: Option<String>,
}

#[derive(Debug, Error)]
pub enum AnalysisError {
    #[error("transcript I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("transcript JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Deserialize)]
struct Transcript {
    #[serde(default)]
    outcome: serde_json::Value,
    #[serde(default)]
    tool_trace: Vec<ToolTrace>,
}

#[derive(Deserialize)]
struct ToolTrace {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    input: serde_json::Value,
    #[serde(default)]
    output: String,
    #[serde(default)]
    is_error: bool,
    #[serde(default)]
    started_after_ms: u64,
    #[serde(default)]
    duration_ms: u64,
}

/// Analyze the observable tool trace in a normalized agent transcript.
///
/// Returns `None` for transcripts without a structured `tool_trace`, allowing
/// reports to remain compatible with other harnesses and historical artifacts.
///
/// # Errors
///
/// Returns an error when the transcript cannot be read or decoded as JSON.
pub fn analyze_transcript(
    path: &Path,
    agent: &str,
    territory: &str,
    model: &str,
) -> Result<Option<TranscriptAnalysis>, AnalysisError> {
    let source = fs::read(path)?;
    let value: serde_json::Value = serde_json::from_slice(&source)?;
    if !value
        .get("tool_trace")
        .is_some_and(serde_json::Value::is_array)
    {
        return Ok(None);
    }
    let transcript: Transcript = serde_json::from_value(value)?;
    if transcript.tool_trace.is_empty() {
        return Ok(None);
    }
    let mut metrics = AnalysisMetrics::default();
    let mut technologies = BTreeSet::new();
    let mut service_units = BTreeSet::new();
    let mut persistent_paths = BTreeSet::new();
    let mut actions = Vec::with_capacity(transcript.tool_trace.len());
    for (index, tool) in transcript.tool_trace.into_iter().enumerate() {
        if is_meta_tool(&tool.name) {
            continue;
        }
        let command = tool_command(&tool);
        let combined = format!("{command}\n{}", tool.output).to_ascii_lowercase();
        let kind = classify(&tool.name, &command);
        metrics.tool_calls += 1;
        metrics.tool_errors += usize::from(tool.is_error);
        match kind {
            ActionKind::Discovery => metrics.discoveries += 1,
            ActionKind::Mutation => {
                metrics.mutations += 1;
                metrics
                    .first_mutation_after_ms
                    .get_or_insert(tool.started_after_ms);
            }
            ActionKind::Lifecycle => {
                metrics.lifecycle_actions += 1;
                metrics
                    .first_mutation_after_ms
                    .get_or_insert(tool.started_after_ms);
            }
            ActionKind::Validation => metrics.validations += 1,
        }
        metrics.last_observed_after_ms = Some(
            metrics
                .last_observed_after_ms
                .unwrap_or(0)
                .max(tool.started_after_ms.saturating_add(tool.duration_ms)),
        );
        collect_architecture(
            &combined,
            &command,
            &mut technologies,
            &mut service_units,
            &mut persistent_paths,
        );
        actions.push(ObservedAction {
            index,
            tool_id: tool.id,
            kind,
            started_after_ms: tool.started_after_ms,
            duration_ms: tool.duration_ms,
            description: tool
                .input
                .get("description")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            command: truncate(&command, 700),
            success: !tool.is_error,
            output_excerpt: (!tool.output.trim().is_empty())
                .then(|| truncate(tool.output.trim(), 280)),
        });
    }
    Ok(Some(TranscriptAnalysis {
        schema_version: ANALYSIS_SCHEMA_VERSION,
        analyzer: format!("aoe-controller/{}", env!("CARGO_PKG_VERSION")),
        source_sha256: format!("{:x}", Sha256::digest(&source)),
        agent: agent.to_owned(),
        territory: territory.to_owned(),
        model: model.to_owned(),
        outcome: transcript
            .outcome
            .get("status")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned),
        metrics,
        architecture: ArchitectureEvidence {
            technologies: technologies.into_iter().collect(),
            service_units: service_units.into_iter().collect(),
            persistent_paths: persistent_paths.into_iter().take(20).collect(),
        },
        actions,
    }))
}

fn tool_command(tool: &ToolTrace) -> String {
    for key in ["command", "cmd", "script"] {
        if let Some(value) = tool.input.get(key).and_then(serde_json::Value::as_str) {
            return value.to_owned();
        }
    }
    if let Some(path) = tool
        .input
        .get("file_path")
        .and_then(serde_json::Value::as_str)
    {
        return format!("{} {path}", tool.name);
    }
    if tool.input.is_null() {
        tool.name.clone()
    } else {
        format!("{} {}", tool.name, tool.input)
    }
}

fn is_meta_tool(name: &str) -> bool {
    matches!(name.to_ascii_lowercase().as_str(), "todowrite" | "plan")
}

fn classify(tool_name: &str, command: &str) -> ActionKind {
    if matches!(
        tool_name.to_ascii_lowercase().as_str(),
        "write" | "edit" | "multiedit" | "notebookedit" | "apply_patch"
    ) {
        return ActionKind::Mutation;
    }
    let value = command.to_ascii_lowercase();
    let lifecycle = [
        "systemctl start",
        "systemctl restart",
        "systemctl enable",
        "systemctl daemon-reload",
        "systemctl reboot",
        " reboot",
        "shutdown",
    ];
    if lifecycle.iter().any(|needle| value.contains(needle)) {
        return ActionKind::Lifecycle;
    }
    let mutations = [
        "cat >",
        "cat <<",
        "tee ",
        "sed -i",
        "install -d",
        "mkdir ",
        "chmod ",
        "chown ",
        "cp ",
        "mv ",
        "rm ",
        "touch ",
        "sqlite3 ",
        "nixos-rebuild",
        "apt install",
        "dnf install",
        "pip install",
        "npm install",
    ];
    if mutations.iter().any(|needle| value.contains(needle)) || value.contains(" > /") {
        return ActionKind::Mutation;
    }
    let validation = [
        "curl ",
        "wget ",
        "systemctl status",
        "systemctl is-active",
        "journalctl",
        "ss ",
        "netstat",
        "pytest",
        "cargo test",
        "npm test",
    ];
    if validation.iter().any(|needle| value.contains(needle)) {
        return ActionKind::Validation;
    }
    ActionKind::Discovery
}

fn collect_architecture(
    combined: &str,
    command: &str,
    technologies: &mut BTreeSet<String>,
    service_units: &mut BTreeSet<String>,
    persistent_paths: &mut BTreeSet<String>,
) {
    for (needles, label) in [
        (&["python", ".py"][..], "Python"),
        (&["sqlite"][..], "SQLite"),
        (&["postgres"][..], "PostgreSQL"),
        (&["redis"][..], "Redis"),
        (&["nginx"][..], "Nginx"),
        (
            &["node_modules", "package.json", "npm ", "/bin/node"][..],
            "Node.js",
        ),
        (&["systemctl", ".service"][..], "systemd"),
        (&["gunicorn"][..], "Gunicorn"),
        (&["ruby ", "gemfile", "bundle exec", ".rb"][..], "Ruby"),
        (&["rails ", "rails/", "activerecord"][..], "Rails"),
        (
            &["cargo.toml", "cargo build", "cargo run", "rustc --", ".rs"][..],
            "Rust",
        ),
    ] {
        if needles.iter().any(|needle| combined.contains(needle)) {
            technologies.insert(label.to_owned());
        }
    }
    for token in command.split(|character: char| {
        character.is_whitespace() || matches!(character, ';' | '|' | '(' | ')' | '<' | '>')
    }) {
        let token = token.trim_matches(['\'', '"', ',', ':', '=']);
        if token.ends_with(".service") && token.len() < 100 {
            service_units.insert(token.trim_start_matches('/').to_owned());
        }
        if ["/etc/", "/opt/", "/srv/", "/var/lib/"]
            .iter()
            .any(|prefix| token.starts_with(prefix))
            && token.len() < 180
        {
            persistent_paths.insert(token.trim_end_matches(['/', '*']).to_owned());
        }
    }
}

fn truncate(value: &str, limit: usize) -> String {
    let mut result: String = value.chars().take(limit).collect();
    if value.chars().count() > limit {
        result.push('…');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{ActionKind, analyze_transcript};

    #[test]
    fn extracts_actions_and_architecture_without_reasoning_text() {
        let root = std::env::temp_dir().join(format!("aoe-analysis-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("temp dir");
        let path = root.join("transcript.json");
        std::fs::write(
            &path,
            r#"{"messages":[{"role":"assistant","content":"private reasoning"}],"model":"m","outcome":{"status":"completed"},"tool_trace":[{"id":"one","name":"Bash","input":{"command":"find /var/lib/accepted-jobs -type f","description":"inspect spool"},"output":"/var/lib/accepted-jobs/a.json","is_error":false,"started_after_ms":100,"duration_ms":5},{"id":"two","name":"Bash","input":{"command":"cat > /opt/app.py <<'PY'\nimport sqlite3\nPY","description":"write app"},"output":"","is_error":false,"started_after_ms":200,"duration_ms":7},{"id":"three","name":"Bash","input":{"command":"systemctl restart job-worker.service"},"output":"","is_error":false,"started_after_ms":300,"duration_ms":9},{"id":"four","name":"Bash","input":{"command":"curl -fsS http://localhost:8080/health"},"output":"ready","is_error":false,"started_after_ms":400,"duration_ms":11}]}"#,
        )
        .expect("transcript");
        let analysis = analyze_transcript(&path, "agent", "territory", "model")
            .expect("analysis")
            .expect("supported transcript");
        assert_eq!(analysis.metrics.first_mutation_after_ms, Some(200));
        assert_eq!(analysis.metrics.validations, 1);
        assert_eq!(analysis.actions[1].kind, ActionKind::Mutation);
        assert!(
            analysis
                .architecture
                .technologies
                .contains(&"SQLite".into())
        );
        assert!(
            analysis
                .architecture
                .service_units
                .contains(&"job-worker.service".into())
        );
        assert!(
            !serde_json::to_string(&analysis)
                .expect("JSON")
                .contains("private reasoning")
        );
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn normalizes_file_tools_and_ignores_planning_tools() {
        let root = std::env::temp_dir().join(format!("aoe-analysis-tools-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("temp dir");
        let path = root.join("transcript.json");
        std::fs::write(
            &path,
            r#"{"tool_trace":[{"name":"TodoWrite","input":{"todos":[{"content":"install nginx"}]}},{"name":"Write","input":{"file_path":"/etc/app.conf","content":"secret-sized body"},"started_after_ms":25}]}"#,
        )
        .expect("transcript");
        let analysis = analyze_transcript(&path, "agent", "territory", "model")
            .expect("analysis")
            .expect("supported transcript");
        assert_eq!(analysis.metrics.tool_calls, 1);
        assert_eq!(analysis.metrics.mutations, 1);
        assert_eq!(analysis.metrics.first_mutation_after_ms, Some(25));
        assert_eq!(analysis.actions[0].command, "Write /etc/app.conf");
        assert!(!analysis.actions[0].command.contains("secret-sized body"));
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}
