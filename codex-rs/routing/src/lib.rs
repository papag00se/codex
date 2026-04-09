//! codex-routing: Task routing engine for multi-agent orchestration.
//!
//! Ported from coding-agent-router (Python). Reference docs:
//! - docs/spec/routing-logic-reference.md
//! - docs/spec/design-principles.md
//!
//! This crate provides:
//! - Task metrics extraction (27 regex-based features)
//! - Route selection algorithm (context filtering → single-eligible fast path → LLM selection → fallback)
//! - Ollama HTTP client with per-endpoint serialization
//! - Routing configuration

pub mod classifier;
pub mod config;
pub mod engine;
pub mod local_dispatch;
pub mod metrics;
pub mod ollama;
pub mod project_config;
pub mod tool_format;
pub mod tool_recovery;
pub mod usage;

pub use classifier::{classify_request, ClassifyResult, RouteTarget};
pub use config::RoutingConfig;
pub use engine::{route_task, RouteDecision};
pub use metrics::{estimate_tokens, extract_task_metrics, TaskMetrics};
pub use ollama::OllamaClientPool;
pub use local_dispatch::{call_ollama_text, OllamaTextResponse};
pub use tool_recovery::{recover_tool_calls, recover_tool_calls_streaming, RecoveredMessage, ToolCall};
