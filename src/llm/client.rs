//! The only HTTP transport for chat completions: one URL build, one retry and
//! telemetry path, and one status/parse mapping shared by the structured review
//! and every plain-text helper.

use super::config::{assert_endpoint_safe, LlmConfig};
use super::schema::review_response_format;
use anyhow::{bail, Result};
use serde_json::json;
use std::sync::LazyLock;
use std::time::Duration;

static LLM_CLIENT: LazyLock<Option<reqwest::Client>> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(crate::env::u64(
            "CODASAURUS_LLM_TIMEOUT_SECS",
            60,
        )))
        .connect_timeout(Duration::from_secs(crate::env::u64(
            "CODASAURUS_LLM_CONNECT_TIMEOUT_SECS",
            10,
        )))
        // Redirects can change the destination after the SSRF check and can
        // also forward a bearer token to an unexpected origin. LiteLLM's
        // canonical endpoint must be configured explicitly.
        .redirect(reqwest::redirect::Policy::none())
        .pool_max_idle_per_host(4)
        .build()
        .ok()
});

const MAX_CHAT_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

pub(crate) fn llm_client() -> Result<&'static reqwest::Client> {
    LLM_CLIENT
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("LLM HTTP client failed to initialize"))
}

/// Shared HTTP client for other LLM-adjacent modules (e.g. `index::embed`) that
/// need the same timeout/pooling behavior without standing up a second client.
pub fn shared_client() -> Result<&'static reqwest::Client> {
    llm_client()
}

/// Body for one structured review request. Text helpers build their own (no
/// `response_format`).
pub(crate) fn chat_body(
    config: &LlmConfig,
    model: &str,
    messages: &[serde_json::Value],
    max_tokens: u32,
) -> serde_json::Value {
    json!({
        "model": model,
        "messages": messages,
        "response_format": review_response_format(),
        "max_tokens": max_tokens,
        "temperature": config.temperature
    })
}

fn text_body(
    model: &str,
    system_prompt: &str,
    user_prompt: &str,
    max_tokens: u32,
) -> serde_json::Value {
    json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": user_prompt}
        ],
        "max_tokens": max_tokens,
        "temperature": 0.2
    })
}

/// POST one body to `{base_url}/chat/completions`.
///
/// LiteLLM owns provider retries, fallbacks, cooldowns, and rate-limit handling.
/// Codasaurus therefore sends each inference request once: retrying a completion
/// here can duplicate a billable or stateful provider call. Responses are
/// bounded before JSON decoding and unknown LiteLLM fields are ignored.
pub(crate) async fn post_chat(
    client: &reqwest::Client,
    config: &LlmConfig,
    model: &str,
    body: &serde_json::Value,
    op: &str,
    prompt_chars: usize,
    strong: bool,
) -> Result<serde_json::Value> {
    assert_endpoint_safe(config).await?;

    let pool = crate::bot::CONFIG_POOL.get();
    let started = std::time::Instant::now();
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let max_tokens = body["max_tokens"]
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(0);

    let mut request = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(body);
    if !config.api_key.is_empty() {
        request = request.bearer_auth(&config.api_key);
    }
    let resp = request.send().await;

    let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let outcome = if resp.is_ok() { "ok" } else { "error" };
    if let Some(pool) = pool {
        crate::db::events::emit_llm_call(
            pool,
            op,
            model,
            prompt_chars,
            max_tokens,
            strong,
            latency_ms,
            outcome,
        )
        .await;
    }

    let resp = match resp {
        Ok(resp) => resp,
        Err(error) => {
            crate::metrics::record_llm_error();
            return Err(error.into());
        }
    };
    let status = resp.status();
    if !status.is_success() {
        crate::metrics::record_llm_error();
        let error_text = bounded_response_text(resp, 16 * 1024).await;
        bail!("LLM API returned {status}: {error_text}");
    }

    let bytes = bounded_response_bytes(resp, MAX_CHAT_RESPONSE_BYTES).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

async fn bounded_response_bytes(response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        bail!("LLM response exceeded {limit} bytes");
    }
    let bytes = response.bytes().await?;
    if bytes.len() > limit {
        bail!("LLM response exceeded {limit} bytes");
    }
    Ok(bytes.to_vec())
}

async fn bounded_response_text(response: reqwest::Response, limit: usize) -> String {
    match bounded_response_bytes(response, limit).await {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(error) => format!("response body unavailable: {error}"),
    }
}

/// Plain-text chat completion on the configured alias.
pub(crate) async fn chat_text(
    client: &reqwest::Client,
    config: &LlmConfig,
    system_prompt: &str,
    user_prompt: &str,
    max_tokens: u32,
) -> Result<String> {
    let prompt_chars = system_prompt.len() + user_prompt.len();
    let model = config.effective_text_model();
    let body = text_body(model, system_prompt, user_prompt, max_tokens);
    let resp = post_chat(
        client,
        config,
        model,
        &body,
        "chat_text",
        prompt_chars,
        false,
    )
    .await?;

    match text_from_response(&resp) {
        TextOutcome::Text(text) => Ok(text),
        TextOutcome::Empty => Err(anyhow::anyhow!("LLM response missing content")),
    }
}

enum TextOutcome {
    Text(String),
    Empty,
}

fn text_from_response(resp: &serde_json::Value) -> TextOutcome {
    let choice = &resp["choices"][0];
    let Some(text) = choice["message"]["content"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        crate::metrics::record_llm_error();
        return TextOutcome::Empty;
    };

    let mut out = text.to_string();
    if choice["finish_reason"].as_str() == Some("length") {
        // Stop at the last complete sentence so a capped reply never ends mid-word.
        out = truncate_to_sentence(&out).to_string();
        out.push_str("\n\n_(reply reached the model token limit; raise the model budget or use a non-reasoning model)_");
    }
    TextOutcome::Text(out)
}

/// Longest prefix of `text` that ends on a sentence boundary.
pub(crate) fn truncate_to_sentence(text: &str) -> &str {
    let mut cut = text.len();
    for (i, c) in text.char_indices().rev() {
        if matches!(c, '.' | '!' | '?') {
            let rest = &text[i + c.len_utf8()..];
            if rest.is_empty() || rest.chars().next().is_some_and(char::is_whitespace) {
                cut = i + c.len_utf8();
                break;
            }
        }
    }
    &text[..cut]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> LlmConfig {
        LlmConfig {
            api_key: "test-key".into(),
            model: "my-review-alias".into(),
            text_model: "my-cheap-alias".into(),
            max_tokens: 2048,
            temperature: 0.1,
            base_url: "http://litellm:4000/v1".into(),
            embedding_model: "my-embedding-alias".into(),
        }
    }

    #[test]
    fn chat_body_sends_strict_schema_and_no_reasoning() {
        let messages = vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "hi"}),
        ];
        let body = chat_body(&config(), "my-review-alias", &messages, 2048);
        assert_eq!(body["model"], "my-review-alias");
        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(body["response_format"]["json_schema"]["strict"], true);
        assert_eq!(body["max_tokens"], 2048);
        assert!(body.get("reasoning").is_none());
    }
}
