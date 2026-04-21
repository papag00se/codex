//! Cross-session memory — persists project knowledge across sessions.
//!
//! After each session, saves a handoff summary to `.codex-multi/memory/`.
//! On session start, loads recent handoffs as context for the planner
//! and classifier. Accumulates over time — the system gets smarter
//! about this project with each session.
//!
//! Format matches the compaction pipeline's durable memory files:
//! task state, decisions, failures to avoid, next steps.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

const MAX_MEMORIES: usize = 20;
const MEMORY_DIR: &str = "memory";

/// A single session's handoff summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMemory {
    pub session_id: String,
    pub timestamp: u64,
    pub goal: String,
    pub outcome: String, // "completed", "failed", "partial"
    pub task_state: String,
    pub decisions: Vec<String>,
    pub failures_to_avoid: Vec<String>,
    pub next_steps: Vec<String>,
    pub files_touched: Vec<String>,
    pub models_used: Vec<String>,
}

/// Manages cross-session memory persistence.
pub struct MemoryStore {
    memory_dir: PathBuf,
}

impl MemoryStore {
    pub fn new(project_dir: &Path) -> Self {
        let memory_dir = project_dir.join(".codex-multi").join(MEMORY_DIR);
        Self { memory_dir }
    }

    /// Save a session's memory.
    pub fn save(&self, memory: &SessionMemory) {
        let _ = std::fs::create_dir_all(&self.memory_dir);
        let filename = format!("{}_{}.json", memory.timestamp, memory.session_id);
        let path = self.memory_dir.join(&filename);

        match serde_json::to_string_pretty(memory) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    warn!(error = %e, "Failed to save session memory");
                } else {
                    info!(path = %path.display(), "Session memory saved");
                    self.prune();
                }
            }
            Err(e) => {
                warn!(error = %e, "Failed to serialize session memory");
            }
        }
    }

    /// Load recent session memories, newest first.
    pub fn load_recent(&self, limit: usize) -> Vec<SessionMemory> {
        let Ok(entries) = std::fs::read_dir(&self.memory_dir) else {
            return Vec::new();
        };

        let mut memories: Vec<(u64, SessionMemory)> = entries
            .flatten()
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "json")
                    .unwrap_or(false)
            })
            .filter_map(|e| {
                let content = std::fs::read_to_string(e.path()).ok()?;
                let memory: SessionMemory = serde_json::from_str(&content).ok()?;
                Some((memory.timestamp, memory))
            })
            .collect();

        memories.sort_by(|a, b| b.0.cmp(&a.0)); // Newest first
        memories.into_iter().take(limit).map(|(_, m)| m).collect()
    }

    /// Format recent memories as context for the planner/classifier.
    pub fn planner_context(&self, limit: usize) -> String {
        let memories = self.load_recent(limit);
        if memories.is_empty() {
            return String::new();
        }

        let mut parts = vec!["Prior session context for this project:".to_string()];
        for (i, m) in memories.iter().enumerate() {
            parts.push(format!(
                "\nSession {} ({}): {}\n  Outcome: {}\n  Key decisions: {}\n  Avoid: {}\n  Next: {}",
                i + 1,
                m.goal,
                m.task_state,
                m.outcome,
                if m.decisions.is_empty() { "none".into() } else { m.decisions.join("; ") },
                if m.failures_to_avoid.is_empty() { "none".into() } else { m.failures_to_avoid.join("; ") },
                if m.next_steps.is_empty() { "none".into() } else { m.next_steps.join("; ") },
            ));
        }
        parts.join("\n")
    }

    /// Keep only the most recent MAX_MEMORIES files.
    fn prune(&self) {
        let Ok(entries) = std::fs::read_dir(&self.memory_dir) else {
            return;
        };

        let mut files: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "json")
                    .unwrap_or(false)
            })
            .map(|e| e.path())
            .collect();

        if files.len() <= MAX_MEMORIES {
            return;
        }

        // Sort by filename (which starts with timestamp) — oldest first
        files.sort();
        let to_remove = files.len() - MAX_MEMORIES;
        for path in files.into_iter().take(to_remove) {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_save_and_load() {
        let dir = std::env::temp_dir().join("session_memory_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".codex-multi")).unwrap();

        let store = MemoryStore::new(&dir);
        store.save(&SessionMemory {
            session_id: "test_1".into(),
            timestamp: 1000,
            goal: "Fix auth bug".into(),
            outcome: "completed".into(),
            task_state: "Auth middleware fixed".into(),
            decisions: vec!["Used JWT instead of session cookies".into()],
            failures_to_avoid: vec!["Don't use bcrypt for tokens".into()],
            next_steps: vec!["Add rate limiting".into()],
            files_touched: vec!["auth.py".into()],
            models_used: vec!["gpt-5.4".into()],
        });
        store.save(&SessionMemory {
            session_id: "test_2".into(),
            timestamp: 2000,
            goal: "Add rate limiting".into(),
            outcome: "partial".into(),
            task_state: "Redis client created, middleware not done".into(),
            decisions: vec![],
            failures_to_avoid: vec![],
            next_steps: vec!["Finish middleware".into()],
            files_touched: vec!["redis_client.py".into()],
            models_used: vec!["qwen3.5:9b".into()],
        });

        let recent = store.load_recent(5);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].session_id, "test_2"); // Newest first
        assert_eq!(recent[1].session_id, "test_1");

        let ctx = store.planner_context(5);
        assert!(ctx.contains("Fix auth bug"));
        assert!(ctx.contains("Add rate limiting"));
        assert!(ctx.contains("JWT instead of session cookies"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_prune() {
        let dir = std::env::temp_dir().join("session_memory_prune_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".codex-multi").join("memory")).unwrap();

        let store = MemoryStore::new(&dir);
        // Save more than MAX_MEMORIES
        for i in 0..25 {
            store.save(&SessionMemory {
                session_id: format!("test_{i}"),
                timestamp: i as u64,
                goal: format!("Goal {i}"),
                outcome: "completed".into(),
                task_state: String::new(),
                decisions: vec![],
                failures_to_avoid: vec![],
                next_steps: vec![],
                files_touched: vec![],
                models_used: vec![],
            });
        }

        let recent = store.load_recent(100);
        assert!(recent.len() <= MAX_MEMORIES);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
