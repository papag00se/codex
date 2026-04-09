from __future__ import annotations

import json
from pathlib import Path
from typing import Iterable, Optional

from ..schemas.config import settings
from .models import ChunkExtraction, DurableMemorySet, MergedState, SessionHandoff


class CompactionStorage:
    def __init__(self, root: Optional[Path] = None) -> None:
        self.root = Path(root or settings.compaction_state_dir)

    def save_chunk_extractions(self, session_id: str, extractions: Iterable[ChunkExtraction]) -> None:
        directory = self._session_dir(session_id) / "chunks"
        directory.mkdir(parents=True, exist_ok=True)
        for extraction in extractions:
            self._write_json(directory / f"chunk-{extraction.chunk_id}.json", extraction.model_dump())

    def save_merged_state(self, session_id: str, state: MergedState) -> None:
        directory = self._session_dir(session_id)
        directory.mkdir(parents=True, exist_ok=True)
        self._write_json(directory / "merged-state.json", state.model_dump())

    def save_refined_state(self, session_id: str, state: MergedState) -> None:
        directory = self._session_dir(session_id)
        directory.mkdir(parents=True, exist_ok=True)
        self._write_json(directory / "refined-state.json", state.model_dump())

    def save_durable_memory(self, session_id: str, memory: DurableMemorySet) -> None:
        directory = self._session_dir(session_id)
        directory.mkdir(parents=True, exist_ok=True)
        self._write_text(directory / "TASK_STATE.md", memory.task_state)
        self._write_text(directory / "DECISIONS.md", memory.decisions)
        self._write_text(directory / "FAILURES_TO_AVOID.md", memory.failures_to_avoid)
        self._write_text(directory / "NEXT_STEPS.md", memory.next_steps)
        self._write_text(directory / "SESSION_HANDOFF.md", memory.session_handoff)

    def save_handoff(self, session_id: str, handoff: SessionHandoff) -> None:
        directory = self._session_dir(session_id)
        directory.mkdir(parents=True, exist_ok=True)
        self._write_json(directory / "handoff.json", handoff.model_dump())

    def load_handoff(self, session_id: str) -> Optional[SessionHandoff]:
        path = self._session_dir(session_id) / "handoff.json"
        if not path.exists():
            return None
        return SessionHandoff.model_validate(json.loads(path.read_text(encoding="utf-8")))

    def load_durable_memory(self, session_id: str) -> Optional[DurableMemorySet]:
        directory = self._session_dir(session_id)
        required = {
            "task_state": directory / "TASK_STATE.md",
            "decisions": directory / "DECISIONS.md",
            "failures_to_avoid": directory / "FAILURES_TO_AVOID.md",
            "next_steps": directory / "NEXT_STEPS.md",
            "session_handoff": directory / "SESSION_HANDOFF.md",
        }
        if not all(path.exists() for path in required.values()):
            return None
        return DurableMemorySet(
            task_state=required["task_state"].read_text(encoding="utf-8"),
            decisions=required["decisions"].read_text(encoding="utf-8"),
            failures_to_avoid=required["failures_to_avoid"].read_text(encoding="utf-8"),
            next_steps=required["next_steps"].read_text(encoding="utf-8"),
            session_handoff=required["session_handoff"].read_text(encoding="utf-8"),
        )

    def _session_dir(self, session_id: str) -> Path:
        return self.root / session_id

    def _write_json(self, path: Path, payload: dict) -> None:
        path.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    def _write_text(self, path: Path, content: str) -> None:
        path.write_text(content, encoding="utf-8")
