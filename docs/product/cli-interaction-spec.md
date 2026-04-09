# UX / CLI Interaction Specification

[< Product Index](index.md) | [Spec Index](../spec/index.md)

## No new commands

There is no `codex run`. The user launches `codex` — the normal interactive TUI — and types their goal. The multi-agent capability is activated automatically by the Codex agent when it determines the goal requires decomposition.

```
codex                     # Launch interactive TUI (existing command, unchanged)
codex exec "goal"         # Non-interactive mode (existing command, unchanged)
```

Both modes gain multi-agent capability: the agent can spawn specialists, route to different models, and verify results — using the existing `multi_agent_v2` agent spawning system. See [Integration Model](../spec/integration-model.md).

Routing logic runs natively in Rust within codex-core (no separate process). Agent roles and routing config are in `~/.codex/config.toml` under `[agents.roles.*]` and `[routing.*]`.

## Subcommands in Detail

### `codex run <goal>`

Start a new supervised multi-agent run.

```
codex run [OPTIONS] <GOAL>

Arguments:
  <GOAL>    Natural language engineering goal (or - for stdin)

Options:
  --plan-only              Generate plan without executing
  --dry-run                Show what would be done without doing it
  --single-agent           Force single-agent mode (no decomposition)
  --task <TASK>            Skip planning, execute a single task directly
  --review                 Review mode: analyze existing changes
  --branch <BRANCH>        Target branch for review mode

  --backend <BACKEND>      Force a specific backend for all tasks
  --budget <AMOUNT>        Maximum API cost in dollars (e.g., --budget 5.00)
  --max-tasks <N>          Maximum number of tasks (default: 20)
  --max-retries <N>        Maximum retries per task (default: 3)
  --timeout <DURATION>     Maximum total run time (e.g., 30m, 2h)
  --parallel <N>           Maximum parallel agents (default: 4)

  --approve-all            Pre-approve all actions (dangerous)
  --deny-all               Auto-deny all approval requests (safe mode)
  --policy <FILE>          Path to approval policy file

  --config <FILE>          Path to config file
  --verbose                Verbose output
  --json                   JSON output (for scripting)
  --no-color               Disable colored output
  --cwd <DIR>              Working directory (default: current)
```

### `codex run status`

Show active runs.

```
codex run status [OPTIONS]

Options:
  --json                   JSON output
  --watch                  Auto-refresh (like watch)
```

Example output:
```
ID          Goal                                Status    Tasks    Elapsed
──────────  ──────────────────────────────────  ────────  ───────  ───────
r_abc123    Add rate limiting with Redis         running   3/7      2m 14s
r_def456    Fix flaky payment test               paused    1/2      0m 45s
```

### `codex run inspect <id>`

Inspect a run in detail.

```
codex run inspect <ID> [OPTIONS]

Options:
  --routing                Show routing decisions
  --task <TASK_ID>         Inspect a specific task
  --events                 Show event stream
  --diff                   Show combined diff
  --artifacts              List artifacts
  --approvals              Show approval history
  --summary                Show run summary (default)
  --json                   JSON output
```

### `codex run resume <id>`

Resume an interrupted run.

```
codex run resume <ID> [OPTIONS]

Options:
  --from-task <TASK_ID>    Resume from a specific task (re-execute it)
  --replan                 Re-run planning before resuming
  --backend <BACKEND>      Override backend for remaining tasks
```

### `codex run cancel <id>`

Cancel an active run.

```
codex run cancel <ID> [OPTIONS]

Options:
  --force                  Kill agents immediately (vs. graceful shutdown)
  --keep-worktrees         Don't clean up worktrees
```

### `codex run retry <id>`

Retry failed tasks.

```
codex run retry <ID> [OPTIONS]

Options:
  --task <TASK_ID>         Retry a specific task
  --backend <BACKEND>      Override backend for retry
  --all                    Retry all failed tasks
```

### `codex run logs <id>`

View run logs.

```
codex run logs <ID> [OPTIONS]

Options:
  --follow                 Follow log output (like tail -f)
  --task <TASK_ID>         Filter to specific task
  --level <LEVEL>          Filter by level (debug, info, warn, error)
  --json                   Raw JSON events
```

### `codex run list`

List recent runs.

```
codex run list [OPTIONS]

Options:
  --limit <N>              Number of runs (default: 20)
  --status <STATUS>        Filter: running, completed, failed, cancelled, paused
  --json                   JSON output
```

### `codex run clean`

Clean up old artifacts.

```
codex run clean [OPTIONS]

Options:
  --older-than <DURATION>  Clean runs older than (default: 30d)
  --dry-run                Show what would be cleaned
  --force                  Skip confirmation
```

## Interactive vs Non-Interactive Modes

### Interactive mode (default when TTY attached)
- TUI panel showing run progress, task status, routing decisions
- Inline approval prompts
- Live log streaming
- Keyboard shortcuts: `q` quit, `p` pause, `a` approve, `d` deny

### Non-interactive mode (--json or no TTY)
- JSON events on stdout
- Approval requests written to filesystem
- Progress via structured JSON lines
- Suitable for CI/CD, scripting, or piping

## Approval Prompt (Interactive)

```
╔══════════════════════════════════════════════════════════════╗
║ APPROVAL REQUIRED — task_4                                   ║
╠══════════════════════════════════════════════════════════════╣
║ Agent:   coder (openai/gpt-5.4)                             ║
║ Action:  shell_exec                                          ║
║ Command: npm install redis@^4.0.0                            ║
║ Policy:  "package installation requires approval"            ║
║                                                              ║
║ [a] approve  [d] deny  [s] skip  [v] view context  [p] pause║
╚══════════════════════════════════════════════════════════════╝
```

## Run Status Output (TUI)

```
┌─ Run r_abc123 ─────────────────────────────────────────────┐
│ Goal: Add rate limiting to API gateway with Redis           │
│ Status: RUNNING  Elapsed: 2m 14s  Budget: $0.28/$5.00      │
├─────────────────────────────────────────────────────────────┤
│ Tasks (3/7 complete):                                       │
│   ✓ task_1  Add Redis client         claude-code     1m 02s │
│   ✓ task_2  Rate limit middleware     openai/gpt-5.4  0m 45s │
│   ● task_3  Integration tests        ollama/qwen3     0m 27s │
│   ○ task_4  Update gateway config    (pending)              │
│   ✓ task_5  Docker compose update    ollama/qwen3     0m 12s │
│   ○ task_6  Full test suite          (blocked: task_3,4)    │
│   ○ task_7  Review all changes       (blocked: task_6)      │
├─────────────────────────────────────────────────────────────┤
│ Live: task_3 — writing test_rate_limiter.py (turn 3/10)     │
└─────────────────────────────────────────────────────────────┘
```

## Example Command Shapes

```bash
# Basic multi-agent run
codex run "Add user authentication with JWT tokens"

# Plan only, then review
codex run --plan-only "Migrate database from Postgres to CockroachDB"

# Budget-constrained run
codex run --budget 2.00 "Fix all TypeScript strict mode errors"

# Force local models only (offline/private)
codex run --backend ollama "Rename the User model to Account throughout the codebase"

# Review a branch
codex run review --branch feature/payments

# Resume with different backend
codex run resume r_abc123 --backend claude-code

# Inspect routing for a completed run
codex run inspect r_abc123 --routing

# Watch all active runs
codex run status --watch

# CI mode: non-interactive with JSON output
codex run --json --approve-all "Run lint fixes and commit" | jq '.event'

# Pipe goal from file
cat goal.md | codex run -

# Single task, no planning
codex run --task "Add error handling to the payment webhook handler"

# Dry run to see what would happen
codex run --dry-run "Upgrade all dependencies to latest"
```

## Configuration File Example

```toml
# ~/.codex/multi-agent.toml (or .codex/multi-agent.toml per project)

[run]
max_parallel_agents = 4
max_tasks = 20
max_retries = 3
default_timeout = "30m"
budget_limit = 10.00

[routing]
prefer_subscription = true          # Use subscription tools before API billing
offline_fallback = "ollama"         # Backend when no network
default_backend = "auto"            # "auto", "openai", "claude-code", "ollama"

[routing.ollama]
router_model = "qwen3:8b-q4_K_M"   # Model for routing decisions
coder_model = "qwen3-coder:30b"    # Local coder model
reasoner_model = "qwen3:14b"       # Local reasoner model

[verification]
enabled = true
command = "make test"               # Verification command
timeout = "5m"
on_failure = "retry"                # "retry", "fail", "skip"

[approval]
policy = "risky-only"               # "always", "risky-only", "never"
timeout = "5m"
default_on_timeout = "deny"

[approval.risky_patterns]
shell = ["rm ", "docker", "systemctl", "kill", "reboot"]
files = ["*.lock", "Dockerfile", "*.yml", "*.yaml", ".env*"]
git = ["push", "rebase", "reset", "merge"]

[providers.openai]
enabled = true
api_key_env = "OPENAI_API_KEY"
cost_category = "api"

[providers.claude_code]
enabled = true
command = "claude"
cost_category = "subscription"

[providers.ollama]
enabled = true
base_url = "http://127.0.0.1:11434"
cost_category = "free"
```
