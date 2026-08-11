//! Append-only event storage and deterministic world-state replay.

mod log;
mod reducer;
mod snapshot;

pub use log::{EventLog, EventLogError, load_events};
pub use reducer::{AgentView, HealthView, WorldState, reduce, replay};
pub use snapshot::{Snapshot, SnapshotError, load_snapshot, write_snapshot};
