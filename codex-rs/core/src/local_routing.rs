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
    pool: Arc<OllamaClientPool>,
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
                Some(RoutingState { config, pool })
            } else {
                info!("Per-request routing disabled — classifier LLM not reachable, all requests go to cloud");
                None
            }
        })
        .await
}

/// Try to route a request to a local Ollama model.
///
/// Returns `Some(ResponseStream)` if the request was handled locally (free).
/// Returns `None` if the request should go to the cloud provider.
///
/// This is called from ModelClientSession::stream() on every model API call.
pub(crate) async fn try_route_local(prompt: &Prompt) -> Option<ResponseStream> {
    let state = get_routing_state().await.as_ref()?;

    // Extract the last user message for classification (not the full history)
    let prompt_text = extract_last_message(prompt);
    if prompt_text.is_empty() {
        return None;
    }

    // Extract tool names (just names, not full schemas — saves context)
    let tool_names: Vec<&str> = prompt
        .tools
        .iter()
        .map(|t| t.name())
        .collect();

    // Count recent tool calls from conversation history
    let (tool_call_count, turn_count) = count_recent_activity(prompt);

    // Ask the classifier LLM where this request should go.
    // This is the judgment call — the LLM decides, not regex.
    let classification = classify_request(
        &prompt_text,
        &tool_names,
        tool_call_count,
        turn_count,
        &state.config,
        &state.pool,
    )
    .await;

    // Deterministic: pick the endpoint based on the classification
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
        // All cloud routes: return None, let the normal cloud path handle it
        RouteTarget::CloudFast
        | RouteTarget::CloudMini
        | RouteTarget::CloudReasoner
        | RouteTarget::CloudCoder => None,
    };

    let endpoint = endpoint?;

    info!(
        model = %endpoint.model,
        endpoint = %endpoint.base_url,
        route = ?classification.route,
        tools_potential = classification.tools_potential,
        reason = %classification.reason,
        "Routing to local model (free)"
    );

    // Build Ollama messages — strip tools if tools_potential is false
    // to save context window on the local model
    let messages = prompt_to_ollama_messages(prompt);
    let system = extract_system_instructions(prompt);

    let result = call_ollama_text(
        &state.pool,
        endpoint,
        messages,
        system.as_deref(),
    )
    .await;

    match result {
        Ok(response) => {
            info!(
                model = %response.model,
                input_tokens = response.input_tokens,
                output_tokens = response.output_tokens,
                "Local model response received"
            );
            Some(ollama_response_to_stream(response))
        }
        Err(e) => {
            warn!(error = %e, "Local model failed, falling back to cloud");
            None
        }
    }
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
