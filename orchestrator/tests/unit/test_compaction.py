"""Tests for compaction pipeline — verifying migrated behavior."""
from __future__ import annotations

from codex_orchestrator.compaction.models import (
    ChunkExtraction,
    MergedState,
    TranscriptChunk,
    SessionHandoff,
    DurableMemorySet,
)
from codex_orchestrator.compaction.merger import merge_states
from codex_orchestrator.compaction.chunking import (
    chunk_transcript_items,
    split_recent_raw_turns,
)
from codex_orchestrator.compaction.normalize import normalize_transcript_for_compaction
from codex_orchestrator.compaction.durable_memory import (
    build_session_handoff,
    render_durable_memory,
)
from codex_orchestrator.compaction.structured_output import (
    chunk_extraction_response_schema,
    normalize_chunk_extraction_payload,
)
from codex_orchestrator.compaction.prompts import (
    build_extraction_payload,
    EXTRACTION_SYSTEM_PROMPT,
    REFINEMENT_SYSTEM_PROMPT,
)


class TestMergeStates:
    def test_merge_two_extractions(self):
        s1 = ChunkExtraction(chunk_id=1, objective="Fix auth", files_touched=["auth.py"])
        s2 = ChunkExtraction(chunk_id=2, objective="Fix auth and add tests", files_touched=["test_auth.py"])
        merged = merge_states([s1, s2])
        # Latest non-empty wins for objective
        assert merged.objective == "Fix auth and add tests"
        # Deduplicated, newer first
        assert "test_auth.py" in merged.files_touched
        assert "auth.py" in merged.files_touched
        assert merged.merged_chunk_count == 2

    def test_merge_empty(self):
        merged = merge_states([])
        assert merged.objective == ""
        assert merged.files_touched == []
        assert merged.merged_chunk_count == 0

    def test_merge_single(self):
        s = ChunkExtraction(chunk_id=1, objective="task", errors=["err1"])
        merged = merge_states([s])
        assert merged.objective == "task"
        assert merged.errors == ["err1"]
        assert merged.merged_chunk_count == 1

    def test_merge_latest_non_empty_objective(self):
        s1 = ChunkExtraction(chunk_id=1, objective="first")
        s2 = ChunkExtraction(chunk_id=2, objective="")
        s3 = ChunkExtraction(chunk_id=3, objective="third")
        merged = merge_states([s1, s2, s3])
        assert merged.objective == "third"

    def test_merge_repo_state_shallow(self):
        s1 = ChunkExtraction(chunk_id=1, repo_state={"branch": "main", "runtime": "py3.11"})
        s2 = ChunkExtraction(chunk_id=2, repo_state={"branch": "feature", "db": "postgres"})
        merged = merge_states([s1, s2])
        assert merged.repo_state["branch"] == "feature"  # Later wins
        assert merged.repo_state["runtime"] == "py3.11"  # Preserved
        assert merged.repo_state["db"] == "postgres"      # Added

    def test_merge_list_dedup_case_insensitive(self):
        s1 = ChunkExtraction(chunk_id=1, files_touched=["Auth.py", "README.md"])
        s2 = ChunkExtraction(chunk_id=2, files_touched=["auth.py", "tests.py"])
        merged = merge_states([s1, s2])
        # Case-insensitive dedup, newer first
        lower_files = [f.lower() for f in merged.files_touched]
        assert lower_files.count("auth.py") == 1
        assert "tests.py" in lower_files
        assert "readme.md" in lower_files

    def test_merge_latest_plan_wins(self):
        s1 = ChunkExtraction(chunk_id=1, latest_plan=["step 1", "step 2"])
        s2 = ChunkExtraction(chunk_id=2, latest_plan=["step A", "step B", "step C"])
        merged = merge_states([s1, s2])
        assert merged.latest_plan == ["step A", "step B", "step C"]

    def test_merge_latest_plan_empty_doesnt_overwrite(self):
        s1 = ChunkExtraction(chunk_id=1, latest_plan=["step 1"])
        s2 = ChunkExtraction(chunk_id=2, latest_plan=[])
        merged = merge_states([s1, s2])
        assert merged.latest_plan == ["step 1"]


class TestSplitRecentRawTurns:
    def test_basic_split(self):
        items = [{"role": "user", "content": f"message {i}"} for i in range(10)]
        compactable, recent = split_recent_raw_turns(items, keep_tokens=20)
        assert len(compactable) + len(recent) == 10
        assert len(recent) > 0
        assert len(compactable) > 0

    def test_zero_keep(self):
        items = [{"role": "user", "content": "hello"}]
        compactable, recent = split_recent_raw_turns(items, keep_tokens=0)
        assert compactable == items
        assert recent == []

    def test_keep_all(self):
        items = [{"role": "user", "content": "hi"}]
        compactable, recent = split_recent_raw_turns(items, keep_tokens=100000)
        assert len(compactable) == 0
        assert len(recent) == 1


class TestChunkTranscriptItems:
    def test_basic_chunking(self):
        items = [{"role": "user", "content": f"message {i} " * 50} for i in range(10)]
        chunks = chunk_transcript_items(items, target_tokens=500, max_tokens=1000, overlap_tokens=100)
        assert len(chunks) > 0
        # All items covered
        all_indices = set()
        for chunk in chunks:
            for i in range(chunk.start_index, chunk.end_index):
                all_indices.add(i)
        assert all_indices == set(range(10))

    def test_single_item(self):
        items = [{"role": "user", "content": "hello"}]
        chunks = chunk_transcript_items(items, target_tokens=1000, max_tokens=2000, overlap_tokens=0)
        assert len(chunks) == 1
        assert chunks[0].chunk_id == 1

    def test_empty_items(self):
        chunks = chunk_transcript_items([], target_tokens=100, max_tokens=200, overlap_tokens=0)
        assert chunks == []


class TestNormalize:
    def test_basic_normalization(self):
        items = [
            {"role": "user", "content": "hello"},
            {"role": "assistant", "content": "hi there"},
            {"role": "user", "content": "thanks"},
        ]
        result = normalize_transcript_for_compaction(items, max_item_tokens=10000)
        # Latest item is preserved tail
        assert len(result.preserved_tail) == 1
        assert len(result.compactable_items) == 2

    def test_encrypted_content_stripped(self):
        items = [
            {"role": "user", "content": "hello", "encrypted_content": "secret"},
            {"role": "assistant", "content": "response"},
        ]
        result = normalize_transcript_for_compaction(items, max_item_tokens=10000)
        for item in result.compactable_items:
            assert "encrypted_content" not in item


class TestBuildSessionHandoff:
    def test_handoff_construction(self):
        state = MergedState(
            objective="Fix auth",
            repo_state={"branch": "main"},
            accepted_fixes=["Fixed login"],
            pending_todos=["Add tests"],
            unresolved_bugs=["Logout broken"],
            errors=["Auth crash"],
            rejected_ideas=["Use JWT"],
            latest_plan=["Step 1", "Step 2"],
        )
        handoff = build_session_handoff(state, recent_raw_turns=[], current_request="continue")
        assert handoff.stable_task_definition == "Fix auth"
        assert handoff.key_decisions == ["Fixed login"]
        assert handoff.unresolved_work == ["Add tests", "Logout broken"]
        assert handoff.failures_to_avoid == ["Auth crash", "Use JWT"]
        assert handoff.latest_plan == ["Step 1", "Step 2"]
        assert handoff.current_request == "continue"


class TestRenderDurableMemory:
    def test_render_produces_all_sections(self):
        state = MergedState(
            objective="Fix auth",
            files_touched=["auth.py"],
            commands_run=["pytest"],
            test_status=["3 passed"],
            accepted_fixes=["login fix"],
            constraints=["no breaking changes"],
            errors=["crash on startup"],
            rejected_ideas=["rewrite from scratch"],
            pending_todos=["add tests"],
            latest_plan=["step 1"],
        )
        memory = render_durable_memory(state, recent_raw_turns=[], current_request="next")
        assert "Fix auth" in memory.task_state
        assert "auth.py" in memory.task_state
        assert "login fix" in memory.decisions
        assert "crash on startup" in memory.failures_to_avoid
        assert "add tests" in memory.next_steps


class TestExtractionSchema:
    def test_schema_has_required_fields(self):
        schema = chunk_extraction_response_schema()
        props = schema["properties"]
        assert "chunk_id" in props
        assert "objective" in props
        assert "repo_state" in props
        assert "files_touched" in props
        assert "commands_run" in props
        assert "errors" in props
        assert "source_token_count" in props
        assert schema.get("additionalProperties") is False

    def test_all_fields_required(self):
        schema = chunk_extraction_response_schema()
        assert set(schema["required"]) == set(schema["properties"].keys())


class TestNormalizeExtractionPayload:
    def test_repo_state_entries_to_dict(self):
        payload = {
            "repo_state": [
                {"key": "branch", "value": "main"},
                {"key": "runtime", "value": "python3.11"},
            ]
        }
        result = normalize_chunk_extraction_payload(payload)
        assert result["repo_state"] == {"branch": "main", "runtime": "python3.11"}

    def test_repo_state_dict_passthrough(self):
        payload = {"repo_state": {"branch": "main"}}
        result = normalize_chunk_extraction_payload(payload)
        assert result["repo_state"] == {"branch": "main"}


class TestPrompts:
    def test_extraction_prompt_loaded(self):
        assert len(EXTRACTION_SYSTEM_PROMPT) > 100
        assert "durable coding-session state" in EXTRACTION_SYSTEM_PROMPT

    def test_refinement_prompt_loaded(self):
        assert len(REFINEMENT_SYSTEM_PROMPT) > 100
        assert "recent raw transcript" in REFINEMENT_SYSTEM_PROMPT

    def test_build_extraction_payload(self):
        chunk = TranscriptChunk(
            chunk_id=1,
            start_index=0,
            end_index=2,
            token_count=100,
            items=[
                {"role": "user", "content": "fix the bug"},
                {"role": "assistant", "content": "I'll look at it"},
            ],
        )
        payload = build_extraction_payload(chunk, repo_context={"cwd": "/tmp"})
        import json
        parsed = json.loads(payload)
        assert parsed["task"] == "Extract chunk-local durable coding-session state."
        assert "chunk" in parsed
        assert parsed["repo_context"] == {"cwd": "/tmp"}
