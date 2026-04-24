//! Handler for the `web_fetch` tool — simple HTTP GET with a browser-like
//! User-Agent. Backend lives in the routing crate.

use codex_routing::web_fetch;
use serde::Deserialize;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;

pub struct WebFetchHandler;

#[derive(Deserialize)]
struct WebFetchArgs {
    url: String,
    #[serde(default)]
    user_agent: Option<String>,
}

impl ToolHandler for WebFetchHandler {
    type Output = FunctionToolOutput;

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation { payload, .. } = invocation;

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "web_fetch handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: WebFetchArgs = parse_arguments(&arguments)?;
        let url = args.url.trim();
        if url.is_empty() {
            return Err(FunctionCallError::RespondToModel(
                "web_fetch: url must not be empty".to_string(),
            ));
        }

        match web_fetch::fetch(url, args.user_agent.as_deref()).await {
            Ok(result) => Ok(FunctionToolOutput::from_text(
                web_fetch::format_result(url, &result),
                Some(true),
            )),
            Err(e) => Err(FunctionCallError::RespondToModel(e.to_string())),
        }
    }
}
