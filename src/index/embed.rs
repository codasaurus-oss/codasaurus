//! `/embeddings` calls for the semantic index (pgvector layer on `repo_symbols`).
//! Reuses the same OpenAI-compatible HTTP conventions as `crate::llm` (client,
//! SSRF check, offline-mode gate) rather than a second HTTP stack.

use anyhow::Result;
use serde_json::json;

const MAX_EMBEDDING_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// Embed a batch of texts (≤100, per OpenAI's `/embeddings` batch limit).
/// Returns `Ok(vec![])` when offline or when no embedding model is configured —
/// callers should treat that as "skip indexing this batch", not an error.
///
/// # Errors
/// Returns an error when endpoint validation or the HTTP request fails.
pub async fn embed_texts(
    config: &crate::llm::LlmConfig,
    texts: &[String],
) -> Result<Vec<Vec<f32>>> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    if !embedding_configured(&config.embedding_model) {
        tracing::debug!("no embedding model configured; skipping semantic index batch");
        return Ok(Vec::new());
    }
    if crate::bot::offline::offline_mode_from_env_and_db(None) {
        return Ok(Vec::new());
    }
    crate::llm::assert_embedding_endpoint_safe(config).await?;

    let client = crate::llm::shared_client()?;
    let url = format!("{}/embeddings", config.base_url.trim_end_matches('/'));
    let body = json!({
        "model": config.embedding_model,
        "input": texts,
    });
    let mut request = client.post(&url).json(&body);
    if !config.api_key.is_empty() {
        request = request.bearer_auth(&config.api_key);
    }
    let resp = request.send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = bounded_response_text(resp, 16 * 1024).await;
        anyhow::bail!(
            "embeddings endpoint returned {status}: {}",
            text.chars().take(300).collect::<String>()
        );
    }
    let bytes = bounded_response_bytes(resp, MAX_EMBEDDING_RESPONSE_BYTES).await?;
    let parsed: serde_json::Value = serde_json::from_slice(&bytes)?;
    let data = parsed["data"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("embeddings response missing `data` array"))?;
    let mut out: Vec<Vec<f32>> = data
        .iter()
        .map(|d| {
            d["embedding"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(serde_json::Value::as_f64)
                        .map(|v| v as f32)
                        .collect::<Vec<f32>>()
                })
                .unwrap_or_default()
        })
        .collect();
    // Reject vectors containing non-finite values (NaN/Inf) — these come from an
    // external, untrusted embeddings endpoint and would otherwise corrupt pgvector
    // distance comparisons downstream.
    out.retain(|v| !v.is_empty() && v.iter().all(|f| f.is_finite()));
    Ok(out)
}

/// An unconfigured embedding model must skip rather than POST an empty name.
fn embedding_configured(model: &str) -> bool {
    !model.trim().is_empty()
}

async fn bounded_response_bytes(response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        anyhow::bail!("embeddings response exceeded {limit} bytes");
    }
    let bytes = response.bytes().await?;
    if bytes.len() > limit {
        anyhow::bail!("embeddings response exceeded {limit} bytes");
    }
    Ok(bytes.to_vec())
}

async fn bounded_response_text(response: reqwest::Response, limit: usize) -> String {
    match bounded_response_bytes(response, limit).await {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(error) => format!("response body unavailable: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedding_skip_requires_a_configured_model() {
        assert!(!embedding_configured(""));
        assert!(!embedding_configured("   "));
        assert!(embedding_configured("my-embedding-alias"));
    }
}
