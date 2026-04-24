# Local Coder Massaging

[< Spec Index](index.md)

## Purpose

This document catalogs every intervention the routing and tool layers apply specifically to help **small local coder models** succeed where they would fail on their own. "Local coder" here means the model wired into the `light_coder` role — currently models in the 4B–30B range at Q4 / Q6 quants running via Ollama (e.g. `devstral-small-2:q4_k_m`, `gemma4:e4b`).

These models are far less capable than frontier cloud models at:
- Tool-use discipline (picking the right tool, getting the arg shape right)
- Patch-format precision (producing valid unified diffs or Codex-native patches)
- Self-correction after an error (they tend to retry the exact same broken call)
- Staying on-task vs. announcing intent and stopping
- Grounding API calls in real documentation rather than guessing URL paths

Everything below exists because we observed one of those failure modes in practice and chose to fix it in the orchestration layer rather than wait for better local models.

---

## At a glance

Plain-English summary of every intervention. Numbers match the detailed sections below.

1. **Skip extras in local-only mode** — Normally a router, reasoner, and compactor help the main model. Locally we can't afford extras, so the Coder handles everything.
2. **Trim the tool menu** — Codex has ~120 tools. Small models get confused by big menus; we show them ~10.
3. **Plain-English tool cheat sheet** — We paste simple "here's how each tool works, with an example" text into the system prompt.
4. **Rewrite shell-ish tool names** — Models call `ls`, `cat`, `grep` like tools. We catch that and convert to a proper `shell` call.
5. **Browser user-agent for curl** — Sites block `curl/8.0`. We auto-add a real browser UA to any curl command.
6. **Web search + web fetch** — No built-in web search in local mode, so we added two tools for looking up real API docs.
7. **Fix broken patches** — Small models mangle patches: (a) write git-diff format, (b) leave `@@ -1,6 +1,6 @@` line numbers, (c) forget `+` prefixes and closing markers. We fix all three automatically.
8. **Better patch error messages** — Default errors are cryptic. We rewrote the common ones to explain what to try next.
9. **Better network errors** — Instead of "error sending request", we show the real cause (DNS, TLS, connection refused, etc.).
10. **Catch announce-without-act** — When the model says "Now I'll do X" and stops, a judge model spots it and we re-prompt "take the action." Up to 3 retries.
11. **Stop repetition** — (a) Same tool + same args 3× → STOP block in next prompt. (b) Same file failing 3× with different commands → same STOP block.
12. **Trim old conversation** — Keep the most recent turn intact, summarize older turns, drop stale file reads, pin errors so the model can't forget them.
13. **Log the model's thinking** — Reasoning text goes to debug logs so we can explain weird behavior later.
14. **Reroute wrong picks** — If the router picked "text-only model" but the conversation has tool calls, we upgrade to the tool-capable Coder.
15. **Diagnostic logs** — Extra logging for "which tools did we pass?", "did the STOP fire?", "what did the bail judge decide?"
16. **LM Studio / OpenAI-compat support** — Ollama and OpenAI-style servers disagree on URLs, payload shape, tool-call encoding, etc. A "flavor" switch lets one codebase talk to both.
17. **Token + time budget knobs** — New per-role `max_tokens` and `timeout_seconds` in `config.toml`. Set either to `0` for unlimited (reasoning models need this).
18. **Pin current file contents** — Models forget a file changed and generate patches based on the old version. We pin live on-disk contents at the top of the prompt.
19. **Catch thinking loops** — Some models spiral: "Actually, wait. Hmm. Let me reconsider." We watch the stream for 6+ self-doubt phrases after half the token budget is burned, abort mid-generation, and re-prompt "stop second-guessing."
20. **Stream the coder's output** — To make #19 work, we switched from "send, wait, parse" to "open stream, watch tokens, abort if needed." Also lets us log reasoning in real time.

---

## 1. Local-only routing

**Problem:** In `local_only` mode there is no cloud fallback. A classifier LLM call is wasted work, a separate reasoner endpoint splits inference load across an extra model, and a dedicated compactor is another model to load/keep warm.

**What we do:**
- Skip the classifier entirely. `route_request` synthesizes a `ClassifyResult` pointing at `LightCoder` without calling the classifier endpoint.
- Never resolve `light_reasoner` — reasoning-shaped requests fall through to the Coder.
- Route compaction through the same `light_coder` endpoint. The dedicated `compactor` role becomes unused in local-only mode.

**Code:** [codex-rs/core/src/local_routing.rs](../../codex-rs/core/src/local_routing.rs) — search for `local_only`. The compaction branch in `route_request` picks `&state.config.light_coder` when `local_only` is set.

**Log signal:** `local_only: bypassing classifier — routing to LightCoder`

---

## 2. Tool catalog curation

**Problem:** Codex exposes ~120 tools (MCP connectors, multi-agent orchestration, dynamic tools, etc.). Small models lose attention when handed that many schemas, or hallucinate tool names that look plausible. Only ~10 of those tools matter for day-to-day coding work.

**What we do:** Filter the tool list down to a curated 10 before sending to the local Coder.

```rust
const LIGHT_CODER_TOOL_NAMES: &[&str] = &[
    "shell", "apply_patch", "list_dir", "view_image", "update_plan",
    "local_web_search", "web_fetch", "request_permissions",
    "exec_command", "write_stdin",
];
```

The subset is controllable per endpoint via `tool_subset: Focused` (default) vs `Full`. `Full` sends the entire catalog for capable local models that can handle it.

**Code:** [codex-rs/core/src/local_routing.rs](../../codex-rs/core/src/local_routing.rs) — `LIGHT_CODER_TOOL_NAMES` and the filter in `try_local_model`.

**Log signal:** `Passing tool set to local coder tool_count=N available_in_prompt=M tools_passed=[...] tools_dropped=[...]`

---

## 3. Per-tool system-prompt hint

**Problem:** The formal tool schema (JSON Schema) that gets sent in the Ollama request is exhaustive but small models don't read it carefully. They call `ls` as a tool name, pass `command: "ls -la"` as a string instead of an array, forget prefixes on `apply_patch` bodies, etc.

**What we do:** Append a plain-English hint block to the system prompt that lists each available tool with its exact arg shape and a concrete example. The hint is rendered by `build_tool_hint` using the same tool names that were actually passed, so a tool that got filtered out never appears in the hint.

Sample hint entries include directives like:
- "If you find yourself wanting to call `ls`, `rg`, `cat`, `git`, or `pytest` directly, that is wrong — wrap it as `shell` with `command: [\"bash\", \"-lc\", \"<the command>\"]`."
- apply_patch: two accepted formats (unified diff + Codex native) with a prefix rule spelled out.
- local_web_search: suggests pairing with `web_fetch` to read a specific result.
- web_fetch: "use this BEFORE writing code against an unfamiliar API or library."

**Code:** [codex-rs/core/src/local_routing.rs](../../codex-rs/core/src/local_routing.rs) — `build_tool_hint` function.

---

## 4. Shell-command alias translation

**Problem:** Models trained on shell sessions emit tool calls like `{"name": "ls", "arguments": {...}}`, `{"name": "cat", "arguments": {"path": "foo.py"}}`, or `{"name": "grep", "arguments": {"pattern": "x"}}` — none of which are real Codex tools. Rejecting these outright wastes a turn.

**What we do:** Detect common shell-command aliases in the tool name and rewrite them into `shell({command: ["bash", "-lc", "..."]})`. Also normalize `shell` calls where `command` was passed as a string instead of an array.

**Code:** [codex-rs/routing/src/tool_aliases.rs](../../codex-rs/routing/src/tool_aliases.rs) — `translate_native_tool_calls`, `translate_to_shell_call`.

**Log signal:** `Translated tool call (native) from=ls to=shell command_line=...`

---

## 5. `curl` User-Agent injection

**Problem:** Many sites behave differently when receiving a `curl/X.Y` User-Agent vs a browser UA — they might serve simplified HTML, return CAPTCHAs, or block the request outright. Training corpora rarely show models setting `-A`, so they just call `curl <url>` and get useless responses.

**What we do:** When the shell handler dispatches a command whose argv starts with `curl` (or a full path like `/usr/bin/curl`), inject `-A "<default browser UA>"` after the `curl` token — unless a UA is already set via `-A`, `--user-agent[=]`, `-H User-Agent:`, or `--header User-Agent:`. The same treatment applies to free-form shell strings via regex: `curl https://example.com | jq` becomes `curl --user-agent '...' https://example.com | jq`.

The default UA is a current Brave/Chrome Linux string (Brave intentionally identifies as Chrome). The same constant is the default for `local_web_search` and `web_fetch`.

**Code:** [codex-rs/routing/src/curl_ua.rs](../../codex-rs/routing/src/curl_ua.rs) + shell handler integration in [codex-rs/core/src/tools/handlers/shell.rs](../../codex-rs/core/src/tools/handlers/shell.rs).

---

## 6. New tools: `local_web_search` and `web_fetch`

**Problem:** In `local_only` mode, OpenAI's built-in `web_search` tool is unavailable. Local coders can't look things up, so they guess URL paths and API shapes from priors — frequently wrong.

**What we do:**
- **`local_web_search`** — Brave Search API with a configured key. Returns ranked titles, URLs, and snippets. Single HTTP GET, no retries.
- **`web_fetch`** — Single HTTP GET against an arbitrary URL, with the Brave/Chrome browser UA. Returns the response body as text for text-like content types, a placeholder for binary. Body capped at 512KB, 30s timeout, only http/https schemes.

Both tools are in `LIGHT_CODER_TOOL_NAMES` and are advertised in the tool hint. The hint explicitly tells the model to use `web_fetch` **before** writing code against an unfamiliar API rather than guessing.

**Code:**
- [codex-rs/routing/src/local_web_search.rs](../../codex-rs/routing/src/local_web_search.rs)
- [codex-rs/routing/src/web_fetch.rs](../../codex-rs/routing/src/web_fetch.rs)
- [codex-rs/tools/src/local_web_search_tool.rs](../../codex-rs/tools/src/local_web_search_tool.rs)
- [codex-rs/tools/src/web_fetch_tool.rs](../../codex-rs/tools/src/web_fetch_tool.rs)
- Handlers under [codex-rs/core/src/tools/handlers/](../../codex-rs/core/src/tools/handlers/).

---

## 7. `apply_patch` input normalization

Local models mangle `apply_patch` in three distinct ways. The normalizer pipeline (`normalize_apply_patch_call`) runs two passes in order:

### 7a. Unified-diff translation

**Problem:** Models emit `git diff` format (`--- a/path` / `+++ b/path` / `@@ -L,N +L,N @@`) because that's what their training corpus is full of. Codex's native format (`*** Begin Patch` / `*** Update File:` / context-anchored hunks) is rare in training data.

**What we do:** Detect unified-diff input and translate to Codex format. Handles:
- `--- a/path` + `+++ b/path` → `*** Update File: path`
- `--- /dev/null` + `+++ b/new.py` → `*** Add File: new.py`
- `--- a/old.py` + `+++ /dev/null` → `*** Delete File: old.py`
- `@@ -L,N +L,N @@ <anchor>` → `@@ <anchor>` (line numbers stripped — Codex matches by context)
- Git noise (`diff --git`, `index abc..def`, `rename from`, mode lines) is skipped
- `\ No newline at end of file` markers are dropped
- `a/`/`b/` path prefixes stripped; tab-delimited `diff -u` timestamps stripped; bare paths accepted

### 7b. Hybrid hunk-header normalization

**Problem:** A different failure mode: the model uses the Codex envelope (`*** Begin Patch` / `*** Update File:`) but puts unified-diff-style hunk headers *inside* it — `@@ -1,6 +1,6 @@`. The unified-diff translator returns `None` because there's no `---`/`+++` file header, and Codex's parser treats everything after `@@ ` as a literal anchor line.

**What we do:** Inside `fix_apply_patch_body`, when we see an `@@` line, strip any ` -L[,N] +L[,N] @@` segment and preserve an anchor text if present.
- `@@ -1,6 +1,6 @@` → `@@`
- `@@ -17,7 +17,7 @@ def foo():` → `@@ def foo():`
- `@@ def bar():` → unchanged
- `@@` → unchanged

### 7c. Prefix repair + end-of-patch terminator

**Problem:** Models emit patch bodies where lines lack any `+` / `-` / ` ` prefix — they just paste code, expecting the tool to figure it out. Also commonly forget the closing `*** End Patch` marker.

**What we do:** Inside a hunk, if a line doesn't start with `+`, `-`, a single space, or empty, prepend `+` (treat as addition). If the body has `*** Begin Patch` but no matching `*** End Patch`, auto-append.

**Code:** [codex-rs/routing/src/tool_aliases.rs](../../codex-rs/routing/src/tool_aliases.rs) — `translate_unified_diff_to_codex`, `normalize_codex_hunk_header`, `fix_apply_patch_body`, and `normalize_apply_patch_call` which wires them together.

**Log signal:** `Translated tool call (native) from=apply_patch to=apply_patch command_line=apply_patch (unified-diff translation + fixed prefixes, N bytes)`

---

## 8. `apply_patch` error-message improvements

Even with normalization, some patches genuinely can't apply — the context lines don't match, the model intended something the tool can't guess at, etc. Default errors like `Failed to find context '-17,7 +17,7 @@'` don't tell the model how to recover.

### 8a. Unified-diff hunk header detection

**Problem:** If a unified-diff-style hunk header slips past normalization (rare edge case), the error that Codex apply_patch produces is opaque.

**What we do:** When `Failed to find context` fires and the context looks like `-N,N +N,N`, swap in a directive error explaining that Codex doesn't use line numbers and instructing the model to omit the header or use a real anchor line.

**Code:** [codex-rs/apply-patch/src/lib.rs](../../codex-rs/apply-patch/src/lib.rs) — `looks_like_unified_diff_hunk_header`.

### 8b. Empty-args interception

**Problem:** After bailing on a hard turn, the model sometimes calls `apply_patch({})` — a syntactically valid but empty tool call. Codex's default error is the terse `missing field input at line 1 column 2`, which doesn't help the model recover.

**What we do:** In the apply_patch handler, if `arguments` is empty or `{}`, return a directive error that shows the expected shape with a concrete example and offers an escape hatch ("or use a different tool if you don't actually need to modify a file").

**Code:** [codex-rs/core/src/tools/handlers/apply_patch.rs](../../codex-rs/core/src/tools/handlers/apply_patch.rs) — the `ToolPayload::Function` branch.

---

## 9. `web_fetch` error enrichment

**Problem:** `reqwest::Error::to_string()` typically produces `error sending request for url (...)` — no visible root cause. DNS failure, TLS cert mismatch, and connection refused all look identical to the model.

**What we do:** Walk the error's `source()` chain up to 5 levels, deduplicate messages, join with ` → `, and prepend a category tag: `[connect]`, `[timeout]`, `[redirect]`, `[body]`, or `[decode]`. A TLS hostname mismatch now surfaces the actual "no alternative certificate subject name matches target host name 'X'" message instead of being buried.

**Code:** [codex-rs/routing/src/web_fetch.rs](../../codex-rs/routing/src/web_fetch.rs) — `describe_reqwest_error`.

---

## 10. Completion verifier (bail detector)

**Problem:** Local models often end a turn with text that announces intent but takes no action — "I will update the imports and then run the tests" with no tool call to actually do it. Codex interprets any text-only response as the end of a turn, emits `task_complete`, and the user is left with a broken task.

**What we do:**
- After each Ollama call, if the response has non-empty text and zero tool calls, send it to a small judge model (the Coder itself in local-only mode) with a prompt defining BAIL vs COMPLETE patterns.
- If the verdict is BAIL, inject a `continuation_prompt` as a synthesized user message telling the model: "You announced an action but did not actually take it. Either take the action, restate the concrete result you produced, or explain why you cannot proceed." Then re-call the model.
- `MAX_BAIL_RETRIES = 3` — the model gets up to 3 nudges to convert announcement into action before Codex gives up.

The verifier prompt explicitly covers:
- "I will X" / "Let me X" / "Now I'll X" and stops
- Plans/intents stated without any tool call
- Findings restated without being applied
- **Code blocks are never actions** — a markdown fence containing source code is a suggestion, not a completed action, unless the same content was passed to `apply_patch` or `shell`.

The verifier uses `light_coder` as its endpoint in `local_only` mode (the classifier is offline by design). In cloud mode it uses the fast classifier.

**Code:** [codex-rs/routing/src/completion_verifier.rs](../../codex-rs/routing/src/completion_verifier.rs) — `verify_completion`, `continuation_prompt`.

**Log signal:** `Completion verifier judged the model's text-only response verdict=Bail|Complete|Unclear`

---

## 11. Repetition alert

**Problem:** Local models frequently get stuck calling the same tool with the same arguments 5-10 times in a row, failing to learn from the identical outputs (or failures). Broader variant: repeatedly poking at the same **file** with subtly different commands (`cat foo.py`, then `wc -l foo.py`, then `head foo.py`) after every call fails — same failure mode but no exact-signature match.

**What we do:** Two detectors share the same STOP-block rendering:

### 11a. Exact-signature repetition

Walk the most recent `ToolCall` items and detect when 3+ consecutive calls share the same `(tool_name, signature)` where signature is a hash of the normalized args.

### 11b. Same-target-failure repetition

Walk the most recent `(ToolCall, ToolOutput)` pairs and detect when 3+ consecutive **failed** calls target the same file path, even with different argument shapes. Catches the "keep trying different ways to read a file that's broken" loop that 11a misses.

When either fires, a `[STOP — REPETITION DETECTED]` block is prepended to the system prelude with:
- The tool name and repeat count
- A summary of the repeated call
- An excerpt of the last output (so the model can see what the actual result was)
- A directive: "STOP making this call. Try a different approach now: change the arguments, use a different tool, or report what you've learned to the user."

**Code:**
- Detection: [codex-rs/routing/src/trim/state_extract.rs](../../codex-rs/routing/src/trim/state_extract.rs) — `detect_repetition`, `detect_same_target_failure_repetition`
- Rendering: [codex-rs/routing/src/trim/render.rs](../../codex-rs/routing/src/trim/render.rs) — `render_repetition_alert`
- Signatures: [codex-rs/routing/src/trim/signatures.rs](../../codex-rs/routing/src/trim/signatures.rs)

**Log signal:** `Repetition alert fired — STOP block will be added to next prelude tool_name=X count=N`

---

## 12. Transcript trimming

**Problem:** Local models have small context windows (4K-32K tokens typical) and lose attention on long transcripts. Sending the raw Codex history would blow the budget and swamp the signal.

**What we do:** `trim_for_local` applies deterministic role-aware trimming:
- The **active turn** (everything from the most recent user message forward) is preserved verbatim.
- **Older turns** are replaced with a synthesized state prelude summarizing files seen, files modified, tests run, errors encountered.
- **Stale reads** (file reads followed by later writes that superseded them) are dropped.
- **Superseded outputs** (older tool outputs for files that have been re-read since) are dropped.
- **Errors are sticky** — any tool output containing an error is preserved regardless of age, so the model can't forget a failure and repeat it.
- The system prompt is never stubbed.

The same trimmer is also used as the first pass of compaction.

**Code:** [codex-rs/routing/src/trim/](../../codex-rs/routing/src/trim/) — entry point `trim_for_local` in `mod.rs`.

**Log signal:** `Trimmed transcript for local model trim_summary=kept N/M items; collapsed K older turns; dropped X stale reads, Y superseded outputs; elided Z chars; ~T input tokens`

---

## 13. Thinking / reasoning channel capture

**Problem:** Reasoning-heavy local models emit their chain-of-thought on a separate channel — `message.thinking` (Ollama) or `choices[0].delta.reasoning_content` / `message.reasoning_content` (OpenAI-compat). We don't feed it back to the model — it's private scratchpad, not user-facing — but losing it entirely makes debugging hard. When a local model makes a weird decision, the "why" often lives in the reasoning channel.

**What we do:** Accumulate reasoning deltas during streaming and, at turn end, log the full reasoning text at `debug!` level. Not part of the model's next-turn input; purely a diagnostic breadcrumb.

**Code:** [codex-rs/core/src/local_routing.rs](../../codex-rs/core/src/local_routing.rs) — `StreamChunk::ReasoningDelta` branch of the coder's stream consumer.

**Log signal:** `Local coder reasoning channel reasoning_len=N reasoning_tokens=T reasoning=<content>` (debug level)

---

## 14. Conversation-state route override

**Problem:** A classifier or heuristic picks `LightReasoner` (a text-only route) but the transcript already has recent tool calls. Local reasoner models choke when handed an assistant message containing `tool_calls` without a corresponding tools array — they typically respond with empty output.

**What we do:** After classification but before dispatch, check if there are recent tool calls in the conversation. If so and the route is `LightReasoner`, upgrade to `LightCoder`. Deterministic override, layered on top of the classifier's output.

**Code:** [codex-rs/core/src/local_routing.rs](../../codex-rs/core/src/local_routing.rs) — `conversation_has_recent_tool_calls` + the branch that upgrades the route.

**Log signal:** `Override: classifier picked LightReasoner but history has tool calls — upgrading to LightCoder`

(Moot in local-only mode since everything goes to `LightCoder` anyway, but preserved for cloud and mixed modes.)

---

## 15. Diagnostic logging

Beyond the per-feature log signals above, several diagnostics were added specifically because local-model problems are hard to reproduce outside the original session:

- **`tools_passed` / `tools_dropped` per turn** — reveals when a tool in `LIGHT_CODER_TOOL_NAMES` is missing from the session's `prompt.tools` (config drift or a feature flag).
- **`apply_patch (fixed prefixes, N bytes)` command line** — shows which normalization passes fired.
- **`Repetition alert fired`** — confirms whether the guard is actually triggering (separate from whether the model listened).
- **`Completion verifier judged`** — the final verdict and endpoint used, so we can tell "Bail was detected and we retried" from "Complete was returned too leniently".

All of these write to the standard tracing log (`~/.codex/log/codex-tui.log`).

---

## 16. OpenAI-compat wire adapter

**Problem:** Ollama and OpenAI-compat servers (LM Studio, llama.cpp's `server`, vLLM, LiteLLM, etc.) disagree on almost every surface: URL paths, payload shapes, tool-call JSON conventions, response-format hints, tool-result message roles. Writing Ollama-only code would lock out every OpenAI-compat server, which is most of the practical local-inference ecosystem.

**What we do:** A `ClientFlavor` enum on `OllamaEndpoint` (`Ollama` default, `OpenAICompat` selected via `provider = "openai-compat"` / `"lmstudio"` / `"openai"` in `config.toml`). Every wire operation branches on the flavor:

- **URL**: `/api/chat` (Ollama) vs `/v1/chat/completions` (OpenAI). Defensively strips trailing `/v1` so `http://host:1234` and `http://host:1234/v1` both resolve to the same endpoint.
- **Payload shape**:
    - Ollama — `options: { num_ctx, num_predict }`, `think: bool`, `format: "json"`.
    - OpenAI — top-level `max_tokens`, no `num_ctx` (server-set), no `think`.
- **Tool-call argument encoding**: Ollama accepts `arguments` as a JSON object; OpenAI requires `arguments` as a JSON-encoded STRING. Renderer branches so trimmed history matches what each server expects.
- **Tool-result messages**: Ollama expects `role: user` with the result wrapped in `<tool_result>` / `<tool_error>` tags; OpenAI expects `role: tool` with a `tool_call_id` field that matches the `id` on the assistant's `tool_calls[]` entry. The trimmer branches here too.
- **Streaming transport**: Ollama streams NDJSON (one JSON object per line); OpenAI streams SSE (`data: {...}` lines terminated by `data: [DONE]`). Two readers, one shared output enum (`StreamChunk`).
- **Usage decoding**: Ollama's `prompt_eval_count` / `eval_count` vs OpenAI's `usage.prompt_tokens` / `usage.completion_tokens` / `usage.completion_tokens_details.reasoning_tokens`.
- **Startup probe**: `/api/version` (Ollama) vs `/v1/models` (OpenAI).
- **response_format**: dropped for OpenAI-compat. The legacy `{"type": "json_object"}` shape that older OpenAI APIs accept is rejected by LM Studio (it demands `"text"` or `"json_schema"`, the latter requiring an actual schema we don't carry). Caller's system prompt enforces JSON instead — the same pattern the coder's tool-call flow already relies on.
- **Error surfaces**: non-2xx status bodies and HTTP-200 `{"error": ...}` bodies (some servers return 200 with an error field) are both decoded and logged so the caller gets a root cause instead of a silent `None`.

**Code:**
- Flavor enum, endpoint plumbing: [codex-rs/routing/src/config.rs](../../codex-rs/routing/src/config.rs)
- Wire branching: [codex-rs/routing/src/ollama.rs](../../codex-rs/routing/src/ollama.rs) — `build_chat_url`, `build_chat_payload`, `build_stream_payload`, `spawn_ollama_stream_reader`, `spawn_openai_sse_reader`, `translate_response_to_ollama_shape`
- Trimmer branching: [codex-rs/routing/src/trim/render.rs](../../codex-rs/routing/src/trim/render.rs) — flavor-aware tool-call and tool-result rendering

**Log signals:** `chat request returned non-success status url=... status=... body=...` / `chat response carried an error body — treating as failure`

---

## 17. Per-role `max_tokens` and `timeout_seconds`

**Problem:** Reasoning-capable local models (Qwopus 3.5, DeepSeek-R1, etc.) can legitimately take 5–30 minutes of wall clock for a single answer when they think heavily. The original 5-minute client timeout killed mid-flight inference; there was no way to set a per-role budget on either wall-clock time or output tokens.

**What we do:** Two new optional fields on the `[models.<role>]` config block:

- `max_tokens = N` — ceiling on generated tokens per response. `0` means unlimited (no cap). Maps to OpenAI `max_tokens` / Ollama `options.num_predict`. Normalized from `Some(0)` → `None` at config load.
- `timeout_seconds = N` — per-request wall-clock timeout. `0` means unlimited (no timeout). Applied to reqwest's `.timeout()` only when `> 0`, so unlimited = the `.timeout()` call is skipped entirely.

Both semantics mirror each other: `0` = the knob is off.

**Code:**
- Config shape: [codex-rs/routing/src/project_config.rs](../../codex-rs/routing/src/project_config.rs) + [codex-rs/routing/src/config.rs](../../codex-rs/routing/src/config.rs) — `endpoint_from_role`
- Wire plumbing: [codex-rs/routing/src/ollama.rs](../../codex-rs/routing/src/ollama.rs) — `build_chat_payload`, `build_stream_payload`, the `.timeout(...)` guards in `chat_with_tools` and `chat_stream`

---

## 18. Current-file-state prelude pin

**Problem:** A local model reads `foo.py`, edits it, reads it again, and then — several turns later — generates an `apply_patch` whose context lines are from the *original* read. The patch fails because the context doesn't match current disk state, and the model can't reliably reason about "which version of the file is authoritative" from scrolling back through the transcript.

**What we do:** The trimmer identifies files that were modified during the active turn (from tool outputs) and injects a dedicated `[Current file state — authoritative...]` block near the top of the prelude with the live on-disk contents. Capped at 10 KB per file; header includes a content hash, line count, and byte count so the model can cross-reference with its own mental model.

**Code:**
- Extraction: [codex-rs/routing/src/trim/state_extract.rs](../../codex-rs/routing/src/trim/state_extract.rs) — `files_modified_in_active_turn`
- Loading: [codex-rs/core/src/local_routing.rs](../../codex-rs/core/src/local_routing.rs) — `load_active_turn_files`
- Rendering: [codex-rs/routing/src/trim/render.rs](../../codex-rs/routing/src/trim/render.rs) — `render_current_files`

---

## 19. Rumination detection (streaming)

**Problem:** Thinking-only local models (ones where the `<think>` channel can't be turned off — weights+template combo baked in) can spiral into self-interrupting loops: "Actually, wait. Let me reconsider. Hmm, on second thought..." until `max_tokens` runs out or the model finally stops. Symptom: after 2–10 minutes of wall clock, a response arrives with `content=""` and `tool_calls=[]`. The turn silently ends with no progress.

**What we do:** A streaming-time phrase-count detector watches the reasoning-channel deltas and aborts in-flight inference when the model shows signs of rumination.

- **Markers**: 23 case-insensitive word-boundary phrases characteristic of self-doubt — `actually`, `wait`, `but wait`, `hold on`, `hmm`, `let me reconsider`, `on second thought`, `let me think again`, `or maybe`, `or perhaps`, `rethinking`, `reconsider`, `going back`, `scratch that`, `nope`, `let me re-examine`, `let me revisit`, `i'm overthinking`, etc.
- **Budget gate**: Detector only fires once reasoning tokens exceed half of `max_tokens` (or half of a 4096 default when unset). Prevents false positives on a model that self-critiques once or twice during a normal chain.
- **Threshold**: ≥ 6 markers after the gate opens → flag as `Ruminating`.
- **Abort**: Dropping the SSE receiver closes the HTTP connection, signaling the server to stop generating and free its slot. No more tokens burned.
- **Re-prompt**: A `[RUMINATION GUARD]` continuation user-message is appended (hits count + approximate reasoning tokens) telling the model to pick the simplest next step and take it via a tool call, then the coder is re-invoked. Shares the same `MAX_BAIL_RETRIES = 3` cap as the completion-verifier loop.

**Code:**
- Detector (pure): [codex-rs/routing/src/rumination_detector.rs](../../codex-rs/routing/src/rumination_detector.rs) — `RuminationDetector`, `count_rumination_markers`, `continuation_prompt`
- Watcher wiring: [codex-rs/core/src/local_routing.rs](../../codex-rs/core/src/local_routing.rs) — the coder's streaming loop

**Log signals:**
- `Rumination watch reasoning_chars=... reasoning_tokens=... budget_gate=... marker_count=... threshold=... gated=true|false` (every 500 bytes of new reasoning)
- `Rumination guard aborted local coder; re-prompting hits=... reasoning_tokens=... continuation_count=...` (on abort)

---

## 20. Streaming coder path with tool-call assembly

**Problem:** Rumination detection (section 19) requires watching reasoning as it streams — the non-streaming request-response pattern wouldn't let us see the loop until the full response returned, which is exactly what we wanted to avoid. But the existing tool-aware call (`chat_with_tools`) was non-streaming, and the existing streaming call (`chat_stream`) didn't carry tools.

**What we do:** New `chat_with_tools_stream` tool-aware streaming path. Two readers (Ollama NDJSON, OpenAI SSE) emit a unified `StreamChunk` enum covering four variants:

- `Delta(String)` — user-visible content delta
- `ReasoningDelta(String)` — private chain-of-thought delta (for the rumination watcher and for diagnostic logging)
- `ToolCallDelta { index, id, name, arguments_delta }` — incremental tool-call info. OpenAI streams tool-call `arguments` as multiple string fragments concatenated per `index`; Ollama typically emits whole tool calls atomically in the final chunk. The accumulator handles both.
- `Done { input_tokens, output_tokens, reasoning_tokens }` — stream terminator with usage.

Caller (`local_routing.rs`) consumes the stream, accumulates content / reasoning / tool-calls, runs the rumination check every 500 bytes of new reasoning, and on normal `Done` assembles a body in Ollama wire shape so the existing bail-verifier and tool-dispatch code works unchanged.

**Code:**
- Streaming pool method: [codex-rs/routing/src/ollama.rs](../../codex-rs/routing/src/ollama.rs) — `chat_with_tools_stream`, `spawn_ollama_stream_reader`, `spawn_openai_sse_reader`
- Caller-side assembly + watcher: [codex-rs/core/src/local_routing.rs](../../codex-rs/core/src/local_routing.rs) — the streaming loop replacing the old `chat_with_tools` call, plus the inline `StreamToolCallAcc`

**Log signal (normal completion):** `Local coder response received content_len=... native_tool_calls=... reasoning_tokens=... continuation_count=...`

---

## Keeping this document current

When you add a new intervention that targets local-model fragility, add a section here that covers:

1. **Problem** — a one-sentence description of the failure mode in the wild
2. **What we do** — the intervention
3. **Code** — file path(s) with clickable links
4. **Log signal** — the grep-able log line that proves the intervention fired

The goal is that a future engineer can read this document, understand why every weird knob exists, and confidently decide whether a given knob is still needed (maybe the next generation of local models doesn't need it anymore).
