You are Claude Code acting as a senior systems architect, staff platform engineer, and implementation planner.

Your assignment is to help design a **docs-first, production-oriented multi-agent coding CLI** built from these starting points:

- the **open source Codex CLI** as the initial CLI foundation
- an existing custom project called **`coding-agent-router`**
- support for **Claude Code**
- support for **multiple execution backends**:
  - subscription-backed tools
  - API-based providers
  - local models via Ollama
  - future providers with the same abstraction layer

This system is for **general software engineering**, especially complex, multi-service systems.
Do not assume any one vendor should dominate the design.

The target is a CLI-centered agentic development platform where:
- a user invokes one CLI
- a supervisor/orchestrator decomposes work into bounded tasks
- multiple specialist agents are engaged automatically
- each agent task can be routed to the most appropriate backend
- backend selection can use policy, heuristics, budget, capability, task type, latency, privacy, and model availability
- execution is observable, auditable, resumable, and verifiable
- the system remains understandable and deterministic at the control-plane level

This is **not** a naive “spawn a swarm and hope” design.
This is a controlled multi-agent coding system with:
- explicit supervisor loops
- event-driven orchestration
- durable state
- verification loops
- approval gates for risky actions
- routing across heterogeneous model backends

## Known starting point

Assume the operator already has:

1. **Codex CLI source downloaded**
   - This is the likely initial CLI shell / UX foundation.
   - You should inspect and reason about how much of its structure should be reused versus wrapped versus forked.

2. **A project named `coding-agent-router`**
   - This already contains logic for deciding when to route work to OpenAI backend models versus local models.
   - Treat this as an existing strategic asset, not a throwaway prototype.
   - You should plan around harvesting its design ideas, interfaces, heuristics, and abstractions where useful.
   - Your plan should include deciding the boundaries between the two projects 
     - It is possible `coding-agent-router` already has code that should instead be in the CLI, breaking that boundary. Include fixing boundary violations.
   - Your plan should include updating `coding-agent-router` as a first-class service to this solution
   - `coding-agent-router` specializes in deciding where to route **on each model request** - not per turn/agent.

3. **Claude Code access**
   - Claude Code should be treated as a first-class participant in the final architecture, not merely a fallback.
   - The system should be capable of using Claude Code for some agent roles and Codex or local models for others.

## Core product idea

Design a CLI platform that can do things like:
- accept a high-level engineering goal
- break it into tasks
- create a plan
- assign specialized agents
- route each task to the best backend/model/tool combination
- execute code changes in a controlled way
- run tests and verification
- ask for approval when needed
- continue the supervisor loop until explicit completion conditions are met
- preserve logs, artifacts, state, and rationale

Examples of backend types to support conceptually:
- Codex / OpenAI-backed coding agent paths
- Claude Code / Anthropic-backed coding paths
- API-invoked models from multiple providers
- local Ollama models
- future router targets

Examples of routing factors to consider:
- code generation quality
- long-context reasoning needs
- diff/refactor capability
- cost sensitivity
- latency sensitivity
- privacy sensitivity
- offline availability
- task criticality
- deterministic workflow requirements
- model-specific strengths
- subscription entitlement vs billable API usage

## Required product framing

The design must clearly distinguish between:

- **CLI surface**
- **orchestrator / supervisor loop**
- **specialist agents**
- **routing layer**
- **provider adapters**
- **durable state**
- **event model**
- **verification loop**
- **policy / approval gates**
- **repository/worktree isolation**
- **observability**

The design must also clearly distinguish what is:
- agentic and heuristic
- deterministic and rule-driven

Deterministic parts should include at minimum:
- state transitions
- retry scheduling
- timeout handling
- policy checks
- approval gating
- artifact publication rules
- event persistence
- concurrency controls

## Architectural constraints

Design for a **single developer machine first**.
Do not assume Kubernetes.
Do not assume distributed deployment in v1.
Do not assume cloud-managed services are required.

Prefer:
- local filesystem plus SQLite for v1 if sufficient
- structured JSON/YAML/TOML config
- subprocess execution where appropriate
- Git worktrees or equivalent repo isolation for parallel work
- simple local process supervision
- adapters for provider-specific backends

Avoid:
- giant opaque agent frameworks without justification
- excessive microservices
- uncontrolled recursive delegation
- chat transcripts as the only memory/state substrate
- magic routing with no observability
- hidden policy logic
- provider lock-in

Favor:
- composable primitives
- explicit interfaces
- typed schemas
- replayable events
- bounded loops
- role-based agents
- pluggable routing
- measurable verification
- human review for risky operations

## Product objective

Produce a **docs-first design package** and then an **execution plan** for building this system.
Store product docs in `docs/product` and spec docs in `docs/spec`
You must not jump straight into coding.
You must first produce product and technical documentation that would let a senior engineer review the system before implementation.

## Your output must follow this exact phase order

# PHASE 1 — Discovery and design synthesis

Before writing final docs, synthesize the problem clearly.

## 1. Problem statement
Define:
- what the product is
- what problem it solves
- why a multi-agent routed coding CLI is useful
- what specific gap exists between a single-agent CLI and this target architecture

## 2. Goals and non-goals
State:
- product goals
- technical goals
- operational goals
- explicit non-goals for v1

## 3. Assumptions
List all major assumptions you are making about:
- Codex CLI as a base
- `coding-agent-router`
- Claude Code participation
- local execution environment
- repository structure
- provider access patterns
- subscription vs API considerations

Where assumptions create risk, flag them.

## 4. Key design questions
Enumerate the most important design questions that the later docs must answer.
Examples:
- wrap Codex CLI vs fork it?
- is routing done per job, per task, or per tool invocation?
- how should Claude Code integrate?
- how should subscription-backed tools be abstracted versus API-backed tools?
- what should local models be allowed to do?
- how do we keep routing decisions auditable?
- how do we bound multi-agent recursion?

# PHASE 2 — PRD and product docs

Create docs-first product documentation.

## 5. PRD
Write a full Product Requirements Document containing:
- summary
- problem
- users/personas
- target use cases
- user stories
- functional requirements
- non-functional requirements
- constraints
- success criteria
- MVP scope
- out-of-scope
- major risks
- open questions

## 6. Product concept doc
Describe the user-facing concept:
- what the CLI feels like
- what workflows it supports
- how a user invokes multi-agent mode
- how they inspect routing decisions
- how they approve risky actions
- how they review outputs
- how the system resumes interrupted work

## 7. UX / CLI interaction spec
Define:
- top-level commands
- likely subcommands
- flags/options
- interactive vs non-interactive modes
- approval prompts
- run status output
- logs/trace views
- dry-run mode
- plan-only mode
- review-only mode

Include concrete example command shapes.

# PHASE 3 — Architecture and technical specification

Now produce the technical design docs.

## 8. System architecture overview
Explain the architecture in plain engineering language.

## 9. Core architectural principles
Include:
- bounded loops
- event-driven orchestration
- durable state
- verification-first
- provider abstraction
- routing transparency
- human escalation
- local-first execution
- repo isolation
- restart/resume safety

## 10. System context diagram
Provide ASCII diagrams showing:
- user
- CLI
- supervisor/orchestrator
- routing engine
- specialist agents
- provider adapters
- durable state
- local repositories/worktrees
- verifier/test runners
- approval gates
- logs/metrics/traces

## 11. Logical components
At minimum define these components:
- CLI shell
- orchestrator/supervisor
- planner agent
- specialist worker agents
- routing engine
- provider capability registry
- provider adapters
- policy engine
- approval gate
- job/task state store
- event bus/queue
- repository/worktree manager
- verifier/reviewer layer
- artifact manager
- observability layer

For each component provide:
- purpose
- responsibilities
- inputs
- outputs
- failure modes
- implementation notes
- alternatives considered

## 12. Agent taxonomy
Define the agents and their roles, such as:
- supervisor
- planner
- coder
- reviewer
- test interpreter
- docs writer
- refactor agent
- dependency analyst
- integration agent
- release/readiness agent

For each agent provide:
- mission
- allowed tools
- forbidden actions
- autonomy limits
- expected outputs
- escalation triggers
- preferred backend/model characteristics

## 13. Routing architecture
This is a critical section.

Design the routing model in depth.

Cover:
- why routing exists
- routing granularity options
- recommended routing granularity for v1
- how subscription-based and API-based providers differ
- how local Ollama models differ
- how Claude Code should integrate
- how Codex-backed paths should integrate
- how provider/model capabilities are represented
- how routing decisions are computed
- how routing decisions are logged and explained
- fallback behavior
- failure behavior
- offline behavior
- budget-aware behavior
- privacy-aware behavior
- quality-sensitive routing
- confidence thresholds
- manual override options

Explicitly discuss whether routing should happen:
- per run
- per phase
- per task
- per retry
- per tool invocation

Make a recommendation and justify it.

## 14. Provider abstraction spec
Define the abstraction layer for heterogeneous backends.

Include concepts such as:
- provider
- model
- tool-enabled agent backend
- subscription-backed execution target
- API-backed execution target
- local execution target
- capability metadata
- cost metadata
- latency metadata
- privacy level
- context window
- code editing strength
- review strength
- reliability score

Define what a provider adapter must expose.

## 15. State model
Define durable state for:
- jobs
- runs
- tasks
- agent assignments
- routing decisions
- approval requests
- artifacts
- retries
- failures
- repo/worktree handles
- timestamps
- cancellation
- timeout
- resume state

Provide concrete schema suggestions.

## 16. Event model
Define event types such as:
- run.created
- plan.requested
- plan.generated
- task.created
- task.assigned
- route.selected
- task.started
- task.completed
- task.failed
- verification.requested
- verification.passed
- verification.failed
- approval.requested
- approval.granted
- approval.denied
- retry.scheduled
- run.paused
- run.resumed
- run.cancelled
- artifact.published

For each event define:
- producer
- consumers
- payload shape
- idempotency concerns

## 17. Verification and safety model
Define:
- what gets verified
- when
- by whom
- what is deterministic vs agentic
- what requires human approval
- what can auto-retry
- what must hard-fail

Include policies for:
- shell commands
- package installation
- editing lockfiles
- editing CI/CD
- changing infra code
- deleting files
- secret access
- network calls
- git history rewriting
- branch merges
- artifact publication

## 18. Repository isolation model
Design how parallel agent work should be isolated.
Discuss:
- same worktree vs separate worktrees
- branch strategy
- patch/diff handoff
- merge/rebase/review flow
- conflict handling
- cleanup
- failure recovery

Make a concrete recommendation for v1.

## 19. Observability model
Define:
- logs
- metrics
- traces
- audit history
- run summaries
- routing summaries
- approval history
- failure reports
- replay/debug strategy

## 20. Security model
Define:
- trust boundaries even on one machine
- secrets handling
- subprocess controls
- provider credential isolation
- local model safety limits
- filesystem write boundaries
- git operation boundaries
- network controls
- least privilege rules

## 21. Technology decision record
For each major subsystem recommend:
- preferred v1 choice
- alternatives
- tradeoffs
- why alternatives are not preferred for v1

At minimum cover:
- CLI strategy
- orchestrator runtime
- routing engine implementation
- provider adapter model
- state store
- queue/event transport
- repo isolation method
- verifier/test harness
- observability stack

## 22. Risks and failure modes
List realistic risks with mitigations, including:
- runaway supervisor loops
- bad routing decisions
- silent degraded quality
- stale provider capability metadata
- subscription/API drift
- model unavailability
- provider-specific UX mismatch
- broken repo state
- duplicate event handling
- false verification passes
- excessive cost
- local model hallucination
- operator overload

# PHASE 4 — Execution plan

Now create a build plan detailed enough to execute.

## 23. Monorepo/project structure
Propose the repository structure.
Include directories for:
- docs
- cli
- orchestrator
- agents
- router
- providers
- policies
- schemas
- storage
- events
- repo isolation
- verification
- tests
- scripts
- examples

## 24. MVP definition
Define the smallest useful v1.
It must include:
- one CLI
- one supervisor/orchestrator
- at least 3 specialist agents
- routing across at least 3 backend categories
- durable state
- event persistence
- repository-aware execution
- verification step
- human approval gate
- resume/retry support
- routing explanation output

Clearly state what is out of scope.

## 25. Milestone plan
Break the work into milestones.
Recommended pattern:
- M0 discovery/import analysis
- M1 schemas/state/events
- M2 CLI foundation
- M3 orchestrator loop
- M4 routing engine
- M5 provider adapters
- M6 repo/worktree isolation
- M7 verifier/approval
- M8 observability
- M9 hardening

For each milestone include:
- goal
- deliverables
- dependencies
- exit criteria

## 26. Detailed task backlog
Create a detailed backlog grouped by milestone.
For each task include:
- ID
- title
- description
- dependencies
- priority
- complexity
- acceptance criteria

## 27. Implementation order
Provide the exact recommended build order with rationale.

## 28. Initial interfaces and contracts
Define initial schemas/contracts for:
- Run
- Task
- RoutingDecision
- ProviderCapability
- ProviderAdapter
- ApprovalRequest
- ArtifactRecord
- RepositoryContext
- WorkerResult
- Event

Provide sample payloads.

## 29. Operational model
Explain:
- how the CLI starts runs
- how the orchestrator persists progress
- how agents receive tasks
- how provider routing is invoked
- how worktrees are created
- how results are verified
- how approvals pause/resume the run
- how crashes/restarts recover
- how completed artifacts are published

## 30. Testing strategy
Define:
- unit tests
- integration tests
- replay tests
- routing tests
- provider adapter contract tests
- repo isolation tests
- approval path tests
- crash recovery tests
- timeout/retry tests
- cost-control tests

## 31. Rollout strategy
Explain how to introduce this system incrementally without over-automation.

## 32. Future expansion
Show how the architecture can later support:
- richer model routing
- more providers
- browser agents
- deployment agents
- research agents
- codebase-wide refactors
- cross-repo workflows
- richer policy engines
- dashboard UI
- distributed execution if ever needed

# PHASE 5 — Implementation scaffolding only

Only after all docs and plans are written, produce scaffolding.

Do not implement the whole system.
Produce starter assets only.

## 33. Starter file tree
Output a concrete starter file tree.

## 34. Initial data models
Provide initial JSON schemas or Pydantic models for:
- Run
- Task
- RoutingDecision
- ProviderCapability
- ApprovalRequest
- ArtifactRecord
- RepositoryContext
- WorkerResult
- Event

## 35. Minimal orchestrator pseudocode
Provide strong pseudocode for:
- supervisor loop
- task creation
- routing selection
- worker dispatch
- verification gate
- approval pause/resume
- retry logic
- crash recovery

## 36. Minimal provider adapter interface
Define the first interface that all provider adapters must satisfy.

## 37. Minimal service/process list
Define the first processes/services/modules to create.

## 38. Week-1 execution checklist
Provide a concrete week-1 checklist for an engineer.

## Output requirements

- Be concrete.
- Be skeptical.
- Do not hand-wave.
- Do not say “best practices” without specifying them.
- Prefer explicit tradeoffs.
- Prefer a simple v1.
- Treat this as a real architecture and product review.
- Use markdown extensively.
- Use tables where useful.
- Use ASCII diagrams where useful.
- Use code fences for schemas and pseudocode.
- Distinguish clearly between:
  - product docs
  - architecture docs
  - execution plan
  - scaffolding

## Additional instructions

Throughout the deliverable, explicitly map decisions back to these five ideas:
- supervisor loop
- event-driven orchestration
- specialist agents
- durable state
- verification loop

Also explicitly identify where systems like:
- Codex-backed workflows
- Claude Code
- API-based providers
- local Ollama models

fit into the architecture.

You should optimize for a design that is:
- portable
- inspectable
- debuggable
- vendor-flexible
- suitable for serious software engineering work

Now produce the full deliverable in the required phase order.