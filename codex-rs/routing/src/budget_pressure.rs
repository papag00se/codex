//! Dynamic budget pressure — shifts routing based on real-time rate limit data.
//!
//! Reads RateLimitSnapshot from cloud responses (already parsed by codex-api)
//! and generates pressure strings for the classifier prompt. As primary usage
//! increases, the classifier gets increasingly reluctant to route to primary.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

/// Tracks the latest rate limit state from cloud responses.
#[derive(Default)]
pub struct BudgetState {
    /// Primary bucket: percentage used (0-100), scaled to u64 for atomic.
    primary_used_pct: AtomicU64,
    /// Secondary bucket: percentage used (0-100).
    secondary_used_pct: AtomicU64,
    /// Unix timestamp when primary resets.
    primary_resets_at: AtomicU64,
}

impl BudgetState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Update from a rate limit snapshot (called after each cloud response).
    pub fn update(&self, primary_pct: f64, secondary_pct: f64, primary_reset: Option<u64>) {
        self.primary_used_pct.store(
            (primary_pct * 100.0) as u64, // Store as basis points for precision
            Ordering::Relaxed,
        );
        self.secondary_used_pct
            .store((secondary_pct * 100.0) as u64, Ordering::Relaxed);
        if let Some(reset) = primary_reset {
            self.primary_resets_at.store(reset, Ordering::Relaxed);
        }
    }

    /// Get current primary usage percentage (0.0-100.0).
    pub fn primary_used(&self) -> f64 {
        self.primary_used_pct.load(Ordering::Relaxed) as f64 / 100.0
    }

    /// Get current secondary usage percentage (0.0-100.0).
    pub fn secondary_used(&self) -> f64 {
        self.secondary_used_pct.load(Ordering::Relaxed) as f64 / 100.0
    }

    /// Generate budget pressure string for the classifier prompt.
    /// Returns empty string if no pressure (usage is low).
    pub fn pressure_context(&self) -> String {
        let primary = self.primary_used();
        let secondary = self.secondary_used();

        if primary < 50.0 {
            // No pressure — plenty of budget
            return String::new();
        }

        if primary >= 90.0 {
            return format!(
                "CRITICAL BUDGET WARNING: Primary usage at {primary:.0}%. \
                 DO NOT use cloud_coder unless the task has already FAILED on cloud_reasoner. \
                 Prefer cloud_mini and cloud_fast. Use local whenever possible."
            );
        }

        if primary >= 70.0 {
            return format!(
                "BUDGET WARNING: Primary usage at {primary:.0}%. \
                 Strongly prefer secondary routes (cloud_fast, cloud_mini, cloud_reasoner) \
                 and local routes over cloud_coder. \
                 Only use cloud_coder for tasks that genuinely require it."
            );
        }

        // 50-70%: gentle nudge
        format!(
            "Budget note: Primary usage at {primary:.0}%. \
             Prefer secondary and local routes when quality allows."
        )
    }

    /// Should cloud_coder be hard-blocked? (deterministic, not LLM decision)
    pub fn should_block_primary(&self) -> bool {
        self.primary_used() >= 95.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_pressure_when_low() {
        let state = BudgetState::new();
        state.update(30.0, 10.0, None);
        assert!(state.pressure_context().is_empty());
    }

    #[test]
    fn test_gentle_pressure() {
        let state = BudgetState::new();
        state.update(60.0, 20.0, None);
        let ctx = state.pressure_context();
        assert!(ctx.contains("60%"));
        assert!(ctx.contains("Prefer secondary"));
    }

    #[test]
    fn test_strong_pressure() {
        let state = BudgetState::new();
        state.update(80.0, 30.0, None);
        let ctx = state.pressure_context();
        assert!(ctx.contains("BUDGET WARNING"));
        assert!(ctx.contains("80%"));
    }

    #[test]
    fn test_critical_pressure() {
        let state = BudgetState::new();
        state.update(92.0, 40.0, None);
        let ctx = state.pressure_context();
        assert!(ctx.contains("CRITICAL"));
        assert!(ctx.contains("92%"));
    }

    #[test]
    fn test_hard_block() {
        let state = BudgetState::new();
        state.update(96.0, 50.0, None);
        assert!(state.should_block_primary());

        state.update(50.0, 20.0, None);
        assert!(!state.should_block_primary());
    }
}
