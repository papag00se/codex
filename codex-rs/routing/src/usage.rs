//! Usage tracking across model buckets.
//!
//! Tracks token usage per model/bucket so routing can prefer secondary
//! buckets and warn when primary is getting drained.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use tracing::warn;

/// Usage record for a single model.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub request_count: u64,
}

impl ModelUsage {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

/// Tracks usage across all models, keyed by model slug.
#[derive(Debug, Default)]
pub struct UsageTracker {
    usage: Mutex<HashMap<String, ModelUsage>>,
    warn_threshold: f64,
}

impl UsageTracker {
    pub fn new(warn_threshold: f64) -> Self {
        Self {
            usage: Mutex::new(HashMap::new()),
            warn_threshold,
        }
    }

    /// Record a request's token usage for a model.
    pub fn record(&self, model: &str, input_tokens: u64, output_tokens: u64) {
        let mut usage = self.usage.lock().unwrap_or_else(|e| e.into_inner());
        let entry = usage.entry(model.to_string()).or_default();
        entry.input_tokens += input_tokens;
        entry.output_tokens += output_tokens;
        entry.request_count += 1;
    }

    /// Get usage for a specific model.
    pub fn get(&self, model: &str) -> ModelUsage {
        let usage = self.usage.lock().unwrap_or_else(|e| e.into_inner());
        usage.get(model).cloned().unwrap_or_default()
    }

    /// Get usage for all models.
    pub fn all(&self) -> HashMap<String, ModelUsage> {
        let usage = self.usage.lock().unwrap_or_else(|e| e.into_inner());
        usage.clone()
    }

    /// Get total usage across primary bucket models.
    pub fn primary_usage(&self) -> ModelUsage {
        let usage = self.usage.lock().unwrap_or_else(|e| e.into_inner());
        let mut total = ModelUsage::default();
        for (model, u) in usage.iter() {
            if is_primary_model(model) {
                total.input_tokens += u.input_tokens;
                total.output_tokens += u.output_tokens;
                total.request_count += u.request_count;
            }
        }
        total
    }

    /// Get total usage across secondary bucket models.
    pub fn secondary_usage(&self) -> ModelUsage {
        let usage = self.usage.lock().unwrap_or_else(|e| e.into_inner());
        let mut total = ModelUsage::default();
        for (model, u) in usage.iter() {
            if !is_primary_model(model) && !is_local_model(model) {
                total.input_tokens += u.input_tokens;
                total.output_tokens += u.output_tokens;
                total.request_count += u.request_count;
            }
        }
        total
    }

    /// Get total local (free) usage.
    pub fn local_usage(&self) -> ModelUsage {
        let usage = self.usage.lock().unwrap_or_else(|e| e.into_inner());
        let mut total = ModelUsage::default();
        for (model, u) in usage.iter() {
            if is_local_model(model) {
                total.input_tokens += u.input_tokens;
                total.output_tokens += u.output_tokens;
                total.request_count += u.request_count;
            }
        }
        total
    }

    /// Summary string for logging.
    pub fn summary(&self) -> String {
        let local = self.local_usage();
        let secondary = self.secondary_usage();
        let primary = self.primary_usage();
        format!(
            "local: {}req/{}tok, secondary: {}req/{}tok, primary: {}req/{}tok",
            local.request_count, local.total_tokens(),
            secondary.request_count, secondary.total_tokens(),
            primary.request_count, primary.total_tokens(),
        )
    }

    /// Check if primary usage exceeds the warning threshold.
    /// Returns a warning message if so.
    pub fn check_primary_threshold(&self, estimated_daily_budget: u64) -> Option<String> {
        if estimated_daily_budget == 0 {
            return None;
        }
        let primary = self.primary_usage();
        let ratio = primary.total_tokens() as f64 / estimated_daily_budget as f64;
        if ratio >= self.warn_threshold {
            let msg = format!(
                "Primary bucket usage at {:.0}% of estimated daily budget ({}/{} tokens). Consider routing more to secondary models.",
                ratio * 100.0,
                primary.total_tokens(),
                estimated_daily_budget,
            );
            warn!("{}", msg);
            Some(msg)
        } else {
            None
        }
    }
}

fn is_primary_model(model: &str) -> bool {
    model.contains("gpt-5.4") && !model.contains("mini") && !model.contains("spark")
        || model.contains("opus")
}

fn is_local_model(model: &str) -> bool {
    model.contains("qwen") || model.contains("devstral") || model.contains("openclaw")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_get() {
        let tracker = UsageTracker::new(0.7);
        tracker.record("gpt-5.4", 1000, 200);
        tracker.record("gpt-5.4", 500, 100);
        let usage = tracker.get("gpt-5.4");
        assert_eq!(usage.input_tokens, 1500);
        assert_eq!(usage.output_tokens, 300);
        assert_eq!(usage.request_count, 2);
    }

    #[test]
    fn test_bucket_classification() {
        let tracker = UsageTracker::new(0.7);
        tracker.record("gpt-5.4", 1000, 200);
        tracker.record("gpt-5.3-codex-spark", 2000, 400);
        tracker.record("qwen3.5:9b", 3000, 600);

        assert_eq!(tracker.primary_usage().request_count, 1);
        assert_eq!(tracker.secondary_usage().request_count, 1);
        assert_eq!(tracker.local_usage().request_count, 1);
    }

    #[test]
    fn test_threshold_warning() {
        let tracker = UsageTracker::new(0.7);
        tracker.record("gpt-5.4", 8000, 2000);
        // 10000 tokens against budget of 12000 = 83% > 70% threshold
        assert!(tracker.check_primary_threshold(12000).is_some());
        // Against larger budget: 10000/100000 = 10% < 70%
        assert!(tracker.check_primary_threshold(100000).is_none());
    }

    #[test]
    fn test_summary() {
        let tracker = UsageTracker::new(0.7);
        tracker.record("qwen3.5:9b", 100, 50);
        tracker.record("gpt-5.3-codex-spark", 200, 100);
        tracker.record("gpt-5.4", 300, 150);
        let s = tracker.summary();
        assert!(s.contains("local: 1req"));
        assert!(s.contains("secondary: 1req"));
        assert!(s.contains("primary: 1req"));
    }
}
