from __future__ import annotations

import hashlib
import json
from contextlib import contextmanager
from pathlib import Path
from typing import Any, Dict, Iterator, List, Optional, Tuple, Union
import requests
from requests.adapters import HTTPAdapter
from urllib3.util.retry import Retry
import fcntl


_OLLAMA_LOCK_DIR = Path("state/ollama_locks")


def _service_lock_path(base_url: str) -> Path:
    digest = hashlib.sha256(base_url.rstrip("/").encode("utf-8")).hexdigest()
    return _OLLAMA_LOCK_DIR / f"{digest}.lock"


@contextmanager
def _hold_service_lock(base_url: str):
    lock_path = _service_lock_path(base_url)
    lock_path.parent.mkdir(parents=True, exist_ok=True)
    with lock_path.open("a+", encoding="utf-8") as lock_file:
        fcntl.flock(lock_file.fileno(), fcntl.LOCK_EX)
        try:
            yield
        finally:
            fcntl.flock(lock_file.fileno(), fcntl.LOCK_UN)


class OllamaClient:
    def __init__(
        self,
        base_url: str,
        timeout_seconds: Union[float, int, Tuple[float, float]],
        *,
        pool_connections: int = 8,
        pool_maxsize: int = 8,
    ) -> None:
        self.base_url = base_url.rstrip("/")
        self.timeout_seconds = timeout_seconds
        self.session = requests.Session()
        adapter = HTTPAdapter(
            pool_connections=pool_connections,
            pool_maxsize=pool_maxsize,
            max_retries=Retry(total=0),
        )
        self.session.mount("http://", adapter)
        self.session.mount("https://", adapter)

    def chat(
        self,
        model: str,
        messages: List[Dict[str, Any]],
        *,
        temperature: float,
        num_ctx: int,
        max_tokens: Optional[int] = None,
        system: Optional[str] = None,
        response_format: Optional[Union[str, Dict[str, Any]]] = None,
        think: Optional[Union[bool, str]] = None,
        tools: Optional[List[Dict[str, Any]]] = None,
    ) -> Dict[str, Any]:
        options: Dict[str, Any] = {
            "temperature": temperature,
            "num_ctx": num_ctx,
        }
        if max_tokens is not None:
            options["num_predict"] = max_tokens
        payload: Dict[str, Any] = {
            "model": model,
            "messages": messages,
            "stream": False,
            "options": options,
        }
        if system:
            payload["messages"] = [{"role": "system", "content": system}] + payload["messages"]
        if response_format == "json":
            payload["format"] = "json"
        elif isinstance(response_format, dict):
            payload["format"] = response_format
        if think is not None:
            payload["think"] = think
        if tools:
            payload["tools"] = tools

        with _hold_service_lock(self.base_url):
            response = self.session.post(
                f"{self.base_url}/api/chat",
                json=payload,
                timeout=self.timeout_seconds,
            )
            response.raise_for_status()
            return response.json()

    def chat_stream(
        self,
        model: str,
        messages: List[Dict[str, Any]],
        *,
        temperature: float,
        num_ctx: int,
        max_tokens: Optional[int] = None,
        system: Optional[str] = None,
        response_format: Optional[Union[str, Dict[str, Any]]] = None,
        think: Optional[Union[bool, str]] = None,
        tools: Optional[List[Dict[str, Any]]] = None,
    ) -> Iterator[Dict[str, Any]]:
        options: Dict[str, Any] = {
            "temperature": temperature,
            "num_ctx": num_ctx,
        }
        if max_tokens is not None:
            options["num_predict"] = max_tokens
        payload: Dict[str, Any] = {
            "model": model,
            "messages": messages,
            "stream": True,
            "options": options,
        }
        if system:
            payload["messages"] = [{"role": "system", "content": system}] + payload["messages"]
        if response_format == "json":
            payload["format"] = "json"
        elif isinstance(response_format, dict):
            payload["format"] = response_format
        if think is not None:
            payload["think"] = think
        if tools:
            payload["tools"] = tools

        with _hold_service_lock(self.base_url):
            with self.session.post(
                f"{self.base_url}/api/chat",
                json=payload,
                timeout=self.timeout_seconds,
                stream=True,
            ) as response:
                response.raise_for_status()
                for line in response.iter_lines(decode_unicode=True):
                    if not line:
                        continue
                    yield json.loads(line)
