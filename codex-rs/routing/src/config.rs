//! Routing configuration — multi-tier model routing.
//!
//! Supports local Ollama (free), cloud secondary buckets (cheap), and
//! cloud primary buckets (conserve). See docs/spec/model-routing-strategy.md.

use serde::{Deserialize, Serialize};
use std::env;

/// A single Ollama endpoint + model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaEndpoint {
    pub base_url: String,
    pub model: String,
    pub num_ctx: usize,
    pub temperature: f64,
    pub timeout_seconds: u64,
    pub enabled: bool,
}

impl OllamaEndpoint {
    fn from_env(url_var: &str, model_var: &str, defaults: (&str, &str)) -> Self {
        Self {
            base_url: env_or(url_var, defaults.0),
            model: env_or(model_var, defaults.1),
            num_ctx: 8192,
            temperature: 0.1,
            timeout_seconds: 300,
            enabled: true,
        }
    }
}

/// Full routing configuration across all tiers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingConfig {
    // --- Local Ollama (free) ---
    pub classifier: OllamaEndpoint,
    pub reasoner: OllamaEndpoint,
    pub reasoner_backup: OllamaEndpoint,
    pub light_coder: OllamaEndpoint,
    pub compactor: OllamaEndpoint,

    // --- Cloud secondary buckets (prefer over primary) ---
    pub codex_spark_enabled: bool,
    pub mini_enabled: bool,
    pub sonnet_enabled: bool,

    // --- Legacy compat with the route selection engine ---
    pub router: RouterModelConfig,
    pub coder: OllamaRouteConfig,

    pub codex_cli_enabled: bool,
}

/// Configuration for the router model (the LLM that picks routes).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterModelConfig {
    pub base_url: String,
    pub model: String,
    pub num_ctx: usize,
    pub temperature: f64,
    pub timeout_seconds: u64,
}

/// Legacy config for the route selection engine's coder/reasoner paths.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaRouteConfig {
    pub base_url: String,
    pub model: String,
    pub num_ctx: usize,
    pub temperature: f64,
    pub timeout_seconds: u64,
    pub enabled: bool,
}

impl RoutingConfig {
    /// Load routing config from environment variables.
    pub fn from_env() -> Self {
        let classifier = OllamaEndpoint::from_env(
            "OLLAMA_CLASSIFIER_URL",
            "OLLAMA_CLASSIFIER_MODEL",
            ("http://sakura-wsl.taile41496.ts.net:11434", "qwen3.5-9b:iq4_xs"),
        );

        let reasoner = OllamaEndpoint::from_env(
            "OLLAMA_REASONER_URL",
            "OLLAMA_REASONER_MODEL",
            ("http://sakura-wsl.taile41496.ts.net:11435", "qwen3.5:9b"),
        );

        let reasoner_backup = OllamaEndpoint::from_env(
            "OLLAMA_REASONER_BACKUP_URL",
            "OLLAMA_REASONER_BACKUP_MODEL",
            ("http://meru-wsl.taile41496.ts.net:11434", "qwen3.5:9b"),
        );

        let light_coder = OllamaEndpoint::from_env(
            "OLLAMA_CODER_URL",
            "OLLAMA_CODER_MODEL",
            ("http://sakura-wsl.taile41496.ts.net:11435", "qwen3.5-9b-opus-openclaw-distilled:tools"),
        );

        let compactor = OllamaEndpoint::from_env(
            "OLLAMA_COMPACTOR_URL",
            "OLLAMA_COMPACTOR_MODEL",
            ("http://sakura-wsl.taile41496.ts.net:11435", "qwen3.5-9b:iq4_xs"),
        );

        Self {
            classifier,
            reasoner: reasoner.clone(),
            reasoner_backup,
            light_coder,
            compactor,
            codex_spark_enabled: env_bool("ENABLE_CODEX_SPARK", true),
            mini_enabled: env_bool("ENABLE_GPT_MINI", true),
            sonnet_enabled: env_bool("ENABLE_SONNET", true),
            // Legacy compat for route selection engine
            router: RouterModelConfig {
                base_url: reasoner.base_url.clone(),
                model: reasoner.model.clone(),
                num_ctx: reasoner.num_ctx,
                temperature: 0.0,
                timeout_seconds: reasoner.timeout_seconds,
            },
            coder: OllamaRouteConfig {
                base_url: env_or("CODER_OLLAMA_BASE_URL", &reasoner.base_url),
                model: env_or("CODER_MODEL", &reasoner.model),
                num_ctx: env_usize("CODER_NUM_CTX", 16384),
                temperature: env_f64("CODER_TEMPERATURE", 0.1),
                timeout_seconds: env_u64("CODER_TIMEOUT_SECONDS", 300),
                enabled: env_bool("ENABLE_LOCAL_CODER", true),
            },
            codex_cli_enabled: false,
        }
    }

    /// Load routing config from a ProjectConfig (`.codex-multi/config.toml`).
    /// Falls back to from_env() for any missing fields.
    pub fn from_project_config(pc: &crate::project_config::ProjectConfig) -> Self {
        let mut config = Self::from_env();

        // Override from project config model roles
        if let Some(role) = pc.get_model("classifier") {
            if let Some(ep) = endpoint_from_role(role) {
                config.classifier = ep;
            }
        }
        if let Some(role) = pc.get_model("light_reasoner") {
            if let Some(ep) = endpoint_from_role(role) {
                config.reasoner = ep;
            }
        }
        if let Some(role) = pc.get_model("light_reasoner_backup") {
            if let Some(ep) = endpoint_from_role(role) {
                config.reasoner_backup = ep;
            }
        }
        if let Some(role) = pc.get_model("light_coder") {
            if let Some(ep) = endpoint_from_role(role) {
                config.light_coder = ep;
            }
        }
        if let Some(role) = pc.get_model("compactor") {
            if let Some(ep) = endpoint_from_role(role) {
                config.compactor = ep;
            }
        }

        // Update the legacy router config to match the reasoner
        config.router = RouterModelConfig {
            base_url: config.reasoner.base_url.clone(),
            model: config.reasoner.model.clone(),
            num_ctx: config.reasoner.num_ctx,
            temperature: 0.0,
            timeout_seconds: config.reasoner.timeout_seconds,
        };

        config
    }
}

/// Extract an OllamaEndpoint from a model role (single entry only).
fn endpoint_from_role(role: &crate::project_config::ModelRole) -> Option<OllamaEndpoint> {
    match role {
        crate::project_config::ModelRole::Single {
            provider,
            endpoint,
            model,
            reasoning,
            num_ctx,
        } => {
            if provider != "ollama" {
                return None; // Only Ollama endpoints can be used as local endpoints
            }
            Some(OllamaEndpoint {
                base_url: endpoint.clone().unwrap_or_else(|| "http://127.0.0.1:11434".into()),
                model: model.clone(),
                num_ctx: num_ctx.unwrap_or(8192),
                temperature: if reasoning == "off" { 0.0 } else { 0.1 },
                timeout_seconds: 300,
                enabled: true,
            })
        }
        crate::project_config::ModelRole::Weighted { .. } => {
            // Weighted roles are for cloud models, not local endpoints
            None
        }
    }
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

fn env_or(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_string())
}

fn env_bool(name: &str, default: bool) -> bool {
    env::var(name)
        .map(|v| matches!(v.trim().to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(default)
}

fn env_usize(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

fn env_f64(name: &str, default: f64) -> f64 {
    env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}
