from __future__ import annotations

import json
import logging
import time
from typing import Any, Dict, List, Optional

from ..providers.ollama_client import OllamaClient
from ..schemas.config import settings
from ..routing.metrics import estimate_tokens
from .merger import merge_states
from .models import ChunkExtraction, MergedState
from .prompts import REFINEMENT_SYSTEM_PROMPT, build_refinement_payload, estimate_refinement_request_tokens
from .structured_output import normalize_chunk_extraction_payload, recent_state_response_schema

logger = logging.getLogger(__name__)
_TOKEN_ESTIMATION_SLACK = 256
_MIN_REFINEMENT_RESPONSE_TOKENS = 512


class CompactionRefiner:
    def __init__(
        self,
        client: Optional[Any] = None,
        *,
        model: Optional[str] = None,
        temperature: Optional[float] = None,
        num_ctx: Optional[int] = None,
    ) -> None:
        self.client = client or OllamaClient(
            settings.compactor_ollama_base_url,
            (settings.ollama_connect_timeout_seconds, settings.compactor_timeout_seconds),
            pool_connections=settings.ollama_pool_connections,
            pool_maxsize=settings.ollama_pool_maxsize,
        )
        self.model = model or settings.compactor_model
        self.temperature = settings.compactor_temperature if temperature is None else temperature
        self.num_ctx = settings.compactor_num_ctx if num_ctx is None else num_ctx
        self.burst_num_ctx = max(self.num_ctx, settings.compactor_burst_num_ctx)
        self.minimum_response_tokens = _MIN_REFINEMENT_RESPONSE_TOKENS
        self.token_estimation_slack = _TOKEN_ESTIMATION_SLACK

    def refine_state(
        self,
        state: MergedState,
        recent_raw_turns: List[Dict[str, Any]],
        *,
        current_request: str,
        repo_context: Optional[Dict[str, Any]] = None,
    ) -> MergedState:
        if not recent_raw_turns:
            return state
        payload = build_refinement_payload(state, recent_raw_turns, current_request, repo_context)
        prompt_tokens = estimate_refinement_request_tokens(
            state,
            recent_raw_turns,
            current_request,
            repo_context,
            model=self.model,
        )
        request_num_ctx, response_token_budget = self._request_budget(prompt_tokens)
        if response_token_budget < self.minimum_response_tokens:
            raise ValueError(
                f"insufficient compaction refiner output budget: prompt_tokens={prompt_tokens} "
                f"available={response_token_budget} minimum={self.minimum_response_tokens}"
            )
        logger.warning(
            "compaction_llm_request stage=refine model=%s prompt_tokens=%s response_tokens=%s num_ctx=%s recent_turn_count=%s merged_chunk_count=%s",
            self.model,
            prompt_tokens,
            response_token_budget,
            request_num_ctx,
            len(recent_raw_turns),
            state.merged_chunk_count,
        )
        started_at = time.monotonic()
        raw = self.client.chat(
            self.model,
            [{"role": "user", "content": payload}],
            temperature=self.temperature,
            num_ctx=request_num_ctx,
            system=REFINEMENT_SYSTEM_PROMPT,
            response_format=recent_state_response_schema(),
            think=False,
        )
        elapsed_ms = int((time.monotonic() - started_at) * 1000)
        logger.warning(
            "compaction_llm_response stage=refine model=%s elapsed_ms=%s prompt_eval_count=%s eval_count=%s",
            self.model,
            elapsed_ms,
            raw.get("prompt_eval_count"),
            raw.get("eval_count"),
        )
        content = raw.get("message", {}).get("content", "")
        try:
            parsed = json.loads(content) if isinstance(content, str) else content
        except json.JSONDecodeError as exc:
            preview = content[:500] if isinstance(content, str) else repr(content)
            raise ValueError(f"compaction refiner returned non-JSON output: {preview}") from exc
        if not isinstance(parsed, dict):
            raise ValueError(f"compaction refiner returned non-object output: {type(parsed).__name__}")
        extracted_payload = normalize_chunk_extraction_payload(parsed)
        extracted_payload.setdefault("chunk_id", state.merged_chunk_count + 1)
        extracted_payload.setdefault("source_token_count", estimate_tokens(recent_raw_turns))
        recent_state = ChunkExtraction.model_validate(extracted_payload)
        if not _recent_state_has_effect(recent_state):
            logger.warning(
                "compaction_refine_noop model=%s recent_turn_count=%s merged_chunk_count=%s",
                self.model,
                len(recent_raw_turns),
                state.merged_chunk_count,
            )
            return state
        merged = merge_states([state, recent_state])
        merged.merged_chunk_count = state.merged_chunk_count
        return merged

    def target_prompt_tokens(self) -> int:
        return max(1, self.num_ctx - self.minimum_response_tokens - self.token_estimation_slack)

    def max_prompt_tokens(self) -> int:
        return max(1, self.burst_num_ctx - self.minimum_response_tokens - self.token_estimation_slack)

    def _response_token_budget(self, prompt_tokens: int) -> int:
        return self._request_budget(prompt_tokens)[1]

    def _request_budget(self, prompt_tokens: int) -> tuple[int, int]:
        default_available = self.num_ctx - prompt_tokens - self.token_estimation_slack
        if default_available >= self.minimum_response_tokens:
            return self.num_ctx, default_available

        request_num_ctx = self.burst_num_ctx
        available = request_num_ctx - prompt_tokens - self.token_estimation_slack
        if available < self.minimum_response_tokens:
            return request_num_ctx, 0
        return request_num_ctx, available


def _recent_state_has_effect(extracted: ChunkExtraction) -> bool:
    return bool(
        extracted.objective
        or extracted.repo_state
        or extracted.files_touched
        or extracted.commands_run
        or extracted.errors
        or extracted.accepted_fixes
        or extracted.rejected_ideas
        or extracted.constraints
        or extracted.environment_assumptions
        or extracted.pending_todos
        or extracted.unresolved_bugs
        or extracted.test_status
        or extracted.external_references
        or extracted.latest_plan
    )
