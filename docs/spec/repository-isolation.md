# Repository Isolation Model

[< Spec Index](index.md) | [Product Index](../product/index.md)

## Problem

When multiple [agents](agent-taxonomy.md) work on the same repository in parallel, they must not interfere with each other's changes. A coder agent writing middleware and another writing tests need separate working directories.

## Options evaluated

| Approach | Isolation | Disk cost | Speed | Testability | Merge complexity |
|----------|-----------|-----------|-------|-------------|-----------------|
| **Git worktrees** | Full filesystem | ~2x per worktree | Fast (seconds) | Full (each worktree can run tests) | Standard git merge |
| Branches + stash | Shared filesystem | Low | Fast | Requires checkout switching | Standard git merge |
| Patch files | None during generation | Minimal | Fast | Cannot test in isolation | Manual apply + resolve |
| Docker containers | Full | High | Slow (image pull) | Full | File extraction |
| Copy directory | Full | Highest | Slow (large repos) | Full | Diff + apply |

## Recommendation for v1: Git worktrees

Git worktrees provide full filesystem isolation with low overhead. Each agent gets its own worktree with its own working directory, allowing:
- Independent file modifications
- Independent test execution
- Independent git commits
- No interference between parallel agents

**Tradeoff accepted:** Worktrees consume disk space (~size of working directory per worktree). For most repositories, this is acceptable. Very large monorepos (>10GB) may need shallow worktrees or a different strategy — flagged as a future concern.

## Worktree lifecycle

```
Task created
    │
    ▼
Create worktree:
    git worktree add .codex-worktrees/<run-id>/<task-id> -b codex/<run-id>/<task-id>
    │
    ▼
Agent executes in worktree directory
    │
    ▼
Agent commits changes to worktree branch
    │
    ▼
Verification runs in worktree directory
    │
    ├── [pass] ──► Merge into run result branch
    │                │
    │                ├── [clean merge] ──► Mark task complete
    │                │
    │                └── [conflict] ──► Flag for human review
    │
    └── [fail] ──► Retry or fail task
    │
    ▼ (on task completion or failure)
Clean up worktree:
    git worktree remove .codex-worktrees/<run-id>/<task-id>
```

## Directory structure

```
/home/user/project/                          # Main repository
├── .codex-worktrees/                        # Worktree root (gitignored)
│   └── r_abc123/                            # Per-run directory
│       ├── task_1/                           # Worktree for task 1
│       │   ├── src/
│       │   ├── tests/
│       │   └── ...                          # Full working tree
│       ├── task_3/                           # Worktree for task 3
│       └── task_5/                           # Worktree for task 5
```

The `.codex-worktrees` directory should be added to `.gitignore` automatically.

## Branch strategy

```
main (or current branch)
  │
  ├── codex/r_abc123/result     ← Run result branch (merge target)
  │     │
  │     ├── codex/r_abc123/task_1  ← Task 1 worktree branch
  │     ├── codex/r_abc123/task_2  ← Task 2 worktree branch
  │     └── codex/r_abc123/task_3  ← Task 3 worktree branch
```

1. On run start: create `codex/<run-id>/result` from current branch
2. Each task worktree branches from `codex/<run-id>/result`
3. On task completion: merge task branch into result branch
4. On run completion: result branch is ready for human review

## Merge/rebase flow

After a task completes and verification passes:

```python
def merge_task_result(task, run):
    # 1. Checkout result branch
    # 2. Attempt merge from task branch
    result = git_merge(f"codex/{run.id}/{task.id}", into=f"codex/{run.id}/result")
    
    if result.clean:
        # Clean merge — proceed
        return MergeResult.SUCCESS
    else:
        # Conflict detected
        git_merge_abort()
        return MergeResult.CONFLICT
```

## Conflict handling

In v1, conflicts are **not auto-resolved**:

1. Detect conflict during merge attempt
2. Abort the merge
3. Flag the conflict in the run status
4. Pause the run
5. Present the conflict to the user:
   ```
   CONFLICT: task_2 and task_4 both modified src/gateway/app.py
   
   Options:
   [m] Manually resolve and continue
   [r] Re-run task_4 on top of task_2's changes
   [s] Skip task_4
   [c] Cancel run
   ```
6. User resolves and resumes

**Why not auto-resolve?** Auto-resolution (even for trivial conflicts) risks silent corruption. The cost of pausing for human review is low. The cost of a bad merge is high.

## Cleanup

### Normal cleanup (task complete or failed)
```bash
git worktree remove .codex-worktrees/<run-id>/<task-id>
git branch -d codex/<run-id>/<task-id>
```

### Run cleanup (run complete or cancelled)
```bash
# Remove all worktrees for the run
git worktree remove .codex-worktrees/<run-id>/*
rm -rf .codex-worktrees/<run-id>/

# Keep result branch for human review
# Delete task branches
git branch -D codex/<run-id>/task_*
```

### Orphan cleanup (on restart after crash)
```python
def cleanup_orphaned_worktrees():
    # List all worktrees
    worktrees = git_worktree_list()
    
    for wt in worktrees:
        if wt.path.startswith(".codex-worktrees/"):
            run_id = extract_run_id(wt.path)
            task_id = extract_task_id(wt.path)
            
            # Check if the task is still active
            task = db.get_task(task_id)
            if task is None or task.status in ("completed", "failed", "cancelled"):
                # Orphaned — clean up
                git_worktree_remove(wt.path)
```

## Failure recovery

| Failure | Recovery |
|---------|----------|
| Worktree creation fails (disk space) | Fail task with clear error |
| Agent crashes mid-execution | Worktree preserved; on retry, clean worktree and recreate |
| Merge fails (conflict) | Pause run, flag for human |
| Process killed during merge | On restart, detect partial merge, abort and retry |
| Orphaned worktree after crash | Cleaned up on next startup |

## Concurrency limits

- Default: 4 parallel worktrees (configurable via `max_parallel_agents` — see [CLI Interaction Spec](../product/cli-interaction-spec.md))
- Hard limit: 10 worktrees (to prevent disk exhaustion)
- Orchestrator queues tasks when all worktree slots are occupied
