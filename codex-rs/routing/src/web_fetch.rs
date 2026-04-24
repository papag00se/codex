//! HTTP fetch backend for the `web_fetch` tool.
//!
//! Single GET request with a browser-like User-Agent, a size cap, and a
//! timeout. Returns the response body as text when the content type is
//! textual; otherwise returns a short placeholder describing what was
//! received. No retries, no redirects beyond reqwest's default, no caching —
//! keep it small.

use crate::local_web_search::DEFAULT_USER_AGENT;

/// Maximum bytes read from a response body. Anything beyond is truncated and
/// a notice is appended. Sized to fit comfortably in a local model's context
/// without letting a single fetch dominate the transcript.
const MAX_BODY_BYTES: usize = 512 * 1024;

/// Per-request timeout. Matches the search tool's expectations — local
/// models shouldn't block for minutes on a single fetch.
const REQUEST_TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Clone)]
pub struct FetchResult {
    pub status: u16,
    pub final_url: String,
    pub content_type: Option<String>,
    pub body: String,
    pub truncated: bool,
}

#[derive(Debug)]
pub enum FetchError {
    InvalidUrl(String),
    Http(String),
    DecodeError(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidUrl(msg) => write!(f, "web_fetch: invalid URL: {msg}"),
            Self::Http(msg) => write!(f, "web_fetch HTTP error: {msg}"),
            Self::DecodeError(msg) => write!(f, "web_fetch decode error: {msg}"),
        }
    }
}

impl std::error::Error for FetchError {}

/// Build a human-readable description of a `reqwest::Error` that walks its
/// `source()` chain so the root cause (DNS lookup failure, TLS certificate
/// mismatch, connection refused, etc.) is visible in the error message we
/// surface to the model. Also tags the failure category (`connect`,
/// `timeout`, `redirect`, `body`) when reqwest can identify it.
fn describe_reqwest_error(err: &reqwest::Error) -> String {
    let mut kind_tags: Vec<&'static str> = Vec::new();
    if err.is_timeout() {
        kind_tags.push("timeout");
    }
    if err.is_connect() {
        kind_tags.push("connect");
    }
    if err.is_redirect() {
        kind_tags.push("redirect");
    }
    if err.is_body() {
        kind_tags.push("body");
    }
    if err.is_decode() {
        kind_tags.push("decode");
    }

    let mut parts: Vec<String> = Vec::new();
    parts.push(err.to_string());
    let mut src: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(err);
    let mut seen = 0usize;
    while let Some(cur) = src {
        let msg = cur.to_string();
        if !parts.iter().any(|p| p == &msg) {
            parts.push(msg);
        }
        seen += 1;
        if seen >= 5 {
            break;
        }
        src = std::error::Error::source(cur);
    }

    let chain = parts.join(" → ");
    if kind_tags.is_empty() {
        chain
    } else {
        format!("[{}] {chain}", kind_tags.join(","))
    }
}

/// Fetch `url` with a GET request. `user_agent` is sent verbatim if `Some`;
/// otherwise [`DEFAULT_USER_AGENT`] is used so ordinary websites see a
/// request that looks like a real browser.
pub async fn fetch(url: &str, user_agent: Option<&str>) -> Result<FetchResult, FetchError> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(FetchError::InvalidUrl("url must not be empty".to_string()));
    }
    let parsed = reqwest::Url::parse(trimmed).map_err(|e| FetchError::InvalidUrl(e.to_string()))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(FetchError::InvalidUrl(format!(
            "unsupported scheme '{}': only http and https are allowed",
            parsed.scheme()
        )));
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| FetchError::Http(describe_reqwest_error(&e)))?;

    let response = client
        .get(parsed.clone())
        .header("User-Agent", user_agent.unwrap_or(DEFAULT_USER_AGENT))
        .header(
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,application/json;q=0.8,*/*;q=0.7",
        )
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await
        .map_err(|e| FetchError::Http(describe_reqwest_error(&e)))?;

    let status = response.status().as_u16();
    let final_url = response.url().to_string();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let bytes = response
        .bytes()
        .await
        .map_err(|e| FetchError::Http(describe_reqwest_error(&e)))?;

    let truncated = bytes.len() > MAX_BODY_BYTES;
    let slice = if truncated {
        &bytes[..MAX_BODY_BYTES]
    } else {
        &bytes[..]
    };

    let body = if is_text_content_type(content_type.as_deref()) {
        String::from_utf8_lossy(slice).into_owned()
    } else {
        format!(
            "[non-text response: {} bytes, content-type={}]",
            bytes.len(),
            content_type.as_deref().unwrap_or("(none)")
        )
    };

    Ok(FetchResult {
        status,
        final_url,
        content_type,
        body,
        truncated,
    })
}

fn is_text_content_type(ct: Option<&str>) -> bool {
    let Some(ct) = ct else {
        // No Content-Type header: be optimistic and try to decode as text.
        return true;
    };
    let ct = ct.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    ct.starts_with("text/")
        || ct == "application/json"
        || ct == "application/xml"
        || ct == "application/xhtml+xml"
        || ct == "application/javascript"
        || ct.ends_with("+json")
        || ct.ends_with("+xml")
}

/// Render a `FetchResult` as a compact text block for a tool output.
pub fn format_result(query_url: &str, result: &FetchResult) -> String {
    let mut out = format!(
        "Fetched: {}\nStatus: {}\n",
        result.final_url, result.status,
    );
    if query_url != result.final_url {
        out.push_str(&format!("Requested: {query_url}\n"));
    }
    if let Some(ct) = &result.content_type {
        out.push_str(&format!("Content-Type: {ct}\n"));
    }
    if result.truncated {
        out.push_str(&format!(
            "Note: body truncated to first {} bytes.\n",
            MAX_BODY_BYTES,
        ));
    }
    out.push_str("\n---\n");
    out.push_str(&result.body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_url() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        assert!(matches!(
            rt.block_on(fetch("", None)),
            Err(FetchError::InvalidUrl(_))
        ));
        assert!(matches!(
            rt.block_on(fetch("   ", None)),
            Err(FetchError::InvalidUrl(_))
        ));
    }

    #[test]
    fn rejects_non_http_schemes() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        assert!(matches!(
            rt.block_on(fetch("file:///etc/passwd", None)),
            Err(FetchError::InvalidUrl(_))
        ));
        assert!(matches!(
            rt.block_on(fetch("ftp://example.com/foo", None)),
            Err(FetchError::InvalidUrl(_))
        ));
    }

    #[test]
    fn content_type_text_detection() {
        assert!(is_text_content_type(Some("text/html")));
        assert!(is_text_content_type(Some("text/html; charset=utf-8")));
        assert!(is_text_content_type(Some("application/json")));
        assert!(is_text_content_type(Some("application/ld+json")));
        assert!(is_text_content_type(Some("application/atom+xml")));
        assert!(is_text_content_type(None));
        assert!(!is_text_content_type(Some("image/png")));
        assert!(!is_text_content_type(Some("application/octet-stream")));
    }
}
