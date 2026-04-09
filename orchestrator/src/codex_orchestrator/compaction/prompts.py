from __future__ import annotations

import json
from typing import Any, Dict, List, Optional

from ..schemas.config import settings
from ..providers.responses_compat import build_chat_request_payload
from ..prompt_loader import load_prompt
from ..routing.metrics import estimate_model_tokens
from .models import ChunkExtraction, MergedState, TranscriptChunk
from .structured_output import chunk_extraction_response_schema, recent_state_response_schema


EXTRACTION_SYSTEM_PROMPT = load_prompt('compaction_extraction_system.md')
REFINEMENT_SYSTEM_PROMPT = load_prompt('compaction_refinement_system.md')


def build_extraction_payload(chunk: TranscriptChunk, repo_context: Optional[Dict[str, Any]] = None) -> str:
    return json.dumps(
        {
            "task": "Extract chunk-local durable coding-session state.",
            "output_contract": {
                "format": "json_object_only",
                "required_keys": list(ChunkExtraction.model_fields.keys()),
            },
            "chunk": _compact_chunk_payload(chunk),
            "repo_context": repo_context or {},
        },
        ensure_ascii=False,
        separators=(",", ":"),
    )


def estimate_extraction_request_tokens(
    chunk: TranscriptChunk,
    repo_context: Optional[Dict[str, Any]] = None,
    *,
    model: Optional[str] = None,
) -> int:
    payload = build_extraction_payload(chunk, repo_context)
    request = build_chat_request_payload(
        model or settings.compactor_model,
        [{"role": "user", "content": payload}],
        system=EXTRACTION_SYSTEM_PROMPT,
        response_format=chunk_extraction_response_schema(),
    )
    return estimate_model_tokens(request, model=model or settings.compactor_model)


def build_refinement_payload(
    state: MergedState,
    recent_raw_turns: List[Dict[str, object]],
    current_request: str,
    repo_context: Optional[Dict[str, Any]] = None,
) -> str:
    _ = state
    return json.dumps(
        {
            "task": "Extract durable coding-session state from newer recent raw turns so it can be merged onto the existing merged state.",
            "output_contract": {
                "format": "json_object_only",
                "required_keys": [
                    key
                    for key in ChunkExtraction.model_fields.keys()
                    if key not in {"chunk_id", "source_token_count"}
                ],
            },
            "recent_events": _compact_transcript_events(recent_raw_turns),
            "current_request": current_request,
            "repo_context": repo_context or {},
        },
        ensure_ascii=False,
        separators=(",", ":"),
    )


def estimate_refinement_request_tokens(
    state: MergedState,
    recent_raw_turns: List[Dict[str, object]],
    current_request: str,
    repo_context: Optional[Dict[str, Any]] = None,
    *,
    model: Optional[str] = None,
) -> int:
    payload = build_refinement_payload(state, recent_raw_turns, current_request, repo_context)
    request = build_chat_request_payload(
        model or settings.compactor_model,
        [{"role": "user", "content": payload}],
        system=REFINEMENT_SYSTEM_PROMPT,
        response_format=recent_state_response_schema(),
    )
    return estimate_model_tokens(request, model=model or settings.compactor_model)


def _compact_chunk_payload(chunk: TranscriptChunk) -> Dict[str, Any]:
    return {
        "id": chunk.chunk_id,
        "start": chunk.start_index,
        "end": chunk.end_index,
        "tok": chunk.token_count,
        "ov": chunk.overlap_from_previous_tokens,
        "events": _compact_transcript_events(chunk.items),
    }


def _compact_transcript_events(items: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    events: List[Dict[str, Any]] = []
    current_workdir: Optional[str] = None
    for item in items:
        role = _compact_role(item.get("role"))
        content = item.get("content")
        if isinstance(content, str):
            event = _message_event(role, content)
            if event is not None:
                events.append(event)
            continue
        if not isinstance(content, list):
            continue
        for block in content:
            compacted, current_workdir = _compact_block(role, block, current_workdir)
            if compacted is None:
                continue
            if isinstance(compacted, list):
                events.extend(compacted)
            else:
                events.append(compacted)
    return events


def _compact_role(role: Any) -> str:
    if role == "user":
        return "u"
    if role == "assistant":
        return "a"
    if isinstance(role, str) and role:
        return role[:1]
    return "?"


def _message_event(role: str, text: Any) -> Optional[Dict[str, Any]]:
    if not isinstance(text, str):
        return None
    content = text.strip()
    if not content:
        return None
    return {"r": role, "k": "msg", "c": content}


def _compact_block(
    role: str,
    block: Any,
    current_workdir: Optional[str],
) -> tuple[Optional[Dict[str, Any] | List[Dict[str, Any]]], Optional[str]]:
    if not isinstance(block, dict):
        return None, current_workdir
    block_type = block.get("type")
    if block_type in {"text", "input_text", "output_text"}:
        return _message_event(role, block.get("text") or block.get("input_text") or block.get("output_text")), current_workdir
    if block_type in {"tool_use", "function_call"}:
        return _compact_tool_call(role, block, current_workdir)
    if block_type in {"tool_result", "function_call_output"}:
        output = block.get("output") or block.get("content")
        event = _message_event(role, output)
        if event is not None:
            event["k"] = "out"
        return event, current_workdir
    return None, current_workdir


def _compact_tool_call(
    role: str,
    block: Dict[str, Any],
    current_workdir: Optional[str],
) -> tuple[Optional[Dict[str, Any] | List[Dict[str, Any]]], Optional[str]]:
    name = block.get("name")
    raw_input = block.get("input")
    if raw_input is None:
        raw_input = block.get("arguments")
    payload = _parse_tool_payload(raw_input)
    if name == "exec_command":
        return _compact_exec_command(role, payload, current_workdir)
    if name == "write_stdin":
        return _compact_write_stdin(role, payload), current_workdir
    if name == "update_plan":
        steps = []
        for step in payload.get("plan", []) if isinstance(payload, dict) else []:
            if not isinstance(step, dict):
                continue
            text = str(step.get("step") or "").strip()
            status = str(step.get("status") or "").strip()
            if text and status:
                steps.append(f"{text} [{status}]")
            elif text:
                steps.append(text)
        if not steps:
            return None, current_workdir
        return {"r": role, "k": "plan", "steps": steps}, current_workdir
    compacted_payload = _compact_tool_payload(payload)
    event = {"r": role, "k": "call", "n": name}
    if compacted_payload:
        event["a"] = compacted_payload
    return event, current_workdir


def _compact_exec_command(
    role: str,
    payload: Dict[str, Any],
    current_workdir: Optional[str],
) -> tuple[Optional[Dict[str, Any]], Optional[str]]:
    cmd = payload.get("cmd")
    if not isinstance(cmd, str) or not cmd.strip():
        return None, current_workdir
    event: Dict[str, Any] = {"r": role, "k": "cmd", "c": cmd.strip()}
    workdir = payload.get("workdir")
    next_workdir = current_workdir
    if isinstance(workdir, str) and workdir.strip():
        normalized = workdir.strip()
        if normalized != current_workdir:
            event["wd"] = normalized
        next_workdir = normalized
    return event, next_workdir


def _compact_write_stdin(role: str, payload: Dict[str, Any]) -> Optional[Dict[str, Any]]:
    session_id = payload.get("session_id")
    if session_id is None:
        return None
    chars = payload.get("chars")
    if isinstance(chars, str) and chars:
        return {"r": role, "k": "stdin", "sid": session_id, "c": chars}
    return {"r": role, "k": "poll", "sid": session_id}


def _parse_tool_payload(raw_input: Any) -> Dict[str, Any]:
    if isinstance(raw_input, dict):
        return raw_input
    if isinstance(raw_input, str):
        try:
            parsed = json.loads(raw_input)
        except json.JSONDecodeError:
            return {"raw": raw_input}
        if isinstance(parsed, dict):
            return parsed
        return {"raw": raw_input}
    return {}


def _compact_tool_payload(payload: Dict[str, Any]) -> Any:
    compacted: Dict[str, Any] = {}
    for key, value in payload.items():
        if key in {"yield_time_ms", "max_output_tokens", "max_output_tokens", "yield_time_ms"}:
            continue
        if value in (None, "", [], {}):
            continue
        if isinstance(value, (str, int, float, bool)):
            compacted[key] = value
            continue
        compacted[key] = json.dumps(value, ensure_ascii=False, separators=(",", ":"))
    return compacted
