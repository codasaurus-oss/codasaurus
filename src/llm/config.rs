//! Provider-neutral LLM configuration: the gateway endpoint, its credential and
//! the stable model aliases the app sends. Provider choice and model slugs live
//! in `litellm.config.yaml`; the only provider-named code left is the one-release
//! legacy credential read below.

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Aliases sent to the gateway. An env override replaces one when an operator
/// still pins a concrete model.
const REVIEW_ALIAS: &str = "codasaurus-review";
const CHEAP_ALIAS: &str = "codasaurus-cheap";
const EMBEDDING_ALIAS: &str = "codasaurus-embedding";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    #[serde(default, skip_serializing)]
    pub api_key: String,

    /// Strong model for structured `review_diff` (quality-critical).
    #[serde(default = "default_model")]
    pub model: String,

    /// Cheap model for summarize / describe / ask / docs helpers.
    #[serde(default)]
    pub text_model: String,

    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,

    #[serde(default = "default_temperature")]
    pub temperature: f32,

    #[serde(default)]
    pub base_url: String,

    /// Model used for `/embeddings` calls (semantic index). BYOK — if the
    /// configured `base_url` doesn't support embeddings, the feature fails
    /// open per-repo rather than failing the review.
    #[serde(default = "default_embedding_model")]
    pub embedding_model: String,
}

/// Env override wins; otherwise the alias goes on the wire.
fn resolve_model(env_value: Option<&str>, alias: &str) -> String {
    env_value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(alias)
        .to_string()
}

fn default_model() -> String {
    resolve_model(env_secret("CODASAURUS_MODEL").as_deref(), REVIEW_ALIAS)
}

fn default_max_tokens() -> u32 {
    2048
}

fn default_temperature() -> f32 {
    0.1
}

fn default_embedding_model() -> String {
    resolve_model(
        env_secret("CODASAURUS_EMBEDDING_MODEL").as_deref(),
        EMBEDDING_ALIAS,
    )
}

/// Masked dashboard placeholders must never be sent back as a credential.
fn usable_secret(value: &str) -> bool {
    !value.is_empty() && !value.contains('•') && !value.contains('*')
}

fn env_secret(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| usable_secret(v))
}

/// Credential precedence: DB `llm_api_key`, legacy DB key, env
/// `CODASAURUS_API_KEY`, then the legacy env key. Callers pass values that
/// already survived [`usable_secret`].
fn resolve_api_key(
    db_primary: Option<&str>,
    db_legacy: Option<&str>,
    env_primary: Option<&str>,
    env_legacy: Option<&str>,
) -> Option<String> {
    db_primary
        .or(db_legacy)
        .or(env_primary)
        .or(env_legacy)
        .map(str::to_string)
}

// shortcut: legacy OpenRouter key read, delete one release after the LiteLLM migration.
fn legacy_env_api_key() -> Option<String> {
    env_secret("OPENROUTER_API_KEY")
}

fn llm_disabled(provider: Option<&str>) -> bool {
    provider.is_some_and(|p| p.eq_ignore_ascii_case("disabled"))
}

enum LlmBlock {
    Offline,
    Disabled,
}

/// Fail-closed reason for skipping LLM config. Reads the dashboard config once
/// instead of three sequential `get_config` calls; the read is cached, so the
/// caller's own `get_all_config` is a cache hit.
async fn block_reason(pool: Option<&crate::db::DbPool>) -> Option<LlmBlock> {
    let Some(pool) = pool else {
        return crate::bot::offline::offline_mode_from_env_and_db(None)
            .then_some(LlmBlock::Offline);
    };
    let entries = crate::db::config::get_all_config(pool).await.ok()?;
    let value = |key: &str| {
        entries
            .iter()
            .find(|e| e.key == key)
            .map(|e| e.value.as_str())
    };
    if crate::bot::offline::offline_mode_from_env_and_db(value("offline_mode")) {
        return Some(LlmBlock::Offline);
    }
    llm_disabled(value("llm_provider")).then_some(LlmBlock::Disabled)
}

/// Lowercased host of a parsed URL; `None` when it does not parse or has no host.
fn parsed_host(base_url: &str) -> Option<String> {
    url::Url::parse(base_url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
}

/// The compose sidecar is deployment-controlled, not user-supplied: its private
/// network address is expected, so only it skips the DNS-resolved SSRF gate.
fn is_gateway_host(base_url: &str) -> bool {
    parsed_host(base_url).as_deref() == Some("litellm")
}

/// Loopback is allowed for local endpoints. Match the parsed host only: a URL
/// whose path or query mentions `localhost` must not qualify.
fn is_loopback_base_url(base_url: &str) -> bool {
    parsed_host(base_url).is_some_and(|host| {
        host == "localhost"
            || host == "127.0.0.1"
            || host == "[::1]"
            || host.ends_with(".localhost")
    })
}

/// Single SSRF gate for every LLM transport entry (chat and embeddings).
pub(crate) async fn assert_endpoint_safe(config: &LlmConfig) -> Result<()> {
    if is_gateway_host(&config.base_url) {
        return Ok(());
    }
    crate::ssrf::validate_llm_base_url_resolved(
        &config.base_url,
        is_loopback_base_url(&config.base_url),
    )
    .await
    .map_err(|e| anyhow::anyhow!(e))
}

/// Public wrapper so non-review LLM calls (embeddings) get the same SSRF gate.
pub async fn assert_embedding_endpoint_safe(config: &LlmConfig) -> Result<()> {
    assert_endpoint_safe(config).await
}

impl LlmConfig {
    /// Assemble a config from already-resolved parts; every constructor ends here.
    fn new(
        api_key: String,
        model: Option<String>,
        text_model: Option<String>,
        base_url: String,
    ) -> Self {
        Self {
            api_key,
            model: model.unwrap_or_else(default_model),
            text_model: text_model.unwrap_or_else(|| {
                resolve_model(env_secret("CODASAURUS_MODEL_CHEAP").as_deref(), CHEAP_ALIAS)
            }),
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
            base_url,
            embedding_model: default_embedding_model(),
        }
    }

    /// Load chat-completions config from the environment.
    ///
    /// `CODASAURUS_BASE_URL` points at any `/v1`-style endpoint (the compose
    /// sidecar by default). Without one there is no endpoint, so this fails
    /// closed. An API key is optional for local endpoints.
    pub fn from_env() -> Option<Self> {
        let base_url = std::env::var("CODASAURUS_BASE_URL")
            .ok()
            .filter(|url| !url.is_empty())?;
        let api_key = resolve_api_key(
            None,
            None,
            env_secret("CODASAURUS_API_KEY").as_deref(),
            legacy_env_api_key().as_deref(),
        )
        .unwrap_or_default();
        Some(Self::new(api_key, None, None, base_url))
    }

    /// Model used for non-review text helpers.
    pub fn effective_text_model(&self) -> &str {
        if self.text_model.trim().is_empty() {
            &self.model
        } else {
            &self.text_model
        }
    }

    /// Prefer dashboard DB settings, fall back to environment.
    /// Returns `None` when offline mode or `llm_provider=disabled` (fail-closed).
    pub async fn from_db_or_env(pool: Option<&crate::db::DbPool>) -> Option<Self> {
        if let Some(block) = block_reason(pool).await {
            match block {
                LlmBlock::Offline => {
                    tracing::info!("offline_mode: skipping LLM config (fail-closed)")
                }
                LlmBlock::Disabled => tracing::info!("llm_provider=disabled: skipping LLM config"),
            }
            return None;
        }
        if let Some(pool) = pool {
            // One round-trip for every key the config needs.
            if let Ok(entries) = crate::db::config::get_all_config(pool).await {
                let mut db_key = None;
                let mut db_legacy_key = None;
                let mut model = None;
                let mut text_model = None;
                for e in entries {
                    match e.key.as_str() {
                        "llm_api_key" if usable_secret(&e.value) => db_key = Some(e.value),
                        "openrouter_api_key" if usable_secret(&e.value) => {
                            db_legacy_key = Some(e.value)
                        }
                        "llm_model" if !e.value.is_empty() => model = Some(e.value),
                        "llm_model_cheap" if !e.value.is_empty() => text_model = Some(e.value),
                        _ => {}
                    }
                }
                let api_key = resolve_api_key(
                    db_key.as_deref(),
                    db_legacy_key.as_deref(),
                    env_secret("CODASAURUS_API_KEY").as_deref(),
                    legacy_env_api_key().as_deref(),
                );
                // The gateway URL is a deployment fact; never take it from DB.
                let base_url = std::env::var("CODASAURUS_BASE_URL")
                    .ok()
                    .filter(|u| !u.is_empty())?;
                return Some(Self::new(
                    api_key.unwrap_or_default(),
                    model,
                    text_model,
                    base_url,
                ));
            }
        }
        Self::from_env()
    }

    /// Human-readable reason when [`from_db_or_env`] returns `None` (for PR replies).
    pub async fn unavailable_reason(pool: Option<&crate::db::DbPool>) -> String {
        match block_reason(pool).await {
            Some(LlmBlock::Disabled) => "LLM provider is set to **disabled** under Settings → LLM. Set it to LiteLLM and save the gateway key.".into(),
            Some(LlmBlock::Offline) if pool.is_some() => "Offline / air-gap mode is on — LLM commands are disabled. Turn it off under Settings → System.".into(),
            Some(LlmBlock::Offline) => "Offline / air-gap mode is on — LLM commands are disabled.".into(),
            None => "No LLM endpoint or API key found. Set `CODASAURUS_BASE_URL` / `CODASAURUS_API_KEY` for the LiteLLM gateway, then try again. If you saved a key under **Settings → LLM**, re-save it (masked `••••` values are not re-sent).".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_model_uses_alias_unless_overridden() {
        assert_eq!(
            resolve_model(None, "codasaurus-review"),
            "codasaurus-review"
        );
        assert_eq!(
            resolve_model(Some(""), "codasaurus-cheap"),
            "codasaurus-cheap"
        );
        assert_eq!(
            resolve_model(Some("   "), "codasaurus-embedding"),
            "codasaurus-embedding"
        );
        assert_eq!(
            resolve_model(Some("pinned-model"), "codasaurus-review"),
            "pinned-model"
        );
    }

    #[test]
    fn effective_text_model_falls_back_to_primary() {
        let cfg = LlmConfig {
            api_key: String::new(),
            model: "codasaurus-review".into(),
            text_model: String::new(),
            max_tokens: 2048,
            temperature: 0.1,
            base_url: "http://litellm:4000/v1".into(),
            embedding_model: "codasaurus-embedding".into(),
        };
        assert_eq!(cfg.effective_text_model(), "codasaurus-review");
    }

    #[test]
    fn api_key_precedence_is_db_new_then_db_legacy_then_env() {
        assert_eq!(
            resolve_api_key(
                Some("db-new"),
                Some("db-legacy"),
                Some("env-new"),
                Some("env-legacy")
            )
            .as_deref(),
            Some("db-new")
        );
        assert_eq!(
            resolve_api_key(None, Some("db-legacy"), Some("env-new"), Some("env-legacy"))
                .as_deref(),
            Some("db-legacy")
        );
        assert_eq!(
            resolve_api_key(None, None, Some("env-new"), Some("env-legacy")).as_deref(),
            Some("env-new")
        );
        assert_eq!(
            resolve_api_key(None, None, None, Some("env-legacy")).as_deref(),
            Some("env-legacy")
        );
        assert_eq!(resolve_api_key(None, None, None, None), None);
        // Masked dashboard placeholders are filtered before precedence runs.
        assert!(!usable_secret("••••1234"));
        assert!(usable_secret("sk-real-key"));
    }

    #[test]
    fn disabled_provider_is_recognized_case_insensitively() {
        assert!(llm_disabled(Some("disabled")));
        assert!(llm_disabled(Some("Disabled")));
        assert!(!llm_disabled(Some("litellm")));
        assert!(!llm_disabled(None));
    }

    #[test]
    fn gateway_host_exception_is_exact() {
        assert!(is_gateway_host("http://litellm:4000/v1"));
        assert!(!is_gateway_host("http://litellm.example.com:4000/v1"));
        assert!(!is_gateway_host("http://evil-litellm:4000/v1"));
    }

    #[test]
    fn loopback_detection_uses_parsed_host() {
        assert!(is_loopback_base_url("http://localhost:4000/v1"));
        assert!(is_loopback_base_url("http://127.0.0.1:4000/v1"));
        assert!(is_loopback_base_url("http://[::1]:4000/v1"));
        assert!(is_loopback_base_url("http://api.localhost:4000/v1"));
        assert!(!is_loopback_base_url("http://evil.example/?next=localhost"));
        assert!(!is_loopback_base_url("http://localhost.evil.example/v1"));
    }

    #[test]
    fn masked_db_credential_cannot_shadow_real_env_key() {
        let masked = "••••••••";
        // from_db_or_env only forwards DB values that survive usable_secret.
        let db_primary = if usable_secret(masked) {
            Some(masked)
        } else {
            None
        };
        assert_eq!(
            resolve_api_key(db_primary, None, Some("sk-real-env-key"), None).as_deref(),
            Some("sk-real-env-key")
        );
    }

    #[test]
    fn env_gateway_url_is_authoritative_without_db() {
        let prev = std::env::var("CODASAURUS_BASE_URL").ok();
        std::env::set_var("CODASAURUS_BASE_URL", "http://gateway.test:4000/v1");
        let cfg = LlmConfig::from_env();
        match prev {
            Some(v) => std::env::set_var("CODASAURUS_BASE_URL", v),
            None => std::env::remove_var("CODASAURUS_BASE_URL"),
        }
        assert_eq!(
            cfg.map(|c| c.base_url).as_deref(),
            Some("http://gateway.test:4000/v1")
        );
    }
}
