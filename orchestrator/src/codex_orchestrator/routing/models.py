from __future__ import annotations

from typing import Any, Dict, List, Literal, Optional, Union
from pydantic import BaseModel, Field


TextContent = Union[str, List[Dict[str, Any]]]


class Message(BaseModel):
    role: str
    content: TextContent


class AnthropicMessagesRequest(BaseModel):
    model: Optional[str] = None
    max_tokens: int = 1024
    system: Optional[TextContent] = None
    messages: List[Message]
    tools: Optional[List[Dict[str, Any]]] = None
    metadata: Optional[Dict[str, Any]] = None


class InvokeRequest(BaseModel):
    prompt: str
    system: Optional[str] = None
    preferred_backend: Optional[str] = None
    metadata: Dict[str, Any] = Field(default_factory=dict)


class RouteDecision(BaseModel):
    route: Literal[
        "local_coder",
        "local_reasoner",
        "codex_cli",
    ]
    confidence: float = 0.0
    reason: str = ""
    metrics_payload: Dict[str, Any] = Field(default_factory=dict)


class InvokeResponse(BaseModel):
    route_decision: RouteDecision
    backend_model: str
    output_text: str
    thinking: Optional[str] = None
    raw: Dict[str, Any] = Field(default_factory=dict)


class CompactRequest(BaseModel):
    session_id: str
    items: List[Dict[str, Any]]
    current_request: str
    repo_context: Dict[str, Any] = Field(default_factory=dict)
    refresh_if_needed: bool = False
