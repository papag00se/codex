//! Ollama HTTP client for routing decisions.
//!
//! This is a minimal client that calls `/api/chat` for the router model.
//! It serializes requests per endpoint using a tokio Semaphore (matching
//! the Python reference's fcntl file locks).
//!
//! See docs/spec/routing-logic-reference.md.

use crate::config::{ClientFlavor, OllamaEndpoint};
use reqwest::Client;
use serde_json::Value as JsonValue;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Semaphore};
use tracing::warn;

/// Per-endpoint semaphore to serialize Ollama requests.
/// Ollama struggles with concurrent requests — this was discovered
/// through testing in the coding-agent-router project.
#[derive(Default)]
pub struct OllamaClientPool {
    semaphores: Mutex<HashMap<String, Arc<Semaphore>>>,
    client: Client,
    /// Tracks the last model used on each endpoint URL.
    /// Warm models avoid 10-20s cold-load penalty.
    warm_models: Mutex<HashMap<String, String>>,
}

impl OllamaClientPool {
    pub fn new() -> Self {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_default();
        Self {
            semaphores: Mutex::new(HashMap::new()),
            warm_models: Mutex::new(HashMap::new()),
            client,
        }
    }

    /// Get the last model used on an endpoint (the "warm" model).
    /// Returns None if no model has been used on this endpoint yet.
    pub async fn warm_model(&self, base_url: &str) -> Option<String> {
        let map = self.warm_models.lock().await;
        map.get(base_url).cloned()
    }

    /// Record which model was just used on an endpoint.
    async fn record_warm_model(&self, base_url: &str, model: &str) {
        let mut map = self.warm_models.lock().await;
        map.insert(base_url.to_string(), model.to_string());
    }

    /// Access the underlying HTTP client (for health checks, etc.).
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Get (or create) the semaphore for a given base URL.
    async fn semaphore_for(&self, base_url: &str) -> Arc<Semaphore> {
        let mut map = self.semaphores.lock().await;
        map.entry(base_url.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(1)))
            .clone()
    }

    /// Call the endpoint's chat completion API. Wrapper around
    /// [`Self::chat_with_tools`] for callers that don't need tool calls.
    pub async fn chat(
        &self,
        endpoint: &OllamaEndpoint,
        messages: Vec<JsonValue>,
        system: Option<&str>,
        response_format: Option<&str>,
    ) -> Option<JsonValue> {
        self.chat_with_tools(endpoint, messages, system, response_format, None)
            .await
    }

    /// Call the endpoint's chat completion API with optional tools.
    ///
    /// Branches internally on [`OllamaEndpoint::flavor`] to build the right
    /// URL and payload shape (Ollama's `/api/chat` vs OpenAI's
    /// `/v1/chat/completions`). The returned `JsonValue` is always
    /// translated to the Ollama shape (`{message: {content, tool_calls?,
    /// thinking?}, prompt_eval_count, eval_count}`) so callers don't need
    /// to know which flavor was used.
    pub async fn chat_with_tools(
        &self,
        endpoint: &OllamaEndpoint,
        messages: Vec<JsonValue>,
        system: Option<&str>,
        response_format: Option<&str>,
        tools: Option<Vec<JsonValue>>,
    ) -> Option<JsonValue> {
        let sem = self.semaphore_for(&endpoint.base_url).await;
        let _permit = sem.acquire().await.ok()?;

        let mut payload_messages = messages;
        if let Some(sys) = system {
            payload_messages.insert(0, json!({"role": "system", "content": sys}));
        }

        let url = build_chat_url(&endpoint.base_url, endpoint.flavor);
        let payload =
            build_chat_payload(endpoint, payload_messages, response_format, tools.as_ref());

        let mut req = self.client.post(&url).json(&payload);
        if endpoint.timeout_seconds > 0 {
            req = req.timeout(Duration::from_secs(endpoint.timeout_seconds));
        }
        let result = req.send().await;

        match result {
            Ok(resp) => {
                let status = resp.status();
                let body_text = resp.text().await.unwrap_or_default();
                if !status.is_success() {
                    let snippet = body_text.chars().take(500).collect::<String>();
                    warn!(
                        url = %url,
                        status = %status,
                        body = %snippet,
                        "chat request returned non-success status"
                    );
                    return None;
                }
                match serde_json::from_str::<JsonValue>(&body_text) {
                    Ok(body) => {
                        // Some OpenAI-compat servers (and Ollama itself for
                        // some failure modes) return HTTP 200 with an
                        // `{"error": ...}` body instead of a real response.
                        // The translator would silently produce empty
                        // content, hiding the actual problem from the
                        // caller — surface it as a None so the
                        // try_local_model warn fires with the cause.
                        if let Some(err) = body.get("error") {
                            let snippet =
                                err.to_string().chars().take(500).collect::<String>();
                            warn!(
                                url = %url,
                                error = %snippet,
                                "chat response carried an error body — treating as failure"
                            );
                            return None;
                        }
                        self.record_warm_model(&endpoint.base_url, &endpoint.model)
                            .await;
                        Some(translate_response_to_ollama_shape(body, endpoint.flavor))
                    }
                    Err(e) => {
                        let snippet = body_text.chars().take(500).collect::<String>();
                        warn!(
                            url = %url,
                            error = %e,
                            body = %snippet,
                            "chat response parse error"
                        );
                        None
                    }
                }
            }
            Err(e) => {
                warn!("chat request error for {url}: {e}");
                None
            }
        }
    }

    /// Streaming chat — returns a receiver that yields content chunks as they arrive.
    /// Each chunk is a partial text delta. The final message includes token usage.
    ///
    /// Branches on [`OllamaEndpoint::flavor`]:
    /// - `Ollama`: NDJSON stream from `/api/chat`, one JSON object per line
    ///   with `{message: {content}, done, prompt_eval_count, eval_count}`.
    /// - `OpenAICompat`: Server-Sent Events from `/v1/chat/completions`,
    ///   each `data: <json>` line carrying `{choices: [{delta: {content}}]}`.
    ///   The terminator is `data: [DONE]`. We send `stream_options:
    ///   {include_usage: true}` so most servers emit a final usage chunk.
    pub async fn chat_stream(
        &self,
        endpoint: &OllamaEndpoint,
        messages: Vec<JsonValue>,
        system: Option<&str>,
    ) -> Option<tokio::sync::mpsc::Receiver<StreamChunk>> {
        let sem = self.semaphore_for(&endpoint.base_url).await;
        let _permit = sem.acquire().await.ok()?;

        let mut payload_messages = messages;
        if let Some(sys) = system {
            payload_messages.insert(0, json!({"role": "system", "content": sys}));
        }

        let payload = build_stream_payload(endpoint, payload_messages, None);
        let url = build_chat_url(&endpoint.base_url, endpoint.flavor);
        let mut req = self.client.post(&url).json(&payload);
        if endpoint.timeout_seconds > 0 {
            req = req.timeout(Duration::from_secs(endpoint.timeout_seconds));
        }
        let response = req.send().await.ok()?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let snippet = body.chars().take(500).collect::<String>();
            warn!(
                url = %url,
                status = %status,
                body = %snippet,
                "stream request returned non-success status"
            );
            return None;
        }

        self.record_warm_model(&endpoint.base_url, &endpoint.model)
            .await;

        let (tx, rx) = tokio::sync::mpsc::channel(64);

        match endpoint.flavor {
            ClientFlavor::Ollama => spawn_ollama_stream_reader(response, tx),
            ClientFlavor::OpenAICompat => spawn_openai_sse_reader(response, tx),
        }

        Some(rx)
    }

    /// Tool-aware streaming chat. Same as [`chat_stream`] but passes a
    /// `tools` array in the request, so the server can emit tool-call
    /// deltas via [`StreamChunk::ToolCallDelta`] during the stream.
    ///
    /// Dropping the returned receiver closes the HTTP connection, which
    /// signals the server to stop generating — this is how the rumination
    /// guard aborts in-flight inference.
    pub async fn chat_with_tools_stream(
        &self,
        endpoint: &OllamaEndpoint,
        messages: Vec<JsonValue>,
        system: Option<&str>,
        tools: Option<Vec<JsonValue>>,
    ) -> Option<tokio::sync::mpsc::Receiver<StreamChunk>> {
        let sem = self.semaphore_for(&endpoint.base_url).await;
        let _permit = sem.acquire().await.ok()?;

        let mut payload_messages = messages;
        if let Some(sys) = system {
            payload_messages.insert(0, json!({"role": "system", "content": sys}));
        }

        let payload = build_stream_payload(endpoint, payload_messages, tools.as_ref());
        let url = build_chat_url(&endpoint.base_url, endpoint.flavor);
        let mut req = self.client.post(&url).json(&payload);
        if endpoint.timeout_seconds > 0 {
            req = req.timeout(Duration::from_secs(endpoint.timeout_seconds));
        }
        let response = req.send().await.ok()?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let snippet = body.chars().take(500).collect::<String>();
            warn!(
                url = %url,
                status = %status,
                body = %snippet,
                "tool-stream request returned non-success status"
            );
            return None;
        }

        self.record_warm_model(&endpoint.base_url, &endpoint.model)
            .await;

        let (tx, rx) = tokio::sync::mpsc::channel(64);
        match endpoint.flavor {
            ClientFlavor::Ollama => spawn_ollama_stream_reader(response, tx),
            ClientFlavor::OpenAICompat => spawn_openai_sse_reader(response, tx),
        }
        Some(rx)
    }
}

/// Build the streaming-mode request payload for the given flavor. Mirrors
/// [`build_chat_payload`] but sets `stream: true` and adds OpenAI's
/// `stream_options: {include_usage: true}` so usage tokens arrive in the
/// final SSE chunk. `tools` — when `Some` — is the same function-calling
/// schema we pass on non-streaming calls.
fn build_stream_payload(
    endpoint: &OllamaEndpoint,
    messages: Vec<JsonValue>,
    tools: Option<&Vec<JsonValue>>,
) -> JsonValue {
    match endpoint.flavor {
        ClientFlavor::Ollama => {
            let mut options = json!({
                "temperature": endpoint.temperature,
                "num_ctx": endpoint.num_ctx,
            });
            if let Some(n) = endpoint.max_tokens {
                options["num_predict"] = json!(n);
            }
            let mut payload = json!({
                "model": &endpoint.model,
                "messages": messages,
                "stream": true,
                "options": options,
                "think": endpoint.think,
            });
            if let Some(t) = tools {
                payload["tools"] = json!(t);
            }
            payload
        }
        ClientFlavor::OpenAICompat => {
            let mut payload = json!({
                "model": &endpoint.model,
                "messages": messages,
                "stream": true,
                "temperature": endpoint.temperature,
                "stream_options": {"include_usage": true},
            });
            if let Some(n) = endpoint.max_tokens {
                payload["max_tokens"] = json!(n);
            }
            if let Some(t) = tools {
                payload["tools"] = json!(t);
            }
            payload
        }
    }
}

fn spawn_ollama_stream_reader(
    response: reqwest::Response,
    tx: tokio::sync::mpsc::Sender<StreamChunk>,
) {
    tokio::spawn(async move {
        use futures::StreamExt;
        let mut byte_stream = response.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk_result) = byte_stream.next().await {
            let Ok(bytes) = chunk_result else { break };
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            // Process complete lines (Ollama sends one JSON object per line)
            while let Some(newline_pos) = buffer.find('\n') {
                let line = buffer[..newline_pos].trim().to_string();
                buffer = buffer[newline_pos + 1..].to_string();

                if line.is_empty() {
                    continue;
                }

                let Ok(obj) = serde_json::from_str::<JsonValue>(&line) else {
                    continue;
                };

                let done = obj.get("done").and_then(|d| d.as_bool()).unwrap_or(false);
                let msg = obj.get("message");
                let content = msg
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_str())
                    .unwrap_or("");
                let thinking = msg
                    .and_then(|m| m.get("thinking"))
                    .and_then(|c| c.as_str())
                    .unwrap_or("");

                if !thinking.is_empty() {
                    let _ = tx
                        .send(StreamChunk::ReasoningDelta(thinking.to_string()))
                        .await;
                }
                if !content.is_empty() {
                    let _ = tx.send(StreamChunk::Delta(content.to_string())).await;
                }
                // Ollama emits any tool_calls atomically in the final chunk
                // (not as per-arg-char deltas like OpenAI SSE). Forward them
                // as one ToolCallDelta per call, with the full argument JSON.
                if let Some(tool_calls) = msg.and_then(|m| m.get("tool_calls")).and_then(|tc| tc.as_array()) {
                    for (index, call) in tool_calls.iter().enumerate() {
                        let func = call.get("function").unwrap_or(call);
                        let name = func
                            .get("name")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let args = func.get("arguments").map(|v| {
                            if let Some(s) = v.as_str() {
                                s.to_string()
                            } else {
                                v.to_string()
                            }
                        }).unwrap_or_default();
                        let _ = tx
                            .send(StreamChunk::ToolCallDelta {
                                index,
                                id: None,
                                name,
                                arguments_delta: args,
                            })
                            .await;
                    }
                }

                if done {
                    let input_tokens = obj
                        .get("prompt_eval_count")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let output_tokens =
                        obj.get("eval_count").and_then(|v| v.as_u64()).unwrap_or(0);
                    let _ = tx
                        .send(StreamChunk::Done {
                            input_tokens,
                            output_tokens,
                            reasoning_tokens: 0, // Ollama doesn't break out reasoning tokens
                        })
                        .await;
                    return;
                }
            }
        }
    });
}

fn spawn_openai_sse_reader(
    response: reqwest::Response,
    tx: tokio::sync::mpsc::Sender<StreamChunk>,
) {
    tokio::spawn(async move {
        use futures::StreamExt;
        let mut byte_stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut input_tokens: u64 = 0;
        let mut output_tokens: u64 = 0;
        let mut reasoning_tokens: u64 = 0;

        while let Some(chunk_result) = byte_stream.next().await {
            let Ok(bytes) = chunk_result else { break };
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            // SSE separates events by blank lines (`\n\n`), but individual
            // `data:` lines arrive any time. We process line-by-line and
            // ignore comments / non-data lines (`event:`, `id:`, `retry:`,
            // and SSE comments starting with `:`).
            while let Some(newline_pos) = buffer.find('\n') {
                let raw_line = buffer[..newline_pos].to_string();
                buffer = buffer[newline_pos + 1..].to_string();
                let line = raw_line.trim_end_matches('\r');

                if line.is_empty() || line.starts_with(':') {
                    continue;
                }
                let Some(payload) = line.strip_prefix("data:") else {
                    // Skip event:/id:/retry: and any other SSE meta lines.
                    continue;
                };
                let payload = payload.trim_start();

                if payload == "[DONE]" {
                    let _ = tx
                        .send(StreamChunk::Done {
                            input_tokens,
                            output_tokens,
                            reasoning_tokens,
                        })
                        .await;
                    return;
                }

                let Ok(obj) = serde_json::from_str::<JsonValue>(payload) else {
                    continue;
                };

                if let Some(usage) = obj.get("usage") {
                    if let Some(p) = usage.get("prompt_tokens").and_then(JsonValue::as_u64) {
                        input_tokens = p;
                    }
                    if let Some(c) = usage.get("completion_tokens").and_then(JsonValue::as_u64) {
                        output_tokens = c;
                    }
                    // OpenAI-compat servers report reasoning tokens under
                    // `usage.completion_tokens_details.reasoning_tokens` (LM
                    // Studio follows this). Critical for the rumination
                    // detector's budget gate.
                    if let Some(r) = usage
                        .get("completion_tokens_details")
                        .and_then(|d| d.get("reasoning_tokens"))
                        .and_then(JsonValue::as_u64)
                    {
                        reasoning_tokens = r;
                    }
                }

                let delta = obj
                    .get("choices")
                    .and_then(|c| c.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|first| first.get("delta"));

                let delta_content = delta
                    .and_then(|d| d.get("content"))
                    .and_then(|c| c.as_str())
                    .unwrap_or("");
                let delta_reasoning = delta
                    .and_then(|d| d.get("reasoning_content"))
                    .and_then(|c| c.as_str())
                    .unwrap_or("");

                if !delta_reasoning.is_empty() {
                    let _ = tx
                        .send(StreamChunk::ReasoningDelta(delta_reasoning.to_string()))
                        .await;
                }
                if !delta_content.is_empty() {
                    let _ = tx
                        .send(StreamChunk::Delta(delta_content.to_string()))
                        .await;
                }

                // Tool-call deltas: each chunk carries zero or more entries
                // in `delta.tool_calls[]`. The FIRST chunk for a given
                // `index` carries `id` and `function.name`; subsequent
                // chunks carry incremental `function.arguments` string
                // fragments that the caller concatenates. We forward them
                // verbatim and let the caller accumulate.
                if let Some(tool_calls) = delta
                    .and_then(|d| d.get("tool_calls"))
                    .and_then(|tc| tc.as_array())
                {
                    for tc in tool_calls {
                        let index = tc
                            .get("index")
                            .and_then(JsonValue::as_u64)
                            .unwrap_or(0) as usize;
                        let id = tc
                            .get("id")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let func = tc.get("function");
                        let name = func
                            .and_then(|f| f.get("name"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let arguments_delta = func
                            .and_then(|f| f.get("arguments"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let _ = tx
                            .send(StreamChunk::ToolCallDelta {
                                index,
                                id,
                                name,
                                arguments_delta,
                            })
                            .await;
                    }
                }
            }
        }

        // Stream ended without seeing [DONE] — still flush a Done event with
        // whatever usage we accumulated (often zero) so the consumer can
        // finalize.
        let _ = tx
            .send(StreamChunk::Done {
                input_tokens,
                output_tokens,
                reasoning_tokens,
            })
            .await;
    });
}

/// A chunk from a streaming chat response. Unified across backends —
/// OpenAI-compat SSE and Ollama NDJSON both surface as this enum.
///
/// Richer than a plain "text delta" because rumination detection needs to
/// distinguish between `reasoning_content` (private thinking the user
/// never sees) and `content` (the final answer text), and because tool-aware
/// calls need to assemble `tool_calls` from multi-chunk `arguments` deltas.
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// Partial `message.content` / `choices[0].delta.content` — the
    /// user-visible assistant answer.
    Delta(String),
    /// Partial reasoning content — `choices[0].delta.reasoning_content`
    /// (OpenAI-compat) or `message.thinking` (Ollama). Kept separate from
    /// `Delta` so watchers can run rumination detection against it
    /// without having to distinguish channels by parsing `<think>` tags.
    ReasoningDelta(String),
    /// Partial tool-call information. OpenAI-compat streams tool_calls as
    /// incremental deltas keyed by `index` — the first chunk for a given
    /// index carries `id` and `name`, subsequent chunks extend the JSON
    /// string in `arguments_delta`. Callers accumulate per-index.
    ToolCallDelta {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments_delta: String,
    },
    /// Stream is complete, with final token usage. `reasoning_tokens`
    /// surfaces the server's reasoning-channel budget consumption when
    /// available (OpenAI-compat `usage.completion_tokens_details
    /// .reasoning_tokens`); 0 if the server didn't report it.
    Done {
        input_tokens: u64,
        output_tokens: u64,
        reasoning_tokens: u64,
    },
}

// ---------------------------------------------------------------------------
// Flavor-aware URL / payload / response translation.
// ---------------------------------------------------------------------------

/// Build the chat endpoint URL for a base URL + flavor. Defensively strips
/// a trailing `/` and — for OpenAI-compat — a trailing `/v1`, so users who
/// write `http://host:1234`, `http://host:1234/`, or `http://host:1234/v1`
/// all end up at `http://host:1234/v1/chat/completions`.
pub(crate) fn build_chat_url(base_url: &str, flavor: ClientFlavor) -> String {
    let base = base_url.trim_end_matches('/');
    match flavor {
        ClientFlavor::Ollama => format!("{base}/api/chat"),
        ClientFlavor::OpenAICompat => {
            let base = base.strip_suffix("/v1").unwrap_or(base);
            format!("{base}/v1/chat/completions")
        }
    }
}

/// Build the request payload for a chat call, branching on flavor.
pub(crate) fn build_chat_payload(
    endpoint: &OllamaEndpoint,
    messages: Vec<JsonValue>,
    response_format: Option<&str>,
    tools: Option<&Vec<JsonValue>>,
) -> JsonValue {
    match endpoint.flavor {
        ClientFlavor::Ollama => {
            let mut options = json!({
                "temperature": endpoint.temperature,
                "num_ctx": endpoint.num_ctx,
            });
            if let Some(n) = endpoint.max_tokens {
                // Ollama's equivalent of `max_tokens` is `num_predict`.
                options["num_predict"] = json!(n);
            }
            let mut payload = json!({
                "model": &endpoint.model,
                "messages": messages,
                "stream": false,
                "options": options,
                "think": endpoint.think,
            });
            if response_format == Some("json") {
                payload["format"] = json!("json");
            }
            if let Some(tools) = tools {
                payload["tools"] = json!(tools);
            }
            payload
        }
        ClientFlavor::OpenAICompat => {
            // OpenAI puts `temperature` at the top level. `num_ctx` has no
            // direct equivalent — the model's context window is fixed on
            // the server side. `think` is Ollama-specific and is silently
            // dropped. `response_format: json` is intentionally NOT
            // forwarded — LM Studio (and some other OpenAI-compat servers)
            // reject the older `{"type": "json_object"}` shape, accepting
            // only `"text"` or `"json_schema"` (the latter requires a
            // real schema that we don't carry). Relying on the caller's
            // system prompt asking for "JSON only" instead; this is how
            // the coder's own tool-call path already works.
            let _ = response_format; // consumed intentionally; see above
            let mut payload = json!({
                "model": &endpoint.model,
                "messages": messages,
                "stream": false,
                "temperature": endpoint.temperature,
            });
            if let Some(n) = endpoint.max_tokens {
                payload["max_tokens"] = json!(n);
            }
            if let Some(tools) = tools {
                payload["tools"] = json!(tools);
            }
            payload
        }
    }
}

/// Translate a chat response into the Ollama shape so callers have a
/// uniform surface (`body.message.content`, `body.message.tool_calls`,
/// `body.message.thinking`, `body.prompt_eval_count`, `body.eval_count`)
/// regardless of flavor. Ollama responses are passed through unchanged;
/// OpenAI responses are rewritten.
pub(crate) fn translate_response_to_ollama_shape(
    body: JsonValue,
    flavor: ClientFlavor,
) -> JsonValue {
    match flavor {
        ClientFlavor::Ollama => body,
        ClientFlavor::OpenAICompat => openai_response_to_ollama(body),
    }
}

fn openai_response_to_ollama(body: JsonValue) -> JsonValue {
    // Extract the first choice's message, if any.
    let message = body
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|first| first.get("message"))
        .cloned()
        .unwrap_or_else(|| json!({"role": "assistant", "content": ""}));

    // Ensure we have at minimum a content string (null → empty) and pass
    // through any tool_calls verbatim (OpenAI's tool_calls shape matches
    // Ollama's well enough that downstream parsers accept either).
    let mut message_out = json!({});
    message_out["role"] = message
        .get("role")
        .cloned()
        .unwrap_or_else(|| json!("assistant"));
    message_out["content"] = match message.get("content") {
        Some(JsonValue::Null) | None => json!(""),
        Some(v) => v.clone(),
    };
    if let Some(tool_calls) = message.get("tool_calls") {
        message_out["tool_calls"] = tool_calls.clone();
    }
    // Some OpenAI-compat servers (including LM Studio's newer builds and
    // vLLM with reasoning models) expose the thinking trace on a separate
    // field. Preserve it under the same name Ollama uses.
    if let Some(reasoning) = message.get("reasoning") {
        message_out["thinking"] = reasoning.clone();
    } else if let Some(reasoning_content) = message.get("reasoning_content") {
        message_out["thinking"] = reasoning_content.clone();
    }

    let usage = body.get("usage").cloned().unwrap_or(JsonValue::Null);
    let prompt_tokens = usage
        .get("prompt_tokens")
        .and_then(JsonValue::as_u64)
        .unwrap_or(0);
    let completion_tokens = usage
        .get("completion_tokens")
        .and_then(JsonValue::as_u64)
        .unwrap_or(0);

    json!({
        "message": message_out,
        "prompt_eval_count": prompt_tokens,
        "eval_count": completion_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ClientFlavor, OllamaEndpoint, ToolSubset};

    fn endpoint(flavor: ClientFlavor) -> OllamaEndpoint {
        OllamaEndpoint {
            base_url: "http://host:1234".to_string(),
            model: "m".to_string(),
            num_ctx: 2048,
            temperature: 0.1,
            timeout_seconds: 10,
            enabled: true,
            think: true,
            tool_subset: ToolSubset::Focused,
            flavor,
            max_tokens: None,
        }
    }

    #[test]
    fn build_chat_url_ollama() {
        assert_eq!(
            build_chat_url("http://host:11434", ClientFlavor::Ollama),
            "http://host:11434/api/chat"
        );
    }

    #[test]
    fn build_chat_url_ollama_strips_trailing_slash() {
        assert_eq!(
            build_chat_url("http://host:11434/", ClientFlavor::Ollama),
            "http://host:11434/api/chat"
        );
    }

    #[test]
    fn build_chat_url_openai_compat() {
        assert_eq!(
            build_chat_url("http://host:1234", ClientFlavor::OpenAICompat),
            "http://host:1234/v1/chat/completions"
        );
    }

    #[test]
    fn build_chat_url_openai_compat_strips_trailing_v1() {
        assert_eq!(
            build_chat_url("http://host:1234/v1", ClientFlavor::OpenAICompat),
            "http://host:1234/v1/chat/completions"
        );
        assert_eq!(
            build_chat_url("http://host:1234/v1/", ClientFlavor::OpenAICompat),
            "http://host:1234/v1/chat/completions"
        );
    }

    #[test]
    fn ollama_payload_has_options_and_think() {
        let ep = endpoint(ClientFlavor::Ollama);
        let payload = build_chat_payload(&ep, vec![json!({"role":"user","content":"hi"})], None, None);
        assert_eq!(payload["model"], "m");
        assert_eq!(payload["options"]["num_ctx"], 2048);
        assert_eq!(payload["think"], true);
    }

    #[test]
    fn openai_payload_flat_temp_no_think_no_num_ctx() {
        let ep = endpoint(ClientFlavor::OpenAICompat);
        let payload = build_chat_payload(&ep, vec![json!({"role":"user","content":"hi"})], None, None);
        assert_eq!(payload["model"], "m");
        assert_eq!(payload["temperature"], 0.1);
        assert!(payload.get("options").is_none());
        assert!(payload.get("think").is_none());
        assert!(payload.get("num_ctx").is_none());
    }

    #[test]
    fn openai_payload_includes_max_tokens_when_set() {
        let mut ep = endpoint(ClientFlavor::OpenAICompat);
        ep.max_tokens = Some(8000);
        let payload = build_chat_payload(&ep, vec![], None, None);
        assert_eq!(payload["max_tokens"], 8000);
    }

    #[test]
    fn openai_payload_omits_max_tokens_when_unset() {
        let ep = endpoint(ClientFlavor::OpenAICompat);
        let payload = build_chat_payload(&ep, vec![], None, None);
        assert!(payload.get("max_tokens").is_none());
    }

    #[test]
    fn ollama_payload_uses_num_predict_for_max_tokens() {
        let mut ep = endpoint(ClientFlavor::Ollama);
        ep.max_tokens = Some(8000);
        let payload = build_chat_payload(&ep, vec![], None, None);
        assert_eq!(payload["options"]["num_predict"], 8000);
        assert!(payload.get("max_tokens").is_none()); // not top-level for Ollama
    }

    #[test]
    fn openai_payload_json_response_format_is_not_forwarded() {
        // LM Studio rejects `{type: "json_object"}`; we rely on the caller's
        // system prompt to enforce JSON output. Assert we don't emit either
        // the OpenAI field or Ollama's `format` field.
        let ep = endpoint(ClientFlavor::OpenAICompat);
        let payload = build_chat_payload(&ep, vec![], Some("json"), None);
        assert!(payload.get("response_format").is_none());
        assert!(payload.get("format").is_none());
    }

    #[test]
    fn ollama_payload_json_response_format_is_format_field() {
        let ep = endpoint(ClientFlavor::Ollama);
        let payload = build_chat_payload(&ep, vec![], Some("json"), None);
        assert_eq!(payload["format"], "json");
    }

    #[test]
    fn openai_response_translates_to_ollama_shape() {
        let openai_body = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "hello",
                    "tool_calls": [{"id": "1", "type": "function", "function": {"name": "x", "arguments": "{}"}}],
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 42,
                "completion_tokens": 7,
                "total_tokens": 49
            }
        });
        let translated = translate_response_to_ollama_shape(openai_body, ClientFlavor::OpenAICompat);
        assert_eq!(translated["message"]["content"], "hello");
        assert_eq!(
            translated["message"]["tool_calls"][0]["function"]["name"],
            "x"
        );
        assert_eq!(translated["prompt_eval_count"], 42);
        assert_eq!(translated["eval_count"], 7);
    }

    #[test]
    fn openai_response_null_content_becomes_empty_string() {
        let openai_body = json!({
            "choices": [{"message": {"role": "assistant", "content": null}}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 0}
        });
        let translated = translate_response_to_ollama_shape(openai_body, ClientFlavor::OpenAICompat);
        assert_eq!(translated["message"]["content"], "");
    }

    #[test]
    fn openai_response_reasoning_mapped_to_thinking() {
        let openai_body = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "answer",
                    "reasoning": "let me think..."
                }
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5}
        });
        let translated = translate_response_to_ollama_shape(openai_body, ClientFlavor::OpenAICompat);
        assert_eq!(translated["message"]["thinking"], "let me think...");
    }

    #[test]
    fn openai_error_body_detection() {
        // We don't actually exercise the dispatcher here (would need a
        // mock HTTP server); instead, prove the structural check we rely
        // on inside chat_with_tools recognizes OpenAI error shapes.
        let body: JsonValue = serde_json::from_str(
            r#"{"error":{"message":"No models loaded","type":"invalid_request_error","param":"model"}}"#,
        )
        .unwrap();
        assert!(body.get("error").is_some());
        assert!(body.get("choices").is_none());
    }

    #[test]
    fn ollama_error_body_detection() {
        // Ollama's error shape is `{"error": "<string>"}`. Same top-level
        // `error` field, so the same check triggers.
        let body: JsonValue = serde_json::from_str(
            r#"{"error":"model 'qwopus-q6-think' not found, try pulling it first"}"#,
        )
        .unwrap();
        assert!(body.get("error").is_some());
        assert!(body.get("message").is_none());
    }

    #[test]
    fn ollama_response_passed_through_unchanged() {
        let body = json!({
            "message": {"role": "assistant", "content": "x"},
            "prompt_eval_count": 1,
            "eval_count": 2
        });
        let translated = translate_response_to_ollama_shape(body.clone(), ClientFlavor::Ollama);
        assert_eq!(translated, body);
    }
}
