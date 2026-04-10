//! Project-level configuration for multi-agent routing.
//!
//! Loaded from `.codex-multi/config.toml` in the working directory.
//! Separate from `~/.codex/config.toml` — does not affect the base Codex config.
//!
//! See docs/spec/model-routing-strategy.md.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// A single model endpoint entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    pub provider: String,
    #[serde(default)]
    pub endpoint: Option<String>,
    pub model: String,
    #[serde(default = "default_weight")]
    pub weight: u32,
    #[serde(default = "default_reasoning")]
    pub reasoning: String,
    #[serde(default)]
    pub num_ctx: Option<usize>,
}

fn default_weight() -> u32 {
    100
}
fn default_reasoning() -> String {
    "off".into()
}

/// A model role — may have a single entry or weighted distribution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ModelRole {
    Single {
        provider: String,
        #[serde(default)]
        endpoint: Option<String>,
        model: String,
        #[serde(default = "default_reasoning")]
        reasoning: String,
        #[serde(default)]
        num_ctx: Option<usize>,
    },
    Weighted {
        entries: Vec<ModelEntry>,
    },
}

/// Routing behavior configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingBehavior {
    #[serde(default = "default_strategy")]
    pub strategy: String,
    #[serde(default)]
    pub compaction_model: Option<String>,
}

impl Default for RoutingBehavior {
    fn default() -> Self {
        Self {
            strategy: default_strategy(),
            compaction_model: None,
        }
    }
}

fn default_strategy() -> String {
    "cost_first".into()
}

/// Supervisor behavior configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupervisorBehavior {
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    #[serde(default = "default_max_retries")]
    pub max_retries_per_task: u32,
    #[serde(default)]
    pub verification_command: Option<String>,
}

fn default_max_iterations() -> u32 {
    50
}
fn default_timeout() -> u64 {
    7200
}
fn default_max_retries() -> u32 {
    3
}

impl Default for SupervisorBehavior {
    fn default() -> Self {
        Self {
            max_iterations: default_max_iterations(),
            timeout_seconds: default_timeout(),
            max_retries_per_task: default_max_retries(),
            verification_command: None,
        }
    }
}

/// Failover chain configuration.
/// Failover chain configuration — defines escalation paths per task type.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FailoverChains {
    #[serde(default)]
    pub reasoning: Vec<String>,
    #[serde(default)]
    pub coding: Vec<String>,
    #[serde(default)]
    pub classification: Vec<String>,
    #[serde(default)]
    pub compaction: Vec<String>,
    #[serde(default)]
    pub review: Vec<String>,
    #[serde(default)]
    pub planning: Vec<String>,
    #[serde(default)]
    pub evaluation: Vec<String>,

    /// Controls how failures are handled before walking the chains.
    #[serde(default)]
    pub behavior: FailoverBehavior,
}

/// Failover behavior — controls retry and escalation parameters.
///
/// Failure types:
/// F1 (rate limit): retry same model with backoff, then walk chain
/// F2 (quota exhausted): walk chain immediately
/// F3 (model unavailable): walk chain immediately
/// F4 (model not found): walk chain immediately, log config error
/// F5 (auth failure): hard-fail, don't retry
/// F6 (timeout): retry same model once, then walk chain
/// F7 (quality failure): walk chain immediately (same model = same garbage)
/// F8 (context overflow): walk chain (need larger context model)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailoverBehavior {
    /// F1 + F6: how many times to retry the same model before walking chain
    #[serde(default = "default_fo_retry_attempts")]
    pub retry_same_attempts: u32,

    /// Backoff between retries of the same model (ms)
    #[serde(default = "default_fo_backoff")]
    pub retry_same_backoff_ms: u64,

    /// F1: if no retry-after header, wait this long (ms)
    #[serde(default = "default_fo_rate_limit_wait")]
    pub rate_limit_default_wait_ms: u64,

    /// F1: maximum wait for a rate limit, even if retry-after says longer
    #[serde(default = "default_fo_rate_limit_max")]
    pub rate_limit_max_wait_ms: u64,

    /// F6: request timeout (ms)
    #[serde(default = "default_fo_timeout")]
    pub timeout_ms: u64,
}

fn default_fo_retry_attempts() -> u32 { 2 }
fn default_fo_backoff() -> u64 { 1000 }
fn default_fo_rate_limit_wait() -> u64 { 5000 }
fn default_fo_rate_limit_max() -> u64 { 30000 }
fn default_fo_timeout() -> u64 { 30000 }

impl Default for FailoverBehavior {
    fn default() -> Self {
        Self {
            retry_same_attempts: default_fo_retry_attempts(),
            retry_same_backoff_ms: default_fo_backoff(),
            rate_limit_default_wait_ms: default_fo_rate_limit_wait(),
            rate_limit_max_wait_ms: default_fo_rate_limit_max(),
            timeout_ms: default_fo_timeout(),
        }
    }
}

/// Usage preservation configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageConfig {
    #[serde(default = "default_warn_threshold")]
    pub primary_warn_threshold: f64,
    #[serde(default = "default_true")]
    pub prefer_secondary: bool,
}

fn default_warn_threshold() -> f64 {
    0.7
}
fn default_true() -> bool {
    true
}

impl Default for UsageConfig {
    fn default() -> Self {
        Self {
            primary_warn_threshold: default_warn_threshold(),
            prefer_secondary: true,
        }
    }
}

/// Agent role configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRole {
    pub nickname: String,
    pub instructions: String,
}

/// The full project-level configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectConfig {
    #[serde(default)]
    pub models: std::collections::HashMap<String, ModelRole>,
    #[serde(default)]
    pub roles: std::collections::HashMap<String, AgentRole>,
    #[serde(default)]
    pub routing: RoutingBehavior,
    #[serde(default)]
    pub supervisor: SupervisorBehavior,
    #[serde(default)]
    pub failover: FailoverChains,
    #[serde(default)]
    pub usage: UsageConfig,
}

impl ProjectConfig {
    /// Load from `.codex-multi/config.toml` in the given directory.
    /// Returns default config if the file doesn't exist.
    pub fn load(dir: &Path) -> Self {
        let config_path = dir.join(".codex-multi").join("config.toml");
        if !config_path.exists() {
            return Self::default();
        }

        match std::fs::read_to_string(&config_path) {
            Ok(content) => match toml::from_str::<ProjectConfig>(&content) {
                Ok(config) => {
                    info!(path = %config_path.display(), "Loaded project config");
                    config
                }
                Err(e) => {
                    warn!(
                        path = %config_path.display(),
                        error = %e,
                        "Failed to parse project config, using defaults"
                    );
                    Self::default()
                }
            },
            Err(e) => {
                warn!(
                    path = %config_path.display(),
                    error = %e,
                    "Failed to read project config, using defaults"
                );
                Self::default()
            }
        }
    }

    /// Get a model role by name.
    pub fn get_model(&self, name: &str) -> Option<&ModelRole> {
        self.models.get(name)
    }

    /// Get the failover chain for a task type.
    pub fn failover_chain(&self, task_type: &str) -> &[String] {
        match task_type {
            "reasoning" => &self.failover.reasoning,
            "coding" => &self.failover.coding,
            "classification" => &self.failover.classification,
            "compaction" => &self.failover.compaction,
            "review" => &self.failover.review,
            "planning" => &self.failover.planning,
            "evaluation" => &self.failover.evaluation,
            _ => &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ProjectConfig::default();
        assert_eq!(config.supervisor.max_iterations, 50);
        assert_eq!(config.supervisor.timeout_seconds, 7200);
        assert_eq!(config.routing.strategy, "cost_first");
        assert!(config.usage.prefer_secondary);
    }

    #[test]
    fn test_parse_single_model() {
        let toml = r#"
[models.classifier]
provider = "ollama"
endpoint = "http://localhost:11434"
model = "qwen3.5-9b:iq4_xs"
reasoning = "off"
"#;
        let config: ProjectConfig = toml::from_str(toml).unwrap();
        assert!(config.models.contains_key("classifier"));
    }

    #[test]
    fn test_parse_weighted_model() {
        let toml = r#"
[models.cloud_coder]
entries = [
    { provider = "openai", model = "gpt-5.3-codex-spark", weight = 40, reasoning = "low" },
    { provider = "openai", model = "gpt-5.4", weight = 30, reasoning = "medium" },
    { provider = "anthropic", model = "opus-4.6", weight = 30, reasoning = "medium" },
]
"#;
        let config: ProjectConfig = toml::from_str(toml).unwrap();
        match config.models.get("cloud_coder") {
            Some(ModelRole::Weighted { entries }) => {
                assert_eq!(entries.len(), 3);
                assert_eq!(entries[0].weight, 40);
                assert_eq!(entries[1].model, "gpt-5.4");
            }
            _ => panic!("Expected weighted model"),
        }
    }

    #[test]
    fn test_parse_full_config() {
        let toml = r#"
[models.classifier]
provider = "ollama"
endpoint = "http://sakura-wsl:11434"
model = "qwen3.5-9b:iq4_xs"

[models.light_reasoner]
provider = "ollama"
endpoint = "http://sakura-wsl:11435"
model = "qwen3.5:9b"
reasoning = "on"

[models.cloud_coder]
entries = [
    { provider = "openai", model = "gpt-5.3-codex-spark", weight = 40 },
    { provider = "openai", model = "gpt-5.4", weight = 30 },
]

[routing]
strategy = "cost_first"
compaction_model = "compactor"

[supervisor]
max_iterations = 30
verification_command = "pytest tests/"

[failover]
reasoning = ["light_reasoner", "light_reasoner_backup", "cloud_reasoner", "cloud_coder"]
coding = ["light_coder", "cloud_fast", "cloud_mini", "cloud_coder"]

[usage]
primary_warn_threshold = 0.8
prefer_secondary = true
"#;
        let config: ProjectConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.supervisor.max_iterations, 30);
        assert_eq!(
            config.supervisor.verification_command,
            Some("pytest tests/".into())
        );
        assert_eq!(config.failover.reasoning.len(), 4);
        assert_eq!(config.failover.coding.len(), 4);
        assert_eq!(config.usage.primary_warn_threshold, 0.8);
    }

    #[test]
    fn test_load_nonexistent() {
        let config = ProjectConfig::load(Path::new("/nonexistent/path"));
        assert_eq!(config.supervisor.max_iterations, 50); // defaults
    }
}
