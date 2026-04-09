//! codex-supervisor: Deterministic supervisor loop for multi-agent orchestration.
//!
//! The supervisor loop drives tasks to completion. No LLM controls the loop — the code does.
//! LLMs provide judgment (planning, evaluation, verification interpretation).
//!
//! See docs/spec/design-principles.md and docs/spec/integration-model.md.

pub mod supervisor_loop;
pub mod task_graph;

pub use supervisor_loop::{run_supervisor, DispatchResult, SupervisorConfig, SupervisorJudge, SupervisorResult, TerminationReason};
pub use task_graph::{Task, TaskGraph, TaskStatus};
