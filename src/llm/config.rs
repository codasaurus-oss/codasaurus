//! Provider-neutral LLM configuration: the gateway endpoint, its credential and
//! the stable model aliases the app sends. No provider name, URL or model slug
//! lives here; `litellm.config.yaml` maps aliases to providers.

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
/// `CODASAURUS_API_KEY`, then the legacy env key.
fn resolve_api_key(
    db_primary: Option<&str>,
    db_legacy: Option<&str>,
    env_primary: Option<&str>,
    env_legacy: Option<&str>,
) -> Option<String> {
    if let Some(v) = db_primary.filter(|v| usable_secret(v)) {
        return Some(v.to_string());
    }
    if let Some(v) = db_legacy.filter(|v| usable_secret(v)) {
        return Some(v.to_string());
    }
    if let Some(v) = env_primary.filter(|v| usable_secret(v)) {
        return Some(v.to_string());
    }
    env_legacy.filter(|v| usable_secret(v)).map(str::to_string)
}

// shortcut: legacy OpenRouter key read, delete one release after the LiteLLM migration.
fn legacy_env_api_key() -> Option<String> {
    env_secret("OPENROUTER_API_KEY")
}

fn llm_disabled(provider: Option<&str>) -> bool {
    provider.is_some_and(|p| p.eq_ignore_ascii_case("disabled"))
}

/// Reject private/metadata LLM endpoints at request time (DNS-resolved).
async fn assert_base_url_safe(base_url: &str) -> Result<()> {
    let host = base_url.to_ascii_lowercase();
    let allow_loopback = host.contains("localhost")
        || host.contains("127.0.0.1")
        || host.contains("[::1]")
        || std::env::var("CODASAURUS_ALLOW_LOCAL_LLM")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
    crate::ssrf::validate_llm_base_url_resolved(base_url, allow_loopback)
        .await
        .map_err(|e| anyhow::anyhow!(e))
}

/// Single SSRF gate for every LLM transport entry (chat and embeddings).
pub(crate) async fn assert_endpoint_safe(config: &LlmConfig) -> Result<()> {
    assert_base_url_safe(&config.base_url).await
}

/// Public wrapper so non-review LLM calls (embeddings) get the same SSRF gate.
pub async fn assert_embedding_endpoint_safe(config: &LlmConfig) -> Result<()> {
    assert_endpoint_safe(config).await
}

impl LlmConfig {
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
        let text_model =
            resolve_model(env_secret("CODASAURUS_MODEL_CHEAP").as_deref(), CHEAP_ALIAS);

        Some(Self {
            api_key,
            model: default_model(),
            text_model,
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
            base_url,
            embedding_model: default_embedding_model(),
        })
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
        if let Some(pool) = pool {
            let db_off = crate::db::config::get_config(pool, "offline_mode")
                .await
                .ok()
                .flatten();
            if crate::bot::offline::offline_mode_from_env_and_db(db_off.as_deref()) {
                tracing::info!("offline_mode: skipping LLM config (fail-closed)");
                return None;
            }
            let provider = crate::db::config::get_config(pool, "llm_provider")
                .await
                .ok()
                .flatten();
            if llm_disabled(provider.as_deref()) {
                tracing::info!("llm_provider=disabled: skipping LLM config");
                return None;
            }
            // One round-trip instead of three sequential get_config calls.
            if let Ok(entries) = crate::db::config::get_all_config(pool).await {
                let mut db_key = None;
                let mut db_legacy_key = None;
                let mut model = None;
                let mut text_model = None;
                let mut base_url = None;
                for e in entries {
                    match e.key.as_str() {
                        "llm_api_key" if usable_secret(&e.value) => db_key = Some(e.value),
                        "openrouter_api_key" if usable_secret(&e.value) => {
                            db_legacy_key = Some(e.value)
                        }
                        "llm_model" if !e.value.is_empty() => model = Some(e.value),
                        "llm_model_cheap" if !e.value.is_empty() => text_model = Some(e.value),
                        "llm_base_url" if !e.value.is_empty() => base_url = Some(e.value),
                        _ => {}
                    }
                }
                let api_key = resolve_api_key(
                    db_key.as_deref(),
                    db_legacy_key.as_deref(),
                    env_secret("CODASAURUS_API_KEY").as_deref(),
                    legacy_env_api_key().as_deref(),
                );
                if base_url.is_none() {
                    base_url = std::env::var("CODASAURUS_BASE_URL")
                        .ok()
                        .filter(|u| !u.is_empty());
                }
                if let Some(base_url) = base_url {
                    let model = model.unwrap_or_else(default_model);
                    let text_model = text_model
                        .or_else(|| env_secret("CODASAURUS_MODEL_CHEAP"))
                        .unwrap_or_else(|| CHEAP_ALIAS.to_string());
                    return Some(Self {
                        api_key: api_key.unwrap_or_default(),
                        model,
                        text_model,
                        max_tokens: default_max_tokens(),
                        temperature: default_temperature(),
                        base_url,
                        embedding_model: default_embedding_model(),
                    });
                }
            }
        } else if crate::bot::offline::offline_mode_from_env_and_db(None) {
            return None;
        }
        Self::from_env()
    }

    /// Human-readable reason when [`from_db_or_env`] returns `None` (for PR replies).
    pub async fn unavailable_reason(pool: Option<&crate::db::DbPool>) -> String {
        if let Some(pool) = pool {
            let db_off = crate::db::config::get_config(pool, "offline_mode")
                .await
                .ok()
                .flatten();
            if crate::bot::offline::offline_mode_from_env_and_db(db_off.as_deref()) {
                return "Offline / air-gap mode is on — LLM commands are disabled. Turn it off under Settings → System.".into();
            }
            let provider = crate::db::config::get_config(pool, "llm_provider")
                .await
                .ok()
                .flatten();
            if llm_disabled(provider.as_deref()) {
                return "LLM provider is set to **disabled** under Settings → LLM. Set it to LiteLLM and save the gateway key.".into();
            }
        } else if crate::bot::offline::offline_mode_from_env_and_db(None) {
            return "Offline / air-gap mode is on — LLM commands are disabled.".into();
        }
        "No LLM endpoint or API key found. Set `CODASAURUS_BASE_URL` / `CODASAURUS_API_KEY` for the LiteLLM gateway, then try again. If you saved a key under **Settings → LLM**, re-save it (masked `••••` values are not re-sent).".into()
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
        // A masked dashboard placeholder never wins over a real credential.
        assert_eq!(
            resolve_api_key(Some("••••1234"), None, Some("env-new"), None).as_deref(),
            Some("env-new")
        );
    }

    #[test]
    fn disabled_provider_is_recognized_case_insensitively() {
        assert!(llm_disabled(Some("disabled")));
        assert!(llm_disabled(Some("Disabled")));
        assert!(!llm_disabled(Some("litellm")));
        assert!(!llm_disabled(None));
    }
}
