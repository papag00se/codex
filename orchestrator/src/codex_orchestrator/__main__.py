"""Entry point: python -m codex_orchestrator"""

from __future__ import annotations

import argparse
import asyncio
import json
import sys


def main() -> None:
    parser = argparse.ArgumentParser(description="Codex Multi-Agent Orchestrator")
    parser.add_argument("--run-id", required=True, help="Run ID to execute or resume")
    parser.add_argument("--config", help="Path to config TOML file")
    parser.add_argument("--mock", action="store_true", help="Use mock providers for testing")
    args = parser.parse_args()

    # Emit a ready event so the CLI knows we're alive
    print(json.dumps({"type": "ready", "run_id": args.run_id}), flush=True)

    # TODO: Wire up supervisor loop
    # For now, read stdin commands and echo events
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
            if msg.get("type") == "cancel":
                print(json.dumps({"type": "cancelled", "run_id": args.run_id}), flush=True)
                break
        except json.JSONDecodeError:
            pass

    print(json.dumps({"type": "complete", "run_id": args.run_id, "status": "completed"}), flush=True)


if __name__ == "__main__":
    main()
