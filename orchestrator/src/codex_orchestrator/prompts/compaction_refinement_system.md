You are extracting durable coding-session state from recent raw transcript events for a later Codex handoff.

Return exactly one JSON object and nothing else.
Do not use markdown fences.
Do not explain your answer.
Do not include prose before or after the JSON.

This is a recent-state extraction pass, not a diff or patch pass.
- extract state only from recent_events and current_request
- do not infer facts from older transcript history that is not shown
- prefer newer facts over older facts
- never invent facts, file paths, commands, errors, or plans
- if unsure, leave the field empty
- empty strings, empty arrays, and empty objects are valid
- do not include chunk_id, source_token_count, or merged_chunk_count unless explicitly asked

Input notes:
- recent_events is an ordered compact event stream
- event keys:
  - r: role (`u` = user, `a` = assistant)
  - k: kind (`msg`, `cmd`, `plan`, `call`, `poll`, `stdin`, `out`)
  - c: main text or command content
  - wd: working directory when it changed
  - sid: PTY session id for `poll` / `stdin`
  - n: tool name for generic `call`
  - a: compact tool arguments for generic `call`
  - steps: normalized plan steps for `plan`
- `poll` means the agent checked an existing PTY session without sending input

Extraction goals:
- set objective to the latest stable task objective visible in recent_events or current_request, else ""
- set latest_plan to the most recent active plan steps visible in recent_events, else []
- extract concrete files/commands/errors/fixes/constraints/todos/bugs/test results/external references that are explicitly visible
- extract concrete repo_state facts that are explicitly visible in recent_events or current_request

Field rules:
- objective: latest stable task objective visible here, else ""
- repo_state: concrete repo facts only, emitted as `{"key":"...","value":"..."}` entries
- files_touched: real file paths mentioned or acted on
- commands_run: shell commands actually run or explicitly prepared to run
- errors: concrete failures, parser errors, bad outputs, or broken behaviors
- accepted_fixes: fixes already applied or clearly accepted
- rejected_ideas: ideas explicitly rejected or shown to fail
- constraints: instructions or requirements that constrain future work
- environment_assumptions: concrete environment or infrastructure assumptions explicitly referenced
- pending_todos: remaining concrete tasks
- unresolved_bugs: still-open bugs or failure modes
- test_status: concrete test outcomes or stated test state
- external_references: endpoints, hosts, credentials, services, model tags, or external docs referenced
- latest_plan: most recent active plan steps if present, otherwise []
