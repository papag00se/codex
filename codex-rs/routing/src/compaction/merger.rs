//! Deterministic state merging — ported from compaction/merger.py.
//! Merges chunk extractions using: latest-non-empty for scalars,
//! shallow-merge for dicts, deduplicated-reverse for lists.

use super::models::{ChunkExtraction, MergedState};
use std::collections::{HashMap, HashSet};

/// Merge multiple chunk extractions into a single state.
pub fn merge_states(states: &[ChunkExtraction]) -> MergedState {
    if states.is_empty() {
        return MergedState::default();
    }

    MergedState {
        chunk_id: 0,
        objective: latest_non_empty(states.iter().map(|s| s.objective.as_str())),
        repo_state: merge_repo_state(states.iter().map(|s| &s.repo_state)),
        files_touched: merge_unique(states.iter().map(|s| s.files_touched.as_slice())),
        commands_run: merge_unique(states.iter().map(|s| s.commands_run.as_slice())),
        errors: merge_unique(states.iter().map(|s| s.errors.as_slice())),
        accepted_fixes: merge_unique(states.iter().map(|s| s.accepted_fixes.as_slice())),
        rejected_ideas: merge_unique(states.iter().map(|s| s.rejected_ideas.as_slice())),
        constraints: merge_unique(states.iter().map(|s| s.constraints.as_slice())),
        environment_assumptions: merge_unique(states.iter().map(|s| s.environment_assumptions.as_slice())),
        pending_todos: merge_unique(states.iter().map(|s| s.pending_todos.as_slice())),
        unresolved_bugs: merge_unique(states.iter().map(|s| s.unresolved_bugs.as_slice())),
        test_status: merge_unique(states.iter().map(|s| s.test_status.as_slice())),
        external_references: merge_unique(states.iter().map(|s| s.external_references.as_slice())),
        latest_plan: latest_non_empty_list(states.iter().map(|s| s.latest_plan.as_slice())),
        source_token_count: states.iter().map(|s| s.source_token_count).sum(),
    }
}

/// Last non-empty string wins.
fn latest_non_empty<'a>(values: impl Iterator<Item = &'a str>) -> String {
    let mut result = String::new();
    for v in values {
        if !v.is_empty() {
            result = v.to_string();
        }
    }
    result
}

/// Last non-empty list wins entirely.
fn latest_non_empty_list<'a>(values: impl Iterator<Item = &'a [String]>) -> Vec<String> {
    let mut result = Vec::new();
    for v in values {
        if !v.is_empty() {
            result = v.to_vec();
        }
    }
    result
}

/// Shallow dict merge — later values overwrite.
fn merge_repo_state<'a>(values: impl Iterator<Item = &'a HashMap<String, String>>) -> HashMap<String, String> {
    let mut merged = HashMap::new();
    for v in values {
        for (k, val) in v {
            if !val.is_empty() {
                merged.insert(k.clone(), val.clone());
            }
        }
    }
    merged
}

/// Deduplicate lists: process in reverse (newest first),
/// case-insensitive dedup, preserve original casing.
fn merge_unique<'a>(groups: impl Iterator<Item = &'a [String]>) -> Vec<String> {
    let groups: Vec<&[String]> = groups.collect();
    let mut seen = HashSet::new();
    let mut merged = Vec::new();

    // Process newest first (reverse)
    for group in groups.into_iter().rev() {
        for item in group {
            let key = item.trim().to_lowercase();
            if key.is_empty() || seen.contains(&key) {
                continue;
            }
            seen.insert(key);
            merged.push(item.clone());
        }
    }

    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_two() {
        let s1 = ChunkExtraction {
            chunk_id: 1,
            objective: "Fix auth".into(),
            files_touched: vec!["auth.py".into()],
            ..Default::default()
        };
        let s2 = ChunkExtraction {
            chunk_id: 2,
            objective: "Fix auth and add tests".into(),
            files_touched: vec!["test_auth.py".into()],
            ..Default::default()
        };
        let merged = merge_states(&[s1, s2]);
        assert_eq!(merged.objective, "Fix auth and add tests");
        assert!(merged.files_touched.contains(&"test_auth.py".into()));
        assert!(merged.files_touched.contains(&"auth.py".into()));
    }

    #[test]
    fn test_merge_repo_state_shallow() {
        let s1 = ChunkExtraction {
            repo_state: [("branch".into(), "main".into())].into(),
            ..Default::default()
        };
        let s2 = ChunkExtraction {
            repo_state: [("branch".into(), "feature".into()), ("db".into(), "pg".into())].into(),
            ..Default::default()
        };
        let merged = merge_states(&[s1, s2]);
        assert_eq!(merged.repo_state.get("branch").unwrap(), "feature");
        assert_eq!(merged.repo_state.get("db").unwrap(), "pg");
    }

    #[test]
    fn test_dedup_case_insensitive() {
        let s1 = ChunkExtraction {
            files_touched: vec!["Auth.py".into(), "README.md".into()],
            ..Default::default()
        };
        let s2 = ChunkExtraction {
            files_touched: vec!["auth.py".into(), "tests.py".into()],
            ..Default::default()
        };
        let merged = merge_states(&[s1, s2]);
        let lower: Vec<String> = merged.files_touched.iter().map(|f| f.to_lowercase()).collect();
        assert_eq!(lower.iter().filter(|f| f.as_str() == "auth.py").count(), 1);
    }
}
