"""Approval request model."""
from __future__ import annotations

import time
from typing import Literal, Optional

from pydantic import BaseModel, Field

from .id import generate_id


class ApprovalRequest(BaseModel):
    id: str = Field(default_factory=lambda: generate_id("apr"))
    task_id: str
    run_id: str

    action_type: Literal[
        "shell_exec", "file_delete", "file_write_risky",
        "git_op", "package_install", "network", "secret_access",
    ]
    action_detail: str
    policy_rule: str

    status: Literal["pending", "approved", "denied", "timeout"] = "pending"
    decided_at: Optional[int] = None
    decided_by: Optional[str] = None

    created_at: int = Field(default_factory=lambda: int(time.time()))
    timeout_seconds: int = 300
