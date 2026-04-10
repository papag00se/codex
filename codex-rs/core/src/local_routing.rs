//! In-process per-request routing to local Ollama models.
//!
//! Hooks into ModelClientSession::stream() to intercept requests that can
//! be handled by a local model (free) instead of the cloud provider.
//!
//! Uses a local classifier LLM to decide where each request goes.
//! See docs/spec/design-principles.md — the LLM makes the judgment call,
//! deterministic code handles the control flow.

use crate::client_common::{Prompt, ResponseStream};
use codex_api::ResponseEvent;
use codex_protocol::models::{ContentItem, ResponseItem};
use codex_protocol::protocol::TokenUsage;
use codex_routing::classifier::{classify_request, RouteTarget};
use codex_routing::config::{OllamaEndpoint, RoutingConfig};
use codex_routing::failover::{self, FailoverAction, FailureType};
use codex_routing::local_dispatch::{call_ollama_text, OllamaTextResponse};
use codex_routing::OllamaClientPool;
use std::sync::Arc;
use tokio::sync::{mpsc, OnceCell};
use tracing::{info, warn};

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

            // Check if the classifier endpoint is reachable via /api/version
            // (fast HTTP GET — doesn't require loading a model into GPU memory,
            // unlike a chat request which can take 30s on cold start)
            let version_url = format!(
                "{}/api/version",
                config.classifier.base_url.trim_end_matches('/')
            );
            let reachable = pool.client()
                .get(&version_url)
                .timeout(std::time::Duration::from_secs(5))
                .send()
                .await
                .is_ok();

            if reachable {
                info!(
                    classifier_url = %config.classifier.base_url,
                    classifier_model = %config.classifier.model,
                    "Per-request routing enabled — classifier LLM reachable"
                );
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
                Some(RoutingState { config, project_config, pool, usage, feedback, codebase_context, classify_cache, budget })
            } else {
                info!("Per-request routing disabled — classifier LLM not reachable, all requests go to cloud");
                None
            }
        })
        .await
}

/// Record session usage to `.codex-multi/usage_log.jsonl`.
/// Called at session exit from the TUI.
pub async fn record_session_usage() {
    let Some(state) = get_routing_state().await.as_ref() else { return };

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
pub(crate) async fn update_budget(primary_pct: f64, secondary_pct: f64, primary_reset: Option<u64>) {
    if let Some(state) = get_routing_state().await.as_ref() {
        state.budget.update(primary_pct, secondary_pct, primary_reset);
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

/// What a role name resolves to — either a local Ollama endpoint or a cloud model slug.
enum ResolvedRole {
    Local(OllamaEndpoint),
    Cloud(String), // model slug
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
        // Cloud roles — resolve to model slug via weighted selection
        "cloud_fast" | "cloud_mini" | "cloud_reasoner" | "cloud_coder" => {
            pick_cloud_model(&state.project_config, role_name)
                .map(ResolvedRole::Cloud)
        }
        // Classifier itself — not a useful failover target
        "classifier" => None,
        _ => None,
    }
}

/// Check if a prompt contains the compaction sentinel.
fn is_compaction_request(prompt: &Prompt) -> bool {
    let text = extract_last_message(prompt);
    text.contains("<<<LOCAL_COMPACT>>>")
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
            if state.config.compactor.enabled {
                info!("Compaction request detected — running full pipeline locally");
                let messages = prompt_to_ollama_messages(prompt);
                let last_msg = extract_last_message(prompt);

                // Strip the sentinel from the current request
                let current_request = last_msg.replace("<<<LOCAL_COMPACT>>>", "").trim().to_string();

                // Convert messages to items for the pipeline
                let items: Vec<serde_json::Value> = messages;

                let compaction_config = codex_routing::compaction::CompactionConfig::default();

                match codex_routing::compaction::compact_transcript(
                    &items,
                    &current_request,
                    &state.pool,
                    &state.config.compactor,
                    &compaction_config,
                ).await {
                    Ok(summary) => {
                        info!(
                            summary_len = summary.len(),
                            "Compaction pipeline complete"
                        );
                        // Return the compacted summary as a text response
                        let response = codex_routing::local_dispatch::OllamaTextResponse {
                            content: summary,
                            model: state.config.compactor.model.clone(),
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
        None => return RouteResult::Default,
    };

    // Extract the last user message for classification
    let prompt_text = extract_last_message(prompt);
    if prompt_text.is_empty() {
        return RouteResult::Default;
    }

    // Extract tool names (just names, not full schemas)
    let tool_names: Vec<&str> = prompt
        .tools
        .iter()
        .map(|t| t.name())
        .collect();

    // Count recent tool calls from conversation history
    let (tool_call_count, turn_count) = count_recent_activity(prompt);

    // G8: Check classifier cache — skip the 3-4s LLM call if confident
    let cached_classification = state.classify_cache
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
    let classification = if let Some(cached) = cached_classification {
        cached
    } else {
        let routing_profile = state.feedback
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
    };

    // G14: Hard-block primary if budget is critical (deterministic, not LLM)
    let route = if state.budget.should_block_primary()
        && classification.route == RouteTarget::CloudCoder
    {
        warn!(
            primary_used = state.budget.primary_used(),
            "Primary budget critical — downgrading cloud_coder to cloud_reasoner"
        );
        RouteTarget::CloudReasoner
    } else {
        classification.route
    };

    // --- Determine the failover chain for this route ---
    let chain_name = chain_name_for_route(&route);
    let initial_role = role_name_for_route(&route);
    let chain = state.project_config.failover_chain(chain_name).to_vec();
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
            Some(ResolvedRole::Local(endpoint)) => {
                // Try this local model
                let result = try_local_model(
                    prompt,
                    &endpoint,
                    &route,
                    &classification,
                    state,
                ).await;

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
                            FailoverAction::RetrySame { wait, attempt: next_attempt } => {
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
                                return RouteResult::Default;
                            }
                            FailoverAction::ChainExhausted { chain_name } => {
                                warn!(chain = %chain_name, "Failover: chain exhausted, using default");
                                return RouteResult::Default;
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
                        return RouteResult::Default;
                    }
                }
            }
        }
    }
}

/// Try executing a request on a local Ollama model.
/// Returns Ok(ResponseStream) on success, Err(FailureType) on failure.
async fn try_local_model(
    prompt: &Prompt,
    endpoint: &OllamaEndpoint,
    route: &RouteTarget,
    classification: &codex_routing::classifier::ClassifyResult,
    state: &RoutingState,
) -> Result<ResponseStream, FailureType> {
    let prompt_text = extract_last_message(prompt);

    info!(
        model = %endpoint.model,
        route = ?route,
        tools_potential = classification.tools_potential,
        reason = %classification.reason,
        "Routing to local model (free)"
    );

    let raw_messages = prompt_to_ollama_messages(prompt);
    let raw_system = extract_system_instructions(prompt);
    let model_name = endpoint.model.clone();
    let route_name = format!("{:?}", route);

    // Estimate pre-strip tokens — what the cloud model would have received.
    // This is the "savings" when we route locally instead.
    let pre_strip_text: String = raw_messages.iter()
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    let pre_strip_tokens = codex_routing::metrics::estimate_tokens(&pre_strip_text) as u64;
    state.usage.record_savings(pre_strip_tokens);

    // Strip context for local models
    let strip_level = match route {
        RouteTarget::LightReasoner => codex_routing::context_strip::StripLevel::Reasoner,
        _ => codex_routing::context_strip::StripLevel::Coder,
    };
    let stripped = codex_routing::context_strip::strip_context(
        &raw_messages,
        raw_system.as_deref(),
        strip_level,
    );
    info!(strip_summary = %stripped.strip_summary, "Context stripped for local model");
    let messages = stripped.messages;
    let system = stripped.system;

    // For LightCoder: pass ESSENTIAL tools only (shell, file ops).
    let use_tools = *route == RouteTarget::LightCoder && classification.tools_potential;
    if use_tools {
        let essential_tools = ["shell", "local_shell", "apply_patch",
            "read_file", "list_dir", "text_editor"];
        let tool_json: Vec<serde_json::Value> = prompt.tools.iter()
            .filter(|t| essential_tools.contains(&t.name()))
            .filter_map(|t| serde_json::to_value(t).ok())
            .collect();
        let ollama_tools = codex_routing::tool_format::to_ollama_tools(&tool_json);

        info!(
            tool_count = ollama_tools.len(),
            "Passing tools to local coder"
        );

        let result = state.pool.chat_with_tools(
            &endpoint.base_url,
            &endpoint.model,
            messages.clone(),
            system.as_deref(),
            endpoint.temperature,
            endpoint.num_ctx,
            None,
            endpoint.timeout_seconds,
            Some(ollama_tools),
        ).await;

        if let Some(body) = result {
            let message = body.get("message").cloned().unwrap_or_default();
            let content = message.get("content")
                .and_then(|c| c.as_str()).unwrap_or("").to_string();
            let native_tool_calls = message.get("tool_calls")
                .and_then(|tc| tc.as_array()).cloned().unwrap_or_default();
            let input_tokens = body.get("prompt_eval_count").and_then(|v| v.as_u64()).unwrap_or(0);
            let output_tokens = body.get("eval_count").and_then(|v| v.as_u64()).unwrap_or(0);

            info!(
                content_len = content.len(),
                native_tool_calls = native_tool_calls.len(),
                "Local coder response received"
            );

            // Record local usage for /stats
            state.usage.record(&model_name, input_tokens, output_tokens);

            return Ok(ollama_tool_response_to_stream(
                content,
                native_tool_calls,
                model_name.clone(),
                input_tokens,
                output_tokens,
            ));
        } else {
            warn!("Local coder with tools failed");
            return Err(FailureType::ModelUnavailable);
        }
    }

    // Streaming path (no tools — reasoner)
    let stream_rx = state.pool.chat_stream(
        &endpoint.base_url,
        &endpoint.model,
        messages,
        system.as_deref(),
        endpoint.temperature,
        endpoint.num_ctx,
        endpoint.timeout_seconds,
    ).await;

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
            content: vec![ContentItem::OutputText { text: String::new() }],
            end_turn: None,
            phase: None,
        };
        let _ = event_tx.send(Ok(ResponseEvent::OutputItemAdded(placeholder))).await;

        let mut full_text = String::new();
        let mut input_tokens = 0u64;
        let mut output_tokens = 0u64;

        while let Some(chunk) = ollama_rx.recv().await {
            match chunk {
                codex_routing::ollama::StreamChunk::Delta(text) => {
                    full_text.push_str(&text);
                    let _ = event_tx.send(Ok(ResponseEvent::OutputTextDelta(text))).await;
                }
                codex_routing::ollama::StreamChunk::Done { input_tokens: it, output_tokens: ot } => {
                    input_tokens = it;
                    output_tokens = ot;
                    break;
                }
            }
        }

        // Record local usage for /stats
        if let Some(state) = get_routing_state().await.as_ref() {
            state.usage.record(&model_for_usage, input_tokens, output_tokens);
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
            let _ = event_tx.send(Ok(ResponseEvent::OutputItemDone(final_message))).await;
        } else {
            if !recovered.content.is_empty() {
                let text_message = ResponseItem::Message {
                    id: Some("local_msg_0".to_string()),
                    role: "assistant".to_string(),
                    content: vec![ContentItem::OutputText { text: recovered.content }],
                    end_turn: None,
                    phase: None,
                };
                let _ = event_tx.send(Ok(ResponseEvent::OutputItemDone(text_message))).await;
            }

            for (i, tc) in recovered.tool_calls.iter().enumerate() {
                let call_id = tc.id.clone()
                    .unwrap_or_else(|| format!("local_call_{i}"));
                let arguments = serde_json::to_string(&tc.arguments)
                    .unwrap_or_else(|_| "{}".into());

                let func_call = ResponseItem::FunctionCall {
                    id: Some(format!("local_fc_{i}")),
                    name: tc.name.clone(),
                    namespace: None,
                    arguments,
                    call_id,
                };
                let _ = event_tx.send(Ok(ResponseEvent::OutputItemAdded(func_call.clone()))).await;
                let _ = event_tx.send(Ok(ResponseEvent::OutputItemDone(func_call))).await;
            }
        }

        let _ = event_tx.send(Ok(ResponseEvent::Completed {
            response_id: "local_response".to_string(),
            token_usage: Some(TokenUsage {
                input_tokens: input_tokens as i64,
                output_tokens: output_tokens as i64,
                ..Default::default()
            }),
        })).await;
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
    use codex_routing::project_config::ModelRole;

    let role = pc.get_model(role_name)?;
    match role {
        ModelRole::Single { model, .. } => Some(model.clone()),
        ModelRole::Weighted { entries } => {
            if entries.is_empty() {
                return None;
            }
            // Weighted random selection
            let total_weight: u32 = entries.iter().map(|e| e.weight).sum();
            if total_weight == 0 {
                return Some(entries[0].model.clone());
            }
            let mut pick = rand_u32() % total_weight;
            for entry in entries {
                if pick < entry.weight {
                    return Some(entry.model.clone());
                }
                pick -= entry.weight;
            }
            // Shouldn't reach here, but fallback to first
            Some(entries[0].model.clone())
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
        FailoverAction::RetrySame { wait, attempt: next_attempt } => {
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
                content: vec![ContentItem::OutputText { text: content.clone() }],
                end_turn: Some(native_tool_calls.is_empty()),
                phase: None,
            };
            let _ = tx.send(Ok(ResponseEvent::OutputItemAdded(text_msg.clone()))).await;
            let _ = tx.send(Ok(ResponseEvent::OutputItemDone(text_msg))).await;
        }

        // Emit native tool calls from Ollama
        for (i, tc) in native_tool_calls.iter().enumerate() {
            let func = tc.get("function").unwrap_or(tc);
            let name = func.get("name").and_then(|n| n.as_str()).unwrap_or("unknown").to_string();
            let call_id = tc.get("id").and_then(|id| id.as_str())
                .map(String::from)
                .unwrap_or_else(|| format!("local_call_{i}"));
            let arguments = func.get("arguments")
                .map(|a| {
                    if a.is_string() { a.as_str().unwrap_or("{}").to_string() }
                    else { serde_json::to_string(a).unwrap_or_else(|_| "{}".into()) }
                })
                .unwrap_or_else(|| "{}".into());

            let func_call = ResponseItem::FunctionCall {
                id: Some(format!("local_fc_{i}")),
                name,
                namespace: None,
                arguments,
                call_id,
            };
            let _ = tx.send(Ok(ResponseEvent::OutputItemAdded(func_call.clone()))).await;
            let _ = tx.send(Ok(ResponseEvent::OutputItemDone(func_call))).await;
        }

        // If no native tool calls, try recovering embedded ones from text
        if native_tool_calls.is_empty() && !content.is_empty() {
            let recovered = codex_routing::tool_recovery::recover_tool_calls(&content, false);
            for (i, tc) in recovered.tool_calls.iter().enumerate() {
                let call_id = tc.id.clone().unwrap_or_else(|| format!("local_call_{i}"));
                let arguments = serde_json::to_string(&tc.arguments).unwrap_or_else(|_| "{}".into());
                let func_call = ResponseItem::FunctionCall {
                    id: Some(format!("local_fc_{i}")),
                    name: tc.name.clone(),
                    namespace: None,
                    arguments,
                    call_id,
                };
                let _ = tx.send(Ok(ResponseEvent::OutputItemAdded(func_call.clone()))).await;
                let _ = tx.send(Ok(ResponseEvent::OutputItemDone(func_call))).await;
            }
        }

        let _ = tx.send(Ok(ResponseEvent::Completed {
            response_id: "local_response".to_string(),
            token_usage: Some(TokenUsage {
                input_tokens: input_tokens as i64,
                output_tokens: output_tokens as i64,
                ..Default::default()
            }),
        })).await;
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
                content: vec![ContentItem::OutputText { text: response.content }],
                end_turn: Some(true),
                phase: None,
            };
            let _ = tx.send(Ok(ResponseEvent::OutputItemAdded(message.clone()))).await;
            let _ = tx.send(Ok(ResponseEvent::OutputItemDone(message))).await;
        } else {
            // Has tool calls
            if !recovered.content.is_empty() {
                let text_msg = ResponseItem::Message {
                    id: Some("local_msg_0".to_string()),
                    role: "assistant".to_string(),
                    content: vec![ContentItem::OutputText { text: recovered.content }],
                    end_turn: None,
                    phase: None,
                };
                let _ = tx.send(Ok(ResponseEvent::OutputItemAdded(text_msg.clone()))).await;
                let _ = tx.send(Ok(ResponseEvent::OutputItemDone(text_msg))).await;
            }

            for (i, tc) in recovered.tool_calls.iter().enumerate() {
                let call_id = tc.id.clone().unwrap_or_else(|| format!("local_call_{i}"));
                let arguments = serde_json::to_string(&tc.arguments).unwrap_or_else(|_| "{}".into());

                let func_call = ResponseItem::FunctionCall {
                    id: Some(format!("local_fc_{i}")),
                    name: tc.name.clone(),
                    namespace: None,
                    arguments,
                    call_id,
                };
                let _ = tx.send(Ok(ResponseEvent::OutputItemAdded(func_call.clone()))).await;
                let _ = tx.send(Ok(ResponseEvent::OutputItemDone(func_call))).await;
            }
        }

        let _ = tx.send(Ok(ResponseEvent::Completed {
            response_id: "local_response".to_string(),
            token_usage: Some(TokenUsage {
                input_tokens: response.input_tokens as i64,
                output_tokens: response.output_tokens as i64,
                ..Default::default()
            }),
        })).await;
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
fn extract_system_instructions(prompt: &Prompt) -> Option<String> {
    let text = prompt.base_instructions.text.clone();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Convert prompt input items to Ollama message format.
fn prompt_to_ollama_messages(prompt: &Prompt) -> Vec<serde_json::Value> {
    let mut messages = Vec::new();

    for item in &prompt.input {
        if let ResponseItem::Message { role, content, .. } = item {
            let text: String = content
                .iter()
                .filter_map(|c| match c {
                    ContentItem::InputText { text } => Some(text.as_str()),
                    ContentItem::OutputText { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");

            if !text.is_empty() {
                messages.push(serde_json::json!({
                    "role": role,
                    "content": text,
                }));
            }
        }
    }

    messages
}
