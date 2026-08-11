use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::WorldState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema_version: u32,
    pub through_sequence: u64,
    pub state: WorldState,
}

#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error("snapshot I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("snapshot JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported snapshot schema version {0}")]
    Schema(u32),
}

/// Atomically write an optimization snapshot. The event log remains authority.
///
/// # Errors
///
/// Returns an error when serialization or atomic replacement fails.
pub fn write_snapshot(path: impl AsRef<Path>, snapshot: &Snapshot) -> Result<(), SnapshotError> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, serde_json::to_vec(snapshot)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}

/// Read and validate a snapshot.
///
/// # Errors
///
/// Returns an error for I/O, malformed JSON, or an unsupported schema.
pub fn load_snapshot(path: impl AsRef<Path>) -> Result<Snapshot, SnapshotError> {
    let snapshot: Snapshot = serde_json::from_slice(&fs::read(path)?)?;
    if snapshot.schema_version != 1 {
        return Err(SnapshotError::Schema(snapshot.schema_version));
    }
    Ok(snapshot)
}
