# Design Principles

[< Spec Index](index.md) | [Product Index](../product/index.md)

> These principles apply to every area of code in the project. They are not suggestions.

## The core mantra: deterministic control, intelligent judgment

Use **deterministic code** for control flow: loops, state transitions, retries, timeouts, dispatch, persistence. The program decides when to continue, when to stop, when to retry.

Use **LLM intelligence** for judgment calls: is the task done? did verification succeed? what subtasks does this goal need? should we escalate or retry? what model fits this task?

**If you're writing a regex to avoid an LLM call, you're doing it wrong.** Hardcoded pattern matching for things like "did the tests pass" or "is this output complete" is brittle — it breaks the moment the format changes. Use the LLM to interpret output. It's what LLMs are for.

**If you're asking an LLM to decide whether to keep looping, you're also doing it wrong.** LLMs are trained to be cautious, check in, ask permission. They will stop mid-work to ask "should I continue?" — which is exactly the behavior we're eliminating. The loop continues because the task graph says there's pending work, not because the model feels like continuing.

## Where this applies

| Aspect | Deterministic code | LLM judgment |
|--------|-------------------|--------------|
| **Supervisor loop** | `while tasks_pending(): dispatch_next()` — the loop is code | "Decompose this goal into subtasks" — planning is LLM |
| **Completion check** | "Are all tasks in terminal state?" — boolean check on state | "Did this agent's output actually accomplish the task?" — LLM evaluates |
| **Verification** | Run `pytest`, capture exit code — deterministic | "These 3 tests failed — is this a real problem or a flaky test?" — LLM interprets |
| **Retry decision** | retry_count < max_retries — deterministic | "The agent failed because X — should we retry with the same approach or try differently?" — LLM decides strategy |
| **Routing** | Filter by context window, check budget — deterministic rules | "Is this task better suited to a code-generation model or a reasoning model?" — LLM classifies |
| **Task scheduling** | Dependency graph: ready = deps all complete — deterministic | "These two tasks look independent but actually share a file — should they be sequential?" — LLM catches what the graph doesn't |
| **Error classification** | HTTP 429 → retry with backoff — deterministic | "This error says 'module not found' — is it a missing import or a broken dependency?" — LLM understands context |
| **Approval gating** | File matches `*.lock` pattern → require approval — deterministic | (Not applicable — policy rules are sufficient here) |

## Anti-patterns to reject

### Anti-pattern: Regex-based output parsing
```python
# WRONG — brittle, breaks when format changes
if re.search(r"(\d+) passed", test_output):
    passed = int(match.group(1))
    return passed > 0
```

```python
# RIGHT — let the LLM interpret
result = await llm.evaluate(
    f"Did these tests pass? Respond with 'yes' or 'no' and explain.\n\n{test_output}"
)
```

### Anti-pattern: LLM-controlled loop
```python
# WRONG — the model decides when to stop, and it WILL stop too early
while True:
    result = await agent.run_turn(task)
    if "I'm done" in result.text or "shall I continue" in result.text:
        break  # Model decided to stop
```

```python
# RIGHT — deterministic loop, LLM evaluates
while task.status == "pending":
    result = await agent.run_turn(task)
    judgment = await llm.evaluate(
        f"The task was: {task.description}\n"
        f"The agent produced: {result.summary}\n"
        f"Is the task complete? Respond 'complete' or 'incomplete' with reason."
    )
    if judgment.decision == "complete":
        task.status = "completed"
    elif task.retry_count >= task.max_retries:
        task.status = "failed"
    else:
        task.retry_count += 1
        # Loop continues — no LLM gets to opt out
```

### Anti-pattern: Hardcoded heuristics for routing
```python
# WRONG — fragile, incomplete, will never cover all cases
if "test" in task.description.lower():
    return "ollama-local"
elif len(task.description) > 500:
    return "frontier-model"
elif any(ext in task.description for ext in [".py", ".ts", ".rs"]):
    return "code-model"
```

```python
# RIGHT — deterministic filtering, then LLM for final selection
eligible = [m for m in models if m.context_window >= estimated_tokens]
eligible = [m for m in eligible if budget.can_afford(m)]
if len(eligible) == 1:
    return eligible[0]  # Deterministic — only one option
decision = await llm.evaluate(
    f"Task: {task.description}\n"
    f"Available models: {eligible}\n"
    f"Which model is best for this task? Return the model ID."
)
return decision.model_id
```

## How existing coding-agent-router code fits this principle

The coding-agent-router's routing algorithm already follows this pattern correctly:

1. **Deterministic:** Filter by context window (remove models that can't fit the request)
2. **Deterministic:** If only one model eligible, return it
3. **Deterministic:** If router payload too large for router model, use fallback order
4. **LLM judgment:** If multiple eligible, ask the router model to choose (with full metrics context)
5. **Deterministic:** If LLM returns invalid answer, use fallback order

The task metrics extraction (27 metrics) is also correct — it uses regex to extract *features* (how many files, how many errors, etc.) that are then fed to the LLM for the actual *decision*. The regex isn't making the judgment — it's building the evidence the LLM needs. That's the right use of pattern matching.

## Summary

```
Deterministic code:    loop | dispatch | retry | timeout | state transition | persist
LLM intelligence:      plan | evaluate | interpret | classify | decide strategy
Never:                 LLM controls loop | regex makes judgment | hardcode what LLM should decide
```
