//! Routing feedback loop — records outcomes to learn routing profiles.
//!
//! After each request, records: model used, route chosen, success/failure,
//! tokens, latency. Over time, builds per-project routing profiles that
//! the classifier uses to make better decisions.
//!
//! Stored in `.codex-multi/routing_history.jsonl`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{info, warn};

/// A single routing outcome record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingOutcome {
    pub timestamp: u64,
    pub route: String,        // classifier's route decision
    pub model: String,        // actual model used
    pub success: bool,        // did the request produce a useful response?
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub latency_ms: u64,
    pub quality_ok: bool,     // did it pass quality check (G7)?
}

/// Aggregated success rates per route.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RouteProfile {
    pub total: u64,
    pub successes: u64,
    pub avg_latency_ms: u64,
    pub avg_tokens: u64,
}

impl RouteProfile {
    pub fn success_rate(&self) -> f64 {
        if self.total == 0 { 0.0 } else { self.successes as f64 / self.total as f64 }
    }
}

/// Manages the feedback loop — records outcomes, computes profiles.
pub struct FeedbackStore {
    history_path: PathBuf,
    profiles: HashMap<String, RouteProfile>,
}

impl FeedbackStore {
    /// Create a new feedback store, loading existing history.
    pub fn new(project_dir: &Path) -> Self {
        let history_path = project_dir
            .join(".codex-multi")
            .join("routing_history.jsonl");
        let mut store = Self {
            history_path,
            profiles: HashMap::new(),
        };
        store.load_profiles();
        store
    }

    /// Record a routing outcome.
    pub fn record(&mut self, outcome: RoutingOutcome) {
        // Update in-memory profiles
        let profile = self.profiles.entry(outcome.route.clone()).or_default();
        profile.total += 1;
        if outcome.success {
            profile.successes += 1;
        }
        let n = profile.total;
        profile.avg_latency_ms =
            ((profile.avg_latency_ms * (n - 1)) + outcome.latency_ms) / n;
        profile.avg_tokens =
            ((profile.avg_tokens * (n - 1)) + outcome.input_tokens + outcome.output_tokens) / n;

        // Append to JSONL file
        if let Some(parent) = self.history_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string(&outcome) {
            Ok(json) => {
                use std::io::Write;
                match std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.history_path)
                {
                    Ok(mut f) => {
                        let _ = writeln!(f, "{json}");
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to write routing history");
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "Failed to serialize routing outcome");
            }
        }
    }

    /// Get aggregated profiles for all routes.
    pub fn profiles(&self) -> &HashMap<String, RouteProfile> {
        &self.profiles
    }

    /// Format profiles as a string for injection into the classifier prompt.
    pub fn profile_context(&self) -> String {
        if self.profiles.is_empty() {
            return String::new();
        }
        let mut lines = vec!["Historical success rates for this project:".to_string()];
        for (route, profile) in &self.profiles {
            if profile.total >= 3 {
                // Only include routes with enough data
                lines.push(format!(
                    "- {route}: {:.0}% success ({}/{} requests), avg {:.0}ms, avg {} tokens",
                    profile.success_rate() * 100.0,
                    profile.successes,
                    profile.total,
                    profile.avg_latency_ms,
                    profile.avg_tokens,
                ));
            }
        }
        if lines.len() == 1 {
            return String::new(); // No routes with enough data
        }
        lines.join("\n")
    }

    /// Load profiles from the history file.
    fn load_profiles(&mut self) {
        let Ok(content) = std::fs::read_to_string(&self.history_path) else {
            return;
        };
        let mut count = 0;
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(outcome) = serde_json::from_str::<RoutingOutcome>(line) {
                let profile = self.profiles.entry(outcome.route.clone()).or_default();
                profile.total += 1;
                if outcome.success {
                    profile.successes += 1;
                }
                let n = profile.total;
                profile.avg_latency_ms =
                    ((profile.avg_latency_ms * (n - 1)) + outcome.latency_ms) / n;
                profile.avg_tokens = ((profile.avg_tokens * (n - 1))
                    + outcome.input_tokens
                    + outcome.output_tokens)
                    / n;
                count += 1;
            }
        }
        if count > 0 {
            info!(
                records = count,
                routes = self.profiles.len(),
                path = %self.history_path.display(),
                "Loaded routing history"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_profile_accumulation() {
        let dir = std::env::temp_dir().join("feedback_test");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::create_dir_all(dir.join(".codex-multi"));

        let mut store = FeedbackStore::new(&dir);
        store.record(RoutingOutcome {
            timestamp: 1, route: "light_reasoner".into(), model: "qwen3.5:9b".into(),
            success: true, input_tokens: 100, output_tokens: 50, latency_ms: 3000, quality_ok: true,
        });
        store.record(RoutingOutcome {
            timestamp: 2, route: "light_reasoner".into(), model: "qwen3.5:9b".into(),
            success: true, input_tokens: 200, output_tokens: 100, latency_ms: 4000, quality_ok: true,
        });
        store.record(RoutingOutcome {
            timestamp: 3, route: "light_reasoner".into(), model: "qwen3.5:9b".into(),
            success: false, input_tokens: 150, output_tokens: 0, latency_ms: 2000, quality_ok: false,
        });

        let profile = store.profiles().get("light_reasoner").unwrap();
        assert_eq!(profile.total, 3);
        assert_eq!(profile.successes, 2);
        assert!((profile.success_rate() - 0.6667).abs() < 0.01);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_profile_persistence() {
        let dir = std::env::temp_dir().join("feedback_persist_test");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(dir.join(".codex-multi"));

        // Write some records
        {
            let mut store = FeedbackStore::new(&dir);
            for i in 0..5 {
                store.record(RoutingOutcome {
                    timestamp: i, route: "cloud_fast".into(), model: "spark".into(),
                    success: i < 4, input_tokens: 100, output_tokens: 50,
                    latency_ms: 1000, quality_ok: true,
                });
            }
        }

        // Load fresh and check
        let store = FeedbackStore::new(&dir);
        let profile = store.profiles().get("cloud_fast").unwrap();
        assert_eq!(profile.total, 5);
        assert_eq!(profile.successes, 4);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_profile_context_format() {
        let dir = std::env::temp_dir().join("feedback_ctx_test");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(dir.join(".codex-multi"));

        let mut store = FeedbackStore::new(&dir);
        for i in 0..5 {
            store.record(RoutingOutcome {
                timestamp: i, route: "light_reasoner".into(), model: "qwen".into(),
                success: true, input_tokens: 100, output_tokens: 50,
                latency_ms: 3000, quality_ok: true,
            });
        }
        let ctx = store.profile_context();
        assert!(ctx.contains("light_reasoner"));
        assert!(ctx.contains("100%"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
