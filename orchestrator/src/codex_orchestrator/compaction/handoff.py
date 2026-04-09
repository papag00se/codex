from __future__ import annotations

import json
from typing import Any, Dict

from pydantic import ValidationError

from ..prompt_loader import render_prompt
from .models import CodexHandoffFlow, DurableMemorySet, SessionHandoff

def validate_codex_handoff_flow(flow: CodexHandoffFlow | Dict[str, Any]) -> CodexHandoffFlow:
    validated = flow if isinstance(flow, CodexHandoffFlow) else CodexHandoffFlow.model_validate(flow)
    for item in validated.durable_memory:
        if not item.get("name"):
            raise ValueError("codex handoff durable memory entries require a non-empty name")
    try:
        SessionHandoff.model_validate(
            {
                **validated.structured_handoff,
                "recent_raw_turns": validated.recent_raw_turns,
                "current_request": validated.current_request,
            }
        )
    except ValidationError as exc:
        raise ValueError("codex handoff structured payload failed schema validation") from exc
    try:
        json.dumps(validated.structured_handoff, ensure_ascii=False)
        json.dumps(validated.recent_raw_turns, ensure_ascii=False)
    except (TypeError, ValueError) as exc:
        raise ValueError("codex handoff payload is not JSON-serializable") from exc
    return validated


def build_codex_handoff_flow(memory: DurableMemorySet, handoff: SessionHandoff, *, current_request: str | None = None) -> CodexHandoffFlow:
    session_handoff = _strip_current_request_section(memory.session_handoff)
    return validate_codex_handoff_flow(
        CodexHandoffFlow(
        durable_memory=[
            {"name": "TASK_STATE.md", "content": memory.task_state},
            {"name": "DECISIONS.md", "content": memory.decisions},
            {"name": "FAILURES_TO_AVOID.md", "content": memory.failures_to_avoid},
            {"name": "NEXT_STEPS.md", "content": memory.next_steps},
            {"name": "SESSION_HANDOFF.md", "content": session_handoff},
        ],
        structured_handoff=handoff.model_dump(exclude={"recent_raw_turns", "current_request"}),
        recent_raw_turns=handoff.recent_raw_turns,
        current_request=current_request if current_request is not None else handoff.current_request,
        )
    )


def render_compacted_flow(flow: CodexHandoffFlow | Dict[str, Any], *, current_request: str = "") -> str:
    payload = validate_codex_handoff_flow(flow).model_dump()
    durable_memory_blocks = "\n\n".join(
        f"### {item['name']}\n{item['content'].rstrip()}" for item in payload.get("durable_memory") or []
    )
    return render_prompt(
        "compacted_flow.md",
        {
            "DURABLE_MEMORY_BLOCKS": durable_memory_blocks,
            "STRUCTURED_HANDOFF": json.dumps(payload.get("structured_handoff") or {}, ensure_ascii=False, indent=2),
            "RECENT_RAW_TURNS": json.dumps(payload.get("recent_raw_turns") or [], ensure_ascii=False, indent=2),
            "CURRENT_REQUEST": current_request or payload.get("current_request") or "",
        },
    )


def render_inline_compaction_summary(flow: CodexHandoffFlow | Dict[str, Any], *, current_request: str = "") -> str:
    payload = validate_codex_handoff_flow(flow).model_dump()

    sections = [
        item["content"].rstrip()
        for item in payload.get("durable_memory") or []
        if isinstance(item, dict) and isinstance(item.get("content"), str) and item["content"].strip()
    ]
    request_text = current_request or payload.get("current_request") or ""
    if request_text:
        sections.append(f"# Current Request\n{request_text}".rstrip())
    return "\n\n".join(section for section in sections if section).strip()


def render_codex_support_prompt(flow: CodexHandoffFlow, *, system: str = "", current_request: str = "") -> str:
    validated = validate_codex_handoff_flow(flow)
    request_text = current_request or validated.current_request
    durable_memory_blocks = "\n\n".join(
        f"### {item['name']}\n{item['content'].rstrip()}" for item in validated.durable_memory
    )
    return render_prompt(
        'codex_support_prompt.md',
        {
            'SYSTEM_SECTION': f"System instructions:\n{system}\n\n" if system else "",
            'DURABLE_MEMORY_BLOCKS': durable_memory_blocks,
            'STRUCTURED_HANDOFF': json.dumps(validated.structured_handoff, ensure_ascii=False, indent=2),
            'RECENT_RAW_TURNS': json.dumps(validated.recent_raw_turns, ensure_ascii=False, indent=2),
            'CURRENT_REQUEST': request_text,
        },
    )


def _strip_current_request_section(content: str) -> str:
    if "## Current Request" not in content:
        return content

    lines = content.splitlines()
    filtered: list[str] = []
    skipping = False
    for line in lines:
        if line == "## Current Request":
            skipping = True
            continue
        if skipping and line.startswith("## "):
            skipping = False
        if not skipping:
            filtered.append(line)

    normalized = "\n".join(filtered).rstrip()
    return normalized + "\n" if normalized else ""
