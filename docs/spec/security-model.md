# Security Model

[< Spec Index](index.md) | [Product Index](../product/index.md)

## Trust boundaries

Even on a single machine, the system has trust boundaries:

```
┌─────────────────────────────────────────────────────┐
│ TRUSTED: Orchestrator, CLI, State Store              │
│   - Full filesystem access for state management      │
│   - Full git access for worktree management          │
│   - Credential access for provider auth              │
├─────────────────────────────────────────────────────┤
│ SEMI-TRUSTED: Provider Adapters                      │
│   - Network access to specific provider endpoints    │
│   - Read access to task descriptions                 │
│   - No direct filesystem write access                │
├─────────────────────────────────────────────────────┤
│ UNTRUSTED: Agent Execution (within worktree)         │
│   - Sandboxed filesystem access (worktree only)      │
│   - Sandboxed network access (restricted)            │
│   - Shell execution gated by policy                  │
│   - No access to credentials or state store          │
└─────────────────────────────────────────────────────┘
```

## Secrets handling

| Secret type | Storage | Access |
|-------------|---------|--------|
| Provider API keys | Environment variables or OS keychain | Read by provider adapters only |
| Provider OAuth tokens | OS keychain (via codex-login) | Read by provider adapters only |
| Repository secrets (.env, .pem) | Never read by agents | Blocked by policy engine |
| State store | Unencrypted local file (SQLite) | Orchestrator only |
| Event log | Unencrypted local file (JSONL) | Orchestrator only |

### Rules
1. API keys are never written to event logs or state store
2. API keys are passed to provider adapters via environment variables, never command-line arguments (which are visible in `ps`)
3. Agent subprocess environments are stripped of all credential environment variables except the one needed for their specific provider
4. Files matching secret patterns (`.env*`, `*.pem`, `*.key`, `credentials.*`) are never read by agents (enforced by the [Policy Engine](verification-safety.md)) unless explicitly allow-listed

## Subprocess controls

Agent backends (Codex CLI, Claude Code) are invoked as subprocesses with:

1. **Working directory restricted to worktree** — `--cwd <worktree_path>`
2. **Environment sanitized** — Only necessary variables passed:
   ```python
   agent_env = {
       "HOME": os.environ["HOME"],
       "PATH": os.environ["PATH"],
       "LANG": os.environ.get("LANG", "en_US.UTF-8"),
       # Provider-specific key (only the one needed):
       "OPENAI_API_KEY": "...",  # or ANTHROPIC_API_KEY
   }
   ```
3. **Process group isolation** — Agent process and its children in a dedicated process group for clean termination
4. **Resource limits** — Timeout enforced externally (SIGTERM after timeout, SIGKILL after grace period)
5. **Sandbox inheritance** — Codex CLI's existing sandbox model (Seatbelt on macOS, seccomp on Linux) is respected. Claude Code's sandbox model is also respected.

## Provider credential isolation

Each provider adapter receives only the credentials it needs:

| Provider | Credential | How passed |
|----------|-----------|------------|
| Codex CLI | OPENAI_API_KEY or OAuth session | Environment variable |
| Claude Code | ANTHROPIC_API_KEY or session | Environment variable |
| Ollama | None (local) | N/A |
| OpenAI API | OPENAI_API_KEY | Environment variable |
| Anthropic API | ANTHROPIC_API_KEY | Environment variable |

No provider adapter can access another provider's credentials.

## Local model safety limits

Ollama models run locally and have no built-in safety alignment comparable to API models. Additional guardrails:

1. **Mandatory verification** — All local model outputs must pass verification before acceptance
2. **Restricted tool access** — Local model agents have the same tool policy as cloud agents (policy engine applies uniformly)
3. **Output size limits** — Local model outputs are bounded (max tokens configured per model)
4. **No credential access** — Local model agents never receive API keys

## Filesystem write boundaries

| Component | Write access |
|-----------|-------------|
| Orchestrator | `~/.codex/multi-agent/` (state, logs, artifacts) |
| Worktree Manager | `.codex-worktrees/` (within repository) |
| Agents | Their assigned worktree directory only |
| Provider Adapters | Temp files only (`/tmp/codex-*`) |

Agents cannot write outside their worktree. This is enforced by:
- Codex CLI's sandbox (for Codex-backed agents)
- Claude Code's sandbox (for Claude-backed agents)
- Working directory restriction for Ollama-backed agents

## Git operation boundaries

| Operation | Allowed by | Scope |
|-----------|-----------|-------|
| git add, commit | Agents | Within worktree branch only |
| git branch | Worktree Manager | codex/* branches only |
| git worktree add/remove | Worktree Manager | .codex-worktrees/ only |
| git merge | Orchestrator | Into result branch only |
| git push | User (after approval) | Never automatic in v1 |
| git rebase, reset | Never | Blocked by policy |

## Network controls

| Component | Network access |
|-----------|---------------|
| Orchestrator | None needed (local-only) |
| Provider Adapters | Outbound to provider endpoints only |
| Agents (cloud-backed) | Via provider subprocess (inherits provider's network access) |
| Agents (local-backed) | Restricted (Ollama is local; agent shell commands have no network by default) |
| Orchestrator (Ollama client) | Outbound to local Ollama endpoints only |

## Least privilege rules

1. No component has more access than it needs
2. Agents run with minimal environment variables
3. Agents run in sandboxed subprocesses
4. The orchestrator never executes user-provided shell commands directly — it delegates to agents
5. Provider adapters parse and validate all responses before passing to orchestrator
6. Policy engine defaults to "deny" for any unrecognized action pattern
