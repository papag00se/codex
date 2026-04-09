"""Routing decision and provider capability models."""
from __future__ import annotations

import time
from typing import Literal, Optional

from pydantic import BaseModel, Field

from .id import generate_id


class RoutingDecision(BaseModel):
    id: str = Field(default_factory=lambda: generate_id("rd"))
    task_id: str
    run_id: str

    selected_backend: str
    selected_model: Optional[str] = None
    confidence: float
    reason: str

    eligible_backends: list[str]
    scores: dict[str, float]
    factors: dict

    is_retry: bool = False
    previous_backend: Optional[str] = None

    created_at: int = Field(default_factory=lambda: int(time.time()))


class ProviderCapability(BaseModel):
    provider_id: str
    provider_type: Literal["agent_backend", "api", "local"]
    cost_category: Literal["subscription", "api", "free"]
    access_method: Literal["subprocess", "http"]

    context_window: int
    max_output_tokens: Optional[int] = None
    supports_tool_use: bool = False
    supports_streaming: bool = False

    strengths: dict[str, float] = Field(default_factory=dict)

    cost_per_1k_input: Optional[float] = None
    cost_per_1k_output: Optional[float] = None
    subscription_weight: Optional[float] = None

    max_concurrent: int = 1
    requires_network: bool = True

    health: Literal["healthy", "degraded", "unavailable"] = "healthy"
    last_health_check: Optional[int] = None


class CostEstimate(BaseModel):
    estimated_input_tokens: int
    estimated_output_tokens: int
    estimated_cost_usd: Optional[float] = None
    subscription_units: Optional[float] = None
    cost_category: Literal["api", "subscription", "free"]


class HealthStatus(BaseModel):
    status: Literal["healthy", "degraded", "unavailable"]
    latency_ms: Optional[int] = None
    error: Optional[str] = None
    last_checked: int = Field(default_factory=lambda: int(time.time()))
    consecutive_failures: int = 0
