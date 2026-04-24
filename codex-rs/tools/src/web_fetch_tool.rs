//! Tool spec for `web_fetch` — single-GET HTTP fetch with a browser-like
//! User-Agent.
//!
//! Dispatched locally (no cloud round-trip). The handler lives in
//! `codex-core` (see `tools/handlers/web_fetch.rs`).

use crate::JsonSchema;
use crate::ResponsesApiTool;
use crate::ToolSpec;
use std::collections::BTreeMap;

pub const WEB_FETCH_TOOL_NAME: &str = "web_fetch";

pub fn create_web_fetch_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "url".to_string(),
            JsonSchema::string(Some(
                "Absolute http:// or https:// URL to fetch. file://, ftp:// and other schemes are rejected."
                    .to_string(),
            )),
        ),
        (
            "user_agent".to_string(),
            JsonSchema::string(Some(
                "Optional User-Agent header. If omitted, a current Brave-style desktop Chrome UA is used so ordinary websites respond as they would to a real browser."
                    .to_string(),
            )),
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: WEB_FETCH_TOOL_NAME.to_string(),
        description:
            "Fetch a web page over HTTP(S) and return the response body as text. Use this to read the content of a specific URL (e.g. documentation, a blog post, an API endpoint) without running curl in a shell. Text-like content types (HTML, JSON, XML, plain text) are returned verbatim up to a 512KB cap; binary responses are replaced with a placeholder."
                .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["url".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}
