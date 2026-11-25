//! Client for the LiteLLM management API. Master key is admin-only; the app
//! uses a model-scoped virtual key for inference.
//!
//! Mirrors LiteLLM's own quickstart: DB-backed models, master key for admin
//! calls, `/key/generate` for the application key.

use anyhow::{bail, Context, Result};
use reqwest::Method;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Clone)]
pub struct ModelRequest {
    pub alias: String,
    pub upstream: String,
    pub api_key: String,
    pub api_base: Option<String>,
}

#[derive(Debug)]
pub struct GatewayModel {
    pub id: String,
    pub name: String,
}

#[derive(Debug)]
pub struct GatewayProvider {
    pub id: String,
    pub name: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct GatewayCatalogModel {
    pub name: String,
    pub mode: String,
}

pub(crate) fn is_text_model_mode(mode: &str) -> bool {
    mode == "text"
}

pub(crate) fn is_embedding_model_mode(mode: &str) -> bool {
    mode == "embedding"
}

fn path_category(path: &str) -> Option<&'static str> {
    let path = path.trim_end_matches('/').to_ascii_lowercase();
    if path.ends_with("/embeddings") {
        return Some("embedding");
    }
    if path.ends_with("/chat/completions")
        || path.ends_with("/completions")
        || path.ends_with("/responses")
        || path.ends_with("/messages")
        || path.ends_with("/generatecontent")
        || path.contains(":generatecontent")
        || path.ends_with("/interactions")
    {
        return Some("text");
    }
    None
}

fn endpoint_category(endpoints: Option<&Value>) -> Option<&'static str> {
    let endpoints = endpoints?.as_array()?;
    let paths: Vec<&str> = endpoints.iter().filter_map(Value::as_str).collect();
    if paths
        .iter()
        .any(|path| path_category(path) == Some("embedding"))
    {
        return Some("embedding");
    }
    if paths.iter().any(|path| path_category(path) == Some("text")) {
        return Some("text");
    }
    (!paths.is_empty()).then_some("other")
}

fn model_category(info: &Value) -> Option<&'static str> {
    if let Some(category) = endpoint_category(info.get("supported_endpoints")) {
        return Some(category);
    }
    if let Some(modalities) = info
        .get("architecture")
        .and_then(|architecture| architecture.get("output_modalities"))
        .and_then(Value::as_array)
    {
        if modalities
            .iter()
            .any(|modality| matches!(modality.as_str(), Some("embedding" | "embeddings")))
        {
            return Some("embedding");
        }
        if modalities
            .iter()
            .any(|modality| modality.as_str() == Some("text"))
        {
            return Some("text");
        }
        if !modalities.is_empty() {
            return Some("other");
        }
    }
    match info.get("mode").and_then(Value::as_str) {
        Some("embedding") => Some("embedding"),
        Some("chat" | "completion" | "responses" | "text") => Some("text"),
        Some(_) => Some("other"),
        None => None,
    }
}

fn model_name<'a>(provider: &str, name: &'a str) -> &'a str {
    let prefix = format!("{provider}/");
    name.strip_prefix(&prefix).unwrap_or(name)
}

fn catalog_categories(
    catalog: &Value,
    provider: &str,
) -> Result<std::collections::BTreeMap<String, Option<&'static str>>> {
    let entries = catalog
        .as_object()
        .context("LiteLLM model catalog has an unexpected shape")?;
    Ok(entries
        .iter()
        .filter_map(|(name, info)| {
            let item_provider = info
                .get("litellm_provider")
                .and_then(Value::as_str)
                .or_else(|| name.split_once('/').map(|(provider, _)| provider))?;
            item_provider
                .eq_ignore_ascii_case(provider)
                .then(|| (model_name(provider, name).to_string(), model_category(info)))
        })
        .collect())
}

pub struct GatewayClient {
    base_url: String,
    admin_key: String,
    http: reqwest::Client,
}

impl GatewayClient {
    pub fn new(base_url: String, admin_key: String) -> Self {
        Self {
            base_url: base_url
                .trim_end_matches('/')
                .trim_end_matches("/v1")
                .to_string(),
            admin_key,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .expect("valid gateway HTTP client"),
        }
    }

    /// Management always uses the master key. The app key (`CODASAURUS_API_KEY`)
    /// is model-scoped and must never be used for admin calls.
    pub fn from_env() -> Option<Self> {
        let base = std::env::var("CODASAURUS_BASE_URL").ok()?;
        let admin = std::env::var("LITELLM_MASTER_KEY").ok()?;
        let base = base.trim();
        let admin = admin.trim();
        if base.is_empty() || admin.is_empty() {
            return None;
        }
        Some(Self::new(base.to_string(), admin.to_string()))
    }

    async fn request<T: serde::de::DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        timeout: Option<Duration>,
    ) -> Result<T> {
        let url = format!("{}{path}", self.base_url);
        let mut req = self
            .http
            .request(method, &url)
            .header("Authorization", format!("Bearer {}", self.admin_key))
            .header("Accept", "application/json");
        if let Some(timeout) = timeout {
            req = req.timeout(timeout);
        }
        if let Some(body) = body {
            req = req.json(&body);
        }
        let resp = req.send().await.context("LiteLLM gateway request failed")?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .context("LiteLLM gateway response unreadable")?;
        if !status.is_success() {
            bail!(
                "LiteLLM gateway {path} returned {status}: {}",
                text.chars().take(200).collect::<String>()
            );
        }
        serde_json::from_str(&text)
            .with_context(|| format!("LiteLLM gateway {path} returned invalid JSON"))
    }

    pub async fn register_model(&self, req: ModelRequest) -> Result<String> {
        let mut params = json!({
            "model": req.upstream,
            "api_key": req.api_key,
        });
        if let Some(base) = req.api_base {
            params["api_base"] = json!(base);
        }
        let body = json!({
            "model_name": req.alias,
            "litellm_params": params,
            "model_info": { "managed_by": "codasaurus" },
        });
        let value: Value = self
            .request(Method::POST, "/model/new", Some(body), None)
            .await?;
        value
            .get("model_id")
            .or_else(|| value.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .context("LiteLLM gateway did not return a model id")
    }

    pub async fn update_model(&self, id: &str, req: ModelRequest) -> Result<()> {
        let mut params = json!({ "model": req.upstream, "api_key": req.api_key });
        if let Some(base) = req.api_base {
            params["api_base"] = json!(base);
        }
        let _: Value = self
            .request(
                Method::PATCH,
                &format!("/model/{id}/update"),
                Some(json!({ "litellm_params": params })),
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn list_models(&self) -> Result<Vec<GatewayModel>> {
        let value: Value = self
            .request(
                Method::GET,
                "/model/info",
                None,
                Some(Duration::from_secs(2)),
            )
            .await?;
        let items = value
            .get("data")
            .or_else(|| value.get("models"))
            .and_then(Value::as_array)
            .context("LiteLLM gateway model list has an unexpected shape")?;
        Ok(items
            .iter()
            .map(|m| GatewayModel {
                id: m
                    .get("model_id")
                    .or_else(|| m.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                name: m
                    .get("model_name")
                    .or_else(|| m.get("alias"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            })
            .filter(|m| !m.id.is_empty() && !m.name.is_empty())
            .collect())
    }

    async fn model_catalog(&self) -> Result<Value> {
        self.request(
            Method::GET,
            "/public/litellm_model_cost_map?catalog_only=true",
            None,
            Some(Duration::from_secs(5)),
        )
        .await
    }

    pub async fn list_providers(&self) -> Result<Vec<GatewayProvider>> {
        let (supported, metadata, endpoints, catalog) = tokio::join!(
            self.request(
                Method::GET,
                "/public/providers",
                None,
                Some(Duration::from_secs(5)),
            ),
            self.request(
                Method::GET,
                "/public/providers/fields",
                None,
                Some(Duration::from_secs(5)),
            ),
            self.request(
                Method::GET,
                "/public/endpoints",
                None,
                Some(Duration::from_secs(5)),
            ),
            self.model_catalog(),
        );
        let supported: Value = supported?;
        let metadata: Value = metadata?;
        let catalog: Value = catalog?;
        let endpoints = match endpoints {
            Ok(endpoints) => endpoints,
            Err(error) => {
                tracing::warn!(error = %error, "LiteLLM endpoint capability lookup failed");
                json!({"endpoints": []})
            }
        };
        let supported = supported
            .as_array()
            .context("LiteLLM provider list has an unexpected shape")?;
        let items = metadata
            .as_array()
            .context("LiteLLM provider metadata has an unexpected shape")?;
        let mut capable_providers: std::collections::HashSet<String> = catalog
            .as_object()
            .context("LiteLLM model catalog has an unexpected shape")?
            .iter()
            .filter(|(_, info)| matches!(model_category(info), Some("text" | "embedding")))
            .filter_map(|(name, info)| {
                info.get("litellm_provider")
                    .and_then(Value::as_str)
                    .or_else(|| name.split_once('/').map(|(provider, _)| provider))
                    .map(str::to_ascii_lowercase)
            })
            .collect();
        for endpoint in endpoints
            .get("endpoints")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if endpoint["endpoint"]
                .as_str()
                .and_then(path_category)
                .is_none()
            {
                continue;
            }
            if let Some(providers) = endpoint.get("providers").and_then(Value::as_array) {
                capable_providers.extend(providers.iter().filter_map(|provider| {
                    provider
                        .get("slug")
                        .and_then(Value::as_str)
                        .map(str::to_ascii_lowercase)
                }));
            }
        }
        let mut names = std::collections::HashMap::new();
        for item in items {
            let Some(id) = item
                .get("litellm_provider")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
            else {
                continue;
            };
            let has_api_key = item
                .get("credential_fields")
                .and_then(Value::as_array)
                .is_some_and(|fields| {
                    fields
                        .iter()
                        .any(|field| field.get("key").and_then(Value::as_str) == Some("api_key"))
                });
            if !has_api_key {
                continue;
            }
            let name = item
                .get("provider_display_name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or(id)
                .to_string();
            let replace = names.get(id).is_none_or(|current: &String| {
                !current.eq_ignore_ascii_case(id) && name.eq_ignore_ascii_case(id)
            });
            if replace {
                names.insert(id.to_string(), name);
            }
        }
        Ok(supported
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .filter(|id| capable_providers.contains(&id.to_ascii_lowercase()))
            .filter_map(|id| {
                names.get(id).map(|name| GatewayProvider {
                    id: id.to_string(),
                    name: name.clone(),
                })
            })
            .collect())
    }

    pub async fn list_model_catalog(&self, provider: &str) -> Result<Vec<GatewayCatalogModel>> {
        let provider = provider.trim().to_ascii_lowercase();
        if provider.is_empty() {
            return Ok(Vec::new());
        }
        let value = self.model_catalog().await?;
        Ok(catalog_categories(&value, &provider)?
            .into_iter()
            .filter_map(|(name, category)| match category {
                Some(category @ ("text" | "embedding")) => Some(GatewayCatalogModel {
                    name,
                    mode: category.to_string(),
                }),
                _ => None,
            })
            .collect())
    }

    /// Discover the provider's catalog through LiteLLM's keyed wildcard listing,
    /// optional provider pass-through, and model metadata.
    pub async fn discover_models(
        &self,
        provider: &str,
        api_key: &str,
        api_base: Option<&str>,
    ) -> Result<Vec<GatewayCatalogModel>> {
        let provider = provider.trim();
        let api_key = api_key.trim();
        if provider.is_empty() || api_key.is_empty() {
            bail!("provider and API key are required for model discovery");
        }
        if !provider
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
        {
            bail!("provider name contains unsupported characters");
        }
        let wildcard = format!("{provider}/*");
        let model_id = self
            .register_model(ModelRequest {
                alias: wildcard.clone(),
                upstream: wildcard,
                api_key: api_key.to_string(),
                api_base: api_base.map(str::to_string),
            })
            .await?;
        let result = self.discover_models_with_deployment(provider).await;
        if let Err(error) = self.delete_model(&model_id).await {
            tracing::warn!(error = %error, "could not remove model discovery deployment");
        }
        result
    }

    async fn discover_models_with_deployment(
        &self,
        provider: &str,
    ) -> Result<Vec<GatewayCatalogModel>> {
        let passthrough_path = format!("/{provider}/v1/models?output_modalities=all");
        let (wildcard, passthrough, metadata, catalog) = tokio::join!(
            self.request::<Value>(
                Method::GET,
                "/v1/models?return_wildcard_routes=false",
                None,
                Some(Duration::from_secs(15)),
            ),
            self.request::<Value>(
                Method::GET,
                &passthrough_path,
                None,
                Some(Duration::from_secs(15)),
            ),
            self.request::<Value>(
                Method::GET,
                "/model/info?scope=expand",
                None,
                Some(Duration::from_secs(5)),
            ),
            self.model_catalog(),
        );
        let passthrough = match passthrough {
            Ok(passthrough) => Ok(passthrough),
            Err(error) => {
                tracing::warn!(error = %error, "provider model catalog with all modalities failed");
                self.request::<Value>(
                    Method::GET,
                    &format!("/{provider}/v1/models"),
                    None,
                    Some(Duration::from_secs(15)),
                )
                .await
            }
        };
        let catalog = match catalog {
            Ok(catalog) => catalog,
            Err(error) => {
                tracing::warn!(error = %error, "LiteLLM model catalog lookup failed");
                json!({})
            }
        };
        let catalog = catalog_categories(&catalog, provider).unwrap_or_default();
        let mut discovered: std::collections::BTreeMap<String, Option<&'static str>> =
            std::collections::BTreeMap::new();

        if let Ok(value) = wildcard {
            if let Some(items) = value
                .get("data")
                .or_else(|| value.get("models"))
                .and_then(Value::as_array)
            {
                let prefix = format!("{provider}/");
                for item in items {
                    let Some(name) = item
                        .as_str()
                        .or_else(|| item.get("id").and_then(Value::as_str))
                        .map(str::trim)
                        .filter(|name| name.starts_with(&prefix))
                    else {
                        continue;
                    };
                    discovered
                        .entry(model_name(provider, name).to_string())
                        .or_insert(None);
                }
            }
        } else if let Err(error) = wildcard {
            tracing::warn!(error = %error, "LiteLLM wildcard model discovery failed");
        }

        if let Ok(value) = passthrough {
            if let Some(items) = value
                .get("data")
                .or_else(|| value.get("models"))
                .and_then(Value::as_array)
            {
                for item in items {
                    let Some(name) = item
                        .as_str()
                        .or_else(|| item.get("id").and_then(Value::as_str))
                        .or_else(|| item.get("name").and_then(Value::as_str))
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                    else {
                        continue;
                    };
                    let category =
                        model_category(item).or_else(|| endpoint_category(item.get("endpoints")));
                    discovered.insert(model_name(provider, name).to_string(), category);
                }
            }
        } else if let Err(error) = passthrough {
            tracing::warn!(error = %error, "provider model pass-through discovery failed");
        }

        let mut metadata_categories: std::collections::BTreeMap<String, Option<&'static str>> =
            std::collections::BTreeMap::new();
        if let Ok(value) = metadata {
            if let Some(items) = value
                .get("data")
                .or_else(|| value.get("models"))
                .and_then(Value::as_array)
            {
                for item in items {
                    let upstream = item
                        .get("litellm_params")
                        .and_then(|params| params.get("model"))
                        .and_then(Value::as_str);
                    let item_provider = item
                        .get("model_info")
                        .and_then(|info| info.get("litellm_provider"))
                        .and_then(Value::as_str)
                        .or_else(|| upstream.and_then(|name| name.split_once('/').map(|(p, _)| p)));
                    if !item_provider.is_some_and(|item| item.eq_ignore_ascii_case(provider)) {
                        continue;
                    }
                    let Some(name) = item
                        .get("model_name")
                        .or_else(|| item.get("id"))
                        .and_then(Value::as_str)
                        .or(upstream)
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                    else {
                        continue;
                    };
                    let info = item.get("model_info").unwrap_or(&Value::Null);
                    if info.get("mode").is_some() || info.get("supported_endpoints").is_some() {
                        metadata_categories
                            .insert(model_name(provider, name).to_string(), model_category(info));
                    }
                }
            }
        } else if let Err(error) = metadata {
            tracing::warn!(error = %error, "LiteLLM model metadata lookup failed");
        }

        let mut models = std::collections::BTreeMap::new();
        for (name, hint) in discovered {
            let known = metadata_categories
                .get(&name)
                .or_else(|| catalog.get(&name));
            let mode = match known {
                Some(Some(category)) => (*category).to_string(),
                Some(None) => continue,
                None => hint.unwrap_or("text").to_string(),
            };
            if is_text_model_mode(&mode) || is_embedding_model_mode(&mode) {
                models.insert(name, mode);
            }
        }
        for (name, category) in catalog {
            if let Some(category) = category {
                let mode = category.to_string();
                if is_text_model_mode(&mode) || is_embedding_model_mode(&mode) {
                    models.entry(name).or_insert(mode);
                }
            }
        }
        Ok(models
            .into_iter()
            .map(|(name, mode)| GatewayCatalogModel { name, mode })
            .collect())
    }

    pub async fn delete_model(&self, id: &str) -> Result<()> {
        let _: Value = self
            .request(
                Method::POST,
                "/model/delete",
                Some(json!({ "id": id })),
                None,
            )
            .await?;
        Ok(())
    }

    /// Revoke a previously issued virtual key so re-saves do not accumulate keys.
    pub async fn revoke_key(&self, key: &str) -> Result<()> {
        let _: Value = self
            .request(
                Method::POST,
                "/key/delete",
                Some(json!({ "keys": [key] })),
                None,
            )
            .await?;
        Ok(())
    }

    /// Issue a model-scoped virtual key. Applications never use the master key.
    /// A positive budget is enforced by LiteLLM and resets daily.
    pub async fn issue_key(&self, alias: &str, daily_budget_usd: f64) -> Result<String> {
        self.issue_key_for_models(&[alias.to_string()], daily_budget_usd)
            .await
    }

    /// Issue a model-scoped virtual key for every configured role.
    pub async fn issue_key_for_models(
        &self,
        aliases: &[String],
        daily_budget_usd: f64,
    ) -> Result<String> {
        validate_budget(daily_budget_usd)?;
        if aliases.is_empty() {
            bail!("at least one model alias is required");
        }
        let mut body = json!({ "models": aliases });
        if daily_budget_usd > 0.0 {
            body["max_budget"] = json!(daily_budget_usd);
            body["budget_duration"] = json!("1d");
        }
        let value: Value = self
            .request(Method::POST, "/key/generate", Some(body), None)
            .await?;
        value
            .get("key")
            .and_then(Value::as_str)
            .map(str::to_string)
            .context("LiteLLM gateway did not return a virtual key")
    }

    /// Set or clear the LiteLLM-enforced daily budget on an existing virtual key.
    pub async fn update_key_budget(&self, key: &str, daily_budget_usd: f64) -> Result<()> {
        validate_budget(daily_budget_usd)?;
        let mut body = json!({ "key": key });
        if daily_budget_usd > 0.0 {
            body["max_budget"] = json!(daily_budget_usd);
            body["budget_duration"] = json!("1d");
        } else {
            body["max_budget"] = Value::Null;
            body["budget_duration"] = Value::Null;
        }
        let _: Value = self
            .request(Method::POST, "/key/update", Some(body), None)
            .await?;
        Ok(())
    }
}

fn validate_budget(value: f64) -> Result<()> {
    if !value.is_finite() || value < 0.0 {
        bail!("daily budget must be a non-negative finite number");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Request;
    use axum::routing::{get, patch, post};
    use axum::{Json, Router};
    use std::sync::{Arc, Mutex};

    fn mock(base: &str) -> GatewayClient {
        GatewayClient::new(base.to_string(), "sk-master".to_string())
    }

    /// Mock LiteLLM endpoint that records `(route, auth, body)` and replies `response`.
    fn capture_route(
        label: &'static str,
        captured: Arc<Mutex<Vec<(String, String, String)>>>,
        response: serde_json::Value,
    ) -> axum::routing::MethodRouter {
        post({
            let c = captured.clone();
            move |req: Request| {
                let c = c.clone();
                let response = response.clone();
                async move {
                    let auth = req
                        .headers()
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    let body = axum::body::to_bytes(req.into_body(), 1 << 20)
                        .await
                        .unwrap();
                    c.lock().unwrap().push((
                        label.into(),
                        auth,
                        String::from_utf8(body.to_vec()).unwrap(),
                    ));
                    Json(response)
                }
            }
        })
    }

    async fn spawn(captured: Arc<Mutex<Vec<(String, String, String)>>>) -> String {
        let app = Router::new()
            .route(
                "/model/new",
                capture_route(
                    "new",
                    captured.clone(),
                    serde_json::json!({"model_id": "m-1"}),
                ),
            )
            .route(
                "/model/info",
                get(|query: axum::extract::Query<std::collections::HashMap<String, String>>| async move {
                    if query.get("scope").map(String::as_str) == Some("expand") {
                        Json(serde_json::json!({"data": [
                            {"model_name": "openai/gpt-4o", "model_info": {"mode": "chat"}},
                            {"model_name": "openai/text-embedding-3-small", "model_info": {"mode": "embedding"}},
                            {"model_name": "openai/decision-model", "model_info": {"mode": "decisions"}},
                            {"model_name": "openai/evaluation-model", "model_info": {"mode": "evaluation"}},
                            {"model_name": "openai/image-model", "model_info": {"mode": "image_generation"}},
                            {"model_name": "openai/new-text-model", "model_info": {}},
                            {"model_name": "openrouter/provider-model", "model_info": {"mode": "chat"}},
                            {"model_name": "openrouter/embedding-model", "model_info": {"mode": "embedding"}},
                            {"model_name": "openrouter/decision-model", "model_info": {"mode": "evaluation"}}
                        ]}))
                    } else {
                        Json(serde_json::json!({"data": [{"model_id": "m-1", "model_name": "alias"}]}))
                    }
                }),
            )
            .route(
                "/public/providers",
                get(|| async {
                    Json(serde_json::json!([
                        "anthropic", "assemblyai", "endpoint-only", "openrouter", "speech-only", "ollama"
                    ]))
                }),
            )
            .route(
                "/public/providers/fields",
                get(|| async {
                    Json(serde_json::json!([
                        {
                            "litellm_provider": "openai",
                            "provider_display_name": "OpenAI-Compatible Endpoints",
                            "credential_fields": [{"key": "api_key", "required": true}]
                        },
                        {
                            "litellm_provider": "openai",
                            "provider_display_name": "OpenAI",
                            "credential_fields": [{"key": "api_key", "required": true}]
                        },
                        {
                            "litellm_provider": "anthropic",
                            "provider_display_name": "Anthropic",
                            "credential_fields": [{"key": "api_key", "required": false}]
                        },
                        {
                            "litellm_provider": "assemblyai",
                            "provider_display_name": "AssemblyAI",
                            "credential_fields": [{"key": "api_key", "required": true}]
                        },
                        {
                            "litellm_provider": "endpoint-only",
                            "provider_display_name": "Endpoint Only",
                            "credential_fields": [{"key": "api_key", "required": true}]
                        },
                        {
                            "litellm_provider": "openrouter",
                            "provider_display_name": "OpenRouter",
                            "credential_fields": [{"key": "api_key", "required": true}]
                        },
                        {
                            "litellm_provider": "speech-only",
                            "provider_display_name": "Speech Only",
                            "credential_fields": [{"key": "api_key", "required": true}]
                        },
                        {
                            "litellm_provider": "ollama",
                            "provider_display_name": "Ollama",
                            "credential_fields": [{"key": "api_base", "required": false}]
                        }
                    ]))
                }),
            )
            .route(
                "/public/endpoints",
                get(|| async {
                    Json(serde_json::json!({"endpoints": [
                        {"key": "chat_completions", "endpoint": "/chat/completions", "providers": [
                            {"slug": "anthropic"}, {"slug": "endpoint-only"}
                        ]},
                        {"key": "embeddings", "endpoint": "/embeddings", "providers": [
                            {"slug": "anthropic"}, {"slug": "endpoint-only"}
                        ]},
                        {"key": "audio_speech", "endpoint": "/audio/speech", "providers": [
                            {"slug": "speech-only"}
                        ]}
                    ]}))
                }),
            )
            .route(
                "/public/litellm_model_cost_map",
                get(|| async {
                    Json(serde_json::json!({
                        "gpt-4o": {"litellm_provider": "openai", "mode": "chat"},
                        "text-embedding-3-small": {"litellm_provider": "openai", "mode": "embedding"},
                        "completion-model": {"litellm_provider": "openai", "mode": "completion"},
                        "responses-model": {"litellm_provider": "openai", "mode": "responses"},
                        "decision-model": {"litellm_provider": "openai", "mode": "decisions"},
                        "evaluation-model": {"litellm_provider": "openai", "mode": "evaluation"},
                        "image-model": {"litellm_provider": "openai", "mode": "image_generation"},
                        "jev-decision-model": {"litellm_provider": "eu/jev", "mode": "decisions"},
                        "claude-sonnet": {"litellm_provider": "anthropic", "mode": "chat"},
                        "openai/gpt-4o-mini": {"mode": "chat"},
                        "provider-model": {"litellm_provider": "openrouter", "mode": "chat"},
                        "embedding-model": {"litellm_provider": "openrouter", "mode": "embedding"},
                        "openrouter/decision-model": {"litellm_provider": "openrouter", "mode": "evaluation"}
                    }))
                }),
            )
            .route(
                "/v1/models",
                get(|| async {
                    Json(serde_json::json!({"data": [
                        {"id": "openai/gpt-4o"},
                        {"id": "openai/text-embedding-3-small"},
                        {"id": "openai/decision-model"},
                        {"id": "openai/evaluation-model"},
                        {"id": "openai/image-model"},
                        {"id": "openai/new-text-model"},
                        {"id": "openrouter/provider-model"},
                        {"id": "openrouter/embedding-model"},
                        {"id": "openrouter/decision-model"}
                    ]}))
                }),
            )
            .route(
                "/openrouter/v1/models",
                get(|query: axum::extract::Query<std::collections::HashMap<String, String>>| async move {
                    let mut data = vec![
                        serde_json::json!({"id": "provider-model", "endpoints": ["/chat/completions"]}),
                        serde_json::json!({"id": "decision-model", "endpoints": ["/chat/completions"]}),
                        serde_json::json!({"id": "new-text-model"}),
                        serde_json::json!({"id": "architecture-decision", "architecture": {"output_modalities": ["decisions"]}}),
                        serde_json::json!({"id": "endpoint-decision", "endpoints": ["/api/alpha/decisions"]}),
                    ];
                    if query.get("output_modalities").map(String::as_str) == Some("all") {
                        data.push(serde_json::json!({"id": "embedding-model", "endpoints": ["/embeddings"]}));
                        data.push(serde_json::json!({"id": "architecture-embedding", "architecture": {"output_modalities": ["embeddings"]}}));
                    }
                    Json(serde_json::json!({"data": data}))
                }),
            )
            .route(
                "/model/delete",
                capture_route("delete", captured.clone(), serde_json::json!({})),
            )
            .route(
                "/key/generate",
                capture_route(
                    "key",
                    captured.clone(),
                    serde_json::json!({"key": "sk-virtual"}),
                ),
            )
            .route(
                "/key/delete",
                capture_route(
                    "revoke",
                    captured.clone(),
                    serde_json::json!({"deleted_keys": ["sk-old"]}),
                ),
            )
            .route(
                "/key/update",
                capture_route("budget", captured.clone(), serde_json::json!({})),
            )
            .route(
                "/model/m-1/update",
                patch({
                    let c = captured.clone();
                    move |req: Request| {
                        let c = c.clone();
                        async move {
                            let body = axum::body::to_bytes(req.into_body(), 1 << 20)
                                .await
                                .unwrap();
                            c.lock().unwrap().push((
                                "update".into(),
                                String::new(),
                                String::from_utf8(body.to_vec()).unwrap(),
                            ));
                            Json(serde_json::json!({}))
                        }
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn register_model_sends_master_key_and_model_payload() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let base = spawn(captured.clone()).await;
        let id = mock(&base)
            .register_model(ModelRequest {
                alias: "alias".into(),
                upstream: "openrouter/liquid/lfm-3-1.7b:free".into(),
                api_key: "provider-key".into(),
                api_base: Some("https://openrouter.ai/api/v1".into()),
            })
            .await
            .unwrap();
        assert_eq!(id, "m-1");
        let rows = captured.lock().unwrap();
        let (_, auth, body) = &rows[0];
        assert_eq!(auth, "Bearer sk-master");
        let v: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(v["model_name"], "alias");
        assert_eq!(
            v["litellm_params"]["model"],
            "openrouter/liquid/lfm-3-1.7b:free"
        );
        assert_eq!(v["litellm_params"]["api_key"], "provider-key");
        assert_eq!(v["model_info"]["managed_by"], "codasaurus");
    }

    #[tokio::test]
    async fn list_models_maps_id_and_name() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let base = spawn(captured.clone()).await;
        let models = mock(&base).list_models().await.unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "m-1");
        assert_eq!(models[0].name, "alias");
    }

    #[tokio::test]
    async fn list_providers_uses_endpoint_and_catalog_capabilities() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let base = spawn(captured.clone()).await;
        let providers = mock(&base).list_providers().await.unwrap();
        assert_eq!(
            providers
                .iter()
                .map(|provider| provider.id.as_str())
                .collect::<Vec<_>>(),
            vec!["anthropic", "endpoint-only", "openrouter"]
        );
        assert_eq!(providers[0].name, "Anthropic");
        assert_eq!(providers[1].name, "Endpoint Only");
        assert_eq!(providers[2].name, "OpenRouter");
    }

    #[tokio::test]
    async fn model_catalog_filters_provider_and_keeps_supported_modes() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let base = spawn(captured.clone()).await;
        let models = mock(&base).list_model_catalog("openai").await.unwrap();
        assert_eq!(
            models,
            vec![
                GatewayCatalogModel {
                    name: "completion-model".into(),
                    mode: "text".into(),
                },
                GatewayCatalogModel {
                    name: "gpt-4o".into(),
                    mode: "text".into(),
                },
                GatewayCatalogModel {
                    name: "gpt-4o-mini".into(),
                    mode: "text".into(),
                },
                GatewayCatalogModel {
                    name: "responses-model".into(),
                    mode: "text".into(),
                },
                GatewayCatalogModel {
                    name: "text-embedding-3-small".into(),
                    mode: "embedding".into(),
                },
            ]
        );
    }

    #[tokio::test]
    async fn issue_key_for_models_scopes_all_aliases() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let base = spawn(captured.clone()).await;
        let key = mock(&base)
            .issue_key_for_models(&["review".into(), "cheap".into(), "embedding".into()], 2.5)
            .await
            .unwrap();
        assert_eq!(key, "sk-virtual");
        let rows = captured.lock().unwrap();
        let body: serde_json::Value = serde_json::from_str(&rows[0].2).unwrap();
        assert_eq!(
            body["models"],
            serde_json::json!(["review", "cheap", "embedding"])
        );
        assert_eq!(body["max_budget"], 2.5);
    }

    #[tokio::test]
    async fn discover_models_rejects_blank_key() {
        let client = mock("http://127.0.0.1:1");
        let error = client
            .discover_models("openrouter", "   ", None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("required"));
    }

    #[tokio::test]
    async fn discover_models_expands_keyed_wildcard_and_filters_non_text() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let base = spawn(captured.clone()).await;
        let models = mock(&base)
            .discover_models("openai", "provider-key", None)
            .await
            .unwrap();
        assert_eq!(
            models,
            vec![
                GatewayCatalogModel {
                    name: "completion-model".into(),
                    mode: "text".into(),
                },
                GatewayCatalogModel {
                    name: "gpt-4o".into(),
                    mode: "text".into(),
                },
                GatewayCatalogModel {
                    name: "gpt-4o-mini".into(),
                    mode: "text".into(),
                },
                GatewayCatalogModel {
                    name: "new-text-model".into(),
                    mode: "text".into(),
                },
                GatewayCatalogModel {
                    name: "responses-model".into(),
                    mode: "text".into(),
                },
                GatewayCatalogModel {
                    name: "text-embedding-3-small".into(),
                    mode: "embedding".into(),
                },
            ]
        );
        let rows = captured.lock().unwrap();
        let created: serde_json::Value = serde_json::from_str(&rows[0].2).unwrap();
        assert_eq!(created["model_name"], "openai/*");
        assert_eq!(created["litellm_params"]["model"], "openai/*");
        assert_eq!(created["litellm_params"]["api_key"], "provider-key");
        assert_eq!(rows[1].0, "delete");
    }

    #[tokio::test]
    async fn discover_models_uses_provider_passthrough_and_cleans_up() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let base = spawn(captured.clone()).await;
        let models = mock(&base)
            .discover_models("openrouter", "provider-key", None)
            .await
            .unwrap();
        assert_eq!(
            models,
            vec![
                GatewayCatalogModel {
                    name: "architecture-embedding".into(),
                    mode: "embedding".into(),
                },
                GatewayCatalogModel {
                    name: "embedding-model".into(),
                    mode: "embedding".into(),
                },
                GatewayCatalogModel {
                    name: "new-text-model".into(),
                    mode: "text".into(),
                },
                GatewayCatalogModel {
                    name: "provider-model".into(),
                    mode: "text".into(),
                },
            ]
        );
        let rows = captured.lock().unwrap();
        let created: serde_json::Value = serde_json::from_str(&rows[0].2).unwrap();
        assert_eq!(created["model_name"], "openrouter/*");
        assert_eq!(created["litellm_params"]["model"], "openrouter/*");
        assert_eq!(created["litellm_params"]["api_key"], "provider-key");
        assert_eq!(rows[1].0, "delete");
    }

    #[tokio::test]
    async fn delete_model_uses_post() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let base = spawn(captured.clone()).await;
        mock(&base).delete_model("m-1").await.unwrap();
        let rows = captured.lock().unwrap();
        assert_eq!(rows[0].0, "delete");
        assert_eq!(rows[0].1, "Bearer sk-master");
    }

    #[tokio::test]
    async fn issue_key_uses_master_key_and_returns_key() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let base = spawn(captured.clone()).await;
        let key = mock(&base).issue_key("alias", 0.0).await.unwrap();
        assert_eq!(key, "sk-virtual");
        let rows = captured.lock().unwrap();
        assert_eq!(rows[0].1, "Bearer sk-master");
        assert_eq!(rows[0].2, r#"{"models":["alias"]}"#);
    }

    #[tokio::test]
    async fn issue_key_sets_daily_budget_and_update_can_clear_it() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let base = spawn(captured.clone()).await;
        let client = mock(&base);

        client.issue_key("alias", 12.5).await.unwrap();
        client.update_key_budget("sk-virtual", 12.5).await.unwrap();
        client.update_key_budget("sk-virtual", 0.0).await.unwrap();

        let rows = captured.lock().unwrap();
        let generated: serde_json::Value = serde_json::from_str(&rows[0].2).unwrap();
        assert_eq!(generated["max_budget"], 12.5);
        assert_eq!(generated["budget_duration"], "1d");

        let updated: serde_json::Value = serde_json::from_str(&rows[1].2).unwrap();
        assert_eq!(updated["key"], "sk-virtual");
        assert_eq!(updated["max_budget"], 12.5);

        let cleared: serde_json::Value = serde_json::from_str(&rows[2].2).unwrap();
        assert!(cleared["max_budget"].is_null());
        assert!(cleared["budget_duration"].is_null());
    }

    #[tokio::test]
    async fn invalid_budgets_are_rejected() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let base = spawn(captured.clone()).await;
        let client = mock(&base);
        for budget in [-1.0, f64::NAN, f64::INFINITY] {
            assert!(client.issue_key("alias", budget).await.is_err());
            assert!(client
                .update_key_budget("sk-virtual", budget)
                .await
                .is_err());
        }
        assert!(captured.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn gateway_error_is_contextual() {
        let err = mock("http://127.0.0.1:1").list_models().await.unwrap_err();
        assert!(
            err.to_string().contains("gateway"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn update_model_uses_patch_and_keeps_id() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let base = spawn(captured.clone()).await;
        mock(&base)
            .update_model(
                "m-1",
                ModelRequest {
                    alias: "alias".into(),
                    upstream: "openrouter/foo".into(),
                    api_key: "new-key".into(),
                    api_base: None,
                },
            )
            .await
            .unwrap();
        let rows = captured.lock().unwrap();
        assert_eq!(rows[0].0, "update");
        assert!(rows[0].2.contains("new-key"));
    }

    #[tokio::test]
    async fn revoke_key_sends_old_key() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let base = spawn(captured.clone()).await;
        mock(&base).revoke_key("sk-old").await.unwrap();
        let rows = captured.lock().unwrap();
        assert_eq!(rows[0].0, "revoke");
        assert!(rows[0].2.contains("sk-old"));
    }
}
