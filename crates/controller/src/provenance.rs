use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use aoe_domain::ArenaManifest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const MATCH_ARTIFACT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchProvenance {
    pub schema_version: u32,
    pub controller_version: String,
    pub source_revision: Option<String>,
    pub arena_id: String,
    pub arena_mode: String,
    pub manifest_sha256: String,
    pub verifier_sha256: String,
    pub adapter_sha256: BTreeMap<String, String>,
    pub compatibility_key: String,
}

#[derive(Debug, Error)]
pub enum ProvenanceError {
    #[error("provenance I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("provenance JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn write_provenance(
    manifest_path: &Path,
    manifest: &ArenaManifest,
    adapters: &HashMap<String, PathBuf>,
    output: &Path,
) -> Result<MatchProvenance, ProvenanceError> {
    let manifest_source = fs::read(manifest_path)?;
    let verifier_sha256 = verifier_digest(manifest_path, manifest)?;
    let manifest_sha256 = digest(&manifest_source);
    let compatibility_key = digest(
        format!(
            "{}\n{}\n{}\n{}",
            MATCH_ARTIFACT_VERSION, manifest.arena.id, manifest_sha256, verifier_sha256
        )
        .as_bytes(),
    );
    let provenance = MatchProvenance {
        schema_version: MATCH_ARTIFACT_VERSION,
        controller_version: env!("CARGO_PKG_VERSION").to_owned(),
        source_revision: source_revision(),
        arena_id: manifest.arena.id.clone(),
        arena_mode: format!("{:?}", manifest.arena.mode).to_lowercase(),
        manifest_sha256,
        verifier_sha256,
        adapter_sha256: adapter_digests(adapters)?,
        compatibility_key,
    };
    fs::write(
        output.join("match.json"),
        serde_json::to_vec_pretty(&provenance)?,
    )?;
    Ok(provenance)
}

fn adapter_digests(
    adapters: &HashMap<String, PathBuf>,
) -> Result<BTreeMap<String, String>, std::io::Error> {
    adapters
        .iter()
        .map(|(name, path)| Ok((name.clone(), digest(&fs::read(path)?))))
        .collect()
}

pub fn read_provenance(path: &Path) -> Result<MatchProvenance, ProvenanceError> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn verifier_digest(
    manifest_path: &Path,
    manifest: &ArenaManifest,
) -> Result<String, std::io::Error> {
    let root = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let mut files: Vec<PathBuf> = manifest
        .build
        .iter()
        .flat_map(|build| &build.milestones)
        .map(|milestone| root.join(&milestone.verifier))
        .collect();
    files.sort();
    let mut hasher = Sha256::new();
    for path in files {
        hasher.update(path.to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update(fs::read(path)?);
        hasher.update([0]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn source_revision() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
