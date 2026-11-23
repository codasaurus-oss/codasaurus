//! Provider-neutral LLM configuration: the gateway endpoint, its credential and
//! the model aliases the operator defined in the gateway. The app names no
//! provider or model; every model value here is user-supplied.

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    #[serde(default, skip_serializing)]
    pub api_key: String,

    /// Strong model for structured `review_diff` (quality-critical). Required:
    /// the alias must exist in the gateway.
    #[serde(default)]
    pub model: String,

    /// Cheap model for summarize / describe / ask / docs helpers. Optional.
    #[serde(default)]
    pub text_model: String,

    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,

    #[serde(default = "default_temperature")]
    pub temperature: f32,

    #[serde(default)]
    pub base_url: String,

    /// Model used for `/embeddings` calls (semantic index). Optional: with no
    /// configured value, embedding is skipped rather than sent with an empty
    /// model name.
    #[serde(default)]
    pub embedding_model: String,
}

/// First non-empty, non-masked value; DB wins over env. A masked dashboard
/// placeholder never reaches the wire.
fn resolve_model(db: Option<&str>, env: Option<&str>) -> Option<String> {
    [db, env]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|v| usable_secret(v))
        .map(str::to_string)
}

fn default_max_tokens() -> u32 {
    2048
}

fn default_temperature() -> f32 {
    0.1
}

/// Masked dashboard placeholders must never be sent back as a credential.
pub(crate) fn usable_secret(value: &str) -> bool {
    !value.is_empty() && !value.contains('•') && !value.contains('*')
}

fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// Credential precedence: DB `llm_api_key`, then env `CODASAURUS_API_KEY`.
/// Masked placeholders are dropped here, so no caller can forward one.
fn resolve_api_key(db_primary: Option<&str>, env_primary: Option<&str>) -> Option<String> {
    [db_primary, env_primary]
        .into_iter()
        .flatten()
        .find(|v| usable_secret(v))
        .map(str::to_string)
}

/// Gateway endpoint from the environment; empty counts as unset (fail closed).
fn resolve_base_url(value: Option<String>) -> Option<String> {
    value.filter(|url| !url.is_empty())
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
        model: String,
        text_model: String,
        base_url: String,
        embedding_model: String,
    ) -> Self {
        Self {
            api_key,
            model,
            text_model,
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
            base_url,
            embedding_model,
        }
    }

    /// Load chat-completions config from the environment.
    ///
    /// `CODASAURUS_BASE_URL` points at any `/v1`-style endpoint (the compose
    /// sidecar by default). Without one there is no endpoint, and without
    /// `CODASAURUS_MODEL` there is no model, so this fails closed. An API key is
    /// optional for local endpoints.
    pub fn from_env() -> Option<Self> {
        let base_url = resolve_base_url(env_var("CODASAURUS_BASE_URL"))?;
        let model = resolve_model(None, env_var("CODASAURUS_MODEL").as_deref())?;
        let api_key =
            resolve_api_key(None, env_var("CODASAURUS_API_KEY").as_deref()).unwrap_or_default();
        let text_model =
            resolve_model(None, env_var("CODASAURUS_MODEL_CHEAP").as_deref()).unwrap_or_default();
        let embedding_model = resolve_model(None, env_var("CODASAURUS_EMBEDDING_MODEL").as_deref())
            .unwrap_or_default();
        Some(Self::new(
            api_key,
            model,
            text_model,
            base_url,
            embedding_model,
        ))
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
                let mut db_model = None;
                let mut db_text_model = None;
                let mut db_embedding_model = None;
                for e in entries {
                    match e.key.as_str() {
                        "llm_api_key" => db_key = Some(e.value),
                        "llm_model" => db_model = Some(e.value),
                        "llm_model_cheap" => db_text_model = Some(e.value),
                        "llm_embedding_model" => db_embedding_model = Some(e.value),
                        _ => {}
                    }
                }
                let env_model = env_var("CODASAURUS_MODEL");
                let model = resolve_model(db_model.as_deref(), env_model.as_deref())?;
                let api_key =
                    resolve_api_key(db_key.as_deref(), env_var("CODASAURUS_API_KEY").as_deref());
                let env_text_model = env_var("CODASAURUS_MODEL_CHEAP");
                let text_model = resolve_model(db_text_model.as_deref(), env_text_model.as_deref())
                    .unwrap_or_default();
                let env_embedding_model = env_var("CODASAURUS_EMBEDDING_MODEL");
                let embedding_model = resolve_model(
                    db_embedding_model.as_deref(),
                    env_embedding_model.as_deref(),
                )
                .unwrap_or_default();
                // The gateway URL is a deployment fact; never take it from DB.
                let base_url = resolve_base_url(env_var("CODASAURUS_BASE_URL"))?;
                return Some(Self::new(
                    api_key.unwrap_or_default(),
                    model,
                    text_model,
                    base_url,
                    embedding_model,
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
            None => "No LLM configured. Set `CODASAURUS_MODEL` (or **Settings → LLM**) to a model alias you defined in LiteLLM, plus `CODASAURUS_BASE_URL` / `CODASAURUS_API_KEY` for the gateway. If you saved a key under **Settings → LLM**, re-save it (masked `••••` values are not re-sent).".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_model_requires_a_value_and_db_wins() {
        // Made-up user values: the app ships no model names.
        assert_eq!(
            resolve_model(Some("my-review-alias"), Some("my-env-alias")).as_deref(),
            Some("my-review-alias")
        );
        assert_eq!(
            resolve_model(None, Some("my-env-alias")).as_deref(),
            Some("my-env-alias")
        );
        assert_eq!(resolve_model(None, None), None);
        assert_eq!(resolve_model(Some("   "), Some("")), None);
        // A masked placeholder never wins over a usable value.
        assert_eq!(
            resolve_model(Some("••••"), Some("my-env-alias")).as_deref(),
            Some("my-env-alias")
        );
    }

    #[test]
    fn effective_text_model_falls_back_to_primary() {
        let cfg = LlmConfig {
            api_key: String::new(),
            model: "my-review-alias".into(),
            text_model: String::new(),
            max_tokens: 2048,
            temperature: 0.1,
            base_url: "http://litellm:4000/v1".into(),
            embedding_model: "my-embedding-alias".into(),
        };
        assert_eq!(cfg.effective_text_model(), "my-review-alias");
    }

    #[test]
    fn api_key_precedence_is_db_then_env() {
        assert_eq!(
            resolve_api_key(Some("db-key"), None).as_deref(),
            Some("db-key")
        );
        assert_eq!(
            resolve_api_key(None, Some("env-key")).as_deref(),
            Some("env-key")
        );
        assert_eq!(
            resolve_api_key(Some("db-key"), Some("env-key")).as_deref(),
            Some("db-key")
        );
        assert_eq!(resolve_api_key(None, None), None);
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
        assert_eq!(
            resolve_api_key(Some("••••••••"), Some("real-key")).as_deref(),
            Some("real-key")
        );
    }

    #[test]
    fn base_url_comes_from_env_and_empty_is_unset() {
        assert_eq!(
            resolve_base_url(Some("http://gateway.test:4000/v1".to_string())).as_deref(),
            Some("http://gateway.test:4000/v1")
        );
        assert_eq!(resolve_base_url(Some(String::new())), None);
        assert_eq!(resolve_base_url(None), None);
    }
}
