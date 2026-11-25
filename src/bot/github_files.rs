//! Fetch repository files via the GitHub Contents API (no local clone).

use crate::retry::{is_reqwest_error_retryable, retry_async, RetryConfig};
use anyhow::Result;
use base64::Engine;
use reqwest::header::{HeaderMap, HeaderValue, IF_NONE_MATCH};
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};
use std::time::{Duration, Instant};

/// Soft cap so we never pull huge binaries / lockfiles into memory.
const MAX_FILE_BYTES: usize = 512_000;
fn gh_cache_max() -> usize {
    crate::env::usize("CODASAURUS_GH_CACHE_MAX_ENTRIES", 2_000)
}

fn gh_cache_ttl() -> Duration {
    Duration::from_secs(crate::env::u64("CODASAURUS_GH_CACHE_TTL_SECS", 3_600))
}

struct GhCacheEntry {
    etag: String,
    body: String,
    at: Instant,
}

static GH_CONTENTS_CACHE: LazyLock<RwLock<HashMap<String, GhCacheEntry>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

fn cache_key(repo: &str, path: &str, git_ref: &str) -> String {
    format!("{repo}\0{path}\0{git_ref}")
}

/// Decode a Contents API body into text, rejecting non-files and oversized blobs.
fn decode_file_content(path: &str, body: &serde_json::Value) -> Result<Option<String>> {
    if body.get("type").and_then(|t| t.as_str()) != Some("file") {
        return Ok(None);
    }
    let size = body["size"].as_u64().unwrap_or(0) as usize;
    if size > MAX_FILE_BYTES {
        tracing::debug!(path, size, "skipping oversized repo file");
        return Ok(None);
    }
    let Some(encoded) = body["content"].as_str() else {
        return Ok(None);
    };
    let cleaned: String = encoded.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(cleaned)
        .map_err(|e| anyhow::anyhow!("base64 decode failed for {path}: {e}"))?;
    if bytes.len() > MAX_FILE_BYTES {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

fn store_gh_cache(key: String, etag: String, body: String) {
    let mut cache = GH_CONTENTS_CACHE.write().unwrap_or_else(|e| e.into_inner());
    cache.insert(
        key,
        GhCacheEntry {
            etag,
            body,
            at: Instant::now(),
        },
    );
    if cache.len() > gh_cache_max() {
        let now = Instant::now();
        let ttl = gh_cache_ttl();
        cache.retain(|_, e| now.duration_since(e.at) < ttl);
        if cache.len() > gh_cache_max() {
            let mut keys: Vec<(String, Instant)> =
                cache.iter().map(|(k, e)| (k.clone(), e.at)).collect();
            keys.sort_unstable_by_key(|(_, t)| *t);
            let drop_n = cache.len() - gh_cache_max() / 2;
            for (k, _) in keys.into_iter().take(drop_n) {
                cache.remove(&k);
            }
        }
    }
}

/// GET `/repos/{repo}/contents/{path}?ref=` and decode file content.
/// Returns `Ok(None)` on 404 or unsupported types (directory / symlink).
/// Uses ETag / `If-None-Match` so unchanged blobs return 304 without counting
/// against the primary GitHub rate limit ([GitHub conditional requests](https://docs.github.com/en/rest/using-the-rest-api/best-practices-for-using-the-rest-api)).
pub async fn fetch_repo_file(
    client: &reqwest::Client,
    headers: &HeaderMap,
    repo: &str,
    path: &str,
    git_ref: &str,
) -> Result<Option<String>> {
    if path.is_empty() || git_ref.is_empty() {
        return Ok(None);
    }
    let key = cache_key(repo, path, git_ref);
    let cached_etag = {
        let cache = GH_CONTENTS_CACHE.read().unwrap_or_else(|e| e.into_inner());
        cache.get(&key).and_then(|e| {
            if e.at.elapsed() < gh_cache_ttl() {
                Some(e.etag.clone())
            } else {
                None
            }
        })
    };

    let encoded = crate::util::url_encode_path(path);
    let url = format!(
        "https://api.github.com/repos/{repo}/contents/{encoded}?ref={}",
        crate::util::url_encode(git_ref)
    );

    let resp = retry_async(
        &RetryConfig::api_default(),
        "fetch_repo_file",
        &is_reqwest_error_retryable,
        || async {
            let mut req = client.get(&url).headers(headers.clone());
            if let Some(ref etag) = cached_etag {
                if let Ok(v) = HeaderValue::from_str(etag) {
                    req = req.header(IF_NONE_MATCH, v);
                }
            }
            req.send().await.map_err(Into::into)
        },
    )
    .await?;

    if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
        let cache = GH_CONTENTS_CACHE.read().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = cache.get(&key) {
            crate::metrics::record_github_cache_hit();
            return Ok(Some(e.body.clone()));
        }
        // Rare: 304 without local body — fall through as miss.
    }

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        crate::metrics::record_github_cache_miss();
        return Ok(None);
    }
    if !resp.status().is_success() {
        tracing::debug!(status = %resp.status(), path, "contents API non-success");
        crate::metrics::record_github_cache_miss();
        return Ok(None);
    }

    crate::metrics::record_github_cache_miss();
    let etag = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body: serde_json::Value = resp.json().await?;
    let content = decode_file_content(path, &body)?;
    if let (Some(text), true) = (&content, !etag.is_empty()) {
        store_gh_cache(key, etag, text.clone());
    }
    Ok(content)
}

/// Fetch file content and blob SHA (needed for Contents API updates).
pub async fn fetch_repo_file_with_sha(
    client: &reqwest::Client,
    headers: &HeaderMap,
    repo: &str,
    path: &str,
    git_ref: &str,
) -> Result<Option<(String, String)>> {
    if path.is_empty() || git_ref.is_empty() {
        return Ok(None);
    }
    let encoded = crate::util::url_encode_path(path);

    let url = format!(
        "https://api.github.com/repos/{repo}/contents/{encoded}?ref={}",
        crate::util::url_encode(git_ref)
    );
    let resp = retry_async(
        &RetryConfig::api_default(),
        "fetch_repo_file_sha",
        &is_reqwest_error_retryable,
        || async {
            client
                .get(&url)
                .headers(headers.clone())
                .send()
                .await
                .map_err(Into::into)
        },
    )
    .await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND || !resp.status().is_success() {
        return Ok(None);
    }
    let body: serde_json::Value = resp.json().await?;
    let sha = body["sha"].as_str().unwrap_or("").to_string();
    if sha.is_empty() {
        return Ok(None);
    }
    let Some(content) = decode_file_content(path, &body)? else {
        return Ok(None);
    };
    Ok(Some((content, sha)))
}

/// Create or update a file on a branch via Contents API.
#[allow(clippy::too_many_arguments)]
pub async fn put_repo_file(
    client: &reqwest::Client,
    headers: &HeaderMap,
    repo: &str,
    path: &str,
    branch: &str,
    content: &str,
    sha: &str,
    message: &str,
) -> Result<()> {
    let encoded = crate::util::url_encode_path(path);
    let url = format!("https://api.github.com/repos/{repo}/contents/{encoded}");
    let b64 = base64::engine::general_purpose::STANDARD.encode(content.as_bytes());
    let body = serde_json::json!({
        "message": message,
        "content": b64,
        "branch": branch,
        "sha": sha,
    });
    let resp = retry_async(
        &RetryConfig::api_default(),
        "put_repo_file",
        &is_reqwest_error_retryable,
        || async {
            client
                .put(&url)
                .headers(headers.clone())
                .json(&body)
                .send()
                .await
                .map_err(Into::into)
        },
    )
    .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("put_repo_file failed: {status} {text}");
    }
    Ok(())
}

/// Try paths in order; return the first that exists.
pub async fn fetch_first_existing(
    client: &reqwest::Client,
    headers: &HeaderMap,
    repo: &str,
    paths: &[&str],
    git_ref: &str,
) -> Result<Option<(String, String)>> {
    for path in paths {
        if let Some(content) = fetch_repo_file(client, headers, repo, path, git_ref).await? {
            return Ok(Some(((*path).to_string(), content)));
        }
    }
    Ok(None)
}

/// Fetch many paths concurrently (bounded). Missing/error paths are omitted.
pub async fn fetch_repo_files_parallel(
    client: &reqwest::Client,
    headers: &HeaderMap,
    repo: &str,
    paths: &[String],
    git_ref: &str,
    max_concurrent: usize,
) -> Vec<(String, String)> {
    use std::sync::Arc;
    use tokio::sync::Semaphore;

    if paths.is_empty() {
        return Vec::new();
    }
    let limit = max_concurrent.clamp(1, 8);
    let semaphore = Arc::new(Semaphore::new(limit));
    let mut handles = Vec::with_capacity(paths.len());

    for path in paths {
        let cl = client.clone();
        let hdrs = headers.clone();
        let repo = repo.to_string();
        let path = path.clone();
        let git_ref = git_ref.to_string();
        let sem = Arc::clone(&semaphore);
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.ok()?;
            match fetch_repo_file(&cl, &hdrs, &repo, &path, &git_ref).await {
                Ok(Some(content)) if !content.is_empty() => Some((path, content)),
                _ => None,
            }
        }));
    }

    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        if let Ok(Some(pair)) = h.await {
            out.push(pair);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn query_encoding_keeps_simple_refs() {
        assert_eq!(crate::util::url_encode("main"), "main");
        assert!(crate::util::url_encode("feat/foo").contains('%'));
    }
}
