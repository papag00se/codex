//! In-process per-request routing to local Ollama models.
//!
//! Hooks into ModelClientSession::stream() to intercept requests that can
//! be handled by a local model (free) instead of the cloud provider.
//!
//! Uses a local classifier LLM to decide where each request goes.
//! See docs/spec/design-principles.md — the LLM makes the judgment call,
//! deterministic code handles the control flow.

#[derive(Default)]
struct StreamToolCallAcc {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

use crate::client_common::{Prompt, ResponseStream};
use codex_api::ResponseEvent;
use codex_protocol::models::{ContentItem, ResponseItem};
use codex_protocol::protocol::TokenUsage;
use codex_routing::OllamaClientPool;
use codex_routing::classifier::{RouteTarget, classify_request};
use codex_routing::config::{OllamaEndpoint, RoutingConfig};
use codex_routing::failover::{self, FailoverAction, FailureType};
use codex_routing::local_dispatch::{OllamaTextResponse, call_ollama_text};
use std::sync::Arc;
use tokio::sync::{OnceCell, mpsc};
use tracing::{info, warn};

/// Tools exposed to the LightCoder route — same in regular and local-only
/// modes. Curated to fit comfortably in a small local model's context window
/// and attention budget. Do not expand without a deliberate reason.
///
/// Names here must exactly match what's actually registered in the Codex tool
/// registry (see `codex-rs/tools/src/`). Names not present are silently
/// dropped by the filter — the model would then see fewer tools than intended,
/// which is how the first cut of this list went wrong.
///
/// Excluded by design: MCP connectors (`mcp__*`), multi-agent orchestration
/// (`spawn_*`, `wait_*`, `supervisor`, …), `js_repl`, `code_mode_*`, and
/// dynamic-tool plumbing. Cloud routes still see all of these.
///
/// Reads + greps + listings happen via `shell` (e.g. `cat`, `rg`, `ls`); there
/// is no dedicated `text_editor`/`grep_files`/`read_file` tool in this Codex
/// install. `list_dir` is kept alongside `shell ls` because it's safer and
/// cloud models use it natively.
const LIGHT_CODER_TOOL_NAMES: &[&str] = &[
    "shell",
    "apply_patch",
    "list_dir",
    "view_image",
    "update_plan",
    "local_web_search",
    "web_fetch",
    "request_permissions",
    "exec_command",
    "write_stdin",
];

/// Translate a slice of native Ollama tool calls, rewriting any whose name is
/// a recognized shell-command alias (e.g. `ls`, `git`, `cat`) into a proper
/// `shell` invocation. Calls whose name is already a registered Codex tool
/// pass through unchanged.
fn translate_native_tool_calls(raw_calls: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    raw_calls
        .into_iter()
        .map(translate_one_native_call)
        .collect()
}

fn translate_one_native_call(mut call: serde_json::Value) -> serde_json::Value {
    // Ollama wraps the call as either {"function": {"name", "arguments"}} or
    // a flat {"name", "arguments"}. Normalize.
    let func_obj_path: &[&str] = if call.get("function").is_some() {
        &["function"]
    } else {
        &[]
    };

    let name = func_obj_path
        .iter()
        .fold(Some(&call), |v, k| v.and_then(|v| v.get(*k)))
        .and_then(|v| v.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if name.is_empty() {
        return call;
    }

    let args_value: serde_json::Value = func_obj_path
        .iter()
        .fold(Some(&call), |v, k| v.and_then(|v| v.get(*k)))
        .and_then(|v| v.get("arguments"))
        .map(|v| {
            if let Some(s) = v.as_str() {
                serde_json::from_str(s).unwrap_or(serde_json::Value::Null)
            } else {
                v.clone()
            }
        })
        .unwrap_or(serde_json::Value::Null);

    // Try shell-alias / shell-shape translation first; fall back to
    // apply_patch normalization. Both rewrite `args` in place.
    let translated = codex_routing::tool_aliases::translate_to_shell_call(&name, &args_value)
        .or_else(|| {
            if name == "apply_patch" {
                codex_routing::tool_aliases::normalize_apply_patch_call(&args_value)
            } else {
                None
            }
        });
    let Some(translated) = translated else {
        return call;
    };

    info!(
        from = %name,
        to = %translated.name,
        command_line = %translated.command_line,
        "Translated tool call (native)"
    );

    let new_arguments = translated.args.to_string();
    if let Some(func) = call.get_mut("function").and_then(|f| f.as_object_mut()) {
        func.insert(
            "name".to_string(),
            serde_json::Value::String(translated.name.to_string()),
        );
        func.insert(
            "arguments".to_string(),
            serde_json::Value::String(new_arguments),
        );
    } else if let Some(obj) = call.as_object_mut() {
        obj.insert(
            "name".to_string(),
            serde_json::Value::String(translated.name.to_string()),
        );
        obj.insert(
            "arguments".to_string(),
            serde_json::Value::String(new_arguments),
        );
    }
    call
}

/// Build a short usage hint listing the tools the local model can call. This
/// gets appended to the system prompt for the LightCoder route only — small
/// models otherwise emit shell command names (`ls`, `rg`, `cat`) as tool
/// names, or guess at the arg shape. Naming the wrapper explicitly and
/// showing concrete examples closes most of the failure modes.
fn build_tool_hint(tool_names: &[&str]) -> String {
    let has = |name: &str| tool_names.contains(&name);
    let mut lines = vec!["You have ONLY the following tools. You MUST call them by these exact names with the exact argument shape shown in the examples — never invent tool names, never guess at argument shapes.".to_string()];

    for name in tool_names {
        let block = match *name {
            "shell" => {
                "- `shell`: Run any shell command. Use this for `ls`, `cat`, `rg`, `grep`, `find`, `mkdir`, `rm`, `cd`, `pwd`, build/test commands, package installs, writing files via heredoc — anything you would type at a terminal.\n  REQUIRED ARG SHAPE: `command` MUST be a JSON array of strings, ALWAYS prefixed with `[\"bash\", \"-lc\", \"<your command line>\"]`.\n  Correct example: `{\"command\": [\"bash\", \"-lc\", \"ls -la\"]}`.\n  WRONG: `{\"command\": \"ls -la\"}` (must be an array).\n  WRONG: `{\"command\": [\"bash\", \"-lc\", \"[bash, -lc, ls]\"]}` (do NOT nest the bash invocation; the third element is your literal shell command)."
            }
            "apply_patch" => {
                "- `apply_patch`: Create, modify, or delete files via a structured patch. Prefer this over `shell echo > file` for writing files.\n\n  TWO FORMATS ACCEPTED — pick whichever is most natural:\n\n  FORMAT A: standard unified diff (the format `git diff` produces). This works as-is — file headers `--- a/path` / `+++ b/path` and hunk headers `@@ -L,N +L,N @@` are fine. Example:\n  ```\n  --- a/handler.py\n  +++ b/handler.py\n  @@ -17,7 +17,7 @@\n   def lambda_handler(event, context):\n  -    url = \"https://api.handle.me/resolve/{handle}\"\n  +    url = \"https://api.handle.me/handles/{handle}\"\n       return requests.get(url)\n  ```\n  `/dev/null` for one side means create or delete: `--- /dev/null` + `+++ b/new.py` adds a new file; `--- a/old.py` + `+++ /dev/null` deletes one.\n\n  FORMAT B: Codex native format. Use this when you want explicit anchor-by-context matching:\n  ```\n  *** Begin Patch\n  *** Update File: handler.py\n  @@ def lambda_handler(event, context):\n  -    url = \"https://api.handle.me/resolve/{handle}\"\n  +    url = \"https://api.handle.me/handles/{handle}\"\n  *** End Patch\n  ```\n  Use `*** Add File: <path>` for new files (every body line prefixed `+`), `*** Update File: <path>` for edits, `*** Delete File: <path>` for deletes.\n\n  PREFIX RULE (both formats) — every non-empty line in a hunk body MUST start with EXACTLY ONE of:\n    `+` ... a line you are ADDING\n    `-` ... a line you are REMOVING (Update only)\n    ` ` (a single space) ... a line that is UNCHANGED, included only as context to anchor the change (Update only)\n  Bare code lines without one of these prefixes are INVALID."
            }
            "list_dir" => {
                "- `list_dir`: List directory contents (safer alternative to `shell ls`). Args: `{\"dir_path\": \"/abs/path\"}`. Path must be absolute."
            }
            "view_image" => {
                "- `view_image`: View a local image file. Args: `{\"path\": \"/abs/path/to/image.png\"}`."
            }
            "update_plan" => {
                "- `update_plan`: Track a multi-step task plan. Args: `{\"plan\": [{\"status\": \"in_progress\", \"step\": \"...\"}]}`."
            }
            "local_web_search" => {
                "- `local_web_search`: Search the web via Brave; returns titles, URLs, and short descriptions. Args: `{\"query\": \"<search terms>\", \"count\": 10}` (count optional, 1-20). Pair this with `web_fetch` to read a specific result."
            }
            "web_fetch" => {
                "- `web_fetch`: Fetch a single http(s) URL and return the page body as text. Use this BEFORE writing code against an unfamiliar API or library — read the docs page rather than guessing the endpoint shape. Args: `{\"url\": \"https://...\"}`. Body is capped at 512KB; binary responses return a placeholder."
            }
            "request_permissions" => {
                "- `request_permissions`: Ask for sandbox escalation when a command would otherwise be blocked (network access for `npm install`/`pip install`/`apt`, writing to a path outside cwd, etc.). Call this BEFORE the command that would fail, with a short justification."
            }
            "exec_command" => {
                "- `exec_command`: Start a long-running shell command with streaming output. Use INSTEAD OF `shell` for: dev servers (`npm run dev`), watch processes, anything that runs more than ~5 seconds, or any command you might need to send input to. Returns a session id you can pair with `write_stdin`."
            }
            "write_stdin" => {
                "- `write_stdin`: Send input to a shell session previously started by `exec_command` (e.g. answer an interactive prompt from `npm init`). Args include the session id and the text to write."
            }
            _ => continue,
        };
        lines.push(block.to_string());
    }

    if has("shell") {
        lines.push(
            "If you find yourself wanting to call a command like `ls`, `rg`, `cat`, `git`, or `pytest` directly, that is wrong — wrap it as `shell` with `command: [\"bash\", \"-lc\", \"<the command>\"]`.".to_string(),
        );
    }

    lines.join("\n\n")
}

/// Global routing state — initialized lazily on first use.
static ROUTING_STATE: OnceCell<Option<RoutingState>> = OnceCell::const_new();

struct RoutingState {
    config: RoutingConfig,
    project_config: codex_routing::project_config::ProjectConfig,
    pool: Arc<OllamaClientPool>,
    usage: codex_routing::usage::UsageTracker,
    feedback: std::sync::Mutex<codex_routing::feedback::FeedbackStore>,
    codebase_context: codex_routing::codebase_context::CodebaseContext,
    classify_cache: std::sync::Mutex<codex_routing::classify_cache::ClassifyCache>,
    budget: Arc<codex_routing::budget_pressure::BudgetState>,
    claude_sessions: codex_routing::claude_cli::ClaudeSessionTracker,
    /// Single-entry cache of the most recent older-turn compaction summary.
    /// Keyed by hash of the older-turn message contents; reused as long as
    /// the older history is unchanged from request to request within a
    /// session. Prevents recompacting the same history each turn.
    inline_compact_cache: std::sync::Mutex<Option<InlineCompactCacheEntry>>,
}

#[derive(Clone)]
struct InlineCompactCacheEntry {
    older_content_hash: u64,
    summary_message: serde_json::Value,
}

/// Initialize the global routing state.
/// Loads from `.codex-multi/config.toml` in the current directory, falling
/// back to environment variables for anything not in the config file.
/// Called once, lazily. Returns None if local routing is not configured.
async fn get_routing_state() -> &'static Option<RoutingState> {
    ROUTING_STATE
        .get_or_init(|| async {
            // Load project config from .codex-multi/config.toml if it exists
            let cwd = std::env::current_dir().unwrap_or_default();
            let project_config = codex_routing::project_config::ProjectConfig::load(&cwd);
            let config = RoutingConfig::from_project_config(&project_config);
            let pool = Arc::new(OllamaClientPool::new());

            // Check if the classifier endpoint is reachable via a fast HTTP
            // GET that doesn't require loading a model into GPU memory
            // (a chat request would take 30s on cold start). The probe path
            // depends on flavor: Ollama exposes `/api/version` (returns
            // `{"version": "0.x.y"}`); OpenAI-compat servers expose
            // `/v1/models` (returns the loaded model list). LM Studio,
            // llama.cpp, and vLLM all support `/v1/models`.
            let version_url = match config.classifier.flavor {
                codex_routing::config::ClientFlavor::Ollama => format!(
                    "{}/api/version",
                    config.classifier.base_url.trim_end_matches('/')
                ),
                codex_routing::config::ClientFlavor::OpenAICompat => {
                    let base = config
                        .classifier
                        .base_url
                        .trim_end_matches('/')
                        .trim_end_matches("/v1");
                    format!("{base}/v1/models")
                }
            };
            let initial_reachable = pool
                .client()
                .get(&version_url)
                .timeout(std::time::Duration::from_secs(5))
                .send()
                .await
                .is_ok();

            // Always create the routing state, even if the classifier is
            // unreachable at startup. Reachability is re-checked per request
            // in `route_request` (the classifier's own fallback returns
            // `CloudCoder` when it can't be reached, so subsequent requests
            // degrade gracefully without locking the entire session into
            // "no routing"). This avoids the failure mode where a single
            // flaky network blip at session start sends every request to
            // cloud for the rest of the session.
            if initial_reachable {
                info!(
                    classifier_url = %config.classifier.base_url,
                    classifier_model = %config.classifier.model,
                    "Per-request routing enabled — classifier LLM reachable at startup"
                );
            } else {
                info!(
                    classifier_url = %config.classifier.base_url,
                    "Classifier LLM not reachable at startup; routing state created anyway, will retry per request"
                );
            }

            let usage = codex_routing::usage::UsageTracker::new(
                project_config.usage.primary_warn_threshold,
            );
            let feedback = std::sync::Mutex::new(
                codex_routing::feedback::FeedbackStore::new(&cwd),
            );
            let codebase_context = codex_routing::codebase_context::CodebaseContext::detect(&cwd);
            let classify_cache = std::sync::Mutex::new(
                codex_routing::classify_cache::ClassifyCache::new(),
            );
            let budget = Arc::new(codex_routing::budget_pressure::BudgetState::new());
            let claude_sessions = codex_routing::claude_cli::ClaudeSessionTracker::new();
            Some(RoutingState {
                config,
                project_config,
                pool,
                usage,
                feedback,
                codebase_context,
                classify_cache,
                budget,
                claude_sessions,
                inline_compact_cache: std::sync::Mutex::new(None),
            })
        })
        .await
}

/// Record session usage to `.codex-multi/usage_log.jsonl`.
/// Called at session exit from the TUI.
pub async fn record_session_usage() {
    let Some(state) = get_routing_state().await.as_ref() else {
        return;
    };

    let cwd = std::env::current_dir().unwrap_or_default();
    let analytics = codex_routing::cost_analytics::CostAnalytics::new(&cwd);

    let local = state.usage.local_usage();
    let secondary = state.usage.secondary_usage();
    let primary = state.usage.primary_usage();
    let total_requests = local.request_count + secondary.request_count + primary.request_count;
    let total_tokens = local.total_tokens() + secondary.total_tokens() + primary.total_tokens();

    if total_requests == 0 {
        return; // Nothing to record
    }

    let savings_pct = if total_requests > 0 {
        ((local.request_count + secondary.request_count) as f64 / total_requests as f64) * 100.0
    } else {
        0.0
    };

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let summary = codex_routing::cost_analytics::SessionUsageSummary {
        session_id: format!("session_{timestamp}"),
        timestamp,
        duration_seconds: 0, // We don't track session duration
        local_requests: local.request_count,
        local_tokens: local.total_tokens(),
        secondary_requests: secondary.request_count,
        secondary_tokens: secondary.total_tokens(),
        primary_requests: primary.request_count,
        primary_tokens: primary.total_tokens(),
        total_requests,
        total_tokens,
        estimated_savings_pct: savings_pct,
    };

    analytics.record_session(&summary);
}

/// Get usage summary string. Returns None if routing is not active.
/// Called from the TUI `/stats` command.
pub async fn usage_summary() -> Option<String> {
    let state = get_routing_state().await.as_ref()?;
    Some(state.usage.summary())
}

/// Record cloud model usage (called from client.rs after cloud responses).
pub(crate) async fn record_cloud_usage(model: &str, input_tokens: u64, output_tokens: u64) {
    if let Some(state) = get_routing_state().await.as_ref() {
        state.usage.record(model, input_tokens, output_tokens);
    }
}

/// Update budget state from rate limit headers (called after cloud responses).
/// primary_pct and secondary_pct are 0.0-100.0.
pub(crate) async fn update_budget(
    primary_pct: f64,
    secondary_pct: f64,
    primary_reset: Option<u64>,
) {
    if let Some(state) = get_routing_state().await.as_ref() {
        state
            .budget
            .update(primary_pct, secondary_pct, primary_reset);
    }
}

/// Result of per-request routing.
pub(crate) enum RouteResult {
    /// Request handled locally — use this stream.
    Local(ResponseStream),
    /// Request should go to cloud, but with this model override.
    /// The slug replaces model_info.slug for this request only.
    /// Carries failover chain context so cloud errors can walk the chain.
    CloudOverride {
        slug: String,
        failover_ctx: Option<CloudFailoverCtx>,
    },
    /// No routing — use the default cloud model.
    Default,
}

/// Cloud failover context — passed to stream() so it can retry on HTTP errors.
#[derive(Clone)]
pub(crate) struct CloudFailoverCtx {
    pub role_name: String,
    pub chain_name: String,
    pub chain: Vec<String>,
    pub behavior: codex_routing::project_config::FailoverBehavior,
}

/// What a role name resolves to.
enum ResolvedRole {
    /// Local Ollama model.
    Local(OllamaEndpoint),
    /// Cloud model via OpenAI Responses API (slug override).
    Cloud(String),
    /// Anthropic model via Claude CLI subprocess.
    ClaudeExec { model: String },
}

/// Map a classifier route to the failover chain name.
fn chain_name_for_route(route: &RouteTarget) -> &'static str {
    match route {
        RouteTarget::LightReasoner => "reasoning",
        RouteTarget::LightCoder => "coding",
        RouteTarget::CloudFast | RouteTarget::CloudMini => "coding",
        RouteTarget::CloudReasoner => "reasoning",
        RouteTarget::CloudCoder => "coding",
    }
}

/// Returns true if local_only mode is requested, even when no RoutingState
/// has been initialized (e.g., classifier endpoint unreachable at startup).
/// Reads from the env var only — config.toml requires RoutingState to load.
fn local_only_env() -> bool {
    matches!(
        std::env::var("CODEX_LOCAL_ONLY")
            .unwrap_or_default()
            .trim()
            .to_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Inside the failover loop, decide whether to surface a local-only error
/// (when local_only is on) or fall through to the default cloud path.
fn cloud_fallback_or_local_error(state: &RoutingState, reason: &str) -> RouteResult {
    if state.config.local_only {
        local_only_error(reason)
    } else {
        RouteResult::Default
    }
}

/// Build a RouteResult that surfaces a local-only error to the user as an
/// assistant message, instead of silently falling through to cloud.
fn local_only_error(reason: &str) -> RouteResult {
    let message = format!(
        "Local-only mode is enabled, but no local model can serve this request: {reason}.\n\nThe request was not sent to any cloud provider. To allow cloud dispatch, disable local-only mode (unset CODEX_LOCAL_ONLY, remove --local-only, or set `local_only = false` in `.codex-multi/config.toml` under [routing])."
    );
    warn!(reason = %reason, "local_only: surfacing error to user");
    RouteResult::Local(ollama_response_to_stream(OllamaTextResponse {
        content: message,
        model: "local-only".to_string(),
        input_tokens: 0,
        output_tokens: 0,
    }))
}

/// Map a classifier route to its role name in the failover chain.
fn role_name_for_route(route: &RouteTarget) -> &'static str {
    match route {
        RouteTarget::LightReasoner => "light_reasoner",
        RouteTarget::LightCoder => "light_coder",
        RouteTarget::CloudFast => "cloud_fast",
        RouteTarget::CloudMini => "cloud_mini",
        RouteTarget::CloudReasoner => "cloud_reasoner",
        RouteTarget::CloudCoder => "cloud_coder",
    }
}

/// Resolve a role name from the failover chain to a concrete endpoint or cloud slug.
fn resolve_role(role_name: &str, state: &RoutingState) -> Option<ResolvedRole> {
    match role_name {
        "light_reasoner" => {
            if state.config.reasoner.enabled {
                Some(ResolvedRole::Local(state.config.reasoner.clone()))
            } else {
                None
            }
        }
        "light_reasoner_backup" => {
            if state.config.reasoner_backup.enabled {
                Some(ResolvedRole::Local(state.config.reasoner_backup.clone()))
            } else {
                None
            }
        }
        "light_coder" => {
            if state.config.light_coder.enabled {
                Some(ResolvedRole::Local(state.config.light_coder.clone()))
            } else {
                None
            }
        }
        "compactor" => {
            if state.config.compactor.enabled {
                Some(ResolvedRole::Local(state.config.compactor.clone()))
            } else {
                None
            }
        }
        // Cloud roles — resolve via weighted selection, dispatch by provider
        "cloud_fast" | "cloud_mini" | "cloud_reasoner" | "cloud_coder" => {
            match pick_cloud_model_with_provider(&state.project_config, role_name) {
                Some((slug, provider)) if provider == "anthropic" => {
                    Some(ResolvedRole::ClaudeExec { model: slug })
                }
                Some((slug, _)) => Some(ResolvedRole::Cloud(slug)),
                None => None,
            }
        }
        // Classifier itself — not a useful failover target
        "classifier" => None,
        _ => None,
    }
}

/// Check if a prompt is a compaction request.
///
/// Two recognizers:
///   1. The legacy `<<<LOCAL_COMPACT>>>` sentinel — kept for callers that
///      explicitly want our specialized local pipeline.
///   2. The opening line of Codex's built-in compaction prompt template
///      (`"CONTEXT CHECKPOINT COMPACTION"`, see
///      `core/templates/compact/prompt.md`). When Codex's `run_compact_task`
///      fires (auto-compact at token limit, or manual `/compact`), the
///      synthesized user message starts with that line. Detecting it lets
///      our specialized pipeline take over for local sessions instead of
///      asking the local model to do the whole summarization itself.
fn is_compaction_request(prompt: &Prompt) -> bool {
    let text = extract_last_message(prompt);
    text.contains("<<<LOCAL_COMPACT>>>") || text.contains("CONTEXT CHECKPOINT COMPACTION")
}

/// Route a request: local model, cloud with model override, or default.
///
/// Called from ModelClientSession::stream() on every model API call.
pub(crate) async fn route_request(prompt: &Prompt) -> RouteResult {
    // Compaction requests: run the full compaction pipeline locally.
    // Detects <<<LOCAL_COMPACT>>> sentinel and runs normalize → chunk →
    // extract → merge → render on local Ollama. No proxy needed.
    if is_compaction_request(prompt) {
        if let Some(state) = get_routing_state().await.as_ref() {
            // In local_only mode the Coder absorbs compaction; in cloud mode
            // the dedicated `compactor` endpoint is used.
            let endpoint = if state.config.local_only {
                &state.config.light_coder
            } else {
                &state.config.compactor
            };
            if endpoint.enabled {
                info!(
                    model = %endpoint.model,
                    "Compaction request detected — running full pipeline locally"
                );

                // Phase 3: feed the compaction pipeline through the same
                // role-aware trimmer used for per-request routing. The
                // compactor inherits all the dedup/stale-removal/error-
                // preservation logic for free, and we stop maintaining two
                // parallel cleanup paths.
                let project_instructions = extract_project_instructions(prompt);
                let trim_input = codex_routing::trim::TrimInput {
                    items: &prompt.input,
                    system_prompt: &prompt.base_instructions.text,
                    user_instructions: project_instructions.as_deref(),
                    // Compaction summarizes history, so pinning fresh file
                    // content doesn't help the compactor and would inflate
                    // the input. Skip the file-state injection here.
                    current_files: None,
                    flavor: endpoint.flavor,
                };
                let trimmed = codex_routing::trim::trim_for_local(
                    &trim_input,
                    endpoint.num_ctx,
                );
                info!(
                    trim_summary = %trimmed.summary.to_log_line(),
                    "Trimmed transcript for compaction input"
                );
                // The compaction pipeline expects bare `{role, content}`
                // dicts — extract those from the trimmed messages and drop
                // any tool-call shapes the compactor can't ingest.
                let items: Vec<serde_json::Value> = trimmed
                    .messages
                    .iter()
                    .filter(|m| m.get("content").and_then(|c| c.as_str()).is_some())
                    .cloned()
                    .collect();

                let last_msg = extract_last_message(prompt);
                let current_request = last_msg
                    .replace("<<<LOCAL_COMPACT>>>", "")
                    .trim()
                    .to_string();

                let compaction_config = codex_routing::compaction::CompactionConfig::default();

                match codex_routing::compaction::compact_transcript(
                    &items,
                    &current_request,
                    &state.pool,
                    endpoint,
                    &compaction_config,
                )
                .await
                {
                    Ok(summary) => {
                        info!(summary_len = summary.len(), "Compaction pipeline complete");
                        // Return the compacted summary as a text response
                        let response = codex_routing::local_dispatch::OllamaTextResponse {
                            content: summary,
                            model: endpoint.model.clone(),
                            input_tokens: 0, // Pipeline doesn't track total
                            output_tokens: 0,
                        };
                        return RouteResult::Local(ollama_response_to_stream(response));
                    }
                    Err(e) => {
                        warn!(error = %e, "Compaction pipeline failed, falling back to cloud");
                        // Fall through to normal routing
                    }
                }
            }
        }
    }

    let state = match get_routing_state().await.as_ref() {
        Some(s) => s,
        None => {
            if local_only_env() {
                return local_only_error("classifier endpoint is not reachable");
            }
            return RouteResult::Default;
        }
    };

    // Extract the last user message for classification
    let prompt_text = extract_last_message(prompt);
    if prompt_text.is_empty() {
        return cloud_fallback_or_local_error(state, "request had no user message text");
    }

    // In local_only mode the classifier is a no-op: there's no cloud tier to
    // pick, and the Coder absorbs every request (reasoning-shaped or not).
    // Skip the LLM call, the cache, and the remap entirely.
    let classification = if state.config.local_only {
        info!("local_only: bypassing classifier — routing to LightCoder");
        codex_routing::classifier::ClassifyResult {
            route: RouteTarget::LightCoder,
            tools_potential: true,
            reason: "local_only: classifier bypassed".to_string(),
        }
    } else {
        // Extract tool names (just names, not full schemas)
        let tool_names: Vec<&str> = prompt.tools.iter().map(|t| t.name()).collect();

        // Count recent tool calls from conversation history
        let (tool_call_count, turn_count) = count_recent_activity(prompt);

        // G8: Check classifier cache — skip the 3-4s LLM call if confident
        let cached_classification = state
            .classify_cache
            .lock()
            .ok()
            .and_then(|cache| cache.try_cached());

        if let Some(ref cached) = cached_classification {
            info!(
                route = ?cached.route,
                reason = %cached.reason,
                "Using cached classification (skipping classifier LLM)"
            );
        }

        // Use cached classification if available, otherwise call the classifier LLM
        if let Some(cached) = cached_classification {
            cached
        } else {
            let routing_profile = state
                .feedback
                .lock()
                .map(|f| f.profile_context())
                .unwrap_or_default();
            let codebase_ctx = state.codebase_context.classifier_context();

            // G14: Add budget pressure to classifier context
            let budget_ctx = state.budget.pressure_context();
            let full_context = if budget_ctx.is_empty() {
                codebase_ctx.clone()
            } else {
                format!("{codebase_ctx}\n{budget_ctx}")
            };

            let result = codex_routing::classifier::classify_request_with_context(
                &prompt_text,
                &tool_names,
                tool_call_count,
                turn_count,
                &state.config,
                &state.pool,
                &routing_profile,
                &full_context,
            )
            .await;

            // Record in cache for future requests
            if let Ok(mut cache) = state.classify_cache.lock() {
                cache.record(&result);
            }

            result
        }
    };

    // G14: Hard-block primary if budget is critical (deterministic, not LLM)
    let mut route =
        if state.budget.should_block_primary() && classification.route == RouteTarget::CloudCoder {
            warn!(
                primary_used = state.budget.primary_used(),
                "Primary budget critical — downgrading cloud_coder to cloud_reasoner"
            );
            RouteTarget::CloudReasoner
        } else {
            classification.route
        };

    // Conversation-state override: if there are recent tool calls in the
    // history but the classifier picked LightReasoner (a text-only route),
    // upgrade to LightCoder. Local models choke when handed an assistant
    // message containing `tool_calls` without a corresponding tools array
    // — they typically respond with empty output. The classifier sees only
    // the latest user turn and can't tell that a tool-use thread is in
    // flight; this is a deterministic correction layered on top.
    if matches!(route, RouteTarget::LightReasoner)
        && conversation_has_recent_tool_calls(prompt)
    {
        info!(
            "Override: classifier picked LightReasoner but history has tool calls — upgrading to LightCoder"
        );
        route = RouteTarget::LightCoder;
    }

    // --- Determine the failover chain for this route ---
    let chain_name = chain_name_for_route(&route);
    let initial_role = role_name_for_route(&route);
    let mut chain = state.project_config.failover_chain(chain_name).to_vec();
    if state.config.local_only {
        chain.retain(|role| !codex_routing::project_config::is_cloud_role(role));
    }
    let behavior = &state.project_config.failover.behavior;

    // Walk the failover chain starting from the classified route.
    let mut current_role = initial_role.to_string();
    let mut attempt: u32 = 0;

    loop {
        // Resolve the current role to a concrete endpoint or cloud slug
        let resolved = resolve_role(&current_role, state);

        match resolved {
            Some(ResolvedRole::Cloud(slug)) => {
                info!(
                    route = %current_role,
                    model = %slug,
                    reason = %classification.reason,
                    "Routing to cloud model (override)"
                );
                return RouteResult::CloudOverride {
                    slug,
                    failover_ctx: Some(CloudFailoverCtx {
                        role_name: current_role.clone(),
                        chain_name: chain_name.to_string(),
                        chain: chain.clone(),
                        behavior: behavior.clone(),
                    }),
                };
            }
            Some(ResolvedRole::ClaudeExec { model }) => {
                info!(
                    route = %current_role,
                    model = %model,
                    reason = %classification.reason,
                    "Routing to Anthropic via Claude CLI"
                );

                let prompt_text = extract_last_message(prompt);
                let claude_binary = state.project_config.cli.claude.clone();
                let cwd = std::env::current_dir().ok();

                // Use conversation-level session key for context resumption
                let session_key = "main";
                let resume_id = state.claude_sessions.get_session(session_key);

                let result = codex_routing::claude_cli::invoke_claude(
                    &claude_binary,
                    &model,
                    &prompt_text,
                    resume_id.as_deref(),
                    cwd.as_deref(),
                )
                .await;

                match result {
                    Ok(cli_result) => {
                        // Track session for resumption
                        if let Some(ref sid) = cli_result.session_id {
                            state.claude_sessions.set_session(session_key, sid);
                        }

                        // Record usage
                        state.usage.record(
                            &cli_result.model,
                            cli_result.input_tokens,
                            cli_result.output_tokens,
                        );

                        let response = OllamaTextResponse {
                            content: cli_result.content,
                            model: cli_result.model,
                            input_tokens: cli_result.input_tokens,
                            output_tokens: cli_result.output_tokens,
                        };
                        return RouteResult::Local(ollama_response_to_stream(response));
                    }
                    Err(e) => {
                        warn!(error = %e, "Claude CLI failed");
                        let action = failover::decide_action(
                            FailureType::ModelUnavailable,
                            &current_role,
                            chain_name,
                            &chain,
                            attempt,
                            None,
                            behavior,
                        );
                        match action {
                            FailoverAction::NextInChain { model_role, reason } => {
                                info!(from = %current_role, to = %model_role, reason = %reason, "Failover from Claude CLI");
                                current_role = model_role;
                                attempt = 0;
                                continue;
                            }
                            FailoverAction::RetrySame {
                                wait,
                                attempt: next,
                            } => {
                                tokio::time::sleep(wait).await;
                                attempt = next;
                                continue;
                            }
                            _ => {
                                return cloud_fallback_or_local_error(
                                    state,
                                    "Claude CLI dispatch failed and no failover chain entry resolved",
                                );
                            }
                        }
                    }
                }
            }
            Some(ResolvedRole::Local(endpoint)) => {
                // Try this local model
                let result =
                    try_local_model(prompt, &endpoint, &route, &classification, state).await;

                match result {
                    Ok(stream) => return RouteResult::Local(stream),
                    Err(failure_type) => {
                        // Local model failed — consult the failover executor
                        let action = failover::decide_action(
                            failure_type,
                            &current_role,
                            chain_name,
                            &chain,
                            attempt,
                            None, // no retry-after for local models
                            behavior,
                        );

                        match action {
                            FailoverAction::RetrySame {
                                wait,
                                attempt: next_attempt,
                            } => {
                                info!(
                                    model = %current_role,
                                    wait_ms = wait.as_millis() as u64,
                                    attempt = next_attempt,
                                    "Failover: retrying same local model"
                                );
                                tokio::time::sleep(wait).await;
                                attempt = next_attempt;
                                continue;
                            }
                            FailoverAction::NextInChain { model_role, reason } => {
                                info!(
                                    from = %current_role,
                                    to = %model_role,
                                    reason = %reason,
                                    "Failover: walking to next model in chain"
                                );
                                current_role = model_role;
                                attempt = 0;
                                continue;
                            }
                            FailoverAction::HardFail { reason } => {
                                warn!(reason = %reason, "Failover: hard fail");
                                return cloud_fallback_or_local_error(
                                    state,
                                    &format!("local model failover hard-failed: {reason}"),
                                );
                            }
                            FailoverAction::ChainExhausted { chain_name } => {
                                warn!(chain = %chain_name, "Failover: chain exhausted, using default");
                                return cloud_fallback_or_local_error(
                                    state,
                                    &format!("local failover chain '{chain_name}' exhausted"),
                                );
                            }
                        }
                    }
                }
            }
            None => {
                // Role can't be resolved (disabled, no config, etc.)
                // Walk to next in chain
                let action = failover::decide_action(
                    FailureType::ModelNotFound,
                    &current_role,
                    chain_name,
                    &chain,
                    0,
                    None,
                    behavior,
                );
                match action {
                    FailoverAction::NextInChain { model_role, reason } => {
                        info!(
                            from = %current_role,
                            to = %model_role,
                            reason = %reason,
                            "Failover: role unresolvable, walking chain"
                        );
                        current_role = model_role;
                        attempt = 0;
                        continue;
                    }
                    _ => {
                        return cloud_fallback_or_local_error(
                            state,
                            "no role in failover chain could be resolved",
                        );
                    }
                }
            }
        }
    }
}

/// Try executing a request on a local Ollama model.
/// Returns Ok(ResponseStream) on success, Err(FailureType) on failure.
///
/// This is the only entry point that hands a transcript to a local model. It
/// runs `trim_for_local` to produce a role-aware, deduplicated prompt; the
/// same trimming logic is also used by the compaction pipeline. Local models
/// are not gimped in any mode-specific way — the only thing that varies is
/// routing (whether we even reach this function vs. dispatching to cloud).
async fn try_local_model(
    prompt: &Prompt,
    endpoint: &OllamaEndpoint,
    route: &RouteTarget,
    classification: &codex_routing::classifier::ClassifyResult,
    state: &RoutingState,
) -> Result<ResponseStream, FailureType> {
    info!(
        model = %endpoint.model,
        route = ?route,
        reason = %classification.reason,
        "Routing to local model (free)"
    );

    let model_name = endpoint.model.clone();
    let route_name = format!("{:?}", route);

    // Estimate what the cloud model would have processed (savings metric).
    let pre_trim_tokens = estimate_prompt_tokens(prompt);
    state.usage.record_savings(pre_trim_tokens as u64);

    // Pull AGENTS.md / CLAUDE.md content out of the conversation so it can
    // be pinned into the persistent context block at the top of the prelude
    // (always visible, never aged out, distinct from rolling content). It's
    // also still preserved as a user message via the trim's user-message rule.
    let project_instructions = extract_project_instructions(prompt);

    // Re-read every file the active turn has edited so the trimmer can pin
    // fresh content into the prelude. Without this the model works from
    // its memory of the pre-patch state and writes patches with stale `-`
    // lines — the same failure mode that caused multi-turn patch loops in
    // early local-model sessions (see docs/spec/local-coder-massaging.md).
    let current_files = load_active_turn_files(&prompt.input);

    // Trim the transcript with role-aware semantic compression.
    let trim_input = codex_routing::trim::TrimInput {
        items: &prompt.input,
        system_prompt: &prompt.base_instructions.text,
        user_instructions: project_instructions.as_deref(),
        current_files: current_files.as_ref(),
        flavor: endpoint.flavor,
    };
    let trimmed = codex_routing::trim::trim_for_local(&trim_input, endpoint.num_ctx);
    info!(
        trim_summary = %trimmed.summary.to_log_line(),
        "Trimmed transcript for local model"
    );

    // If the trimmed transcript still exceeds ~85% of the local model's
    // context window, summarize the older-turn portion via the compaction
    // pipeline and replace it with a single summary message. The active turn
    // is always preserved verbatim. Cached by hash of the older content so
    // we don't recompact identical history each turn.
    let trimmed = maybe_inline_compact(trimmed, endpoint, state).await;

    let codex_routing::trim::TrimResult {
        system: trimmed_system,
        messages,
        ..
    } = trimmed;

    // LightCoder route gets a curated tool subset — applied identically in
    // regular and local-only modes. The full Codex tool catalog is ~120
    // schemas (MCP connectors, multi-agent orchestration, dynamic tools, …),
    // which exceeds the local model's context window and overwhelms its
    // attention. We expose only the tools a coding model actually needs to
    // execute work in the workspace. Cloud routes still receive the full set.
    //
    // Adding a new tool to this list is a deliberate decision: keep it
    // focused on capabilities the local model can use successfully.
    let use_tools = matches!(route, RouteTarget::LightCoder);
    if use_tools {
        // Subset selection driven by the endpoint's `tool_subset` config.
        // `Focused` (default) keeps the small curated set for local models
        // that lose attention on a 120-tool catalog. `Full` exposes the
        // entire catalog for capable local models that can handle it.
        let tool_json: Vec<serde_json::Value> = match endpoint.tool_subset {
            codex_routing::config::ToolSubset::Focused => prompt
                .tools
                .iter()
                .filter(|t| LIGHT_CODER_TOOL_NAMES.contains(&t.name()))
                .filter_map(|t| serde_json::to_value(t).ok())
                .collect(),
            codex_routing::config::ToolSubset::Full => prompt
                .tools
                .iter()
                .filter_map(|t| serde_json::to_value(t).ok())
                .collect(),
        };
        let ollama_tools = codex_routing::tool_format::to_ollama_tools(&tool_json);

        // Append a tool-usage hint to the system prompt. Small local models
        // habitually emit shell command names (`ls`, `rg`, `cat`) as tool
        // names because that's how their training data shaped them. Telling
        // them explicitly which tool wraps which capability avoids the
        // hallucination loop without restricting what they can do.
        let tool_names: Vec<&str> = ollama_tools
            .iter()
            .filter_map(|t| {
                t.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
            })
            .collect();
        let hint = build_tool_hint(&tool_names);
        let system = Some(format!("{trimmed_system}\n\n{hint}"));

        let dropped_tool_names: Vec<&str> = match endpoint.tool_subset {
            codex_routing::config::ToolSubset::Focused => LIGHT_CODER_TOOL_NAMES
                .iter()
                .copied()
                .filter(|name| !tool_names.contains(name))
                .collect(),
            codex_routing::config::ToolSubset::Full => Vec::new(),
        };
        info!(
            tool_count = ollama_tools.len(),
            available_in_prompt = prompt.tools.len(),
            subset = ?endpoint.tool_subset,
            tools_passed = ?tool_names,
            tools_dropped = ?dropped_tool_names,
            "Passing tool set to local coder"
        );

        // Loop here so we can re-call the model up to MAX_BAIL_RETRIES times
        // when the completion verifier flags a "bail" — see the body of the
        // loop for details. Set to 3 so a model that's making genuine
        // progress (e.g. ran a probe but hasn't applied the result yet)
        // gets enough nudges to land the change before we give up.
        const MAX_BAIL_RETRIES: usize = 3;
        let mut effective_messages = messages.clone();
        let mut continuation_count = 0usize;
        let last_user_message = extract_last_message(prompt);

        loop {
            // Streaming coder call with in-flight rumination detection.
            // See rumination_detector.rs for the heuristic; the watcher
            // here aborts the HTTP connection (by dropping the receiver)
            // when the detector flags a loop, then re-prompts with a
            // guard directive so we don't burn 10 min of reasoning on a
            // model that's stuck self-interrupting.
            let Some(mut stream_rx) = state
                .pool
                .chat_with_tools_stream(
                    endpoint,
                    effective_messages.clone(),
                    system.as_deref(),
                    Some(ollama_tools.clone()),
                )
                .await
            else {
                warn!("Local coder stream failed to start");
                return Err(FailureType::ModelUnavailable);
            };

            let detector = codex_routing::rumination_detector::RuminationDetector
                ::from_endpoint_max_tokens(endpoint.max_tokens);

            let mut content = String::new();
            let mut reasoning = String::new();
            let mut tool_call_acc: std::collections::BTreeMap<usize, StreamToolCallAcc> =
                std::collections::BTreeMap::new();
            let mut input_tokens = 0u64;
            let mut output_tokens = 0u64;
            let mut reasoning_tokens_seen = 0u64;
            // Re-run the rumination regex at most every N bytes of new
            // reasoning so a long stream doesn't quadratic-scan itself.
            const RUMINATION_CHECK_STRIDE: usize = 500;
            let mut next_check_at = RUMINATION_CHECK_STRIDE;

            let mut rumination_trigger: Option<(usize, usize)> = None;
            let mut stream_ended_cleanly = false;

            while let Some(chunk) = stream_rx.recv().await {
                match chunk {
                    codex_routing::ollama::StreamChunk::Delta(text) => {
                        content.push_str(&text);
                    }
                    codex_routing::ollama::StreamChunk::ReasoningDelta(text) => {
                        reasoning.push_str(&text);
                        if reasoning.len() >= next_check_at {
                            next_check_at = reasoning.len() + RUMINATION_CHECK_STRIDE;
                            // Prefer the server's reported reasoning-token
                            // count when the usage chunk has already landed;
                            // otherwise estimate from char count. Most SSE
                            // servers only emit usage in the final chunk,
                            // so the estimate is what actually fires the
                            // budget gate mid-stream.
                            let tokens = if reasoning_tokens_seen > 0 {
                                reasoning_tokens_seen as usize
                            } else {
                                codex_routing::rumination_detector::estimate_reasoning_tokens(
                                    &reasoning,
                                )
                            };
                            let marker_count =
                                codex_routing::rumination_detector::count_rumination_markers(
                                    &reasoning,
                                );
                            let budget_gate = detector.budget_gate();
                            let gated = tokens >= budget_gate;
                            info!(
                                reasoning_chars = reasoning.len(),
                                reasoning_tokens = tokens,
                                budget_gate,
                                marker_count,
                                threshold = detector.threshold(),
                                gated,
                                "Rumination watch"
                            );
                            if gated && marker_count >= detector.threshold() {
                                rumination_trigger = Some((marker_count, tokens));
                                break;
                            }
                        }
                    }
                    codex_routing::ollama::StreamChunk::ToolCallDelta {
                        index, id, name, arguments_delta,
                    } => {
                        let acc = tool_call_acc.entry(index).or_default();
                        if let Some(v) = id { acc.id = Some(v); }
                        if let Some(v) = name { acc.name = Some(v); }
                        acc.arguments.push_str(&arguments_delta);
                    }
                    codex_routing::ollama::StreamChunk::Done {
                        input_tokens: it,
                        output_tokens: ot,
                        reasoning_tokens: rt,
                    } => {
                        input_tokens = it;
                        output_tokens = ot;
                        reasoning_tokens_seen = rt;
                        stream_ended_cleanly = true;
                        break;
                    }
                }
            }

            // Dropping stream_rx here (end of scope or explicit) closes the
            // TCP connection when the stream task next tries to send,
            // which signals LM Studio / Ollama to stop generating.
            drop(stream_rx);

            if let Some((hits, rumination_tokens)) = rumination_trigger {
                info!(
                    hits,
                    reasoning_tokens = rumination_tokens,
                    reasoning_len = reasoning.len(),
                    continuation_count,
                    "Rumination guard aborted local coder; re-prompting"
                );
                if continuation_count >= MAX_BAIL_RETRIES {
                    warn!("Rumination guard hit retry cap; returning last partial response");
                    // Fall through to assemble whatever we got.
                } else {
                    let guard = codex_routing::rumination_detector::continuation_prompt(
                        hits,
                        rumination_tokens,
                    );
                    effective_messages.push(serde_json::json!({
                        "role": "user",
                        "content": guard,
                    }));
                    continuation_count += 1;
                    continue;
                }
            }

            if !stream_ended_cleanly && rumination_trigger.is_none() {
                warn!("Local coder stream closed without Done; treating as unavailable");
                return Err(FailureType::ModelUnavailable);
            }

            if !reasoning.is_empty() {
                tracing::debug!(
                    reasoning_len = reasoning.len(),
                    reasoning_tokens = reasoning_tokens_seen,
                    reasoning = %reasoning,
                    "Local coder reasoning channel"
                );
            }

            // Assemble tool calls from the accumulator in Ollama wire shape.
            let raw_tool_calls: Vec<serde_json::Value> = tool_call_acc
                .into_values()
                .map(|acc| {
                    serde_json::json!({
                        "function": {
                            "name": acc.name.unwrap_or_default(),
                            "arguments": acc.arguments,
                        }
                    })
                })
                .collect();
            let native_tool_calls = translate_native_tool_calls(raw_tool_calls);

            info!(
                content_len = content.len(),
                native_tool_calls = native_tool_calls.len(),
                reasoning_tokens = reasoning_tokens_seen,
                continuation_count,
                "Local coder response received"
            );

            // Record local usage for /stats
            state.usage.record(&model_name, input_tokens, output_tokens);

            // Completion verification: when the model produced text-only with
            // no tool calls, ask the classifier whether the response is a
            // legitimate completion or an "announcement-then-bail." If a
            // bail, inject a continuation prompt and re-call (capped).
            if native_tool_calls.is_empty()
                && !content.trim().is_empty()
                && continuation_count < MAX_BAIL_RETRIES
                && !last_user_message.trim().is_empty()
            {
                // In local_only mode the classifier endpoint is offline by
                // design — route the verifier through the Coder so the
                // bail-detector still works. In cloud mode keep using the
                // small classifier (fast and warm).
                let verifier_endpoint = if state.config.local_only {
                    &state.config.light_coder
                } else {
                    &state.config.classifier
                };
                let verdict = codex_routing::completion_verifier::verify_completion(
                    &last_user_message,
                    &content,
                    verifier_endpoint,
                    &state.pool,
                )
                .await;
                info!(
                    verdict = ?verdict,
                    "Completion verifier judged the model's text-only response"
                );
                if matches!(
                    verdict,
                    codex_routing::completion_verifier::CompletionVerdict::Bail
                ) {
                    let continuation = codex_routing::completion_verifier::continuation_prompt(
                        &content,
                    );
                    // Preserve the model's prior text in history so it can
                    // see what it just said, then append the verifier's
                    // continuation prompt as a synthesized user message.
                    effective_messages.push(serde_json::json!({
                        "role": "assistant",
                        "content": content,
                    }));
                    effective_messages.push(serde_json::json!({
                        "role": "user",
                        "content": continuation,
                    }));
                    continuation_count += 1;
                    continue;
                }
            }

            return Ok(ollama_tool_response_to_stream(
                content,
                native_tool_calls,
                model_name.clone(),
                input_tokens,
                output_tokens,
            ));
        }
    }

    // Streaming path (no tools — reasoner). No tool hint needed since this
    // route is text-only by design.
    let stream_rx = state
        .pool
        .chat_stream(endpoint, messages, Some(trimmed_system.as_str()))
        .await;

    let Some(mut ollama_rx) = stream_rx else {
        warn!("Local model stream failed to start");
        return Err(FailureType::ModelUnavailable);
    };

    let (event_tx, event_rx) = mpsc::channel(64);
    let model_for_usage = model_name.clone();
    let model_for_task = model_name.clone();
    let route_for_task = route_name.clone();

    tokio::spawn(async move {
        let _ = event_tx.send(Ok(ResponseEvent::Created)).await;

        let placeholder = ResponseItem::Message {
            id: Some("local_msg_0".to_string()),
            role: "assistant".to_string(),
            content: vec![ContentItem::OutputText {
                text: String::new(),
            }],
            end_turn: None,
            phase: None,
        };
        let _ = event_tx
            .send(Ok(ResponseEvent::OutputItemAdded(placeholder)))
            .await;

        let mut full_text = String::new();
        let mut input_tokens = 0u64;
        let mut output_tokens = 0u64;

        while let Some(chunk) = ollama_rx.recv().await {
            match chunk {
                codex_routing::ollama::StreamChunk::Delta(text) => {
                    full_text.push_str(&text);
                    let _ = event_tx
                        .send(Ok(ResponseEvent::OutputTextDelta(text)))
                        .await;
                }
                codex_routing::ollama::StreamChunk::ReasoningDelta(_)
                | codex_routing::ollama::StreamChunk::ToolCallDelta { .. } => {
                    // Reasoner path is text-only — ignore reasoning and
                    // tool-call deltas if the server happens to emit any.
                }
                codex_routing::ollama::StreamChunk::Done {
                    input_tokens: it,
                    output_tokens: ot,
                    ..
                } => {
                    input_tokens = it;
                    output_tokens = ot;
                    break;
                }
            }
        }

        // Record local usage for /stats
        if let Some(state) = get_routing_state().await.as_ref() {
            state
                .usage
                .record(&model_for_usage, input_tokens, output_tokens);
        }

        let recovered = codex_routing::tool_recovery::recover_tool_calls(&full_text, false);

        if recovered.tool_calls.is_empty() {
            let final_message = ResponseItem::Message {
                id: Some("local_msg_0".to_string()),
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText { text: full_text }],
                end_turn: Some(true),
                phase: None,
            };
            let _ = event_tx
                .send(Ok(ResponseEvent::OutputItemDone(final_message)))
                .await;
        } else {
            if !recovered.content.is_empty() {
                let text_message = ResponseItem::Message {
                    id: Some("local_msg_0".to_string()),
                    role: "assistant".to_string(),
                    content: vec![ContentItem::OutputText {
                        text: recovered.content,
                    }],
                    end_turn: None,
                    phase: None,
                };
                let _ = event_tx
                    .send(Ok(ResponseEvent::OutputItemDone(text_message)))
                    .await;
            }

            for (i, tc) in recovered.tool_calls.iter().enumerate() {
                let call_id = tc.id.clone().unwrap_or_else(|| format!("local_call_{i}"));
                let (final_name, final_args) =
                    match codex_routing::tool_aliases::translate_to_shell_call(
                        &tc.name,
                        &tc.arguments,
                    ) {
                        Some(translated) => {
                            info!(
                                from = %tc.name,
                                to = "shell",
                                command_line = %translated.command_line,
                                "Translated shell-alias tool call (recovered)"
                            );
                            (translated.name.to_string(), translated.args)
                        }
                        None => (tc.name.clone(), tc.arguments.clone()),
                    };
                let arguments = serde_json::to_string(&final_args).unwrap_or_else(|_| "{}".into());

                let func_call = ResponseItem::FunctionCall {
                    id: Some(format!("local_fc_{i}")),
                    name: final_name,
                    namespace: None,
                    arguments,
                    call_id,
                };
                let _ = event_tx
                    .send(Ok(ResponseEvent::OutputItemAdded(func_call.clone())))
                    .await;
                let _ = event_tx
                    .send(Ok(ResponseEvent::OutputItemDone(func_call)))
                    .await;
            }
        }

        let _ = event_tx
            .send(Ok(ResponseEvent::Completed {
                response_id: "local_response".to_string(),
                token_usage: Some(TokenUsage {
                    input_tokens: input_tokens as i64,
                    output_tokens: output_tokens as i64,
                    ..Default::default()
                }),
            }))
            .await;
    });

    info!(
        model = %model_name,
        route = %route_name,
        "Streaming from local model (free)"
    );

    Ok(ResponseStream { rx_event: event_rx })
}

/// Pick a cloud model from the project config's weighted entries for a role.
/// Returns None if no config exists for this role.
fn pick_cloud_model(
    pc: &codex_routing::project_config::ProjectConfig,
    role_name: &str,
) -> Option<String> {
    pick_cloud_model_with_provider(pc, role_name).map(|(slug, _)| slug)
}

/// Pick a cloud model and its provider from the project config.
/// Returns (model_slug, provider_name) or None.
fn pick_cloud_model_with_provider(
    pc: &codex_routing::project_config::ProjectConfig,
    role_name: &str,
) -> Option<(String, String)> {
    use codex_routing::project_config::ModelRole;

    let role = pc.get_model(role_name)?;
    match role {
        ModelRole::Single {
            provider, model, ..
        } => Some((model.clone(), provider.clone())),
        ModelRole::Weighted { entries } => {
            if entries.is_empty() {
                return None;
            }
            let total_weight: u32 = entries.iter().map(|e| e.weight).sum();
            if total_weight == 0 {
                return Some((entries[0].model.clone(), entries[0].provider.clone()));
            }
            let mut pick = rand_u32() % total_weight;
            for entry in entries {
                if pick < entry.weight {
                    return Some((entry.model.clone(), entry.provider.clone()));
                }
                pick -= entry.weight;
            }
            Some((entries[0].model.clone(), entries[0].provider.clone()))
        }
    }
}

/// Handle a cloud API error by consulting the failover executor.
/// Returns Some(new_slug) if we should retry with a different model,
/// or None if we should propagate the error.
///
/// Called from client.rs when a cloud request fails with an HTTP error.
pub(crate) async fn handle_cloud_failover(
    ctx: &mut CloudFailoverCtx,
    status_code: Option<u16>,
    error_message: &str,
    attempt: &mut u32,
    retry_after_ms: Option<u64>,
) -> Option<String> {
    let failure_type = failover::classify_failure(
        status_code,
        error_message,
        false, // not a quality failure (we don't check cloud response quality)
        false, // not context overflow (would need specific detection)
    );

    let action = failover::decide_action(
        failure_type,
        &ctx.role_name,
        &ctx.chain_name,
        &ctx.chain,
        *attempt,
        retry_after_ms,
        &ctx.behavior,
    );

    match action {
        FailoverAction::RetrySame {
            wait,
            attempt: next_attempt,
        } => {
            info!(
                model = %ctx.role_name,
                wait_ms = wait.as_millis() as u64,
                attempt = next_attempt,
                "Cloud failover: retrying same model"
            );
            tokio::time::sleep(wait).await;
            *attempt = next_attempt;
            // Return the same slug — caller should retry the request
            let state = get_routing_state().await.as_ref()?;
            pick_cloud_model(&state.project_config, &ctx.role_name)
        }
        FailoverAction::NextInChain { model_role, reason } => {
            info!(
                from = %ctx.role_name,
                to = %model_role,
                reason = %reason,
                "Cloud failover: walking to next model in chain"
            );
            // Update context for potential future failures
            ctx.role_name = model_role.clone();
            *attempt = 0;

            // Resolve the next role — only cloud models (local would need
            // a full re-route which we don't do from the cloud path)
            let state = get_routing_state().await.as_ref()?;
            pick_cloud_model(&state.project_config, &model_role)
        }
        FailoverAction::HardFail { reason } => {
            warn!(reason = %reason, "Cloud failover: hard fail");
            None
        }
        FailoverAction::ChainExhausted { chain_name } => {
            warn!(chain = %chain_name, "Cloud failover: chain exhausted");
            None
        }
    }
}

/// Simple random u32 — no external crate dependency.
fn rand_u32() -> u32 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    std::time::Instant::now().hash(&mut hasher);
    std::thread::current().id().hash(&mut hasher);
    hasher.finish() as u32
}

// --- Response translation ---

/// Convert an Ollama text response into a ResponseStream that codex-core expects.
///
/// Event sequence must be: Created → OutputItemAdded → OutputItemDone → Completed.
/// Do NOT send ServerModel (triggers reroute detection when model name differs).
/// Do NOT send OutputTextDelta before OutputItemAdded (panics).
fn ollama_response_to_stream(response: OllamaTextResponse) -> ResponseStream {
    let (tx, rx) = mpsc::channel(16);

    tokio::spawn(async move {
        // 1. Created
        let _ = tx.send(Ok(ResponseEvent::Created)).await;

        let message = ResponseItem::Message {
            id: Some("local_msg_0".to_string()),
            role: "assistant".to_string(),
            content: vec![ContentItem::OutputText {
                text: response.content,
            }],
            end_turn: Some(true),
            phase: None,
        };

        // 2. OutputItemAdded — registers the item so deltas/done can reference it
        let _ = tx
            .send(Ok(ResponseEvent::OutputItemAdded(message.clone())))
            .await;

        // 3. OutputItemDone — the complete message
        let _ = tx.send(Ok(ResponseEvent::OutputItemDone(message))).await;

        // 4. Completed with usage
        let _ = tx
            .send(Ok(ResponseEvent::Completed {
                response_id: "local_response".to_string(),
                token_usage: Some(TokenUsage {
                    input_tokens: response.input_tokens as i64,
                    output_tokens: response.output_tokens as i64,
                    ..Default::default()
                }),
            }))
            .await;
    });

    ResponseStream { rx_event: rx }
}

/// Convert an Ollama response with native tool_calls to a ResponseStream.
/// Handles both native Ollama tool_calls and embedded JSON tool calls.
fn ollama_tool_response_to_stream(
    content: String,
    native_tool_calls: Vec<serde_json::Value>,
    model: String,
    input_tokens: u64,
    output_tokens: u64,
) -> ResponseStream {
    let (tx, rx) = mpsc::channel(16);

    tokio::spawn(async move {
        let _ = tx.send(Ok(ResponseEvent::Created)).await;

        // Emit text content if any
        if !content.is_empty() {
            let text_msg = ResponseItem::Message {
                id: Some("local_msg_0".to_string()),
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: content.clone(),
                }],
                end_turn: Some(native_tool_calls.is_empty()),
                phase: None,
            };
            let _ = tx
                .send(Ok(ResponseEvent::OutputItemAdded(text_msg.clone())))
                .await;
            let _ = tx.send(Ok(ResponseEvent::OutputItemDone(text_msg))).await;
        }

        // Emit native tool calls from Ollama
        for (i, tc) in native_tool_calls.iter().enumerate() {
            let func = tc.get("function").unwrap_or(tc);
            let name = func
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("unknown")
                .to_string();
            let call_id = tc
                .get("id")
                .and_then(|id| id.as_str())
                .map(String::from)
                .unwrap_or_else(|| format!("local_call_{i}"));
            let arguments = func
                .get("arguments")
                .map(|a| {
                    if a.is_string() {
                        a.as_str().unwrap_or("{}").to_string()
                    } else {
                        serde_json::to_string(a).unwrap_or_else(|_| "{}".into())
                    }
                })
                .unwrap_or_else(|| "{}".into());

            let func_call = ResponseItem::FunctionCall {
                id: Some(format!("local_fc_{i}")),
                name,
                namespace: None,
                arguments,
                call_id,
            };
            let _ = tx
                .send(Ok(ResponseEvent::OutputItemAdded(func_call.clone())))
                .await;
            let _ = tx.send(Ok(ResponseEvent::OutputItemDone(func_call))).await;
        }

        // If no native tool calls, try recovering embedded ones from text
        if native_tool_calls.is_empty() && !content.is_empty() {
            let recovered = codex_routing::tool_recovery::recover_tool_calls(&content, false);
            for (i, tc) in recovered.tool_calls.iter().enumerate() {
                let call_id = tc.id.clone().unwrap_or_else(|| format!("local_call_{i}"));
                let (final_name, final_args) =
                    match codex_routing::tool_aliases::translate_to_shell_call(
                        &tc.name,
                        &tc.arguments,
                    ) {
                        Some(t) => (t.name.to_string(), t.args),
                        None => (tc.name.clone(), tc.arguments.clone()),
                    };
                let arguments = serde_json::to_string(&final_args).unwrap_or_else(|_| "{}".into());
                let func_call = ResponseItem::FunctionCall {
                    id: Some(format!("local_fc_{i}")),
                    name: final_name,
                    namespace: None,
                    arguments,
                    call_id,
                };
                let _ = tx
                    .send(Ok(ResponseEvent::OutputItemAdded(func_call.clone())))
                    .await;
                let _ = tx.send(Ok(ResponseEvent::OutputItemDone(func_call))).await;
            }
        }

        let _ = tx
            .send(Ok(ResponseEvent::Completed {
                response_id: "local_response".to_string(),
                token_usage: Some(TokenUsage {
                    input_tokens: input_tokens as i64,
                    output_tokens: output_tokens as i64,
                    ..Default::default()
                }),
            }))
            .await;
    });

    ResponseStream { rx_event: rx }
}

/// Convert an Ollama response with potential tool calls into a ResponseStream.
/// Runs tool-call recovery to extract embedded function calls.
#[allow(dead_code)]
fn ollama_response_to_stream_with_tools(response: OllamaTextResponse) -> ResponseStream {
    let (tx, rx) = mpsc::channel(16);

    tokio::spawn(async move {
        let _ = tx.send(Ok(ResponseEvent::Created)).await;

        let recovered = codex_routing::tool_recovery::recover_tool_calls(&response.content, false);

        if recovered.tool_calls.is_empty() {
            // No tool calls — just text
            let message = ResponseItem::Message {
                id: Some("local_msg_0".to_string()),
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: response.content,
                }],
                end_turn: Some(true),
                phase: None,
            };
            let _ = tx
                .send(Ok(ResponseEvent::OutputItemAdded(message.clone())))
                .await;
            let _ = tx.send(Ok(ResponseEvent::OutputItemDone(message))).await;
        } else {
            // Has tool calls
            if !recovered.content.is_empty() {
                let text_msg = ResponseItem::Message {
                    id: Some("local_msg_0".to_string()),
                    role: "assistant".to_string(),
                    content: vec![ContentItem::OutputText {
                        text: recovered.content,
                    }],
                    end_turn: None,
                    phase: None,
                };
                let _ = tx
                    .send(Ok(ResponseEvent::OutputItemAdded(text_msg.clone())))
                    .await;
                let _ = tx.send(Ok(ResponseEvent::OutputItemDone(text_msg))).await;
            }

            for (i, tc) in recovered.tool_calls.iter().enumerate() {
                let call_id = tc.id.clone().unwrap_or_else(|| format!("local_call_{i}"));
                let arguments =
                    serde_json::to_string(&tc.arguments).unwrap_or_else(|_| "{}".into());

                let func_call = ResponseItem::FunctionCall {
                    id: Some(format!("local_fc_{i}")),
                    name: tc.name.clone(),
                    namespace: None,
                    arguments,
                    call_id,
                };
                let _ = tx
                    .send(Ok(ResponseEvent::OutputItemAdded(func_call.clone())))
                    .await;
                let _ = tx.send(Ok(ResponseEvent::OutputItemDone(func_call))).await;
            }
        }

        let _ = tx
            .send(Ok(ResponseEvent::Completed {
                response_id: "local_response".to_string(),
                token_usage: Some(TokenUsage {
                    input_tokens: response.input_tokens as i64,
                    output_tokens: response.output_tokens as i64,
                    ..Default::default()
                }),
            }))
            .await;
    });

    ResponseStream { rx_event: rx }
}

// --- Prompt extraction helpers ---

/// Extract the last user message from the prompt.
/// This is what the classifier sees — just the current request, not full history.
fn extract_last_message(prompt: &Prompt) -> String {
    for item in prompt.input.iter().rev() {
        if let ResponseItem::Message { role, content, .. } = item {
            if role == "user" {
                let text: String = content
                    .iter()
                    .filter_map(|c| match c {
                        ContentItem::InputText { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if !text.is_empty() {
                    return text;
                }
            }
        }
    }
    String::new()
}

/// Count recent tool calls and turns from conversation history.
fn count_recent_activity(prompt: &Prompt) -> (usize, usize) {
    let mut tool_calls = 0;
    let mut turns = 0;

    // Count from the last ~10 items
    for item in prompt.input.iter().rev().take(10) {
        match item {
            ResponseItem::Message { .. } => turns += 1,
            ResponseItem::FunctionCall { .. } | ResponseItem::LocalShellCall { .. } => {
                tool_calls += 1;
            }
            _ => {}
        }
    }

    (tool_calls, turns)
}

/// Extract system instructions from the prompt.
/// Returns true if the conversation contains tool calls (function calls or
/// local-shell calls) anywhere in the most recent ~20 items. Used to detect
/// in-flight tool-use threads that would break a route switch to a no-tools
/// path like LightReasoner.
fn conversation_has_recent_tool_calls(prompt: &Prompt) -> bool {
    let count = prompt.input.len();
    let start = count.saturating_sub(20);
    prompt.input[start..].iter().any(|item| {
        matches!(
            item,
            ResponseItem::FunctionCall { .. }
                | ResponseItem::LocalShellCall { .. }
                | ResponseItem::CustomToolCall { .. }
                | ResponseItem::FunctionCallOutput { .. }
                | ResponseItem::CustomToolCallOutput { .. }
        )
    })
}

/// Read every file the active turn has edited (via `apply_patch` Add/Update)
/// and return a `path -> current_content` map. Missing files, unreadable
/// files, and non-UTF-8 files are silently skipped. Returns `None` when the
/// active turn hasn't modified any files, so the trimmer's file-state block
/// is omitted entirely in the common case.
///
/// Paths are resolved against the process `cwd` — matching how every other
/// local-coder tool handler in this crate resolves paths. The trimmer has
/// no IO of its own by design; this function is the only place the routing
/// layer reads from disk on behalf of the prelude builder.
fn load_active_turn_files(
    items: &[codex_protocol::models::ResponseItem],
) -> Option<std::collections::HashMap<String, String>> {
    let paths = codex_routing::trim::files_modified_in_active_turn(items);
    if paths.is_empty() {
        return None;
    }
    let cwd = std::env::current_dir().ok();
    let mut out = std::collections::HashMap::with_capacity(paths.len());
    for path in paths {
        let candidate = match &cwd {
            Some(base) => base.join(&path),
            None => std::path::PathBuf::from(&path),
        };
        if let Ok(content) = std::fs::read_to_string(&candidate) {
            out.insert(path, content);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Extract the AGENTS.md / CLAUDE.md content from the conversation, if any.
/// Codex injects these as a user message early in `prompt.input` with a
/// recognizable header. We pull the content (between the `<INSTRUCTIONS>`
/// markers when present, otherwise the full message) so it can be pinned to
/// the local model's persistent-context block — same content, more prominent
/// placement than just being one user message in a long history.
fn extract_project_instructions(prompt: &Prompt) -> Option<String> {
    for item in &prompt.input {
        let ResponseItem::Message { role, content, .. } = item else {
            continue;
        };
        if role != "user" {
            continue;
        }
        let text: String = content
            .iter()
            .filter_map(|c| match c {
                ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                    Some(text.as_str())
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        if !is_project_instructions_message(&text) {
            continue;
        }
        // Strip the surrounding `<INSTRUCTIONS>...</INSTRUCTIONS>` if present
        // so the prelude doesn't carry the wrapper tags.
        let body = strip_instructions_wrapper(&text);
        if !body.trim().is_empty() {
            return Some(body);
        }
    }
    None
}

fn is_project_instructions_message(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with("# AGENTS.md")
        || trimmed.starts_with("# CLAUDE.md")
        || trimmed.starts_with("AGENTS.md instructions")
        || trimmed.starts_with("CLAUDE.md instructions")
}

fn strip_instructions_wrapper(text: &str) -> String {
    let Some(start) = text.find("<INSTRUCTIONS>") else {
        return text.to_string();
    };
    let after_open = &text[start + "<INSTRUCTIONS>".len()..];
    let inner = match after_open.find("</INSTRUCTIONS>") {
        Some(end) => &after_open[..end],
        None => after_open,
    };
    inner.trim_matches(['\n', '\r']).to_string()
}

/// Trigger threshold: when trim's estimate exceeds this fraction of the local
/// model's context window, we summarize the older portion inline. Leaves
/// headroom for the active turn + the model's own response.
const INLINE_COMPACT_TRIGGER_FRACTION: usize = 85;

/// If the trimmed transcript still exceeds the local model's context budget,
/// run the compaction pipeline on the older-turn portion and replace it with
/// a single summary message. The active turn is left untouched.
///
/// Cached by hash of the older-turn message contents so repeated requests
/// within a session reuse the same summary instead of recompacting.
async fn maybe_inline_compact(
    mut trimmed: codex_routing::trim::TrimResult,
    endpoint: &OllamaEndpoint,
    state: &RoutingState,
) -> codex_routing::trim::TrimResult {
    let trigger = endpoint.num_ctx.saturating_mul(INLINE_COMPACT_TRIGGER_FRACTION) / 100;
    if trimmed.summary.estimated_input_tokens <= trigger {
        return trimmed;
    }
    let older_count = trimmed.summary.older_turn_message_count;
    if older_count == 0 {
        // Nothing to compact — the active turn alone is over budget. Trying
        // to summarize the active turn would lose the user's current request.
        warn!(
            estimated_tokens = trimmed.summary.estimated_input_tokens,
            target_ctx = endpoint.num_ctx,
            "Trimmed transcript exceeds local context budget but has no older turns to compact"
        );
        return trimmed;
    }
    if !state.config.compactor.enabled {
        warn!(
            estimated_tokens = trimmed.summary.estimated_input_tokens,
            target_ctx = endpoint.num_ctx,
            "Trimmed transcript over budget but compactor endpoint is disabled — sending as-is"
        );
        return trimmed;
    }

    // Hash the older messages so we can reuse the summary if the conversation
    // history hasn't shifted between requests.
    let older_messages: Vec<serde_json::Value> =
        trimmed.messages[..older_count].to_vec();
    let active_messages: Vec<serde_json::Value> =
        trimmed.messages[older_count..].to_vec();
    let content_hash = hash_messages(&older_messages);

    if let Some(cached) = state
        .inline_compact_cache
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        && cached.older_content_hash == content_hash
    {
        info!("Reusing cached inline-compaction summary");
        let mut new_messages = vec![cached.summary_message];
        new_messages.extend(active_messages);
        let new_token_estimate = estimate_combined_tokens(&trimmed.system, &new_messages);
        trimmed.messages = new_messages;
        trimmed.summary.older_turn_message_count = 1;
        trimmed.summary.estimated_input_tokens = new_token_estimate;
        return trimmed;
    }

    info!(
        estimated_tokens = trimmed.summary.estimated_input_tokens,
        target_ctx = endpoint.num_ctx,
        older_count,
        "Trimmed transcript over budget — running inline compaction"
    );

    let compaction_config = codex_routing::compaction::CompactionConfig::default();
    // Use the most recent older user message as the "current request" anchor
    // for the summary.
    let anchor = older_messages
        .iter()
        .rev()
        .find_map(|m| {
            if m.get("role").and_then(|r| r.as_str()) == Some("user") {
                m.get("content").and_then(|c| c.as_str()).map(str::to_string)
            } else {
                None
            }
        })
        .unwrap_or_else(|| "(rolling summary)".to_string());

    let summary_text = match codex_routing::compaction::compact_transcript(
        &older_messages,
        &anchor,
        &state.pool,
        &state.config.compactor,
        &compaction_config,
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "Inline compaction failed — sending trimmed transcript as-is");
            return trimmed;
        }
    };

    let summary_message = serde_json::json!({
        "role": "user",
        "content": format!(
            "[earlier conversation summarized]\n\n{summary_text}"
        ),
    });

    if let Ok(mut guard) = state.inline_compact_cache.lock() {
        *guard = Some(InlineCompactCacheEntry {
            older_content_hash: content_hash,
            summary_message: summary_message.clone(),
        });
    }

    let mut new_messages = vec![summary_message];
    new_messages.extend(active_messages);
    let new_token_estimate = estimate_combined_tokens(&trimmed.system, &new_messages);
    info!(
        before_tokens = trimmed.summary.estimated_input_tokens,
        after_tokens = new_token_estimate,
        "Inline compaction complete"
    );
    trimmed.messages = new_messages;
    trimmed.summary.older_turn_message_count = 1;
    trimmed.summary.estimated_input_tokens = new_token_estimate;
    trimmed
}

/// Sum the token estimate of the system prompt and the text content of every
/// message — same shape `trim_for_local` uses internally.
fn estimate_combined_tokens(system: &str, messages: &[serde_json::Value]) -> usize {
    let messages_text: String = messages
        .iter()
        .filter_map(|m| {
            m.get("content")
                .and_then(|c| c.as_str())
                .map(str::to_string)
        })
        .collect::<Vec<_>>()
        .join("\n");
    codex_routing::metrics::estimate_tokens(system)
        + codex_routing::metrics::estimate_tokens(&messages_text)
}

fn hash_messages(messages: &[serde_json::Value]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::Hash;
    use std::hash::Hasher;
    let mut hasher = DefaultHasher::new();
    for m in messages {
        if let Some(s) = m.get("content").and_then(|c| c.as_str()) {
            s.hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// Rough estimate of how many tokens the cloud model would have processed for
/// this prompt. Used as the savings metric when routing locally.
///
/// Walks every message item in the prompt, counts text length, and applies the
/// shared `estimate_tokens` heuristic. Tool calls and outputs are not counted
/// here — we underestimate slightly, but this is only a coarse savings number.
fn estimate_prompt_tokens(prompt: &Prompt) -> usize {
    let mut acc = String::new();
    if !prompt.base_instructions.text.is_empty() {
        acc.push_str(&prompt.base_instructions.text);
        acc.push('\n');
    }
    for item in &prompt.input {
        if let ResponseItem::Message { content, .. } = item {
            for c in content {
                match c {
                    ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                        acc.push_str(text);
                        acc.push('\n');
                    }
                    _ => {}
                }
            }
        }
    }
    codex_routing::metrics::estimate_tokens(&acc)
}
