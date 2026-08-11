use std::env;
use std::path::{Path, PathBuf};

use aoe_domain::{ArenaManifest, EventEnvelope};
use aoe_replay::{load_events, replay};
use aoe_tui::{RenderOptions, event_summary, render_world};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CommandError {
    #[error("manifest error: {0}")]
    Manifest(#[from] aoe_domain::ManifestError),
    #[error("event log error: {0}")]
    Log(#[from] aoe_replay::EventLogError),
    #[error("JSON encoding failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("event sequence {0} does not exist")]
    MissingSequence(u64),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValidationReport {
    pub valid: bool,
    pub arena: String,
    pub territories: usize,
    pub agents: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    pub name: String,
    pub available: bool,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorReport {
    pub ready: bool,
    pub checks: Vec<Check>,
}

/// Validate one arena and its controller-visible local references.
///
/// # Errors
///
/// Returns an error if the manifest cannot be loaded.
pub fn validate(path: &Path) -> Result<ValidationReport, CommandError> {
    let manifest = ArenaManifest::load(path)?;
    let mut warnings = Vec::new();
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    for territory in &manifest.territories {
        let local = territory
            .nixos_config
            .split_once('#')
            .map_or(territory.nixos_config.as_str(), |(path, _)| path);
        let local = Path::new(local);
        let candidate = if local.is_absolute() {
            local.to_owned()
        } else {
            cwd.join(local)
        };
        if !candidate.exists() {
            warnings.push(format!(
                "{} Nix flake path does not exist: {}",
                territory.id,
                candidate.display()
            ));
        }
    }
    Ok(ValidationReport {
        valid: warnings.is_empty(),
        arena: manifest.arena.id,
        territories: manifest.territories.len(),
        agents: manifest.agents.len(),
        warnings,
    })
}

/// Render a stored event log as JSON state or terminal text.
///
/// # Errors
///
/// Returns an error if the log is invalid or JSON cannot be encoded.
pub fn replay_log(path: &Path, json: bool, options: RenderOptions) -> Result<String, CommandError> {
    let events = load_events(path)?;
    let state = replay(&events);
    if json {
        Ok(serde_json::to_string_pretty(&state)?)
    } else {
        Ok(render_world(&state, &events, options))
    }
}

/// Inspect one immutable event by sequence.
///
/// # Errors
///
/// Returns an error if the log is invalid, the sequence is absent, or JSON
/// cannot be encoded.
pub fn inspect(path: &Path, sequence: u64, json: bool) -> Result<String, CommandError> {
    let events = load_events(path)?;
    let event = events
        .iter()
        .find(|event| event.sequence == sequence)
        .ok_or(CommandError::MissingSequence(sequence))?;
    if json {
        Ok(serde_json::to_string_pretty(event)?)
    } else {
        Ok(format_event(event))
    }
}

fn format_event(event: &EventEnvelope) -> String {
    format!(
        "sequence: {}\nelapsed_ms: {}\nevent: {}\n",
        event.sequence,
        event.elapsed_ms,
        event_summary(&event.event)
    )
}

#[must_use]
pub fn doctor() -> DoctorReport {
    let checks = ["nix", "qemu-system-x86_64", "ssh"]
        .into_iter()
        .map(|name| {
            let path = find_in_path(name);
            Check {
                name: name.to_owned(),
                available: path.is_some(),
                detail: path.map_or_else(
                    || "not found in PATH".into(),
                    |value| value.display().to_string(),
                ),
            }
        })
        .collect::<Vec<_>>();
    DoctorReport {
        ready: checks.iter().all(|check| check.available),
        checks,
    }
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|path| {
        env::split_paths(&path)
            .map(|directory| directory.join(name))
            .find(|candidate| candidate.is_file())
    })
}
