from __future__ import annotations

from typing import Any, Dict, List

from .models import DurableMemorySet, MergedState, SessionHandoff


def build_session_handoff(state: MergedState, recent_raw_turns: List[Dict[str, Any]], current_request: str) -> SessionHandoff:
    return SessionHandoff(
        stable_task_definition=state.objective,
        repo_state=state.repo_state,
        key_decisions=state.accepted_fixes,
        unresolved_work=[*state.pending_todos, *state.unresolved_bugs],
        latest_plan=state.latest_plan,
        failures_to_avoid=[*state.errors, *state.rejected_ideas],
        recent_raw_turns=recent_raw_turns,
        current_request=current_request,
    )


def render_durable_memory(state: MergedState, recent_raw_turns: List[Dict[str, Any]], current_request: str) -> DurableMemorySet:
    handoff = build_session_handoff(state, recent_raw_turns, current_request)
    return DurableMemorySet(
        task_state=_render_sections(
            "Task State",
            [
                ("Objective", [state.objective] if state.objective else []),
                ("Files Touched", state.files_touched),
                ("Commands Run", state.commands_run),
                ("Test Status", state.test_status),
            ],
        ),
        decisions=_render_sections("Decisions", [("Accepted Fixes", state.accepted_fixes), ("Constraints", state.constraints)]),
        failures_to_avoid=_render_sections("Failures To Avoid", [("Errors", state.errors), ("Rejected Ideas", state.rejected_ideas)]),
        next_steps=_render_sections("Next Steps", [("Pending TODOs", state.pending_todos), ("Latest Plan", state.latest_plan)]),
        session_handoff=_render_sections(
            "Session Handoff",
            [
                ("Stable Task Definition", [handoff.stable_task_definition] if handoff.stable_task_definition else []),
                ("Unresolved Work", handoff.unresolved_work),
            ],
        ),
    )


def _render_sections(title: str, sections: List[tuple[str, List[str]]]) -> str:
    lines = [f"# {title}"]
    for heading, items in sections:
        lines.append(f"## {heading}")
        if items:
            lines.extend(f"- {item}" for item in items)
        else:
            lines.append("- none")
    return "\n".join(lines) + "\n"
