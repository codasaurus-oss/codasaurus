//! The only HTTP transport for chat completions: one URL build, one retry and
//! telemetry path, and one status/parse mapping shared by the structured review
//! and every plain-text helper.

use super::config::{assert_endpoint_safe, LlmConfig};
use super::schema::review_response_format;
use crate::retry::{is_reqwest_error_retryable, retry_async, RetryConfig};
use anyhow::{bail, Result};
use serde_json::json;
use std::sync::LazyLock;
use std::time::Duration;

static LLM_CLIENT: LazyLock<Option<reqwest::Client>> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .connect_timeout(Duration::from_secs(10))
        .pool_max_idle_per_host(4)
        .build()
        .ok()
});

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
/// The SSRF gate runs once here, not per attempt. Transport errors, 5xx and 429
/// retry with the default API retry budget; any other non-2xx surfaces the
/// gateway's body. Responses are parsed permissively: the gateway echoes the
/// alias and adds `x-litellm-*` fields, so unknown fields are never rejected.
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
    let max_tokens = body["max_tokens"].as_u64().unwrap_or(0) as u32;

    let resp = retry_async(
        &RetryConfig::api_default(),
        "llm_chat_completion",
        &is_reqwest_error_retryable,
        || async {
            let mut request = client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(body);
            if !config.api_key.is_empty() {
                request = request.bearer_auth(&config.api_key);
            }
            let resp = request.send().await?;
            // 5xx / 429 come back as a status-carrying reqwest error so the
            // shared predicate retries them. Read the gateway body first so its
            // diagnostic is not lost when the response is converted.
            let status = resp.status();
            if status.is_server_error() || status.as_u16() == 429 {
                let err = resp
                    .error_for_status_ref()
                    .expect_err("retryable status yields an error");
                let body = resp.text().await.unwrap_or_default();
                tracing::warn!(
                    %status,
                    body = %body.chars().take(500).collect::<String>(),
                    "LLM gateway returned a retryable status"
                );
                return Err(err.into());
            }
            Ok(resp)
        },
    )
    .await;

    let latency_ms = started.elapsed().as_millis() as u64;
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
        Err(e) => {
            crate::metrics::record_llm_error();
            let status = e
                .downcast_ref::<reqwest::Error>()
                .and_then(reqwest::Error::status);
            return Err(match status {
                Some(status) => anyhow::anyhow!("LLM API returned {status}"),
                None => e,
            });
        }
    };
    let status = resp.status();
    if !status.is_success() {
        crate::metrics::record_llm_error();
        let error_text = resp.text().await.unwrap_or_default();
        bail!("LLM API returned {status}: {error_text}");
    }

    Ok(resp.json().await?)
}

/// Plain-text chat completion on the cheap alias, with one retry when a
/// reasoning model spends its whole token budget and returns no content.
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

    // Free / alias routers sometimes pick a reasoning model that spends the
    // entire budget thinking and returns no content. Retry once with room to answer.
    match text_from_response(&resp) {
        TextOutcome::Text(text) => Ok(text),
        TextOutcome::Empty => {
            tracing::warn!("empty LLM content; retrying with a larger budget");
            let retry_tokens = max_tokens.saturating_mul(3).max(1500);
            let body = text_body(model, system_prompt, user_prompt, retry_tokens);
            let resp = post_chat(
                client,
                config,
                model,
                &body,
                "chat_text_retry",
                prompt_chars,
                false,
            )
            .await?;
            match text_from_response(&resp) {
                TextOutcome::Text(text) => Ok(text),
                TextOutcome::Empty => Err(anyhow::anyhow!("LLM response missing content")),
            }
        }
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
