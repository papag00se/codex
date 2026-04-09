# Verification and Safety Model

[< Spec Index](index.md) | [Product Index](../product/index.md)

## What gets verified

| Artifact | Verification method | When | By whom |
|----------|-------------------|------|---------|
| Code changes (diffs) | Test execution | After every code task | Verifier (deterministic) |
| Code changes (quality) | LLM review | After code tasks on critical paths | Reviewer agent (agentic) |
| Shell command proposals | Policy pattern match | Before execution | Policy engine (deterministic) |
| File deletions | Policy pattern match | Before execution | Policy engine (deterministic) |
| Git operations | Policy pattern match | Before execution | Policy engine (deterministic) |
| Package installations | Policy pattern match | Before execution | Policy engine (deterministic) |
| Merged worktree | Conflict check | After merge | Worktree manager (deterministic) |
| Full run result | Full test suite | After all tasks complete | Verifier (deterministic) |

## Verification flow

```
Task completes
    │
    ▼
Run verification command (if configured)
    │
    ├── Exit code 0 → verification.passed
    │       │
    │       ├── [No approval needed] → task.completed
    │       │
    │       └── [Approval needed] → approval.requested → wait
    │
    ├── Exit code non-0 → verification.failed
    │       │
    │       ├── [Retries remain] → retry with feedback
    │       │
    │       └── [No retries] → task.failed
    │
    └── Command not found / timeout → verification.error
            │
            └── Log warning, treat as unverified-complete
```

## Deterministic vs agentic verification

| Verification type | Nature | Implementation |
|-------------------|--------|----------------|
| Test execution | Deterministic | Subprocess: run test command, check exit code |
| Lint check | Deterministic | Subprocess: run linter, check exit code |
| Type check | Deterministic | Subprocess: run type checker, check exit code |
| Test output interpretation | Agentic | LLM parses test failures and suggests fixes |
| Code review | Agentic | LLM analyzes diffs for bugs, security, style |
| Conflict detection | Deterministic | Git merge with --no-commit, check status |

**Rule:** Deterministic verification always runs first. Agentic verification is optional and additive.

## What requires human approval

### Default policy (risky-only)

| Action | Requires Approval | Rationale |
|--------|------------------|-----------|
| Read files | No | No side effects |
| Write files (source code) | No | Contained in worktree, verified |
| Write files (config, infra) | Yes | Higher blast radius |
| Shell commands (read-only: ls, cat, grep) | No | No side effects |
| Shell commands (build: make, npm run) | No | Standard development |
| Shell commands (install: npm install, pip install) | Yes | Modifies dependencies |
| Shell commands (destructive: rm, docker rm) | Yes | Data loss risk |
| Shell commands (system: systemctl, kill) | Yes | System state change |
| Git add, commit (within worktree) | No | Contained in worktree branch |
| Git push | Yes | Leaves local machine |
| Git merge, rebase | Yes | Modifies shared history |
| Git reset, force operations | Yes | Destructive |
| Network calls (API requests) | No | Required for provider communication |
| Network calls (downloads) | Yes | Untrusted content |
| Secret/credential access | Yes | Always |

### Policy configuration

```toml
[approval.policy]
default = "approve"    # Default for unmatched actions

# Shell commands
[approval.policy.shell]
approve = ["ls", "cat", "grep", "find", "head", "tail", "wc", "diff",
           "make", "npm run", "npm test", "pytest", "cargo test", "go test"]
deny = ["rm -rf /", ":(){ :|:& };:", "dd if="]
require_approval = ["rm", "docker", "systemctl", "kill", "brew install",
                     "apt install", "npm install", "pip install"]

# File operations
[approval.policy.files]
approve = ["*.py", "*.ts", "*.js", "*.rs", "*.go", "*.java", "*.rb",
           "*.html", "*.css", "*.md", "*.txt", "*.json", "*.toml", "*.yaml"]
require_approval = ["*.lock", "Dockerfile*", "docker-compose*",
                     ".github/*", ".env*", "*.tf", "*.tfvars",
                     "Makefile", "Justfile", "*.sh"]
deny = [".git/config", ".ssh/*", "*.pem", "*.key"]

# Git operations
[approval.policy.git]
approve = ["add", "commit", "status", "log", "diff", "branch"]
require_approval = ["push", "merge", "rebase", "tag"]
deny = ["push --force", "reset --hard", "clean -fd"]
```

## What can auto-retry

| Failure type | Auto-retry? | Max retries | Notes |
|--------------|------------|-------------|-------|
| Agent turn limit exceeded | Yes | 3 | May escalate to stronger backend |
| Verification failed (tests fail) | Yes | 3 | Agent gets failure feedback |
| Provider timeout | Yes | 2 | Same backend first, then fallback |
| Provider rate limit (429) | Yes | 3 | With retry-after delay |
| Provider server error (5xx) | Yes | 2 | Then fallback to different provider |
| Agent produces no changes | Yes | 1 | May indicate misunderstood task |

## What must hard-fail

| Failure type | Action | Rationale |
|--------------|--------|-----------|
| Provider auth failure | Disable provider, fail task if no fallback | Cannot recover without user action |
| Policy denial | Fail task | User or policy has forbidden the action |
| All retries exhausted | Fail task, inform user | Bounded retries are exhausted |
| Budget exhausted | Pause run | Cannot spend more without user approval |
| Merge conflict | Pause run | Cannot auto-resolve in v1 |
| Worktree corruption | Fail task, clean worktree | Cannot recover corrupted state |
| SQLite corruption | Halt run | Cannot persist state |

## Action-specific policies

### Shell commands
- Sandboxed by default (no network, restricted filesystem — see [Security Model](security-model.md) — inheriting Codex CLI's sandbox model)
- Commands matching `require_approval` patterns pause for human review
- Commands matching `deny` patterns are blocked with error
- Command output is captured and included in task artifacts

### Package installation
- Always requires approval (default policy)
- After approval, runs in worktree (does not affect main checkout)
- Lockfile changes are tracked as artifacts

### Editing lockfiles
- Treated as high-risk file modification
- Requires approval (default policy)
- Diff of lockfile changes included in review

### Editing CI/CD
- Files matching `.github/*`, `.gitlab-ci.yml`, `Jenkinsfile`, etc.
- Always requires approval (default policy)
- Flagged in run summary

### Changing infra code
- Files matching `*.tf`, `*.tfvars`, `docker-compose*`, `k8s/*`, etc.
- Always requires approval (default policy)
- Cannot be auto-retried (approval persists through retries)

### Deleting files
- `rm` commands and file deletion operations require approval
- Deletion of test files or generated files may be auto-approved if policy allows

### Secret access
- Any access to `.env*`, `*.pem`, `*.key`, credentials files is blocked by default
- Must be explicitly allow-listed in policy

### Network calls
- Agent-initiated network calls (curl, wget) require approval
- Provider API calls do not (they're the mechanism of execution)

### Git history rewriting
- `git push --force`, `git reset --hard`, `git rebase` on shared branches: **always denied by default**
- Within worktree branches: allowed without approval (isolated)

### Branch merges
- Merging worktree results into main branch requires verification to pass first
- If merge conflicts: pause and flag for human

### Artifact publication
- Writing results to disk: automatic
- Pushing branches: requires approval
- Creating PRs: requires approval
