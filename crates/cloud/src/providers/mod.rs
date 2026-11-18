//! Sandbox infrastructure. Guest RPC owns interactive tools and agent execution.
pub mod artifact;
pub mod boat;
#[cfg(test)]
mod boat_tests;
pub mod boxd;
mod boxd_contract;
#[cfg(test)]
mod boxd_tests;
pub(crate) mod boxd_wire;
pub mod contract;
pub mod daytona;
pub mod e2b;
pub mod prized;
pub mod registry;
mod transport;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderError {
    NotFound,
    Rejected,
    Conflict,
    RateLimited,
    Unavailable,
    /// A mutation may have been accepted. Reconcile its identity before retrying.
    OutcomeUnknown,
    InvalidResponse,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MachineState {
    Running,
    /// Suspended in memory; a different operation from waking a disk hibernation.
    Standby,
    Paused,
    Stopped,
    Starting,
    Unknown(String),
}

#[derive(Clone, Debug)]
pub struct Machine {
    pub id: String,
    pub name: String,
    pub state: MachineState,
    pub vcpu: Option<u32>,
    pub memory_bytes: Option<u64>,
}

/// One machine in a provider listing, compared against the database by the
/// provider reconciliation pass. A listing never proves that a machine is absent.
#[derive(Clone, Debug)]
pub struct ListedMachine {
    pub id: String,
    pub name: String,
    pub state: MachineState,
    pub created_at_ms: Option<i64>,
}

/// One machine's provider usage over a closed window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineUsage {
    pub machine_id: String,
    pub running_seconds: u64,
    pub cost_micros: i64,
}

pub struct ExecResult {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: i32,
}
