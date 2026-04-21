//! Render durable memory and session handoff — ported from compaction/durable_memory.py.

use super::models::{ChunkExtraction, DurableMemorySet, MergedState, SessionHandoff};

/// Build a session handoff from merged state.
pub fn build_handoff(
    state: &MergedState,
    recent_raw_turns: Vec<serde_json::Value>,
    current_request: &str,
) -> SessionHandoff {
    SessionHandoff {
        stable_task_definition: state.objective.clone(),
        repo_state: state.repo_state.clone(),
        key_decisions: state.accepted_fixes.clone(),
        unresolved_work: [state.pending_todos.clone(), state.unresolved_bugs.clone()].concat(),
        latest_plan: state.latest_plan.clone(),
        failures_to_avoid: [state.errors.clone(), state.rejected_ideas.clone()].concat(),
        recent_raw_turns,
        current_request: current_request.into(),
    }
}

/// Render durable memory files from merged state.
pub fn render_durable_memory(
    state: &MergedState,
    recent_raw_turns: &[serde_json::Value],
    current_request: &str,
) -> DurableMemorySet {
    DurableMemorySet {
        task_state: render_section(
            "Task State",
            &[
                ("Objective", &[state.objective.clone()]),
                ("Files Touched", &state.files_touched),
                ("Commands Run", &state.commands_run),
                ("Test Status", &state.test_status),
            ],
        ),
        decisions: render_section(
            "Decisions",
            &[
                ("Accepted Fixes", &state.accepted_fixes),
                ("Constraints", &state.constraints),
            ],
        ),
        failures_to_avoid: render_section(
            "Failures To Avoid",
            &[
                ("Errors", &state.errors),
                ("Rejected Ideas", &state.rejected_ideas),
            ],
        ),
        next_steps: render_section(
            "Next Steps",
            &[
                ("Pending TODOs", &state.pending_todos),
                ("Latest Plan", &state.latest_plan),
            ],
        ),
        session_handoff: render_section(
            "Session Handoff",
            &[
                ("Stable Task Definition", &[state.objective.clone()]),
                (
                    "Unresolved Work",
                    &[state.pending_todos.clone(), state.unresolved_bugs.clone()].concat(),
                ),
            ],
        ),
    }
}

/// Render an inline compaction summary (what the model sees).
pub fn render_compaction_summary(memory: &DurableMemorySet, current_request: &str) -> String {
    let mut parts = Vec::new();

    for (name, content) in [
        ("TASK_STATE", &memory.task_state),
        ("DECISIONS", &memory.decisions),
        ("FAILURES_TO_AVOID", &memory.failures_to_avoid),
        ("NEXT_STEPS", &memory.next_steps),
        ("SESSION_HANDOFF", &memory.session_handoff),
    ] {
        if !content.is_empty() {
            parts.push(format!("### {name}\n{content}"));
        }
    }

    if !current_request.is_empty() {
        parts.push(format!("# Current Request\n{current_request}"));
    }

    parts.join("\n\n")
}

fn render_section(title: &str, sections: &[(&str, &[String])]) -> String {
    let mut lines = vec![format!("# {title}")];
    for (heading, items) in sections {
        lines.push(format!("## {heading}"));
        if items.is_empty() {
            lines.push("- none".into());
        } else {
            for item in *items {
                if !item.is_empty() {
                    lines.push(format!("- {item}"));
                }
            }
        }
    }
    lines.push(String::new()); // trailing newline
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_handoff() {
        let state = MergedState {
            objective: "Fix auth".into(),
            accepted_fixes: vec!["Fixed login".into()],
            pending_todos: vec!["Add tests".into()],
            errors: vec!["Crash on startup".into()],
            ..Default::default()
        };
        let handoff = build_handoff(&state, vec![], "continue");
        assert_eq!(handoff.stable_task_definition, "Fix auth");
        assert_eq!(handoff.key_decisions, vec!["Fixed login"]);
        assert_eq!(handoff.unresolved_work, vec!["Add tests"]);
        assert_eq!(handoff.failures_to_avoid, vec!["Crash on startup"]);
    }

    #[test]
    fn test_render_memory() {
        let state = MergedState {
            objective: "Fix auth".into(),
            files_touched: vec!["auth.py".into()],
            accepted_fixes: vec!["Fixed login".into()],
            ..Default::default()
        };
        let memory = render_durable_memory(&state, &[], "continue");
        assert!(memory.task_state.contains("Fix auth"));
        assert!(memory.task_state.contains("auth.py"));
        assert!(memory.decisions.contains("Fixed login"));
    }
}
