"""Provider adapter base class."""
from __future__ import annotations

from abc import ABC, abstractmethod
from typing import AsyncIterator

from ..schemas.run import Task
from ..schemas.worker import WorkerResult, RepositoryContext
from ..schemas.routing import ProviderCapability, CostEstimate, HealthStatus


class ProviderAdapter(ABC):
    """Abstract base class for provider adapters."""

    @abstractmethod
    async def execute_task(self, task: Task, context: RepositoryContext) -> WorkerResult:
        ...

    @abstractmethod
    async def capabilities(self) -> ProviderCapability:
        ...

    @abstractmethod
    async def health(self) -> HealthStatus:
        ...

    @abstractmethod
    async def estimate_cost(self, task: Task) -> CostEstimate:
        ...

    @abstractmethod
    async def cancel(self, task_id: str) -> None:
        ...
