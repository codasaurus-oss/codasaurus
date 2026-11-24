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
        validate_budget(daily_budget_usd)?;
        let mut body = json!({ "models": [alias] });
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
                get(|| async {
                    Json(serde_json::json!({"data": [{"model_id": "m-1", "model_name": "alias"}]}))
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
