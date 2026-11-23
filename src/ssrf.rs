//! SSRF guards for user-supplied URLs (custom LLM endpoints, etc.).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use url::Url;

fn is_loopback_host_name(host: &str) -> bool {
    host == "localhost" || host.ends_with(".localhost") || host == "127.0.0.1" || host == "::1"
}

fn is_metadata_host(host: &str) -> bool {
    matches!(
        host,
        "metadata.google.internal" | "metadata" | "metadata.azure.com"
    ) || host.ends_with(".internal")
        || host.ends_with(".local")
}

fn validate_url(
    raw: &str,
    allowed_schemes: &[&str],
    scheme_hint: &str,
    allow_loopback_host: bool,
    allow_loopback_ip: bool,
) -> Result<(), String> {
    let url = Url::parse(raw).map_err(|e| format!("Invalid URL: {e}"))?;
    if !allowed_schemes.contains(&url.scheme()) {
        return Err(format!(
            "Unsupported URL scheme '{}' — {scheme_hint}",
            url.scheme()
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| "URL must include a host".to_string())?;
    let host_lower = host.to_ascii_lowercase();
    if is_loopback_host_name(&host_lower) {
        return if allow_loopback_host {
            Ok(())
        } else {
            Err(format!("Host '{host}' is not allowed (SSRF protection)"))
        };
    }
    if is_metadata_host(&host_lower) {
        return Err(format!("Host '{host}' is not allowed (SSRF protection)"));
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_blocked_ip(ip, allow_loopback_ip) {
            return Err(format!(
                "IP address '{ip}' is not allowed (SSRF protection)"
            ));
        }
    }
    Ok(())
}

fn host_is_loopback(raw: &str) -> bool {
    Url::parse(raw)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| is_loopback_host_name(&host))
}

/// Validate that a URL is safe to request from the server (blocks SSRF to private/metadata IPs).
///
/// When `allow_loopback` is true (Ollama local inference), `localhost` / `127.0.0.1`
/// are permitted. Private LAN and cloud-metadata addresses are always rejected.
pub fn validate_llm_base_url(raw: &str, allow_loopback: bool) -> Result<(), String> {
    validate_url(
        raw,
        &["http", "https"],
        "use http or https",
        allow_loopback,
        allow_loopback,
    )
}

/// Like [`validate_llm_base_url`], then resolve DNS and reject private/metadata answers
/// (mitigates DNS rebinding to link-local / RFC1918).
pub async fn validate_llm_base_url_resolved(raw: &str, allow_loopback: bool) -> Result<(), String> {
    validate_llm_base_url(raw, allow_loopback)?;
    resolve_and_check_host(raw, allow_loopback).await
}

/// Alias for non-LLM HTTP(S) integrations (Jira base URL, etc.).
pub fn validate_http_url(raw: &str, allow_loopback: bool) -> Result<(), String> {
    validate_llm_base_url(raw, allow_loopback)
}

/// Alias for non-LLM HTTP(S) integrations with DNS rebinding checks.
pub async fn validate_http_url_resolved(raw: &str, allow_loopback: bool) -> Result<(), String> {
    validate_llm_base_url_resolved(raw, allow_loopback).await
}

/// Validate a user-supplied Postgres URL before connecting (setup wizard).
///
/// Allows only `postgres` / `postgresql` schemes. Loopback (`localhost`,
/// `127.0.0.1`, `::1`) is permitted for local setup; private LAN and
/// cloud-metadata addresses are always rejected.
pub fn validate_postgres_url(raw: &str) -> Result<(), String> {
    // Local setup always permits loopback hostnames, but only those hostnames
    // may resolve to loopback IPs (checked here for literals, in the resolved variant for DNS).
    validate_url(
        raw,
        &["postgres", "postgresql"],
        "use postgres:// or postgresql://",
        true,
        host_is_loopback(raw),
    )
}

/// Like [`validate_postgres_url`], then resolve DNS and reject private/metadata answers.
pub async fn validate_postgres_url_resolved(raw: &str) -> Result<(), String> {
    validate_postgres_url(raw)?;
    resolve_and_check_host(raw, host_is_loopback(raw)).await
}

async fn resolve_and_check_host(raw: &str, allow_loopback: bool) -> Result<(), String> {
    let url = Url::parse(raw).map_err(|e| format!("Invalid URL: {e}"))?;
    let host = url
        .host_str()
        .ok_or_else(|| "URL must include a host".to_string())?;

    // Literal IPs already checked.
    if host.parse::<IpAddr>().is_ok() {
        return Ok(());
    }

    let port = url
        .port()
        .or_else(|| match url.scheme() {
            "postgres" | "postgresql" => Some(5432),
            "https" => Some(443),
            "http" => Some(80),
            _ => None,
        })
        .unwrap_or(443);
    let addrs = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| format!("DNS lookup failed for '{host}': {e}"))?;

    let mut saw_any = false;
    for addr in addrs {
        saw_any = true;
        if is_blocked_ip(addr.ip(), allow_loopback) {
            return Err(format!(
                "Host '{host}' resolves to blocked address {} (SSRF protection)",
                addr.ip()
            ));
        }
    }
    if !saw_any {
        return Err(format!("Host '{host}' resolved to no addresses"));
    }
    Ok(())
}

fn is_blocked_ip(ip: IpAddr, allow_loopback: bool) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_v4(v4, allow_loopback),
        IpAddr::V6(v6) => is_blocked_v6(v6, allow_loopback),
    }
}

fn is_blocked_v4(ip: Ipv4Addr, allow_loopback: bool) -> bool {
    if allow_loopback && ip.is_loopback() {
        return false;
    }
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_unspecified()
        || ip.octets()[0] == 0
        // Carrier-grade NAT 100.64.0.0/10
        || (ip.octets()[0] == 100 && (ip.octets()[1] & 0xc0) == 64)
        // AWS/GCP metadata
        || ip == Ipv4Addr::new(169, 254, 169, 254)
        // Benchmarking / documentation
        || (ip.octets()[0] == 198 && (ip.octets()[1] == 18 || ip.octets()[1] == 19))
}

fn is_blocked_v6(ip: Ipv6Addr, allow_loopback: bool) -> bool {
    if allow_loopback && ip.is_loopback() {
        return false;
    }
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_unique_local()
        || ip.is_unicast_link_local()
        || ip
            .to_ipv4_mapped()
            .is_some_and(|v4| is_blocked_v4(v4, allow_loopback))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_public_https() {
        assert!(validate_llm_base_url("https://api.example.com/v1", false).is_ok());
    }

    #[test]
    fn blocks_localhost_by_default() {
        assert!(validate_llm_base_url("http://localhost:11434", false).is_err());
        assert!(validate_llm_base_url("http://127.0.0.1:11434", false).is_err());
    }

    #[test]
    fn allows_localhost_when_flag_set() {
        assert!(validate_llm_base_url("http://localhost:11434", true).is_ok());
    }

    #[test]
    fn blocks_metadata() {
        assert!(validate_llm_base_url("http://169.254.169.254/latest", false).is_err());
        assert!(validate_llm_base_url("http://169.254.169.254/latest", true).is_err());
    }

    #[test]
    fn blocks_private() {
        assert!(validate_llm_base_url("http://10.0.0.1/v1", false).is_err());
        assert!(validate_llm_base_url("http://192.168.1.1/v1", true).is_err());
    }

    #[test]
    fn rejects_non_http() {
        assert!(validate_llm_base_url("ftp://example.com", false).is_err());
    }

    #[tokio::test]
    async fn resolve_allows_public_host() {
        let r = validate_llm_base_url_resolved("https://api.example.com/v1", false).await;
        assert!(r.is_ok(), "{r:?}");
    }

    #[test]
    fn postgres_allows_loopback_local_setup() {
        assert!(validate_postgres_url("postgres://u:p@localhost:5432/db").is_ok());
        assert!(validate_postgres_url("postgresql://u:p@127.0.0.1:5432/db").is_ok());
    }

    #[test]
    fn postgres_blocks_private_and_metadata() {
        assert!(validate_postgres_url("postgres://u:p@10.0.0.1:5432/db").is_err());
        assert!(validate_postgres_url("postgres://u:p@169.254.169.254:5432/db").is_err());
        assert!(validate_postgres_url("postgres://u:p@192.168.1.1:5432/db").is_err());
    }

    #[test]
    fn postgres_rejects_non_postgres_scheme() {
        assert!(validate_postgres_url("https://example.com/db").is_err());
        assert!(validate_postgres_url("http://localhost:5432/db").is_err());
    }
}
