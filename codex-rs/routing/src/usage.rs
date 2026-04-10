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
    /// Estimated cloud tokens avoided by routing locally.
    /// This is the pre-strip token count — what the cloud model would have received.
    cloud_tokens_saved: Mutex<u64>,
    warn_threshold: f64,
}

impl UsageTracker {
    pub fn new(warn_threshold: f64) -> Self {
        Self {
            usage: Mutex::new(HashMap::new()),
            cloud_tokens_saved: Mutex::new(0),
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

    /// Record cloud tokens saved by routing a request locally.
    /// `pre_strip_tokens` is the estimated token count of the full conversation
    /// before context stripping — what the cloud model would have received.
    pub fn record_savings(&self, pre_strip_tokens: u64) {
        let mut saved = self.cloud_tokens_saved.lock().unwrap_or_else(|e| e.into_inner());
        *saved += pre_strip_tokens;
    }

    /// Get total estimated cloud tokens saved.
    pub fn cloud_tokens_saved(&self) -> u64 {
        *self.cloud_tokens_saved.lock().unwrap_or_else(|e| e.into_inner())
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

    /// Summary string for logging and /stats display.
    pub fn summary(&self) -> String {
        let local = self.local_usage();
        let secondary = self.secondary_usage();
        let primary = self.primary_usage();
        let saved = self.cloud_tokens_saved();
        let total_req = local.request_count + secondary.request_count + primary.request_count;
        let local_pct = if total_req > 0 {
            (local.request_count as f64 / total_req as f64) * 100.0
        } else {
            0.0
        };
        format!(
            "Routing stats this session:\n\
             \n\
             Local (free):  {} requests, {} tokens\n\
             Secondary:     {} requests, {} tokens\n\
             Primary:       {} requests, {} tokens\n\
             \n\
             Cloud tokens saved: ~{}\n\
             Local routing rate: {:.0}% of requests",
            local.request_count, local.total_tokens(),
            secondary.request_count, secondary.total_tokens(),
            primary.request_count, primary.total_tokens(),
            saved,
            local_pct,
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
        tracker.record_savings(5000);
        let s = tracker.summary();
        assert!(s.contains("1 requests, 150 tokens")); // local
        assert!(s.contains("1 requests, 300 tokens")); // secondary
        assert!(s.contains("1 requests, 450 tokens")); // primary
        assert!(s.contains("Cloud tokens saved: ~5000"));
        assert!(s.contains("33%")); // 1 local out of 3 total
    }
}
