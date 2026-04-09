"""Configuration schema — migrated from coding-agent-router/app/config.py.

All defaults preserved from original. See docs/spec/routing-logic-reference.md
and docs/spec/compaction-reference.md for the significance of each value.
"""
from __future__ import annotations

import os
from dataclasses import dataclass


def _bool(name: str, default: bool) -> bool:
    value = os.getenv(name)
    if value is None:
        return default
    return value.strip().lower() in {"1", "true", "yes", "on"}


@dataclass(frozen=True)
class Settings:
    # --- Router model ---
    router_ollama_base_url: str = os.getenv("ROUTER_OLLAMA_BASE_URL", "http://127.0.0.1:11434")
    router_model: str = os.getenv("ROUTER_MODEL", "qwen3:8b-q4_K_M")
    router_num_ctx: int = int(os.getenv("ROUTER_NUM_CTX", "8192"))
    router_temperature: float = float(os.getenv("ROUTER_TEMPERATURE", "0.0"))
    router_timeout_seconds: int = int(os.getenv("ROUTER_TIMEOUT_SECONDS", "1800"))

    # --- Local coder ---
    coder_ollama_base_url: str = os.getenv("CODER_OLLAMA_BASE_URL", "http://127.0.0.1:11435")
    coder_model: str = os.getenv("CODER_MODEL", "qwen3-coder:30b-a3b-q4_K_M")
    coder_num_ctx: int = int(os.getenv("CODER_NUM_CTX", "16384"))
    coder_temperature: float = float(os.getenv("CODER_TEMPERATURE", "0.1"))
    coder_timeout_seconds: int = int(os.getenv("CODER_TIMEOUT_SECONDS", "1800"))

    # --- Local reasoner ---
    reasoner_ollama_base_url: str = os.getenv("REASONER_OLLAMA_BASE_URL", "http://127.0.0.1:11436")
    reasoner_model: str = os.getenv("REASONER_MODEL", "qwen3:14b")
    reasoner_num_ctx: int = int(os.getenv("REASONER_NUM_CTX", "16384"))
    reasoner_temperature: float = float(os.getenv("REASONER_TEMPERATURE", "0.1"))
    reasoner_timeout_seconds: int = int(os.getenv("REASONER_TIMEOUT_SECONDS", "1800"))

    # --- Compactor ---
    compactor_ollama_base_url: str = os.getenv("COMPACTOR_OLLAMA_BASE_URL", "http://127.0.0.1:11435")
    compactor_model: str = os.getenv("COMPACTOR_MODEL", "qwen3.5:9b")
    compactor_num_ctx: int = int(os.getenv("COMPACTOR_NUM_CTX", "16384"))
    compactor_burst_num_ctx: int = int(os.getenv("COMPACTOR_BURST_NUM_CTX", "17408"))
    compactor_temperature: float = float(os.getenv("COMPACTOR_TEMPERATURE", "0.0"))
    compactor_timeout_seconds: int = int(os.getenv("COMPACTOR_TIMEOUT_SECONDS", "1800"))
    compactor_target_chunk_tokens: int = int(os.getenv("COMPACTOR_TARGET_CHUNK_TOKENS", "10000"))
    compactor_max_chunk_tokens: int = int(os.getenv("COMPACTOR_MAX_CHUNK_TOKENS", "10000"))
    compactor_max_prompt_tokens: int = int(os.getenv("COMPACTOR_MAX_PROMPT_TOKENS", "12256"))
    compactor_overlap_tokens: int = int(os.getenv("COMPACTOR_OVERLAP_TOKENS", "1500"))
    compactor_keep_raw_tokens: int = int(os.getenv("COMPACTOR_KEEP_RAW_TOKENS", "8000"))
    compactor_merge_batch_size: int = int(os.getenv("COMPACTOR_MERGE_BATCH_SIZE", "8"))

    # --- Compaction state ---
    compaction_state_dir: str = os.getenv("COMPACTION_STATE_DIR", "state/compaction")
    enable_incremental_compaction: bool = _bool("ENABLE_INCREMENTAL_COMPACTION", False)
    log_compaction_payloads: bool = _bool("LOG_COMPACTION_PAYLOADS", False)
    inline_compact_sentinel: str = os.getenv("INLINE_COMPACT_SENTINEL", "<<<LOCAL_COMPACT>>>")

    # --- Codex CLI ---
    enable_codex_cli: bool = _bool("ENABLE_CODEX_CLI", False)
    codex_cmd: str = os.getenv("CODEX_CMD", "codex")
    codex_workdir: str = os.getenv("CODEX_WORKDIR", ".")
    codex_exec_model_provider: str = os.getenv("CODEX_EXEC_MODEL_PROVIDER", "openai")
    codex_exec_model: str = os.getenv("CODEX_EXEC_MODEL", "gpt-5.4")
    codex_timeout_seconds: int = int(os.getenv("CODEX_TIMEOUT_SECONDS", "1800"))

    # --- Connection pooling ---
    ollama_connect_timeout_seconds: float = float(os.getenv("OLLAMA_CONNECT_TIMEOUT_SECONDS", "5"))
    ollama_pool_connections: int = int(os.getenv("OLLAMA_POOL_CONNECTIONS", "8"))
    ollama_pool_maxsize: int = int(os.getenv("OLLAMA_POOL_MAXSIZE", "16"))

    # --- Feature flags ---
    enable_local_coder: bool = _bool("ENABLE_LOCAL_CODER", True)
    enable_local_reasoner: bool = _bool("ENABLE_LOCAL_REASONER", True)
    default_cloud_backend: str = os.getenv("DEFAULT_CLOUD_BACKEND", "codex_cli")
    fail_open: bool = _bool("FAIL_OPEN", False)


settings = Settings()
