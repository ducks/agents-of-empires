//! External health observation and deterministic game rules.

mod build;
mod probe;
mod rules;

pub use build::{BuildOutcome, BuildReferee, BuildStanding};
pub use probe::{HealthObservation, HealthProbe, HttpProbe, ProbeTarget};
pub use rules::{MatchOutcome, Referee, RefereeError, Standing};
