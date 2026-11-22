//! Shared GitHub App JWT helpers.

use anyhow::{Context, Result};
use base64::Engine;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use std::time::{SystemTime, UNIX_EPOCH};

/// Resolve the GitHub App private key from environment.
///
/// Tries `GITHUB_APP_PRIVATE_KEY` (raw PEM) first, then `GITHUB_APP_PRIVATE_KEY_B64`
/// (standard base64 — safer for PaaS `.env` files).
pub fn resolve_private_key_from_env() -> Option<String> {
    if let Ok(key) = std::env::var("GITHUB_APP_PRIVATE_KEY") {
        if !key.trim().is_empty() {
            return Some(key);
        }
    }
    let Ok(b64) = std::env::var("GITHUB_APP_PRIVATE_KEY_B64") else {
        return None;
    };
    if b64.trim().is_empty() {
        return None;
    }
    base64::engine::general_purpose::STANDARD
        .decode(b64.as_bytes())
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
}

/// Like [`resolve_private_key_from_env`], but errors when neither env var is set.
pub fn require_private_key_from_env() -> Result<String> {
    resolve_private_key_from_env().ok_or_else(|| {
        anyhow::anyhow!("GITHUB_APP_PRIVATE_KEY or GITHUB_APP_PRIVATE_KEY_B64 required")
    })
}

/// Create a signed GitHub App JWT (RS256) for the given App ID and PEM private key.
pub fn create_app_jwt(app_id: &str, private_key_pem: &str) -> Result<String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let payload = serde_json::json!({
        "iat": now.saturating_sub(60),
        "exp": now + 600,
        "iss": app_id,
    });

    let key = EncodingKey::from_rsa_pem(private_key_pem.as_bytes())
        .context("Failed to parse GitHub App private key PEM")?;

    encode(&Header::new(Algorithm::RS256), &payload, &key)
        .context("Failed to create GitHub App JWT")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test-only RSA key, generated for this suite. Never used against GitHub.
    const VALID_TEST_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvAIBADANBgkqhkiG9w0BAQEFAASCBKYwggSiAgEAAoIBAQCsQkahErqDc1gW
B3j9HgqYse3nUvyYkWJcg6ZLadP2OrEq3GPZpXblvSKiboLmWyA/oyG+R2nXsTt/
/S/L4KnehcmkPaLtDE1Bxge4v6IvqkKcTS4vs/058d2KblVbCJqkCZpNoUgKF/3G
mjVmB/u+8ynR/B3K1XhaUg3lL0vjZrJfGmkquJLaqKiNWU4FJjfNOU+uWEpJRvJQ
fcRxGmZKj/z/yPdTm2QEDk2GO0M1qGlpqf2Da+7JOe3PY6nsWh/iGgLXzOPwg7Fo
Zitr/h3iu//P9+T9aSEb3jycswqUX0EZqNSHHDXj6AzP1O9U/wjMRjBB3QAhZT+n
ULfw90EFAgMBAAECggEAEEyBFFKKVU+/oO36E7EY/ZOwV/fxMJpo/SnzvST5Sk4J
X23VAM34LwqzUT01o4gxx/OuiKuHJ0J8x8J0GE1cpVJxXtn6JYXJdZTAPEndlbb9
cH6ZBSh5qFbrSGLiLqfkweqqr/e3skTM3VLQBKRyvIGTjmvVgdCq/5jN+cZ83ro7
0Oc/8eTZn5xaroEdzZhHxD4eRNQOqKQ0K6gay0djxMTUBzeKLU208KHjEYGW8iu5
FLYVT8XZgRNg5e8I20t4/JZ5u6Fgc4VMpMeJ4Su2NqXYKv5S+gsJ8kCZFRbDyzgW
v+Rs81lnnsm9b0zDJ5vbUPCN11UdrD+VtAaAJk/1jQKBgQDtsXwiDuHp3RoK930A
QeTtJcWBj4/zTtA6bkrWqN45p8XAOaDOVFuc93qbg1Sq4ip3yluHZEOaz53z3tkh
DlUBHMmscurGtx+8IrcqLUNWvSFV1X207zqqI5Czo4OyrKTNXvzU986OlObjjx1Y
Ax/qhKwnp06lFNu0utzdOtk5cwKBgQC5hqTOjfR8of2oE8i4/QHoAX8tpyEJylUp
laXNe4W9aUHYNIakZoEemQf2ZvH4fFXOQ/rxkNiZC7JFRPgRoF6xsmMGlqAjtb9F
UWaQ7JeMDhyCOpyemQ8hgC+sUzsdM1LU1EKwrQ3fHbz+HBck3b15phr4ttZQGADl
T0b4xgxdpwKBgHW2mAsl46u129fiiP0/pOfs9MsdFeUg+u51AIseRi+3bq+r2n9U
N6qcxROPDrcyGmYIyRtcBD7+4vKLQmykdGSApT/KCYsv1X+OkbdJgzfJomCnwgGB
GER6KIdOS0KJTXSHvh6aOqlSBKZdkeKsaXnzCm745HHMyqHOiOLm6f0RAoGAaSNY
uoXQC+FxcpLjqZIYRnaysBiKrJWxdYONFsntnM3NtsztkLVWbxSiYo0si3lK1z3H
NKa1WOY7/O189+A+Sc2iONWboyEYaBwdV/v8Vi6ZEWRN5cMvw/h1C9+PHJk9U8eG
k8E+nzanKAqbQxU2j34GwvQ6t+EigNFQkyMJVvkCgYAhNUfxk1owciPxf16P8vJP
txptVh77/eHtcD0WaHhXoQZnMQux5lflP9GSvGQ1evnmbUa5xoaXoU5uxiaisU6l
675I/mNOxsTJaf/OK1nBky8i/jg6qp+YTbA180h7JmRsnQdWnTJ9lXxOZp+6kVsB
MffZDWyb0nBmC4LBvWSSDw==
-----END PRIVATE KEY-----"#;

    // Minimal RSA private key for unit tests (not used against GitHub).
    const TEST_PEM: &str = r#"-----BEGIN RSA PRIVATE KEY-----
MIIEpAIBAAKCAQEA0Z3VS5JJcds3xfn/ygWyF6PZX6VVwxY4H2Kz8VQwqZxqHqvF
PLACEHOLDER
-----END RSA PRIVATE KEY-----"#;

    #[test]
    fn creates_a_signed_jwt_with_a_valid_key() {
        // Regression: jsonwebtoken 10 panics at encode time when no crypto
        // provider feature is enabled.
        let jwt = create_app_jwt("12345", VALID_TEST_KEY).expect("jwt");
        assert_eq!(jwt.split('.').count(), 3);
    }

    #[test]
    fn rejects_invalid_pem() {
        let err = create_app_jwt("123", "not-a-pem").unwrap_err();
        assert!(
            err.to_string().contains("private key")
                || err.to_string().contains("PEM")
                || err.to_string().contains("key")
        );
    }

    #[test]
    fn rejects_placeholder_pem() {
        // Even with PEM-looking headers, garbage body must fail.
        assert!(create_app_jwt("123", TEST_PEM).is_err());
    }

    #[test]
    fn resolve_private_key_prefers_raw_pem() {
        std::env::remove_var("GITHUB_APP_PRIVATE_KEY");
        std::env::remove_var("GITHUB_APP_PRIVATE_KEY_B64");
        std::env::set_var(
            "GITHUB_APP_PRIVATE_KEY",
            "-----BEGIN RSA PRIVATE KEY-----\nX\n-----END RSA PRIVATE KEY-----",
        );
        let key = resolve_private_key_from_env().expect("raw pem");
        assert!(key.contains("BEGIN RSA"));
        std::env::remove_var("GITHUB_APP_PRIVATE_KEY");
    }
}
