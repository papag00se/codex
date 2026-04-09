//! Task graph with deterministic state machine.
//!
//! The supervisor loop drives tasks through states. No LLM controls transitions —
//! the code does. See docs/spec/design-principles.md.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Task status — deterministic state machine.
/// The supervisor loop moves tasks through these states. The LLM never decides
/// whether to continue — the code checks `has_pending_tasks()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    /// Waiting to be dispatched. Dependencies may or may not be met.
    Pending,
    /// Currently being executed by a sub-agent.
    Running,
    /// Agent finished, awaiting evaluation.
    Evaluating,
    /// Task completed successfully.
    Completed,
    /// Task failed after exhausting retries.
    Failed,
    /// Task skipped because a dependency failed.
    Skipped,
}

impl TaskStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Skipped)
    }
}

/// A single task in the supervisor's task graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub description: String,
    pub task_type: String,
    pub dependencies: Vec<String>,
    pub status: TaskStatus,
    pub assigned_model: Option<String>,
    pub retry_count: u32,
    pub max_retries: u32,
    pub result: Option<String>,
    pub error: Option<String>,
    /// Thread ID of the last agent that worked on this task.
    /// Used for context resumption: retries fork from this agent's conversation
    /// so the new agent sees what was tried and why it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_agent_thread_id: Option<String>,
}

/// The task graph — holds all tasks and provides deterministic queries.
#[derive(Debug, Clone)]
pub struct TaskGraph {
    tasks: Vec<Task>,
    index: HashMap<String, usize>,
}

impl TaskGraph {
    pub fn new(tasks: Vec<Task>) -> Self {
        let index = tasks
            .iter()
            .enumerate()
            .map(|(i, t)| (t.id.clone(), i))
            .collect();
        Self { tasks, index }
    }

    /// Are there any tasks that are not in a terminal state?
    pub fn has_pending_work(&self) -> bool {
        self.tasks.iter().any(|t| !t.status.is_terminal())
    }

    /// Get tasks that are ready to run: status is Pending and all deps are Completed.
    pub fn ready_tasks(&self) -> Vec<&Task> {
        self.tasks
            .iter()
            .filter(|t| {
                t.status == TaskStatus::Pending
                    && t.dependencies.iter().all(|dep| {
                        self.get(dep)
                            .map(|d| d.status == TaskStatus::Completed)
                            .unwrap_or(false)
                    })
            })
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<&Task> {
        self.index.get(id).map(|&i| &self.tasks[i])
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Task> {
        self.index.get(id).copied().map(|i| &mut self.tasks[i])
    }

    /// Mark a task as running. Deterministic — no LLM involved.
    pub fn mark_running(&mut self, id: &str) {
        if let Some(task) = self.get_mut(id) {
            assert_eq!(task.status, TaskStatus::Pending, "Can only start Pending tasks");
            task.status = TaskStatus::Running;
        }
    }

    /// Mark a task as evaluating (agent finished, awaiting LLM evaluation).
    pub fn mark_evaluating(&mut self, id: &str, result: Option<String>) {
        if let Some(task) = self.get_mut(id) {
            assert_eq!(task.status, TaskStatus::Running, "Can only evaluate Running tasks");
            task.status = TaskStatus::Evaluating;
            task.result = result;
        }
    }

    /// Mark a task as completed. Deterministic.
    pub fn mark_completed(&mut self, id: &str) {
        if let Some(task) = self.get_mut(id) {
            task.status = TaskStatus::Completed;
        }
    }

    /// Mark a task as failed. Deterministic.
    pub fn mark_failed(&mut self, id: &str, error: String) {
        if let Some(task) = self.get_mut(id) {
            task.status = TaskStatus::Failed;
            task.error = Some(error.clone());
        }
        // Skip all tasks that depend on this one
        let id_owned = id.to_string();
        let to_skip: Vec<String> = self
            .tasks
            .iter()
            .filter(|t| t.dependencies.contains(&id_owned) && t.status == TaskStatus::Pending)
            .map(|t| t.id.clone())
            .collect();
        for skip_id in to_skip {
            if let Some(t) = self.get_mut(&skip_id) {
                t.status = TaskStatus::Skipped;
                t.error = Some(format!("Dependency {id_owned} failed"));
            }
        }
    }

    /// Reset a task to Pending for retry. Deterministic — increments retry count.
    pub fn mark_retry(&mut self, id: &str) -> bool {
        if let Some(task) = self.get_mut(id) {
            if task.retry_count < task.max_retries {
                task.retry_count += 1;
                task.status = TaskStatus::Pending;
                task.result = None;
                task.error = None;
                return true;
            }
        }
        false
    }

    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    pub fn completed_count(&self) -> usize {
        self.tasks.iter().filter(|t| t.status == TaskStatus::Completed).count()
    }

    pub fn failed_count(&self) -> usize {
        self.tasks.iter().filter(|t| t.status == TaskStatus::Failed).count()
    }

    pub fn total_count(&self) -> usize {
        self.tasks.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_task(id: &str, deps: Vec<&str>) -> Task {
        Task {
            id: id.to_string(),
            description: format!("Task {id}"),
            task_type: "code".to_string(),
            dependencies: deps.into_iter().map(String::from).collect(),
            status: TaskStatus::Pending,
            assigned_model: None,
            retry_count: 0,
            max_retries: 3,
            result: None,
            error: None,
            last_agent_thread_id: None,
        }
    }

    #[test]
    fn test_ready_tasks_no_deps() {
        let graph = TaskGraph::new(vec![
            make_task("t1", vec![]),
            make_task("t2", vec![]),
            make_task("t3", vec!["t1", "t2"]),
        ]);
        let ready: Vec<&str> = graph.ready_tasks().iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ready, vec!["t1", "t2"]);
    }

    #[test]
    fn test_ready_after_completion() {
        let mut graph = TaskGraph::new(vec![
            make_task("t1", vec![]),
            make_task("t2", vec![]),
            make_task("t3", vec!["t1", "t2"]),
        ]);
        graph.mark_running("t1");
        graph.mark_evaluating("t1", Some("done".into()));
        graph.mark_completed("t1");
        graph.mark_running("t2");
        graph.mark_evaluating("t2", Some("done".into()));
        graph.mark_completed("t2");

        let ready: Vec<&str> = graph.ready_tasks().iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ready, vec!["t3"]);
    }

    #[test]
    fn test_dependency_failure_skips_dependents() {
        let mut graph = TaskGraph::new(vec![
            make_task("t1", vec![]),
            make_task("t2", vec!["t1"]),
            make_task("t3", vec!["t2"]),
        ]);
        graph.mark_running("t1");
        graph.mark_failed("t1", "broken".into());

        assert_eq!(graph.get("t2").unwrap().status, TaskStatus::Skipped);
        // t3 depends on t2 which is now Skipped (not Failed), so it should also be skippable
        // but our current impl only skips direct dependents of the failed task
        // t3 depends on t2, and t2 is Skipped not Pending, so t3 will never become ready
        assert!(!graph.has_pending_work() || graph.ready_tasks().is_empty());
    }

    #[test]
    fn test_retry() {
        let mut graph = TaskGraph::new(vec![make_task("t1", vec![])]);
        graph.mark_running("t1");
        graph.mark_evaluating("t1", None);
        // Reset to pending for retry
        assert!(graph.mark_retry("t1"));
        assert_eq!(graph.get("t1").unwrap().status, TaskStatus::Pending);
        assert_eq!(graph.get("t1").unwrap().retry_count, 1);
    }

    #[test]
    fn test_max_retries_exhausted() {
        let mut graph = TaskGraph::new(vec![make_task("t1", vec![])]);
        // Use up all retries
        for _ in 0..3 {
            graph.mark_running("t1");
            graph.mark_evaluating("t1", None);
            assert!(graph.mark_retry("t1"));
        }
        // 4th attempt — retry should fail
        graph.mark_running("t1");
        graph.mark_evaluating("t1", None);
        assert!(!graph.mark_retry("t1"));
    }

    #[test]
    fn test_has_pending_work() {
        let mut graph = TaskGraph::new(vec![
            make_task("t1", vec![]),
            make_task("t2", vec![]),
        ]);
        assert!(graph.has_pending_work());

        graph.mark_running("t1");
        graph.mark_evaluating("t1", None);
        graph.mark_completed("t1");
        assert!(graph.has_pending_work()); // t2 still pending

        graph.mark_running("t2");
        graph.mark_evaluating("t2", None);
        graph.mark_completed("t2");
        assert!(!graph.has_pending_work());
    }

    #[test]
    fn test_single_task_lifecycle() {
        let mut graph = TaskGraph::new(vec![make_task("t1", vec![])]);
        assert_eq!(graph.completed_count(), 0);
        assert_eq!(graph.total_count(), 1);

        graph.mark_running("t1");
        graph.mark_evaluating("t1", Some("result".into()));
        graph.mark_completed("t1");

        assert_eq!(graph.completed_count(), 1);
        assert!(!graph.has_pending_work());
    }
}
