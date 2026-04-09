"""Minimal compatibility shim for building Responses API request payloads.

Migrated from coding-agent-router/app/clients/responses_client.py — only the
build_chat_request_payload function, used by compaction/prompts.py for token
estimation.
"""
from __future__ import annotations

import json
from typing import Any, Dict, List, Optional, Union


def build_chat_request_payload(
    model: str,
    messages: List[Dict[str, Any]],
    *,
    system: Optional[str] = None,
    max_tokens: Optional[int] = None,
    response_format: Optional[Union[str, Dict[str, Any]]] = None,
    store: bool = False,
    stream: bool = True,
) -> Dict[str, Any]:
    payload: Dict[str, Any] = {
        "model": model,
        "input": _responses_input(messages),
        "store": store,
        "stream": stream,
    }
    if system:
        payload["instructions"] = system
    if max_tokens is not None:
        payload["max_output_tokens"] = max_tokens
    if isinstance(response_format, dict):
        payload["text"] = {"format": _json_schema_text_format(response_format)}
    return payload


def _responses_input(messages: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    items: List[Dict[str, Any]] = []
    for msg in messages:
        role = msg.get("role", "user")
        content = msg.get("content", "")
        if isinstance(content, str):
            items.append({"type": "message", "role": role, "content": [{"type": "input_text", "text": content}]})
        elif isinstance(content, list):
            items.append({"type": "message", "role": role, "content": content})
        else:
            items.append({"type": "message", "role": role, "content": [{"type": "input_text", "text": str(content)}]})
    return items


def _json_schema_text_format(schema: Dict[str, Any]) -> Dict[str, Any]:
    return {
        "type": "json_schema",
        "name": schema.get("title", "response"),
        "schema": schema,
        "strict": True,
    }
