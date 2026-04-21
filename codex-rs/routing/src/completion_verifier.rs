//! Verify whether a local model's no-tool-call response actually completes
//! the user's task, or whether it's an "announcement-then-bail" — the model
//! says "now I'll do X" and then ends the turn without doing X.
//!
//! Asks the classifier endpoint (small, fast, already-warm) for a binary
//! judgment. The classifier is well-suited for this kind of structured
//! short-output decision.

use crate::OllamaClientPool;
use crate::config::OllamaEndpoint;
use serde::Deserialize;
use tracing::warn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionVerdict {
    /// The agent's response is a legitimate completion (task done, deferring
    /// appropriately to the user, or asking a clarifying question).
    Complete,
    /// The agent announced an action but didn't take it, or otherwise
    /// terminated the turn prematurely.
    Bail,
    /// Verifier was unreachable or returned unparseable output. Treat as
    /// Complete (don't intervene if we can't judge).
    Unclear,
}

const VERIFIER_SYSTEM_PROMPT: &str = "\
You audit AI coding agents to detect when they end a turn prematurely.\n\
Return JSON only with no markdown fencing.\n\
\n\
The agent COMPLETED the task if any of the following are true:\n\
- The agent describes a concrete result it produced or tested (e.g. \"created file X with Y content\")\n\
- The agent tells the user the task is done and explains the outcome\n\
- The agent asks the user a clarifying question that genuinely blocks progress\n\
- The agent reports an error or limitation that prevents proceeding\n\
\n\
The agent BAILED if any of the following are true:\n\
- The agent says it WILL do something but the message ends without showing the action was performed\n\
- The agent describes its plan/intent but no tool was actually invoked to execute it\n\
- The agent says \"now I'll X\" or \"let me X\" or \"next I'll X\" with a colon, then stops\n\
- The agent restates findings but doesn't apply them when applying them was the obvious next step\n\
\n\
Be lenient — if the agent's message reasonably could be a stopping point, mark COMPLETE.\n\
Only mark BAIL when the agent clearly announced action it did not perform.";

#[derive(Deserialize)]
struct VerifierResponse {
    verdict: String,
    #[serde(default)]
    #[allow(dead_code)]
    reason: String,
}

/// Ask the classifier whether the agent's final response is a real completion.
///
/// `user_message` is the most recent user request driving the current turn.
/// `agent_message` is the agent's final text output for this turn.
pub async fn verify_completion(
    user_message: &str,
    agent_message: &str,
    classifier: &OllamaEndpoint,
    pool: &OllamaClientPool,
) -> CompletionVerdict {
    if agent_message.trim().is_empty() {
        // Empty messages can't be judged; let upstream handle.
        return CompletionVerdict::Unclear;
    }

    let user_payload = serde_json::json!({
        "user_request": user_message,
        "agent_final_message": agent_message,
        "task": "Did the agent COMPLETE the task or BAIL? Reply with JSON only.",
        "schema": {"verdict": "complete | bail", "reason": "<one short sentence>"},
    });
    let user_payload_str = match serde_json::to_string(&user_payload) {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "failed to serialize completion verifier payload");
            return CompletionVerdict::Unclear;
        }
    };

    let response = pool
        .chat(
            &classifier.base_url,
            &classifier.model,
            vec![serde_json::json!({"role": "user", "content": user_payload_str})],
            Some(VERIFIER_SYSTEM_PROMPT),
            0.0,
            classifier.num_ctx,
            Some("json"),
            classifier.timeout_seconds,
        )
        .await;

    let Some(body) = response else {
        warn!("Completion verifier classifier unreachable");
        return CompletionVerdict::Unclear;
    };

    let content = body
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("");
    let stripped = crate::classifier::strip_think_tags(content);

    let parsed: Result<VerifierResponse, _> = serde_json::from_str(stripped.trim());
    match parsed {
        Ok(resp) => match resp.verdict.to_lowercase().as_str() {
            "complete" => CompletionVerdict::Complete,
            "bail" => CompletionVerdict::Bail,
            other => {
                warn!(verdict = %other, "verifier returned unknown verdict");
                CompletionVerdict::Unclear
            }
        },
        Err(e) => {
            warn!(error = %e, content = %&stripped[..stripped.len().min(200)], "verifier returned non-JSON");
            CompletionVerdict::Unclear
        }
    }
}

/// Build the continuation prompt to inject when verification flags a bail.
/// Appended as a `user`-role message before re-calling the local model.
pub fn continuation_prompt(prior_response: &str) -> String {
    format!(
        "[VERIFIER] Your previous response was: \"{}\"\n\n\
         You announced an action but did not actually take it. Either:\n\
         (a) Take the action you announced — invoke the tools needed to complete it.\n\
         (b) If the task is genuinely done, restate the concrete result you produced (which file, which test, which output).\n\
         (c) If you cannot proceed, explain WHY and what you need.\n\n\
         Do not just announce intent again. Act, or report concretely.",
        // Truncate to keep the prompt compact.
        prior_response.chars().take(400).collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_agent_message_returns_unclear() {
        // No live test of the network path; just verify the empty-input branch.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let endpoint = OllamaEndpoint {
            base_url: "http://127.0.0.1:9999".into(),
            model: "x".into(),
            num_ctx: 4096,
            temperature: 0.0,
            timeout_seconds: 5,
            enabled: true,
            think: false,
            tool_subset: crate::config::ToolSubset::Focused,
        };
        let pool = OllamaClientPool::new();
        let verdict = rt.block_on(verify_completion("hi", "", &endpoint, &pool));
        assert_eq!(verdict, CompletionVerdict::Unclear);
    }

    #[test]
    fn continuation_prompt_includes_prior_response_excerpt() {
        let prompt = continuation_prompt("Now I'll create the file:");
        assert!(prompt.contains("Now I'll create the file:"));
        assert!(prompt.contains("VERIFIER"));
        assert!(prompt.contains("Take the action"));
    }
}
