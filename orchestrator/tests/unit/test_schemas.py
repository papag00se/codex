"""Tests for core Pydantic schemas."""
from __future__ import annotations

import json
import pytest

from codex_orchestrator.schemas.id import generate_id
from codex_orchestrator.schemas.run import Run, Task, VALID_TRANSITIONS, validate_transition
from codex_orchestrator.schemas.events import Event, RUN_CREATED, TASK_COMPLETED, TASK_FAILED
from codex_orchestrator.schemas.routing import RoutingDecision, ProviderCapability, CostEstimate, HealthStatus
from codex_orchestrator.schemas.worker import WorkerResult, RepositoryContext
from codex_orchestrator.schemas.approval import ApprovalRequest
from codex_orchestrator.schemas.artifacts import ArtifactRecord


class TestIdGeneration:
    def test_generate_id_has_prefix(self):
        rid = generate_id("r")
        assert rid.startswith("r_")

    def test_generate_id_length(self):
        rid = generate_id("task", length=8)
        assert rid.startswith("task_")
        assert len(rid) == 5 + 8  # prefix + underscore + body

    def test_generate_id_unique(self):
        ids = {generate_id("r") for _ in range(100)}
        assert len(ids) == 100


class TestRun:
    def test_create_run(self):
        r = Run(goal="test goal", repo_path="/tmp/repo", base_branch="main")
        assert r.id.startswith("r_")
        assert r.status == "planned"
        assert r.goal == "test goal"
        assert r.budget_spent == 0.0
        assert r.current_iteration == 0

    def test_run_serialization(self):
        r = Run(goal="test", repo_path="/tmp", base_branch="main")
        data = json.loads(r.model_dump_json())
        assert data["goal"] == "test"
        assert data["status"] == "planned"

    def test_run_invalid_status(self):
        with pytest.raises(Exception):
            Run(goal="test", repo_path="/tmp", base_branch="main", status="invalid")


class TestTask:
    def test_create_task(self):
        t = Task(run_id="r_test", description="do thing", task_type="code")
        assert t.id.startswith("task_")
        assert t.status == "planned"
        assert t.retry_count == 0
        assert t.max_retries == 3

    def test_task_with_dependencies(self):
        t = Task(
            run_id="r_test",
            description="depends on others",
            task_type="code",
            dependencies=["task_1", "task_2"],
        )
        assert t.dependencies == ["task_1", "task_2"]

    def test_task_invalid_type(self):
        with pytest.raises(Exception):
            Task(run_id="r_test", description="bad", task_type="invalid")


class TestTaskTransitions:
    def test_valid_transitions(self):
        assert validate_transition("planned", "routed")
        assert validate_transition("planned", "skipped")
        assert validate_transition("planned", "cancelled")
        assert validate_transition("routed", "assigned")
        assert validate_transition("assigned", "running")
        assert validate_transition("running", "verifying")
        assert validate_transition("running", "failed")
        assert validate_transition("verifying", "completed")
        assert validate_transition("verifying", "awaiting_approval")
        assert validate_transition("awaiting_approval", "completed")
        assert validate_transition("awaiting_approval", "failed")
        assert validate_transition("failed", "planned")  # retry

    def test_invalid_transitions(self):
        assert not validate_transition("completed", "running")
        assert not validate_transition("cancelled", "running")
        assert not validate_transition("skipped", "running")
        assert not validate_transition("planned", "running")  # must go through routed
        assert not validate_transition("running", "completed")  # must go through verifying

    def test_all_states_have_transitions(self):
        for state in VALID_TRANSITIONS:
            assert isinstance(VALID_TRANSITIONS[state], set)

    def test_terminal_states_have_no_forward_transitions(self):
        assert VALID_TRANSITIONS["completed"] == set()
        assert VALID_TRANSITIONS["cancelled"] == set()
        assert VALID_TRANSITIONS["skipped"] == set()


class TestEvent:
    def test_create_event(self):
        e = Event(type=RUN_CREATED, run_id="r_test", sequence=1)
        assert e.id.startswith("evt_")
        assert e.type == "run.created"
        assert e.sequence == 1

    def test_event_with_data(self):
        e = Event(
            type=TASK_COMPLETED,
            run_id="r_test",
            task_id="task_1",
            sequence=5,
            data={"status": "success", "cost_usd": 0.12},
        )
        assert e.data["cost_usd"] == 0.12


class TestRoutingDecision:
    def test_create_routing_decision(self):
        rd = RoutingDecision(
            task_id="task_1",
            run_id="r_test",
            selected_backend="ollama-coder",
            confidence=0.92,
            reason="Best fit for code generation",
            eligible_backends=["ollama-coder", "claude-code"],
            scores={"ollama-coder": 0.92, "claude-code": 0.78},
            factors={"task_type": "code", "complexity": "low"},
        )
        assert rd.id.startswith("rd_")
        assert rd.confidence == 0.92


class TestProviderCapability:
    def test_create_provider_capability(self):
        pc = ProviderCapability(
            provider_id="ollama-coder",
            provider_type="local",
            cost_category="free",
            access_method="http",
            context_window=16384,
            supports_tool_use=True,
            strengths={"code_generation": 0.65},
        )
        assert pc.health == "healthy"
        assert pc.strengths["code_generation"] == 0.65


class TestWorkerResult:
    def test_success_result(self):
        wr = WorkerResult(
            task_id="task_1",
            status="success",
            files_changed=["src/auth.py"],
            tokens_input=3200,
            cost_usd=0.12,
        )
        assert wr.status == "success"
        assert wr.cost_usd == 0.12

    def test_failure_result(self):
        wr = WorkerResult(
            task_id="task_1",
            status="failure",
            error_text="Agent exceeded max turns",
        )
        assert wr.status == "failure"


class TestApprovalRequest:
    def test_create_approval(self):
        ar = ApprovalRequest(
            task_id="task_1",
            run_id="r_test",
            action_type="shell_exec",
            action_detail="npm install redis",
            policy_rule="package installation requires approval",
        )
        assert ar.id.startswith("apr_")
        assert ar.status == "pending"
        assert ar.timeout_seconds == 300
