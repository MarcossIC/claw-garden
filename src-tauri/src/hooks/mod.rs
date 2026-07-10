pub mod compiled;
pub mod error;
pub mod matcher;
pub mod permission;
pub mod runner;
pub mod types;

pub use compiled::{CompiledAgentHookConfig, CompiledHookCommand};
pub use error::ConfigError;
pub use matcher::HookRule;
pub use permission::{PermissionRequest, UserDecision};
pub use runner::{merge_input, HookAbortSignal, HookRunner};
pub use types::{
    AgentContext, AgentHookConfig, FinalDecision, GardenEvent, HookCommand, HookEvent, HookPhase,
    HookRunResult, OnTimeoutPolicy, PermissionDecision, PermissionResult, StderrDestination,
    StderrEntry, StderrLevel,
};
