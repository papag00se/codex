# System Context Diagram

[< Spec Index](index.md) | [Product Index](../product/index.md)

See [System Architecture](system-architecture.md) for narrative description and [Logical Components](logical-components.md) for detailed component specifications.

## High-Level System Context

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           DEVELOPER MACHINE                              │
│                                                                          │
│  ┌──────────┐                                                           │
│  │  User     │                                                           │
│  │ (human)   │                                                           │
│  └────┬─────┘                                                           │
│       │ goal / approve / cancel                                          │
│       ▼                                                                  │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │                         CLI LAYER                                 │   │
│  │  codex run <goal>  |  status  |  inspect  |  resume  |  cancel   │   │
│  └────────────────────────────┬─────────────────────────────────────┘   │
│                               │                                          │
│                               ▼                                          │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │                     ORCHESTRATOR / SUPERVISOR                     │   │
│  │                                                                   │   │
│  │  ┌─────────┐  ┌──────────┐  ┌──────────┐  ┌─────────────────┐  │   │
│  │  │ Planner │  │ Scheduler│  │ Verifier │  │ Approval Gate   │  │   │
│  │  └────┬────┘  └────┬─────┘  └────┬─────┘  └───────┬─────────┘  │   │
│  │       │            │             │                 │             │   │
│  │  ┌────┴────────────┴─────────────┴─────────────────┴──────────┐ │   │
│  │  │                    TASK STATE MACHINE                        │ │   │
│  │  │  planned → routed → assigned → running → verifying →       │ │   │
│  │  │  approved → complete  (or: failed / cancelled / retrying)   │ │   │
│  │  └────────────────────────────────────────────────────────────┘ │   │
│  └────────────────────────────┬─────────────────────────────────────┘   │
│                               │                                          │
│              ┌────────────────┼────────────────┐                        │
│              ▼                ▼                 ▼                        │
│  ┌───────────────┐  ┌──────────────┐  ┌───────────────┐                │
│  │ ROUTING LAYER │  │ AGENT LAYER  │  │ INFRA LAYER   │                │
│  │               │  │              │  │               │                │
│  │ ┌───────────┐ │  │ ┌──────────┐ │  │ ┌───────────┐ │                │
│  │ │Task Router│ │  │ │  Coder   │ │  │ │ SQLite DB │ │                │
│  │ │(per-task) │ │  │ │  Agent   │ │  │ │ (state)   │ │                │
│  │ └─────┬─────┘ │  │ ├──────────┤ │  │ ├───────────┤ │                │
│  │       │       │  │ │ Reviewer │ │  │ │ JSONL Log │ │                │
│  │ ┌─────┴─────┐ │  │ │  Agent   │ │  │ │ (events)  │ │                │
│  │ │  coding-  │ │  │ ├──────────┤ │  │ ├───────────┤ │                │
│  │ │  agent-   │ │  │ │  Test    │ │  │ │ Worktree  │ │                │
│  │ │  router   │ │  │ │Interpret.│ │  │ │ Manager   │ │                │
│  │ │(per-req)  │ │  │ └──────────┘ │  │ └───────────┘ │                │
│  │ └───────────┘ │  └──────────────┘  └───────────────┘                │
│  └───────┬───────┘                                                      │
│          │                                                               │
│          ▼                                                               │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │                      PROVIDER ADAPTERS                            │   │
│  │                                                                   │   │
│  │  ┌────────────┐  ┌────────────┐  ┌──────────┐  ┌────────────┐  │   │
│  │  │  Codex CLI │  │Claude Code │  │  Ollama  │  │ OpenAI API │  │   │
│  │  │  Adapter   │  │  Adapter   │  │ Adapter  │  │  Adapter   │  │   │
│  │  └─────┬──────┘  └─────┬──────┘  └────┬─────┘  └─────┬──────┘  │   │
│  │        │               │              │               │         │   │
│  └────────┼───────────────┼──────────────┼───────────────┼─────────┘   │
│           │               │              │               │              │
└───────────┼───────────────┼──────────────┼───────────────┼──────────────┘
            │               │              │               │
            ▼               ▼              ▼               ▼
     ┌────────────┐  ┌────────────┐  ┌──────────┐  ┌────────────┐
     │ codex exec │  │  claude    │  │ Ollama   │  │ OpenAI     │
     │ (process)  │  │ (process)  │  │ (local)  │  │ (cloud)    │
     └────────────┘  └────────────┘  └──────────┘  └────────────┘
```

## Data Flow Diagram

```
                    ┌─────────────────────┐
                    │    Event Log        │
                    │    (JSONL)          │
                    └──────▲──────────────┘
                           │ append
                           │
User ──goal──► CLI ──► Orchestrator ──task──► Router ──► Provider ──► Backend
                    │      │                                │
                    │      │◄─── result ◄─── agent ◄───────┘
                    │      │
                    │      ├──verify──► Verifier ──► test runner
                    │      │              │
                    │      │◄── pass/fail─┘
                    │      │
                    │      ├──approve──► Approval Gate ──► User
                    │      │                  │
                    │      │◄── granted/denied┘
                    │      │
                    │      ├──persist──► SQLite DB
                    │      │
                    │      └──worktree──► Git Worktree Manager
                    │
                    └──output──► User
```

## Component Interaction for a Single Task

```
Orchestrator                  Router              Provider           Agent
    │                           │                    │                 │
    ├── route_task(task) ──────►│                    │                 │
    │                           ├── select_backend ──┤                 │
    │◄── routing_decision ──────┤                    │                 │
    │                           │                    │                 │
    ├── create_worktree ───────────────────────────────────────────────┤
    │                           │                    │                 │
    ├── dispatch(task, wt) ────────────────────────►│                 │
    │                           │                    ├── spawn ───────►│
    │                           │                    │                 │
    │                           │                    │◄── result ──────┤
    │◄── worker_result ─────────────────────────────┤                 │
    │                           │                    │                 │
    ├── verify(result) ─────────────────────────────────────────────── │
    │◄── verification_result    │                    │                 │
    │                           │                    │                 │
    ├── [if risky] request_approval ──────────────────────────────── User
    │◄── approval_decision      │                    │                 │
    │                           │                    │                 │
    ├── merge_worktree          │                    │                 │
    ├── persist_state           │                    │                 │
    ├── emit_event              │                    │                 │
    │                           │                    │                 │
```
