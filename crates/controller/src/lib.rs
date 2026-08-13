pub mod cli;
pub mod commands;
pub mod provenance;
pub mod report;
pub mod runner;

pub use cli::{Cli, Command, ParseError};
pub use commands::{DoctorReport, ValidationReport, doctor, inspect, replay_log, validate};
pub use provenance::{MatchProvenance, read_provenance, write_provenance};
pub use report::{ReportError, ReportSummary, generate_reports};
pub use runner::{RunOptions, run_match};
