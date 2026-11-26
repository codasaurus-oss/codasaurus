//! Minimal JSON-RPC 2.0 client for remote MCP servers over Streamable HTTP.
//!
//! Deliberately narrow: only the methods needed to test and discover a server
//! (`initialize`, `tools/list`). No stdio/subprocess transport —
//! that would mean spawning binaries influenced indirectly by PR content.

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::sync::LazyLock;
use std::time::Duration;

/// Bound memory from a misbehaving or malicious server.
const MAX_RESPONSE_BYTES: usize = 256 * 1024;

static CLIENT: LazyLock<Option<reqwest::Client>> = LazyLock::new(|| build_client().ok());

fn build_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
}

fn append_limited(body: &mut Vec<u8>, chunk: &[u8], limit: usize) -> Result<()> {
    if chunk.len() > limit.saturating_sub(body.len()) {
        bail!("MCP response exceeded {limit} bytes");
    }
    body.extend_from_slice(chunk);
    Ok(())
}

async fn read_body_limited(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    let content_length = response.content_length();
    if content_length.is_some_and(|length| length > limit as u64) {
        bail!("MCP response exceeded {limit} bytes");
    }
    let capacity = content_length
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or(limit)
        .min(limit);
    let mut body = Vec::with_capacity(capacity);
    while let Some(chunk) = response
        .chunk()
        .await
        .context("reading MCP response body")?
    {
        append_limited(&mut body, &chunk, limit)?;
    }
    Ok(body)
}

fn sse_data(body: &str) -> Vec<String> {
    let mut events = Vec::new();
    let mut data = String::new();
    for line in body.lines() {
        if line.is_empty() {
            if !data.is_empty() {
                events.push(std::mem::take(&mut data));
            }
            continue;
        }
        if let Some(value) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(value.strip_prefix(' ').unwrap_or(value));
        }
    }
    if !data.is_empty() {
        events.push(data);
    }
    events
}

fn parse_rpc_response(content_type: &str, body: &[u8]) -> Result<Value> {
    let text = std::str::from_utf8(body).context("MCP response is not valid UTF-8")?;
    let is_sse = content_type
        .split(';')
        .next()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/event-stream"));
    let rpc = if is_sse {
        sse_data(text)
            .into_iter()
            .filter_map(|data| serde_json::from_str::<Value>(&data).ok())
            .find(|message| {
                message.get("id") == Some(&json!(1))
                    && (message.get("result").is_some() || message.get("error").is_some())
            })
            .context("no matching JSON-RPC response in MCP SSE stream")?
    } else {
        serde_json::from_str(text.trim()).context("invalid MCP JSON-RPC response")?
    };
    if let Some(error) = rpc.get("error") {
        bail!("MCP server error: {error}");
    }
    rpc.get("result")
        .cloned()
        .context("MCP JSON-RPC response is missing result")
}

async fn call_rpc(base_url: &str, api_key: &str, method: &str, params: Value) -> Result<Value> {
    crate::ssrf::validate_http_url_resolved(base_url, false)
        .await
        .map_err(anyhow::Error::msg)?;

    let client = CLIENT.as_ref().context("MCP HTTP client not available")?;
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });

    let mut req = client
        .post(base_url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .json(&body);
    if !api_key.is_empty() {
        req = req.bearer_auth(api_key);
    }

    let resp = req.send().await.context("MCP request failed")?;
    if !resp.status().is_success() {
        bail!("MCP server returned HTTP {}", resp.status());
    }
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();

    let bytes = read_body_limited(resp, MAX_RESPONSE_BYTES).await?;
    parse_rpc_response(&content_type, &bytes)
}

pub async fn initialize(base_url: &str, api_key: &str) -> Result<Value> {
    call_rpc(
        base_url,
        api_key,
        "initialize",
        json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "codasaurus", "version": env!("CARGO_PKG_VERSION") },
        }),
    )
    .await
}

pub async fn tools_list(base_url: &str, api_key: &str) -> Result<Vec<Value>> {
    let result = call_rpc(base_url, api_key, "tools/list", json!({})).await?;
    Ok(result
        .get("tools")
        .and_then(|t| t.as_array())
        .cloned()
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::Redirect;
    use axum::{routing::get, Router};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn sse_response_skips_notifications_before_the_rpc_result() {
        let body = b"data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\n\
                    data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"tools\":[]}}\n\n";

        let result = parse_rpc_response("text/event-stream", body).unwrap();

        assert_eq!(result, json!({ "tools": [] }));
    }

    #[test]
    fn sse_response_joins_multiline_data_fields() {
        let body = b"data: {\"jsonrpc\":\"2.0\",\"id\":1,\n\
                    data: \"result\":{\"ok\":true}}\n\n";

        let result = parse_rpc_response("text/event-stream", body).unwrap();

        assert_eq!(result, json!({ "ok": true }));
    }

    #[test]
    fn response_without_a_result_is_an_error() {
        let error =
            parse_rpc_response("application/json", br#"{"jsonrpc":"2.0","id":1}"#).unwrap_err();

        assert!(error.to_string().contains("missing result"), "{error}");
    }

    #[test]
    fn invalid_utf8_response_is_an_error() {
        let error = parse_rpc_response("application/json", b"\xff").unwrap_err();

        assert!(error.to_string().contains("UTF-8"), "{error}");
    }

    #[test]
    fn bounded_body_rejects_a_chunk_larger_than_the_remaining_capacity() {
        let mut body = Vec::new();

        append_limited(&mut body, b"abcd", 5).unwrap();
        let error = append_limited(&mut body, b"ef", 5).unwrap_err();

        assert!(error.to_string().contains("exceeded"), "{error}");
        assert_eq!(body, b"abcd");
    }

    #[tokio::test]
    async fn client_does_not_follow_redirects() {
        let target_hits = Arc::new(AtomicUsize::new(0));
        let hits = Arc::clone(&target_hits);
        let app = Router::new()
            .route(
                "/redirect",
                get(|| async { Redirect::temporary("/target") }),
            )
            .route(
                "/target",
                get(move || {
                    hits.fetch_add(1, Ordering::SeqCst);
                    async { "target" }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let response = build_client()
            .unwrap()
            .get(format!("http://{address}/redirect"))
            .send()
            .await
            .unwrap();
        server.abort();

        assert!(response.status().is_redirection());
        assert_eq!(target_hits.load(Ordering::SeqCst), 0);
    }
}
