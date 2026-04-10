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
use codex_routing::config::RoutingConfig;
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

/// Get usage summary string. Returns None if routing is not active.
pub(crate) async fn usage_summary() -> Option<String> {
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
    CloudOverride(String),
    /// No routing — use the default cloud model.
    Default,
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

    match route {
        // --- Local routes: call Ollama directly ---
        RouteTarget::LightReasoner | RouteTarget::LightCoder => {
            let endpoint = match classification.route {
                RouteTarget::LightReasoner => {
                    if state.config.reasoner.enabled {
                        Some(&state.config.reasoner)
                    } else if state.config.reasoner_backup.enabled {
                        Some(&state.config.reasoner_backup)
                    } else {
                        None
                    }
                }
                RouteTarget::LightCoder => {
                    if state.config.light_coder.enabled {
                        Some(&state.config.light_coder)
                    } else {
                        None
                    }
                }
                _ => None,
            };

            let Some(endpoint) = endpoint else {
                return RouteResult::Default;
            };

            info!(
                model = %endpoint.model,
                route = ?classification.route,
                tools_potential = classification.tools_potential,
                reason = %classification.reason,
                "Routing to local model (free)"
            );

            let raw_messages = prompt_to_ollama_messages(prompt);
            let raw_system = extract_system_instructions(prompt);
            let model_name = endpoint.model.clone();
            let route_name = format!("{:?}", classification.route);

            // Strip context for local models — remove binary, truncate,
            // collapse polls, keep only recent turns.
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
            // Not all 97 tools — that would fill the entire context window.
            // For LightReasoner: no tools (saves context).
            let use_tools = route == RouteTarget::LightCoder && classification.tools_potential;
            if use_tools {
                // Only pass tools the local model can usefully call
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

                // Use non-streaming path with tools for now
                // (streaming + tools is complex — the model may emit tool calls mid-stream)
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

                    // Build response — handle both native tool_calls and embedded ones
                    return RouteResult::Local(ollama_tool_response_to_stream(
                        content,
                        native_tool_calls,
                        model_name.clone(),
                        input_tokens,
                        output_tokens,
                    ));
                } else {
                    warn!("Local coder with tools failed, falling back to cloud");
                    return RouteResult::Default;
                }
            }

            // G5: Try streaming from local model (no tools — reasoner path)
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
                warn!("Local model stream failed to start, falling back to cloud");
                return RouteResult::Default;
            };

            // Build the ResponseEvent stream with real-time deltas
            let prompt_text_owned = prompt_text.clone();
            let feedback = state.feedback.lock().ok().map(|_| ());  // Just check lock works
            let _ = feedback;

            // We need references to state in the spawn — clone what we need
            let usage_ref = &state.usage;
            let feedback_mutex = &state.feedback;

            // Can't move references into spawn — use a different approach.
            // Collect feedback data and record after the stream.
            let started = std::time::Instant::now();

            let (event_tx, event_rx) = mpsc::channel(64);
            let model_for_task = model_name.clone();
            let route_for_task = route_name.clone();

            tokio::spawn(async move {
                // Send Created + OutputItemAdded first
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

                // Check for tool calls in the response (local coder may emit them)
                let recovered = codex_routing::tool_recovery::recover_tool_calls(&full_text, false);

                if recovered.tool_calls.is_empty() {
                    // Pure text response — send as message
                    let final_message = ResponseItem::Message {
                        id: Some("local_msg_0".to_string()),
                        role: "assistant".to_string(),
                        content: vec![ContentItem::OutputText { text: full_text }],
                        end_turn: Some(true),
                        phase: None,
                    };
                    let _ = event_tx.send(Ok(ResponseEvent::OutputItemDone(final_message))).await;
                } else {
                    // Has tool calls — send text message then function calls
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

                    // Emit each tool call as a FunctionCall item
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

            RouteResult::Local(ResponseStream { rx_event: event_rx })
        }

        // --- Cloud routes: pick model from config (with weighted distribution) ---
        RouteTarget::CloudFast
        | RouteTarget::CloudMini
        | RouteTarget::CloudReasoner
        | RouteTarget::CloudCoder => {
            let role_name = match classification.route {
                RouteTarget::CloudFast => "cloud_fast",
                RouteTarget::CloudMini => "cloud_mini",
                RouteTarget::CloudReasoner => "cloud_reasoner",
                RouteTarget::CloudCoder => "cloud_coder",
                _ => unreachable!(),
            };

            let model_slug = pick_cloud_model(&state.project_config, role_name);

            match model_slug {
                Some(slug) => {
                    info!(
                        route = role_name,
                        model = %slug,
                        reason = %classification.reason,
                        "Routing to cloud model (override)"
                    );
                    RouteResult::CloudOverride(slug)
                }
                None => {
                    info!(
                        route = role_name,
                        reason = %classification.reason,
                        "No config for cloud route, using default model"
                    );
                    RouteResult::Default
                }
            }
        }
    }
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
