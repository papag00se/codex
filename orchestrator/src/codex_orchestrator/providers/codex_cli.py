from __future__ import annotations

from typing import Any, Dict
import subprocess


class CodexCLIClient:
    def __init__(self, cmd: str, workdir: str, timeout_seconds: int, *, model_provider: str, model: str) -> None:
        self.cmd = cmd
        self.workdir = workdir
        self.timeout_seconds = timeout_seconds
        self.model_provider = model_provider
        self.model = model

    def exec_prompt(self, prompt: str, *, workdir: str | None = None) -> Dict[str, Any]:
        target_workdir = workdir or self.workdir
        cmd = [
            self.cmd,
            "exec",
            "--skip-git-repo-check",
            "-c",
            f"model_provider={self.model_provider}",
            "-c",
            f"model={self.model}",
        ]
        if target_workdir:
            cmd.extend(["-C", target_workdir])
        cmd.append(prompt)
        proc = subprocess.run(
            cmd,
            cwd=target_workdir,
            capture_output=True,
            text=True,
            timeout=self.timeout_seconds,
            check=False,
        )
        return {
            "returncode": proc.returncode,
            "stdout": proc.stdout,
            "stderr": proc.stderr,
        }
