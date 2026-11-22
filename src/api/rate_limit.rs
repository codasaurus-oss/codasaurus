//! Bounded rate limiting for the dashboard API and GitHub webhook endpoint.
//! Counters live in Redis with a TTL, so all replicas share one budget and a
//! restart cannot reset it. `check_auth_rate_limit` in `auth.rs` predates this
//! and stays separate (keyed by IP+email, not IP/session).

use axum::extract::{ConnectInfo, Request, State};
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::Response;
use ipnet::IpNet;
use std::net::{IpAddr, SocketAddr};
use std::sync::LazyLock;
use std::time::Duration;

use super::errors::ApiError;
use super::AppState;
use crate::redis::StateStore;

fn env_u32(key: &str, default: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Requests per minute per session/IP for authenticated dashboard/API routes.
fn api_rate_limit_per_min() -> u32 {
    env_u32("CODASAURUS_API_RATE_LIMIT_PER_MIN", 100)
}

/// Requests per minute per source IP for the GitHub webhook receiver. GitHub
/// can burst deliveries (bulk re-runs, org pushes), so this is higher.
fn webhook_rate_limit_per_min() -> u32 {
    env_u32("CODASAURUS_WEBHOOK_RATE_LIMIT_PER_MIN", 300)
}

static TRUSTED_PROXIES: LazyLock<Vec<IpNet>> = LazyLock::new(|| {
    let raw = std::env::var("CODASAURUS_TRUSTED_PROXIES")
        .unwrap_or_else(|_| "127.0.0.0/8,::1/128".into());
    if raw.trim().is_empty() {
        return Vec::new();
    }
    raw.split(',')
        .filter_map(|entry| entry.trim().parse::<IpNet>().ok())
        .collect()
});

/// Peers allowed to override the client address via forwarding headers.
pub fn trusted_proxies() -> &'static [IpNet] {
    &TRUSTED_PROXIES
}

/// Header-only peer guess retained for the legacy login limiter in `auth.rs`,
/// whose handlers have no `ConnectInfo` extension available.
pub(crate) fn client_ip_from_headers(headers: &HeaderMap) -> String {
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first) = xff.split(',').next() {
            let t = first.trim();
            if !t.is_empty() {
                return t.to_string();
            }
        }
    }
    if let Some(rip) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        let t = rip.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    "unknown".into()
}

/// Resolve the client address: forwarding headers only count when the direct
/// peer is an allowlisted proxy, otherwise the socket peer wins.
pub fn client_ip(peer: IpAddr, headers: &HeaderMap, trusted: &[IpNet]) -> String {
    if trusted.iter().any(|net| net.contains(&peer)) {
        let forwarded = client_ip_from_headers(headers);
        if forwarded != "unknown" {
            return forwarded;
        }
    }
    peer.to_string()
}

/// Redis-backed fixed-window counter. `Err` carries the retry delay.
pub async fn check(
    store: &StateStore,
    key: &str,
    limit: u32,
    window: Duration,
) -> Result<(), Duration> {
    match store.incr_expires(key, window).await {
        Ok(count) if count <= u64::from(limit) => Ok(()),
        Ok(_) => Err(window),
        Err(err) => {
            tracing::error!(error = %err, "redis rate limit check failed; failing closed");
            Err(window)
        }
    }
}

fn too_many(retry_after: Duration, msg: &str) -> ApiError {
    ApiError::too_many_requests_after(msg, retry_after.as_secs().max(1))
}

/// Axum middleware for authenticated REST/dashboard routes. Keyed by session
/// cookie when present (so one user's tabs share a budget), else client IP.
pub async fn api_rate_limit_middleware(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let key = match super::auth::extract_token(req.headers()) {
        Some(token) => format!("rl:s:{token}"),
        None => format!(
            "rl:ip:{}",
            client_ip(peer.ip(), req.headers(), trusted_proxies())
        ),
    };
    check(
        &state.redis,
        &key,
        api_rate_limit_per_min(),
        Duration::from_secs(60),
    )
    .await
    .map_err(|retry_after| too_many(retry_after, "Too many requests. Please slow down."))?;
    Ok(next.run(req).await)
}

/// Called directly from the webhook handler (a bare `post()` route with no
/// state/middleware stack — see `serve.rs`).
pub async fn check_webhook_rate_limit(
    store: &StateStore,
    peer: IpAddr,
    headers: &HeaderMap,
) -> Result<(), ApiError> {
    let key = format!("rl:hook:{}", client_ip(peer, headers, trusted_proxies()));
    check(
        store,
        &key,
        webhook_rate_limit_per_min(),
        Duration::from_secs(60),
    )
    .await
    .map_err(|retry_after| too_many(retry_after, "Too many webhook deliveries."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_ip_ignores_forwarded_header_from_untrusted_peer() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "10.0.0.5".parse().unwrap());
        let peer = "203.0.113.9".parse().unwrap();
        assert_eq!(client_ip(peer, &headers, &[]), "203.0.113.9");
    }

    #[test]
    fn client_ip_trusts_forwarded_header_from_allowlisted_peer() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "10.0.0.5".parse().unwrap());
        let peer = "127.0.0.1".parse().unwrap();
        let trusted = ["127.0.0.0/8".parse().unwrap()];
        assert_eq!(client_ip(peer, &headers, &trusted), "10.0.0.5");
    }
}
