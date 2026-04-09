//! Ollama HTTP client for routing decisions.
//!
//! This is a minimal client that calls `/api/chat` for the router model.
//! It serializes requests per endpoint using a tokio Semaphore (matching
//! the Python reference's fcntl file locks).
//!
//! See docs/spec/routing-logic-reference.md.

use reqwest::Client;
use serde_json::Value as JsonValue;
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
}

impl OllamaClientPool {
    pub fn new() -> Self {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_default();
        Self {
            semaphores: Mutex::new(HashMap::new()),
            client,
        }
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

    /// Call Ollama's /api/chat endpoint with serialization.
    ///
    /// Returns the parsed JSON response, or None on error.
    pub async fn chat(
        &self,
        base_url: &str,
        model: &str,
        messages: Vec<JsonValue>,
        system: Option<&str>,
        temperature: f64,
        num_ctx: usize,
        response_format: Option<&str>,
        timeout_seconds: u64,
    ) -> Option<JsonValue> {
        let sem = self.semaphore_for(base_url).await;
        let _permit = sem.acquire().await.ok()?;

        let mut payload_messages = messages;
        if let Some(sys) = system {
            payload_messages.insert(
                0,
                serde_json::json!({"role": "system", "content": sys}),
            );
        }

        let mut payload = serde_json::json!({
            "model": model,
            "messages": payload_messages,
            "stream": false,
            "options": {
                "temperature": temperature,
                "num_ctx": num_ctx,
            },
        });

        if response_format == Some("json") {
            payload["format"] = serde_json::json!("json");
        }

        // Disable thinking for router calls
        payload["think"] = serde_json::json!(false);

        let url = format!("{}/api/chat", base_url.trim_end_matches('/'));
        let result = self
            .client
            .post(&url)
            .json(&payload)
            .timeout(Duration::from_secs(timeout_seconds))
            .send()
            .await;

        match result {
            Ok(resp) => match resp.json::<JsonValue>().await {
                Ok(body) => Some(body),
                Err(e) => {
                    warn!("Ollama response parse error for {url}: {e}");
                    None
                }
            },
            Err(e) => {
                warn!("Ollama request error for {url}: {e}");
                None
            }
        }
    }
}
