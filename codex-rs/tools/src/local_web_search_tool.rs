//! Tool spec for `local_web_search` — Brave Search backend.
//!
//! This is a separate tool from OpenAI's built-in `web_search` because it
//! is dispatched locally (no cloud round-trip) and only requires a
//! Brave API key. The handler lives in `codex-core` (see
//! `tools/handlers/local_web_search.rs`).

use crate::JsonSchema;
use crate::ResponsesApiTool;
use crate::ToolSpec;
use std::collections::BTreeMap;

pub const LOCAL_WEB_SEARCH_TOOL_NAME: &str = "local_web_search";

pub fn create_local_web_search_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "query".to_string(),
            JsonSchema::string(Some(
                "The search query. Use natural language; the search engine handles synonyms and ranking."
                    .to_string(),
            )),
        ),
        (
            "count".to_string(),
            JsonSchema::number(Some(
                "Number of results to return, between 1 and 20. Defaults to 10 if omitted."
                    .to_string(),
            )),
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: LOCAL_WEB_SEARCH_TOOL_NAME.to_string(),
        description:
            "Search the web for up-to-date information using the Brave Search API. Returns a ranked list of titles, URLs, and short descriptions. Use this for any question whose answer may have changed since model training or that requires looking up specific external facts."
                .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["query".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}
