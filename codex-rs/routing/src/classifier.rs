//! LLM-based request classifier for per-request routing.
//!
//! Uses a local classifier model (e.g., qwen3.5-9b:iq4_xs on a 1080)
//! to decide where each request should go and whether tools are needed.
//!
//! See docs/spec/design-principles.md — the LLM makes the judgment call,
//! deterministic code handles the control flow.

use crate::config::{OllamaEndpoint, RoutingConfig};
use crate::ollama::OllamaClientPool;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

const CLASSIFIER_PROMPT: &str = "\
You are a REQUEST CLASSIFIER. You do NOT execute requests. You ONLY classify them.

Routes (cheapest first):
- light_reasoner: Free. Questions, explanations, yes/no, summaries.
- light_coder: Free with tools. Single file reads, grep, small single-line edits only.
- cloud_fast: Cheap cloud. Use for: ANY test-and-fix loop involving a single test file, single-file refactors, applying known patterns, rename across one file. This is the go-to for simple coding tasks that are too complex for the free local model.
- cloud_mini: Medium cloud. Use for: multi-file edits, integration/E2E/Playwright/browser tests, investigations spanning 2+ files, dependency changes.
- cloud_reasoner: Strong cloud. Code review, architecture, cross-file security analysis, complex planning.
- cloud_coder: Strongest (conserve). ONLY for: large-scale refactors, complex multi-step debugging across many files, tasks that failed on cheaper models.

CLASSIFY this request. Return ONLY JSON: {\"route\": \"...\", \"tools_potential\": true/false, \"reason\": \"...\"}

Available tools: ";

/// The result of classifying a request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifyResult {
    pub route: RouteTarget,
    pub tools_potential: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RouteTarget {
    #[serde(rename = "light_reasoner")]
    LightReasoner,
    #[serde(rename = "light_coder")]
    LightCoder,
    #[serde(rename = "cloud_fast")]
    CloudFast,
    #[serde(rename = "cloud_mini")]
    CloudMini,
    #[serde(rename = "cloud_reasoner")]
    CloudReasoner,
    #[serde(rename = "cloud_coder")]
    CloudCoder,
}

/// Raw JSON response from the classifier LLM.
#[derive(Deserialize)]
struct ClassifierResponse {
    route: String,
    tools_potential: Option<bool>,
    reason: Option<String>,
}

/// Classify a request using the local classifier LLM.
///
/// Calls the classifier model (cheap local LLM) to decide:
/// 1. Which model tier should handle this request
/// 2. Whether the model will likely need to call tools
///
/// Falls back to CloudCoder if the classifier is unreachable or returns garbage.
pub async fn classify_request(
    prompt_text: &str,
    tool_names: &[&str],
    recent_tool_call_count: usize,
    recent_turn_count: usize,
    config: &RoutingConfig,
    pool: &OllamaClientPool,
) -> ClassifyResult {
    let classifier_ep = &config.classifier;
    if !classifier_ep.enabled {
        return fallback("classifier disabled");
    }

    // Build the classifier prompt — minimal context, fast
    let tools_str = tool_names.join(", ");
    let user_content = format!(
        "{CLASSIFIER_PROMPT}{tools_str}\n\
         Recent context: {recent_tool_call_count} tool calls in last {recent_turn_count} turns\n\
         Request to classify: {prompt_text}",
    );

    // Short timeout for classifier — if it takes more than 10 seconds,
    // skip local routing and go to cloud. First call may be slow (cold model load)
    // but subsequent calls should be fast (<3s).
    let classify_future = pool.chat(
            &classifier_ep.base_url,
            &classifier_ep.model,
            vec![serde_json::json!({"role": "user", "content": user_content})],
            None,
            0.0, // Deterministic
            classifier_ep.num_ctx,
            Some("json"),
            10, // Hard 10s timeout for the Ollama HTTP call
        );

    let response = match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        classify_future,
    ).await {
        Ok(r) => r,
        Err(_) => {
            warn!("Classifier timed out (>10s), falling back to cloud_coder");
            return fallback("classifier timeout");
        }
    };

    let Some(body) = response else {
        warn!("Classifier LLM unreachable, falling back to cloud_coder");
        return fallback("classifier unreachable");
    };

    let content = body
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("");

    // Strip <think>...</think> tags that qwen3.5 models sometimes add
    let content = strip_think_tags(content);
    let content = content.trim();

    // Parse the JSON response
    let parsed: ClassifierResponse = match serde_json::from_str(content) {
        Ok(p) => p,
        Err(_) => {
            // Try to salvage: sometimes the model returns JSON but with extra fields
            // or returns a tool call instead of a classification
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(content) {
                if let Some(route) = v.get("route").and_then(|r| r.as_str()) {
                    ClassifierResponse {
                        route: route.to_string(),
                        tools_potential: v.get("tools_potential").and_then(|t| t.as_bool()),
                        reason: v.get("reason").and_then(|r| r.as_str()).map(String::from),
                    }
                } else {
                    warn!(
                        content = %&content[..content.len().min(200)],
                        "Classifier returned JSON without 'route' field, falling back"
                    );
                    return fallback("no route in response");
                }
            } else {
                warn!(
                    content = %&content[..content.len().min(200)],
                    "Classifier returned non-JSON, falling back"
                );
                return fallback("invalid classifier response");
            }
        }
    };

    let route = match parsed.route.as_str() {
        "light_reasoner" => RouteTarget::LightReasoner,
        "light_coder" => RouteTarget::LightCoder,
        "cloud_fast" => RouteTarget::CloudFast,
        "cloud_mini" => RouteTarget::CloudMini,
        "cloud_reasoner" => RouteTarget::CloudReasoner,
        "cloud_coder" => RouteTarget::CloudCoder,
        other => {
            warn!(route = %other, "Classifier returned unknown route, falling back to cloud_coder");
            RouteTarget::CloudCoder
        }
    };

    let result = ClassifyResult {
        route,
        tools_potential: parsed.tools_potential.unwrap_or(true),
        reason: parsed.reason.unwrap_or_default(),
    };

    info!(
        route = ?result.route,
        tools_potential = result.tools_potential,
        reason = %result.reason,
        "Request classified"
    );

    result
}

/// Strip `<think>...</think>` blocks from model output.
fn strip_think_tags(text: &str) -> String {
    let mut result = text.to_string();
    while let Some(start) = result.find("<think>") {
        if let Some(end) = result.find("</think>") {
            result = format!("{}{}", &result[..start], &result[end + 8..]);
        } else {
            result = result[..start].to_string();
            break;
        }
    }
    result
}

fn fallback(reason: &str) -> ClassifyResult {
    ClassifyResult {
        route: RouteTarget::CloudCoder,
        tools_potential: true,
        reason: reason.to_string(),
    }
}
