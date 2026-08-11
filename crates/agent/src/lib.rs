//! Harness-independent concurrent agent execution.

mod command;
mod controller;
mod protocol;

pub use command::{CommandAdapter, write_result};
pub use controller::{AgentAdapter, AgentController, AgentControllerError};
pub use protocol::{AgentInvocation, AgentResult, AgentStatus, AgentUsage};
