from __future__ import annotations

import json
from typing import Any, Dict, Iterable, List, Mapping
from uuid import uuid4


def is_devstral_model(model: str) -> bool:
    return "devstral" in model.lower()


def normalize_ollama_tools(tools: List[Dict[str, Any]] | None) -> List[Dict[str, Any]] | None:
    if not tools:
        return None

    normalized: List[Dict[str, Any]] = []
    for tool in tools:
        if not isinstance(tool, dict):
            continue
        if tool.get("type") == "function" and isinstance(tool.get("function"), dict):
            normalized.append(tool)
            continue

        name = tool.get("name")
        if not name:
            continue

        normalized.append(
            {
                "type": "function",
                "function": {
                    "name": name,
                    "description": tool.get("description", ""),
                    "parameters": tool.get("input_schema", tool.get("parameters", {"type": "object", "properties": {}})),
                },
            }
        )
    return normalized or None


def anthropic_messages_to_ollama(messages: Iterable[Any]) -> List[Dict[str, Any]]:
    adapted: List[Dict[str, Any]] = []
    for message in messages:
        role = getattr(message, "role", None) or message.get("role")
        content = getattr(message, "content", None) if hasattr(message, "content") else message.get("content")
        if isinstance(content, str):
            adapted.append({"role": role, "content": content})
            continue
        if not isinstance(content, list):
            adapted.append({"role": role, "content": _stringify(content)})
            continue

        pending_text: List[str] = []
        pending_tool_calls: List[Dict[str, Any]] = []

        def flush() -> None:
            if role == "assistant" and (pending_text or pending_tool_calls):
                payload: Dict[str, Any] = {"role": "assistant"}
                if pending_text:
                    payload["content"] = "\n".join(pending_text)
                if pending_tool_calls:
                    payload["tool_calls"] = list(pending_tool_calls)
                adapted.append(payload)
            elif pending_text:
                adapted.append({"role": role, "content": "\n".join(pending_text)})
            pending_text.clear()
            pending_tool_calls.clear()

        for block in content:
            if not isinstance(block, dict):
                pending_text.append(str(block))
                continue

            block_type = block.get("type")
            if block_type == "text":
                pending_text.append(block.get("text", ""))
                continue

            if role == "assistant" and block_type == "tool_use":
                pending_tool_calls.append(
                    {
                        "type": "function",
                        "id": block.get("id"),
                        "function": {
                            "name": block.get("name", ""),
                            "arguments": block.get("input", {}),
                        },
                    }
                )
                continue

            if role == "user" and block_type == "tool_result":
                flush()
                adapted.append(
                    {
                        "role": "tool",
                        "tool_name": block.get("name") or block.get("tool_name") or block.get("tool_use_name") or block.get("tool_use_id") or "tool",
                        "content": _stringify(block.get("content")),
                    }
                )
                continue

            pending_text.append(_stringify(block))

        flush()
    return adapted


def recover_ollama_message(message: Dict[str, Any]) -> Dict[str, Any]:
    if not isinstance(message, dict) or message.get("tool_calls"):
        return message

    content = message.get("content")
    if not isinstance(content, str):
        return message

    parsed = _parse_json_blob(content)
    if isinstance(parsed, dict):
        tool_calls = _normalize_tool_calls(parsed.get("tool_calls"))
        if tool_calls:
            recovered = dict(message)
            recovered["tool_calls"] = tool_calls
            recovered["content"] = parsed.get("content", "")
            return recovered

    recovered_text, embedded_tool_calls = _recover_embedded_tool_blocks(content)
    if not embedded_tool_calls:
        return message

    recovered = dict(message)
    recovered["tool_calls"] = embedded_tool_calls
    recovered["content"] = recovered_text
    return recovered


def recover_stream_ollama_message(message: Dict[str, Any]) -> Dict[str, Any]:
    if not isinstance(message, dict) or message.get("tool_calls"):
        return message

    content = message.get("content")
    if not isinstance(content, str):
        return message

    parsed = _parse_json_blob(content)
    if isinstance(parsed, dict):
        tool_calls = _normalize_tool_calls(parsed.get("tool_calls"))
        if tool_calls:
            recovered = dict(message)
            recovered["tool_calls"] = tool_calls
            recovered["content"] = parsed.get("content", "")
            return recovered

    recovered_text, embedded_tool_calls = _recover_embedded_tool_blocks(content, streaming=True)
    recovered = dict(message)
    recovered["content"] = recovered_text
    if embedded_tool_calls:
        recovered["tool_calls"] = embedded_tool_calls
    return recovered


def ollama_message_to_anthropic_content(message: Dict[str, Any]) -> List[Dict[str, Any]]:
    normalized = recover_ollama_message(message)
    blocks: List[Dict[str, Any]] = []

    content = normalized.get("content")
    if isinstance(content, str) and content:
        blocks.append({"type": "text", "text": content})

    for tool_call in _normalize_tool_calls(normalized.get("tool_calls")):
        function = tool_call.get("function", {})
        blocks.append(
            {
                "type": "tool_use",
                "id": tool_call.get("id") or f"toolu_{uuid4().hex[:24]}",
                "name": function.get("name", ""),
                "input": function.get("arguments", {}),
            }
        )

    return blocks or [{"type": "text", "text": ""}]


def _normalize_tool_calls(tool_calls: Any) -> List[Dict[str, Any]]:
    if not isinstance(tool_calls, list):
        return []

    normalized: List[Dict[str, Any]] = []
    for tool_call in tool_calls:
        if not isinstance(tool_call, dict):
            continue

        if isinstance(tool_call.get("function"), dict):
            function = dict(tool_call["function"])
        else:
            function = {
                "name": tool_call.get("name", ""),
                "arguments": tool_call.get("arguments", {}),
            }

        arguments = function.get("arguments", {})
        if isinstance(arguments, str):
            try:
                arguments = json.loads(arguments)
            except json.JSONDecodeError:
                arguments = {"raw": arguments}

        normalized.append(
            {
                "type": "function",
                "id": tool_call.get("id"),
                "function": {
                    "name": function.get("name", ""),
                    "arguments": arguments if isinstance(arguments, dict) else {"value": arguments},
                },
            }
        )
    return normalized


def _parse_json_blob(content: str) -> Dict[str, Any] | None:
    text = content.strip()
    if text.startswith("```"):
        lines = text.splitlines()
        if len(lines) >= 3:
            text = "\n".join(lines[1:-1]).strip()
    if not text.startswith("{"):
        return None
    try:
        parsed = json.loads(text)
    except json.JSONDecodeError:
        return None
    return parsed if isinstance(parsed, dict) else None


def _recover_embedded_tool_blocks(content: str, *, streaming: bool = False) -> tuple[str, List[Dict[str, Any]]]:
    paragraphs = [part.strip() for part in content.split("\n\n")]
    kept: List[str] = []
    tool_calls: List[Dict[str, Any]] = []

    for index, paragraph in enumerate(paragraphs):
        candidate = paragraph
        if candidate.startswith("[USER]\n") or candidate.startswith("[ASSISTANT]\n"):
            candidate = candidate.split("\n", 1)[1].strip()
        parsed = _parse_json_blob(candidate)
        if not isinstance(parsed, dict):
            if streaming and index == len(paragraphs) - 1 and _looks_like_partial_tool_block(paragraph):
                continue
            kept.append(paragraph)
            continue
        if parsed.get("type") == "tool_use" and parsed.get("name"):
            tool_calls.append(
                {
                    "type": "function",
                    "id": parsed.get("id"),
                    "function": {
                        "name": parsed.get("name", ""),
                        "arguments": parsed.get("input", {}) if isinstance(parsed.get("input"), dict) else {},
                    },
                }
            )
            continue
        if parsed.get("type") == "tool_result":
            continue
        kept.append(paragraph)

    return "\n\n".join(part for part in kept if part).strip(), tool_calls


def _looks_like_partial_tool_block(content: str) -> bool:
    text = content.strip()
    if text.startswith("[USER]\n") or text.startswith("[ASSISTANT]\n"):
        text = text.split("\n", 1)[1].strip() if "\n" in text else ""
    if not text.startswith("{"):
        return False
    return '"type"' in text and ('tool_use' in text or 'tool_result' in text or 'tool_calls' in text)


def _stringify(value: Any) -> str:
    if value is None:
        return ""
    if isinstance(value, str):
        return value
    return json.dumps(value, ensure_ascii=False)
