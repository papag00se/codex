//! Persistent cost analytics across sessions.
//!
//! Appends usage summaries to `.codex-multi/usage_log.jsonl` at session end.
//! Provides session and aggregate summaries for cost analysis.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// A single session's usage summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionUsageSummary {
    pub session_id: String,
    pub timestamp: u64,
    pub duration_seconds: u64,
    pub local_requests: u64,
    pub local_tokens: u64,
    pub secondary_requests: u64,
    pub secondary_tokens: u64,
    pub primary_requests: u64,
    pub primary_tokens: u64,
    pub total_requests: u64,
    pub total_tokens: u64,
    pub estimated_savings_pct: f64, // % of requests that went to free/secondary
}

/// Aggregate usage across multiple sessions.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AggregateUsage {
    pub session_count: u64,
    pub total_local_tokens: u64,
    pub total_secondary_tokens: u64,
    pub total_primary_tokens: u64,
    pub total_tokens: u64,
    pub avg_savings_pct: f64,
}

/// Manages persistent cost analytics.
pub struct CostAnalytics {
    log_path: PathBuf,
}

impl CostAnalytics {
    pub fn new(project_dir: &Path) -> Self {
        let log_path = project_dir.join(".codex-multi").join("usage_log.jsonl");
        Self { log_path }
    }

    /// Record a session's usage summary.
    pub fn record_session(&self, summary: &SessionUsageSummary) {
        if let Some(parent) = self.log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string(summary) {
            Ok(json) => {
                use std::io::Write;
                match std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.log_path)
                {
                    Ok(mut f) => {
                        let _ = writeln!(f, "{json}");
                        info!(
                            session = %summary.session_id,
                            local = summary.local_requests,
                            secondary = summary.secondary_requests,
                            primary = summary.primary_requests,
                            savings = format!("{:.0}%", summary.estimated_savings_pct),
                            "Session usage recorded"
                        );
                    }
                    Err(e) => warn!(error = %e, "Failed to write usage log"),
                }
            }
            Err(e) => warn!(error = %e, "Failed to serialize usage summary"),
        }
    }

    /// Load aggregate usage from all recorded sessions.
    pub fn aggregate(&self) -> AggregateUsage {
        let Ok(content) = std::fs::read_to_string(&self.log_path) else {
            return AggregateUsage::default();
        };

        let mut agg = AggregateUsage::default();
        let mut savings_sum = 0.0;

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(summary) = serde_json::from_str::<SessionUsageSummary>(line) {
                agg.session_count += 1;
                agg.total_local_tokens += summary.local_tokens;
                agg.total_secondary_tokens += summary.secondary_tokens;
                agg.total_primary_tokens += summary.primary_tokens;
                agg.total_tokens += summary.total_tokens;
                savings_sum += summary.estimated_savings_pct;
            }
        }

        if agg.session_count > 0 {
            agg.avg_savings_pct = savings_sum / agg.session_count as f64;
        }
        agg
    }

    /// Format a human-readable summary.
    pub fn summary_string(&self) -> String {
        let agg = self.aggregate();
        if agg.session_count == 0 {
            return "No usage data recorded yet.".into();
        }
        format!(
            "Usage across {} sessions:\n\
             Local (free): {} tokens\n\
             Secondary: {} tokens\n\
             Primary: {} tokens\n\
             Total: {} tokens\n\
             Avg savings: {:.0}% of requests went to free/secondary",
            agg.session_count,
            agg.total_local_tokens,
            agg.total_secondary_tokens,
            agg.total_primary_tokens,
            agg.total_tokens,
            agg.avg_savings_pct,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_aggregate() {
        let dir = std::env::temp_dir().join("cost_analytics_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".codex-multi")).unwrap();

        let analytics = CostAnalytics::new(&dir);
        analytics.record_session(&SessionUsageSummary {
            session_id: "s1".into(),
            timestamp: 1000,
            duration_seconds: 300,
            local_requests: 10,
            local_tokens: 5000,
            secondary_requests: 5,
            secondary_tokens: 3000,
            primary_requests: 2,
            primary_tokens: 2000,
            total_requests: 17,
            total_tokens: 10000,
            estimated_savings_pct: 88.0,
        });
        analytics.record_session(&SessionUsageSummary {
            session_id: "s2".into(),
            timestamp: 2000,
            duration_seconds: 600,
            local_requests: 20,
            local_tokens: 10000,
            secondary_requests: 3,
            secondary_tokens: 2000,
            primary_requests: 1,
            primary_tokens: 1000,
            total_requests: 24,
            total_tokens: 13000,
            estimated_savings_pct: 96.0,
        });

        let agg = analytics.aggregate();
        assert_eq!(agg.session_count, 2);
        assert_eq!(agg.total_local_tokens, 15000);
        assert_eq!(agg.total_primary_tokens, 3000);
        assert!((agg.avg_savings_pct - 92.0).abs() < 0.1);

        let summary = analytics.summary_string();
        assert!(summary.contains("2 sessions"));
        assert!(summary.contains("92%"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
