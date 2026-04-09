# Future Expansion

[< Spec Index](index.md) | [Product Index](../product/index.md)

> See [Gaps](gaps.md) for the prioritized list of known gaps with implementation notes.

## How the architecture supports future growth

The architecture is designed with explicit extension points. Each future capability maps to existing interfaces.

## Richer model routing

**Extension point:** [ProviderCapability](provider-abstraction.md) schema, [routing scorer](routing-architecture.md)

**How:** Add new capability dimensions (e.g., `code_review_security: 0.9`, `refactor_large_codebase: 0.85`). Add new scoring factors (e.g., model freshness, training data recency). Optionally: replace rule-based scorer with learned routing model trained on historical run data.

**No architectural changes needed.** Just new fields in capability config and new weights in the scorer.

## More providers

**Extension point:** [ProviderAdapter](provider-adapter-interface.md) ABC

**How:** Implement new adapter class. Examples:
- `GeminiAdapter` — Google Gemini API
- `MistralAdapter` — Mistral API
- `DeepSeekAdapter` — DeepSeek API
- `GroqAdapter` — Groq inference
- `BedrockAdapter` — AWS Bedrock
- `AzureAdapter` — Azure OpenAI

Each adapter implements `execute_task`, `capabilities`, `health`, `estimate_cost`, `cancel`. Register in config. Routing engine automatically includes them in scoring.

## Browser agents

**Extension point:** [Agent taxonomy](agent-taxonomy.md), new agent type

**How:** Add `BrowserAgent` that can interact with web UIs (for testing frontend changes, checking deployed services). Requires a new tool type (browser automation via Playwright/Puppeteer) and a new provider adapter that manages browser sessions.

**Architectural addition:** Tool registry needs a "browser" tool type. Policy engine needs browser-specific rules (allowed domains, etc.).

## Deployment agents

**Extension point:** Agent taxonomy, new agent type

**How:** Add `DeploymentAgent` that can trigger deployments, check health, and rollback. Extremely risky — requires robust approval policy.

**Architectural addition:** New approval policy category ("deployment") with mandatory human approval and confirmation.

## Research agents

**Extension point:** Agent taxonomy, new agent type

**How:** Add `ResearchAgent` that can search documentation, Stack Overflow, GitHub issues to gather context for tasks. Read-only, no code modifications.

**Architectural addition:** New tool type (web search). New provider consideration (some models are better at search/synthesis).

## Codebase-wide refactors

**Extension point:** Planner, parallelism model

**How:** Planner produces a large task graph (50+ tasks) for a refactor like "rename User to Account everywhere." Tasks are small (one file each) and highly parallel. Routing: all go to fast/cheap backend (Ollama). Verification: type checker + tests after all merges.

**Architectural consideration:** May need "batch merge" — merge all worktrees at once instead of incrementally. The worktree manager needs a `merge_all` method that detects conflicts across the full batch.

## Cross-repo workflows

**Extension point:** RepositoryContext, worktree manager

**How:** A Run can span multiple repositories. Each task specifies its target repo. Worktrees are created in the correct repo. Routing decisions consider which repo a task targets.

**Architectural addition:** `RepositoryContext` becomes a list. Worktree manager supports multiple repos. Task schema adds `repo_path` field.

## Richer policy engines

**Extension point:** Policy engine

**How:** Replace pattern matching with a full policy language (e.g., OPA/Rego, or a custom DSL). Policies can reference task metadata, routing decisions, agent identity, time of day, etc.

**Architectural change:** Policy engine interface stays the same (`evaluate(action) → decision`), but the implementation becomes pluggable.

## Dashboard UI

**Extension point:** State store, event log

**How:** A web UI that reads from the same SQLite state store and JSONL event log. No new data infrastructure — the UI is a read-only view of existing data.

**Architectural addition:** The state store may need HTTP API exposure (simple FastAPI wrapper) or the UI reads SQLite directly.

## Distributed execution

**Extension point:** Event bus, worker dispatch

**How:** Replace in-process asyncio channels with a real message queue (Redis Streams, NATS). Replace subprocess agent dispatch with remote worker dispatch. State store remains SQLite (or upgrades to Postgres).

**Architectural changes:**
- Event bus: asyncio.Queue → Redis Streams
- Worker dispatch: local subprocess → remote task queue
- State store: SQLite → Postgres (for concurrent access from multiple workers)
- Worktree manager: git worktrees on local disk → git clone on worker machines

This is the largest architectural change and should only be pursued if single-machine performance is genuinely insufficient. The v1 architecture is designed so this migration affects only the transport and storage layers — the orchestrator logic, routing, and agent taxonomy remain unchanged.
