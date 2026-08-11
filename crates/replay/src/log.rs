use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use aoe_domain::{Event, EventEnvelope};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EventLogError {
    #[error("event log I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("event log contains invalid JSON at line {line}: {detail}")]
    Json { line: usize, detail: String },
    #[error("unsupported event schema version {0}")]
    Schema(u32),
    #[error("event sequence mismatch: expected {expected}, found {actual}")]
    Sequence { expected: u64, actual: u64 },
}

/// Append-only JSONL writer with sequence validation.
pub struct EventLog {
    path: PathBuf,
    writer: BufWriter<File>,
    next_sequence: u64,
}

impl EventLog {
    /// Open or create an event log after validating its existing contents.
    ///
    /// # Errors
    ///
    /// Returns an error for I/O, malformed existing events, unsupported schema,
    /// or a broken sequence.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, EventLogError> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let existing = load_events(&path)?;
        let next_sequence = existing
            .last()
            .map_or(0, |event| event.sequence.saturating_add(1));
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            path,
            writer: BufWriter::new(file),
            next_sequence,
        })
    }

    /// Append one event. Lifecycle and outcome events are synchronized to disk.
    ///
    /// # Errors
    ///
    /// Returns an error for a bad schema, unexpected sequence, serialization,
    /// or write failure.
    pub fn append(&mut self, event: &EventEnvelope) -> Result<(), EventLogError> {
        validate_event(event, self.next_sequence)?;
        serde_json::to_writer(&mut self.writer, event).map_err(|error| EventLogError::Json {
            line: usize::try_from(self.next_sequence.saturating_add(1)).unwrap_or(usize::MAX),
            detail: error.to_string(),
        })?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        if durable(&event.event) {
            self.writer.get_ref().sync_data()?;
        }
        self.next_sequence = self.next_sequence.saturating_add(1);
        Ok(())
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Load a complete event log. An invalid, unterminated final line is treated as
/// an interrupted append and ignored.
///
/// # Errors
///
/// Returns an error for I/O, invalid complete lines, unsupported schemas, or
/// non-contiguous sequences.
pub fn load_events(path: impl AsRef<Path>) -> Result<Vec<EventEnvelope>, EventLogError> {
    let path = path.as_ref();
    let source = match fs::read(path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let terminated = source.ends_with(b"\n");
    let lines: Vec<_> = source.split(|byte| *byte == b'\n').collect();
    let mut events = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let parsed = serde_json::from_slice::<EventEnvelope>(line);
        let event = match parsed {
            Ok(event) => event,
            Err(_) if index == lines.len().saturating_sub(1) && !terminated => break,
            Err(error) => {
                return Err(EventLogError::Json {
                    line: index + 1,
                    detail: error.to_string(),
                });
            }
        };
        let expected = u64::try_from(events.len()).unwrap_or(u64::MAX);
        validate_event(&event, expected)?;
        events.push(event);
    }
    Ok(events)
}

fn validate_event(event: &EventEnvelope, expected: u64) -> Result<(), EventLogError> {
    if event.schema_version != 1 {
        return Err(EventLogError::Schema(event.schema_version));
    }
    if event.sequence != expected {
        return Err(EventLogError::Sequence {
            expected,
            actual: event.sequence,
        });
    }
    Ok(())
}

fn durable(event: &Event) -> bool {
    matches!(
        event,
        Event::MatchStateChanged { .. }
            | Event::TerritoryStateChanged { .. }
            | Event::TerritoryEliminated { .. }
            | Event::InfrastructureFailure { .. }
            | Event::MatchFinished { .. }
    )
}
