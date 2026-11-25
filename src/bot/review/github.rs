use crate::retry::{is_reqwest_error_retryable, retry_async, RetryConfig};
use crate::state::ReviewState;
use anyhow::Result;
use reqwest::Method;
use std::sync::LazyLock;
use std::time::Duration;

const PER_PAGE: usize = crate::util::github::PR_FILES_PER_PAGE;
/// GitHub exposes at most 3,000 PR files (30 pages of 100).
const MAX_PR_FILE_PAGES: usize = crate::util::github::MAX_PR_FILE_PAGES;

/// Build a production-configured GitHub API client with timeouts and pooling.
pub(crate) static GITHUB_CLIENT: LazyLock<Option<reqwest::Client>> = LazyLock::new(|| {
    match reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .connect_timeout(Duration::from_secs(10))
        .pool_max_idle_per_host(10)
        .pool_idle_timeout(Duration::from_secs(180))
        .tcp_nodelay(true)
        .build()
    {
        Ok(client) => Some(client),
        Err(error) => {
            tracing::error!(error = %error, "failed to build GitHub API client");
            None
        }
    }
});

/// Fetch the complete PR object for a comment-triggered review.
pub async fn fetch_pull_request(
    token: &str,
    repo_name: &str,
    pr_number: i64,
) -> Result<serde_json::Value> {
    let client = GITHUB_CLIENT
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("GitHub API client not available (failed to initialize)"))?;
    let headers = github_api_headers(&format!("Bearer {token}"))?;
    let url = format!("https://api.github.com/repos/{repo_name}/pulls/{pr_number}");
    github_get(
        client,
        &headers,
        &url,
        "fetch_pull_request",
        &RetryConfig::api_default(),
    )
    .await
}

/// Concatenate the first `max_files` non-empty patches into one bounded diff.
pub(crate) fn unified_diff_from_files(
    files: &[serde_json::Value],
    max_files: usize,
    max_bytes: usize,
) -> String {
    let mut diff = String::new();
    for f in files.iter().take(max_files) {
        let name = f["filename"].as_str().unwrap_or("?");
        let patch = f["patch"].as_str().unwrap_or("");
        if patch.is_empty() {
            continue;
        }
        use std::fmt::Write as _;
        let _ = write!(diff, "--- a/{name}\n+++ b/{name}\n{patch}\n");
        if diff.len() > max_bytes {
            break;
        }
    }
    diff
}

/// GET a GitHub API resource with the shared retry policy.
pub(crate) async fn github_get<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    headers: &reqwest::header::HeaderMap,
    url: &str,
    op: &'static str,
    retry: &RetryConfig,
) -> Result<T> {
    github_request(Method::GET, client, headers, url, None, op, retry).await
}

/// POST a JSON body to a GitHub API resource with the shared retry policy.
/// `body` is `None` for endpoints like the installation-token exchange that
/// take no parameters.
pub(crate) async fn github_post<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    headers: &reqwest::header::HeaderMap,
    url: &str,
    body: Option<&serde_json::Value>,
    op: &'static str,
    retry: &RetryConfig,
) -> Result<T> {
    github_request(Method::POST, client, headers, url, body, op, retry).await
}

async fn github_request<T: serde::de::DeserializeOwned>(
    method: Method,
    client: &reqwest::Client,
    headers: &reqwest::header::HeaderMap,
    url: &str,
    body: Option<&serde_json::Value>,
    op: &'static str,
    retry: &RetryConfig,
) -> Result<T> {
    retry_async(retry, op, &is_reqwest_error_retryable, || async {
        let mut req = client.request(method.clone(), url).headers(headers.clone());
        if let Some(body) = body {
            req = req.json(body);
        }
        req.send()
            .await?
            .error_for_status()?
            .json()
            .await
            .map_err(Into::into)
    })
    .await
}

/// Same auth/User-Agent headers reused across all GitHub API calls.
pub(crate) fn github_api_headers(auth_header: &str) -> Result<reqwest::header::HeaderMap> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        reqwest::header::HeaderValue::from_str(auth_header)
            .map_err(|e| anyhow::anyhow!("Invalid GitHub auth token: {e}"))?,
    );
    headers.insert(
        reqwest::header::ACCEPT,
        reqwest::header::HeaderValue::from_static("application/vnd.github+json"),
    );
    headers.insert(
        reqwest::header::USER_AGENT,
        reqwest::header::HeaderValue::from_static(concat!(
            "codasaurus/",
            env!("CARGO_PKG_VERSION")
        )),
    );
    Ok(headers)
}

/// Fetch all changed files for a PR. GitHub caps this endpoint at 3,000 files.
pub(crate) async fn fetch_pr_files(
    client: &reqwest::Client,
    repo_name: &str,
    pr_number: i64,
    auth_header: &str,
) -> Result<Vec<serde_json::Value>> {
    let headers = github_api_headers(auth_header)?;
    let mut all_files = Vec::new();
    for page_number in 1..=MAX_PR_FILE_PAGES {
        let url = format!(
            "https://api.github.com/repos/{repo_name}/pulls/{pr_number}/files?per_page={PER_PAGE}&page={page_number}"
        );
        let page: Vec<serde_json::Value> = github_get(
            client,
            &headers,
            &url,
            "fetch_pr_files_page",
            &RetryConfig::api_default(),
        )
        .await?;
        let is_last_page = page.len() < PER_PAGE;
        all_files.extend(page);
        if is_last_page || page_number == MAX_PR_FILE_PAGES {
            return Ok(all_files);
        }
    }

    unreachable!("the bounded page loop always returns")
}

/// Fingerprint prefixes already posted on this PR, read from GitHub because a
/// persisted finding may never have been commented. Failures return empty so a
/// duplicate is preferred to hiding a blocking finding.
pub(crate) async fn posted_inline_fingerprints(
    client: &reqwest::Client,
    headers: &reqwest::header::HeaderMap,
    repo_name: &str,
    pr_number: i64,
) -> std::collections::HashSet<String> {
    let mut seen = std::collections::HashSet::new();
    for page in 1..=MAX_INLINE_COMMENT_PAGES {
        let url = format!(
            "https://api.github.com/repos/{repo_name}/pulls/{pr_number}/comments?per_page={PER_PAGE}&page={page}"
        );
        let fetched = github_get::<Vec<serde_json::Value>>(
            client,
            headers,
            &url,
            "list_review_comments",
            &RetryConfig::api_default(),
        )
        .await;
        let Ok(comments) = fetched else {
            tracing::warn!(
                repo = repo_name,
                pr_number,
                "listing review comments failed; inline comments may be duplicated"
            );
            break;
        };
        let count = comments.len();
        for c in comments {
            if let Some(body) = c["body"].as_str() {
                seen.extend(parse_fingerprint_markers(body));
            }
        }
        if count < PER_PAGE {
            break;
        }
    }
    seen
}

/// Pages of inline comments to scan. Past this a PR has more review comments
/// than we would ever post, and the scan costs more than a duplicate.
const MAX_INLINE_COMMENT_PAGES: usize = 10;

/// Pull every `fingerprint: <hex>` marker out of one comment body. The marker is
/// written by [`crate::bot::markdown::inline_finding_comment`]; keep the two in step.
fn parse_fingerprint_markers(body: &str) -> Vec<String> {
    body.match_indices("fingerprint: ")
        .map(|(i, m)| {
            body[i + m.len()..]
                .chars()
                .take_while(char::is_ascii_hexdigit)
                .collect::<String>()
        })
        .filter(|fp: &String| !fp.is_empty())
        .collect()
}

/// Post or update an issue comment using a named slot for idempotency
/// (`walkthrough`, `llm_summary`, `describe`, …) so slots never overwrite each other.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn post_or_update_comment(
    client: &reqwest::Client,
    auth_header: &str,
    repo_name: &str,
    pr_number: i64,
    body: &str,
    state: &Option<ReviewState>,
    kind: &str,
) -> Result<i64> {
    let url = format!("https://api.github.com/repos/{repo_name}/issues/{pr_number}/comments");

    let headers = github_api_headers(auth_header)?;

    if let Some(ref s) = state {
        match s.get_comment_id_async(repo_name, pr_number, kind).await {
            Ok(Some(comment_id)) => {
                let update_url = format!(
                    "https://api.github.com/repos/{repo_name}/issues/comments/{comment_id}"
                );
                match retry_async(
                    &RetryConfig::api_default(),
                    "update_comment",
                    &is_reqwest_error_retryable,
                    || async {
                        client
                            .patch(&update_url)
                            .headers(headers.clone())
                            .json(&serde_json::json!({"body": body}))
                            .send()
                            .await
                            .map_err(Into::into)
                    },
                )
                .await
                {
                    Ok(resp) if resp.status().is_success() => {
                        tracing::info!(%kind, comment_id, "updated existing PR comment in place");
                        return Ok(comment_id);
                    }
                    Ok(resp) => {
                        tracing::warn!(
                            %kind,
                            comment_id,
                            status = %resp.status(),
                            "failed to update comment — posting a new one"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            %kind,
                            comment_id,
                            error = %e,
                            "failed to update comment — posting a new one"
                        );
                    }
                }
            }
            Ok(None) => {}
            Err(e) => {
                // Do not treat DB failure as "no slot" silently — log so operators notice
                // duplicate comments when persistence is down.
                tracing::warn!(
                    %kind,
                    error = %e,
                    "failed to load comment slot; posting a new comment"
                );
            }
        }
    }

    let resp: serde_json::Value = retry_async(
        &RetryConfig::api_default(),
        "create_comment",
        &is_reqwest_error_retryable,
        || async {
            client
                .post(&url)
                .headers(headers.clone())
                .json(&serde_json::json!({"body": body}))
                .send()
                .await?
                .error_for_status()?
                .json()
                .await
                .map_err(Into::into)
        },
    )
    .await?;

    let comment_id = resp["id"].as_i64().unwrap_or(0);
    if comment_id > 0 {
        if let Some(ref s) = state {
            if let Err(e) = s
                .set_comment_id_async(repo_name, pr_number, kind, comment_id)
                .await
            {
                tracing::warn!(%kind, comment_id, error = %e, "failed to store comment ID");
            }
        }
    }

    Ok(comment_id)
}

/// Parse GitHub `Link: <url>; rel="next"` header (SSRF-safe: api.github.com only).
pub(crate) fn next_github_link(headers: &reqwest::header::HeaderMap) -> Option<String> {
    let link = headers.get(reqwest::header::LINK)?.to_str().ok()?;
    for part in link.split(',') {
        let part = part.trim();
        if !(part.contains("rel=\"next\"") || part.contains("rel='next'")) {
            continue;
        }
        let start = part.find('<')? + 1;
        let end = part.find('>')?;
        if start >= end {
            continue;
        }
        let candidate = &part[start..end];
        let Ok(url) = url::Url::parse(candidate) else {
            continue;
        };
        let Some(host) = url.host_str() else {
            continue;
        };
        if host.eq_ignore_ascii_case("api.github.com") {
            return Some(candidate.to_string());
        }
    }
    None
}

/// True when GitHub already has a review for `head_sha`. When `marker` is set,
/// only reviews whose body contains it are considered, so an independent review
/// can be posted for the same commit without colliding with idempotency checks.
pub(crate) async fn review_exists(
    client: &reqwest::Client,
    auth_header: &str,
    repo_name: &str,
    pr_number: i64,
    head_sha: &str,
    marker: Option<&str>,
) -> Result<bool> {
    if head_sha.is_empty() {
        return Ok(false);
    }
    let url =
        format!("https://api.github.com/repos/{repo_name}/pulls/{pr_number}/reviews?per_page=100");
    let headers = github_api_headers(auth_header)?;
    let reviews: Vec<serde_json::Value> = github_get(
        client,
        &headers,
        &url,
        "list_pr_reviews",
        &RetryConfig::quick(),
    )
    .await?;
    Ok(reviews.iter().any(|r| {
        let sha_matches = r
            .get("commit_id")
            .and_then(|v| v.as_str())
            .is_some_and(|sha| sha == head_sha);
        sha_matches
            && marker.is_none_or(|m| {
                r.get("body")
                    .and_then(|v| v.as_str())
                    .is_some_and(|body| body.contains(m))
            })
    }))
}

#[cfg(test)]
mod tests {
    use super::parse_fingerprint_markers;

    #[test]
    fn reads_the_marker_inline_comments_actually_write() {
        let f = crate::detectors::Finding {
            detector: "secrets".into(),
            severity: "blocking",
            file: "a.rs".into(),
            line: 3,
            column: 0,
            message: "hardcoded token".into(),
            suggestion: None,
            evidence: None,
            codemod: None,
            confidence: None,
            judge_rationale: None,
            reachability: None,
        };
        let body = crate::bot::markdown::inline_finding_comment(&f);
        let fp = crate::bot::markdown::short_fp(&f);
        assert_eq!(parse_fingerprint_markers(&body), vec![fp]);
    }

    #[test]
    fn ignores_bodies_without_a_marker() {
        assert!(parse_fingerprint_markers("nice catch, fixing now").is_empty());
        // The trailing `</code>` must not end up in the fingerprint.
        assert_eq!(
            parse_fingerprint_markers("<code>fingerprint: abc123</code>"),
            vec!["abc123".to_string()]
        );
    }
}
