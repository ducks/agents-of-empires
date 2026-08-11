//! Disposable VM lifecycle and isolated arena networking.

mod network;
mod nix;
mod supervisor;

pub use network::{NetworkAssignment, NetworkPlan, NetworkPlanError};
pub use nix::NixVmDriver;
pub use supervisor::{ArenaSupervisor, RuntimeError, TerritoryDriver, TerritoryHandle};
