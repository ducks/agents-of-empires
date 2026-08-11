//! External health observation and deterministic game rules.

mod probe;
mod rules;

pub use probe::{HealthObservation, HealthProbe, HttpProbe, ProbeTarget};
pub use rules::{MatchOutcome, Referee, RefereeError, Standing};
