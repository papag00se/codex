//! Deterministic supervisor loop.
//!
//! The loop continues as long as tasks are pending. No LLM decides whether to continue.
//! LLMs provide judgment (planning, evaluation, interpretation). The loop provides control flow.
//!
//! See docs/spec/design-principles.md.

use crate::task_graph::{Task, TaskGraph, TaskStatus};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tracing::{info, warn};

/// Configuration for the supervisor loop bounds.
#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    pub max_iterations: u32,
    pub timeout: Duration,
    pub max_retries_per_task: u32,
    pub verification_command: Option<String>,
}

impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            max_iterations: 50,
            timeout: Duration::from_secs(7200),
            max_retries_per_task: 3,
            verification_command: None,
        }
    }
}

/// The outcome of the supervisor loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupervisorResult {
    pub completed_tasks: usize,
    pub failed_tasks: usize,
    pub total_tasks: usize,
    pub iterations_used: u32,
    pub termination_reason: TerminationReason,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TerminationReason {
    AllTasksComplete,
    AllTasksTerminal,
    MaxIterationsReached,
    TimeoutReached,
    NoProgress,
}

/// Trait for LLM judgment calls. The supervisor loop calls these but never lets
/// the LLM control whether the loop continues.
///
/// Implement this trait to connect the supervisor to actual LLM providers
/// (or mock it for testing).
/// Result of dispatching a task — includes the output and the agent's thread ID
/// so retries can fork from the previous agent's conversation.
#[derive(Debug, Clone)]
pub struct DispatchResult {
    /// The agent's final output text.
    pub output: String,
    /// The agent's thread ID — stored on the task so retries can resume context.
    pub agent_thread_id: Option<String>,
}

#[allow(async_fn_in_trait)]
pub trait SupervisorJudge {
    /// Decompose a goal into a list of tasks. (LLM judgment: planning)
    async fn plan_tasks(&self, goal: &str) -> Vec<Task>;

    /// Dispatch a task to a sub-agent and wait for it to finish.
    /// Returns the agent's output and thread ID.
    /// The task's `last_agent_thread_id` is available for forking context.
    async fn dispatch_task(&self, task: &Task) -> Result<DispatchResult, String>;

    /// Evaluate whether a task's output means the task is complete.
    /// (LLM judgment: completion evaluation)
    async fn evaluate_completion(&self, task: &Task, output: &str) -> bool;

    /// Run verification command and ask LLM to interpret the result.
    /// Returns true if verification passed.
    /// (Deterministic: run command. LLM judgment: interpret output.)
    async fn verify(&self, task: &Task, verification_command: &str) -> bool;
}

/// Run the deterministic supervisor loop.
///
/// This function does not return until all tasks are terminal, the timeout is reached,
/// or max iterations are exhausted. No LLM can opt out.
pub async fn run_supervisor<J: SupervisorJudge>(
    goal: &str,
    config: &SupervisorConfig,
    judge: &J,
) -> SupervisorResult {
    info!(goal = goal, "Supervisor starting");

    // Phase 1: Ask LLM to decompose goal into tasks (LLM judgment: planning)
    let planned_tasks = judge.plan_tasks(goal).await;
    if planned_tasks.is_empty() {
        info!("Planner returned no tasks — treating goal as single task");
        let single = Task {
            id: "task_1".into(),
            description: goal.to_string(),
            task_type: "code".into(),
            dependencies: vec![],
            status: TaskStatus::Pending,
            assigned_model: None,
            retry_count: 0,
            max_retries: config.max_retries_per_task,
            result: None,
            error: None,
            last_agent_thread_id: None,
        };
        return run_loop(TaskGraph::new(vec![single]), config, judge).await;
    }

    let mut tasks = planned_tasks;
    for t in &mut tasks {
        t.max_retries = config.max_retries_per_task;
    }
    info!(task_count = tasks.len(), "Plan generated");

    run_loop(TaskGraph::new(tasks), config, judge).await
}

/// The actual deterministic loop. No LLM controls this.
async fn run_loop<J: SupervisorJudge>(
    mut graph: TaskGraph,
    config: &SupervisorConfig,
    judge: &J,
) -> SupervisorResult {
    let start = Instant::now();
    let mut iteration: u32 = 0;

    // DETERMINISTIC: loop continues while tasks are pending
    while graph.has_pending_work() {
        // DETERMINISTIC: check bounds
        if iteration >= config.max_iterations {
            warn!(iteration, "Max iterations reached");
            return result(&graph, iteration, TerminationReason::MaxIterationsReached);
        }
        if start.elapsed() >= config.timeout {
            warn!(?config.timeout, "Timeout reached");
            return result(&graph, iteration, TerminationReason::TimeoutReached);
        }

        iteration += 1;
        let ready = graph.ready_tasks().iter().map(|t| t.id.clone()).collect::<Vec<_>>();

        if ready.is_empty() {
            // No ready tasks but work is pending — tasks are running or blocked
            // on deps that will never complete (cascading skip didn't reach them).
            // This means no progress is possible.
            if graph.tasks().iter().all(|t| t.status.is_terminal() || t.status == TaskStatus::Pending) {
                // All non-terminal tasks are Pending but none are ready — stuck
                warn!(iteration, "No progress possible — stuck tasks");
                return result(&graph, iteration, TerminationReason::NoProgress);
            }
            // Some tasks are Running/Evaluating — wait and retry
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        }

        // DETERMINISTIC: dispatch all ready tasks
        for task_id in ready {
            graph.mark_running(&task_id);
            info!(task = %task_id, iteration, "Dispatching task");

            // LLM JUDGMENT: agent executes the task
            let task = graph.get(&task_id).unwrap().clone();
            let dispatch_result = judge.dispatch_task(&task).await;

            match dispatch_result {
                Ok(dr) => {
                    // Store the agent's thread ID so retries can fork from it
                    if let Some(ref tid) = dr.agent_thread_id {
                        if let Some(t) = graph.get_mut(&task_id) {
                            t.last_agent_thread_id = Some(tid.clone());
                        }
                    }
                    let output = dr.output;
                    graph.mark_evaluating(&task_id, Some(output.clone()));

                    // LLM JUDGMENT: is the task complete?
                    let task = graph.get(&task_id).unwrap().clone();
                    let is_complete = judge.evaluate_completion(&task, &output).await;

                    if is_complete {
                        // DETERMINISTIC: run verification if configured
                        if let Some(ref cmd) = config.verification_command {
                            let verified = judge.verify(&task, cmd).await;
                            if verified {
                                graph.mark_completed(&task_id);
                                info!(task = %task_id, "Task completed and verified");
                            } else {
                                // DETERMINISTIC: retry or fail
                                if graph.mark_retry(&task_id) {
                                    let task = graph.get(&task_id).unwrap();
                                    info!(task = %task_id, retry = task.retry_count, "Verification failed — retrying");
                                } else {
                                    graph.mark_failed(&task_id, "Verification failed after max retries".into());
                                    warn!(task = %task_id, "Task failed — verification exhausted retries");
                                }
                            }
                        } else {
                            graph.mark_completed(&task_id);
                            info!(task = %task_id, "Task completed (no verification configured)");
                        }
                    } else {
                        // LLM says task not complete — retry
                        if graph.mark_retry(&task_id) {
                            let task = graph.get(&task_id).unwrap();
                            info!(task = %task_id, retry = task.retry_count, "Task incomplete — retrying");
                        } else {
                            graph.mark_failed(&task_id, "Task incomplete after max retries".into());
                            warn!(task = %task_id, "Task failed — incomplete after max retries");
                        }
                    }
                }
                Err(error) => {
                    graph.mark_evaluating(&task_id, None);
                    // DETERMINISTIC: retry or fail
                    if graph.mark_retry(&task_id) {
                        let task = graph.get(&task_id).unwrap();
                        info!(task = %task_id, retry = task.retry_count, error = %error, "Task errored — retrying");
                    } else {
                        graph.mark_failed(&task_id, error.clone());
                        warn!(task = %task_id, error = %error, "Task failed — errored after max retries");
                    }
                }
            }
        }
    }

    result(&graph, iteration, if graph.failed_count() > 0 {
        TerminationReason::AllTasksTerminal
    } else {
        TerminationReason::AllTasksComplete
    })
}

fn result(graph: &TaskGraph, iterations: u32, reason: TerminationReason) -> SupervisorResult {
    SupervisorResult {
        completed_tasks: graph.completed_count(),
        failed_tasks: graph.failed_count(),
        total_tasks: graph.total_count(),
        iterations_used: iterations,
        termination_reason: reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock judge that always succeeds.
    struct AlwaysSucceedsJudge;

    impl SupervisorJudge for AlwaysSucceedsJudge {
        async fn plan_tasks(&self, _goal: &str) -> Vec<Task> {
            vec![
                Task {
                    id: "t1".into(),
                    description: "First task".into(),
                    task_type: "code".into(),
                    dependencies: vec![],
                    status: TaskStatus::Pending,
                    assigned_model: None,
                    retry_count: 0,
                    max_retries: 3,
                    result: None,
                    error: None,
            last_agent_thread_id: None,
                },
                Task {
                    id: "t2".into(),
                    description: "Second task".into(),
                    task_type: "code".into(),
                    dependencies: vec!["t1".into()],
                    status: TaskStatus::Pending,
                    assigned_model: None,
                    retry_count: 0,
                    max_retries: 3,
                    result: None,
                    error: None,
            last_agent_thread_id: None,
                },
            ]
        }

        async fn dispatch_task(&self, _task: &Task) -> Result<DispatchResult, String> {
            Ok(DispatchResult { output: "done".into(), agent_thread_id: None })
        }

        async fn evaluate_completion(&self, _task: &Task, _output: &str) -> bool {
            true
        }

        async fn verify(&self, _task: &Task, _cmd: &str) -> bool {
            true
        }
    }

    /// Mock judge where the first attempt fails, second succeeds.
    struct FailsThenSucceedsJudge;

    impl SupervisorJudge for FailsThenSucceedsJudge {
        async fn plan_tasks(&self, _goal: &str) -> Vec<Task> {
            vec![Task {
                id: "t1".into(),
                description: "Flaky task".into(),
                task_type: "code".into(),
                dependencies: vec![],
                status: TaskStatus::Pending,
                assigned_model: None,
                retry_count: 0,
                max_retries: 3,
                result: None,
                error: None,
            last_agent_thread_id: None,
            }]
        }

        async fn dispatch_task(&self, task: &Task) -> Result<DispatchResult, String> {
            if task.retry_count == 0 {
                Err("first attempt fails".into())
            } else {
                Ok(DispatchResult { output: "done on retry".into(), agent_thread_id: None })
            }
        }

        async fn evaluate_completion(&self, _task: &Task, _output: &str) -> bool {
            true
        }

        async fn verify(&self, _task: &Task, _cmd: &str) -> bool {
            true
        }
    }

    /// Mock judge that always fails.
    struct AlwaysFailsJudge;

    impl SupervisorJudge for AlwaysFailsJudge {
        async fn plan_tasks(&self, _goal: &str) -> Vec<Task> {
            vec![Task {
                id: "t1".into(),
                description: "Doomed task".into(),
                task_type: "code".into(),
                dependencies: vec![],
                status: TaskStatus::Pending,
                assigned_model: None,
                retry_count: 0,
                max_retries: 2,
                result: None,
                error: None,
            last_agent_thread_id: None,
            }]
        }

        async fn dispatch_task(&self, _task: &Task) -> Result<DispatchResult, String> {
            Err("always fails".into())
        }

        async fn evaluate_completion(&self, _task: &Task, _output: &str) -> bool {
            false
        }

        async fn verify(&self, _task: &Task, _cmd: &str) -> bool {
            false
        }
    }

    #[tokio::test]
    async fn test_happy_path_two_sequential_tasks() {
        let config = SupervisorConfig::default();
        let result = run_supervisor("do stuff", &config, &AlwaysSucceedsJudge).await;
        assert_eq!(result.completed_tasks, 2);
        assert_eq!(result.failed_tasks, 0);
        assert!(matches!(result.termination_reason, TerminationReason::AllTasksComplete));
    }

    #[tokio::test]
    async fn test_retry_then_succeed() {
        let config = SupervisorConfig::default();
        let result = run_supervisor("flaky work", &config, &FailsThenSucceedsJudge).await;
        assert_eq!(result.completed_tasks, 1);
        assert_eq!(result.failed_tasks, 0);
        assert!(matches!(result.termination_reason, TerminationReason::AllTasksComplete));
        assert!(result.iterations_used >= 2); // At least one retry
    }

    #[tokio::test]
    async fn test_max_retries_exhausted() {
        let config = SupervisorConfig {
            max_retries_per_task: 2,
            ..Default::default()
        };
        let result = run_supervisor("doomed", &config, &AlwaysFailsJudge).await;
        assert_eq!(result.completed_tasks, 0);
        assert_eq!(result.failed_tasks, 1);
        assert!(matches!(result.termination_reason, TerminationReason::AllTasksTerminal));
    }

    #[tokio::test]
    async fn test_max_iterations_bound() {
        // Judge that returns empty plan → single task, always says incomplete
        struct NeverDoneJudge;
        impl SupervisorJudge for NeverDoneJudge {
            async fn plan_tasks(&self, _goal: &str) -> Vec<Task> { vec![] }
            async fn dispatch_task(&self, _task: &Task) -> Result<DispatchResult, String> {
                Ok(DispatchResult { output: "partial".into(), agent_thread_id: None })
            }
            async fn evaluate_completion(&self, _task: &Task, _output: &str) -> bool {
                false // Never complete — but loop doesn't care, it retries
            }
            async fn verify(&self, _task: &Task, _cmd: &str) -> bool { false }
        }

        let config = SupervisorConfig {
            max_iterations: 5,
            max_retries_per_task: 100, // High so we hit max_iterations first
            ..Default::default()
        };
        let result = run_supervisor("infinite work", &config, &NeverDoneJudge).await;
        assert!(matches!(result.termination_reason, TerminationReason::MaxIterationsReached));
        assert!(result.iterations_used <= 5);
    }

    #[tokio::test]
    async fn test_empty_plan_becomes_single_task() {
        struct EmptyPlanJudge;
        impl SupervisorJudge for EmptyPlanJudge {
            async fn plan_tasks(&self, _goal: &str) -> Vec<Task> { vec![] }
            async fn dispatch_task(&self, _task: &Task) -> Result<DispatchResult, String> {
                Ok(DispatchResult { output: "done".into(), agent_thread_id: None })
            }
            async fn evaluate_completion(&self, _task: &Task, _output: &str) -> bool { true }
            async fn verify(&self, _task: &Task, _cmd: &str) -> bool { true }
        }

        let config = SupervisorConfig::default();
        let result = run_supervisor("simple goal", &config, &EmptyPlanJudge).await;
        assert_eq!(result.completed_tasks, 1);
        assert_eq!(result.total_tasks, 1);
    }

    #[tokio::test]
    async fn test_verification_failure_triggers_retry() {
        struct VerificationFailsOnceJudge { call_count: std::sync::atomic::AtomicU32 }
        impl SupervisorJudge for VerificationFailsOnceJudge {
            async fn plan_tasks(&self, _goal: &str) -> Vec<Task> { vec![] }
            async fn dispatch_task(&self, _task: &Task) -> Result<DispatchResult, String> {
                Ok(DispatchResult { output: "done".into(), agent_thread_id: None })
            }
            async fn evaluate_completion(&self, _task: &Task, _output: &str) -> bool { true }
            async fn verify(&self, _task: &Task, _cmd: &str) -> bool {
                let count = self.call_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                count > 0 // First call fails, rest succeed
            }
        }

        let config = SupervisorConfig {
            verification_command: Some("pytest".into()),
            ..Default::default()
        };
        let judge = VerificationFailsOnceJudge {
            call_count: std::sync::atomic::AtomicU32::new(0),
        };
        let result = run_supervisor("test me", &config, &judge).await;
        assert_eq!(result.completed_tasks, 1);
        assert!(result.iterations_used >= 2);
    }
}
