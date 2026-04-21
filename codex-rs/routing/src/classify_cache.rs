//! Classifier result caching — reduces the 3-4s latency per request.
//!
//! If the last N requests all went to the same route, skip the classifier
//! for the next request and use the cached route. Resets when context changes
//! significantly (tool call count changes, conversation turns over).
//!
//! Also supports async-ahead: start the cloud request immediately while
//! classifying in parallel. If classification says "local", cancel cloud.
//! (Not yet implemented — cache is the v1 approach.)

use crate::classifier::{ClassifyResult, RouteTarget};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

const CACHE_SIZE: usize = 5;
const CACHE_TTL: Duration = Duration::from_secs(30);
const CONFIDENCE_THRESHOLD: usize = 3; // Skip classifier after 3 consecutive same-route decisions

/// Caches recent classification results to skip the classifier when confident.
pub struct ClassifyCache {
    recent: VecDeque<CacheEntry>,
}

struct CacheEntry {
    route: RouteTarget,
    tools_potential: bool,
    timestamp: Instant,
}

impl ClassifyCache {
    pub fn new() -> Self {
        Self {
            recent: VecDeque::with_capacity(CACHE_SIZE),
        }
    }

    /// Check if we can skip the classifier and use a cached result.
    /// Returns Some(ClassifyResult) if confident, None if classifier should run.
    pub fn try_cached(&self) -> Option<ClassifyResult> {
        // Not enough history
        if self.recent.len() < CONFIDENCE_THRESHOLD {
            return None;
        }

        // Check if all recent entries are fresh
        let now = Instant::now();
        let all_fresh = self
            .recent
            .iter()
            .all(|e| now.duration_since(e.timestamp) < CACHE_TTL);
        if !all_fresh {
            return None;
        }

        // Check if all recent entries have the same route
        let first_route = self.recent.front()?.route;
        let all_same = self.recent.iter().all(|e| e.route == first_route);
        if !all_same {
            return None;
        }

        let tools_potential = self.recent.front()?.tools_potential;

        Some(ClassifyResult {
            route: first_route,
            tools_potential,
            reason: format!(
                "cached — last {} requests all {:?}",
                self.recent.len(),
                first_route
            ),
        })
    }

    /// Record a classification result.
    pub fn record(&mut self, result: &ClassifyResult) {
        if self.recent.len() >= CACHE_SIZE {
            self.recent.pop_front();
        }
        self.recent.push_back(CacheEntry {
            route: result.route,
            tools_potential: result.tools_potential,
            timestamp: Instant::now(),
        });
    }

    /// Clear the cache (e.g., when context changes significantly).
    pub fn clear(&mut self) {
        self.recent.clear();
    }
}

impl Default for ClassifyCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_result(route: RouteTarget) -> ClassifyResult {
        ClassifyResult {
            route,
            tools_potential: false,
            reason: "test".into(),
        }
    }

    #[test]
    fn test_no_cache_when_empty() {
        let cache = ClassifyCache::new();
        assert!(cache.try_cached().is_none());
    }

    #[test]
    fn test_no_cache_when_insufficient() {
        let mut cache = ClassifyCache::new();
        cache.record(&make_result(RouteTarget::LightReasoner));
        cache.record(&make_result(RouteTarget::LightReasoner));
        // Only 2, need 3
        assert!(cache.try_cached().is_none());
    }

    #[test]
    fn test_cache_hit_after_threshold() {
        let mut cache = ClassifyCache::new();
        for _ in 0..3 {
            cache.record(&make_result(RouteTarget::LightReasoner));
        }
        let cached = cache.try_cached();
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().route, RouteTarget::LightReasoner);
    }

    #[test]
    fn test_no_cache_when_mixed() {
        let mut cache = ClassifyCache::new();
        cache.record(&make_result(RouteTarget::LightReasoner));
        cache.record(&make_result(RouteTarget::CloudFast));
        cache.record(&make_result(RouteTarget::LightReasoner));
        assert!(cache.try_cached().is_none());
    }

    #[test]
    fn test_clear() {
        let mut cache = ClassifyCache::new();
        for _ in 0..5 {
            cache.record(&make_result(RouteTarget::CloudMini));
        }
        assert!(cache.try_cached().is_some());
        cache.clear();
        assert!(cache.try_cached().is_none());
    }
}
