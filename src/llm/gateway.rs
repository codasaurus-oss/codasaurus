//! Client for the LiteLLM management API. Master key is admin-only; the app
//! uses a model-scoped virtual key for inference.
//!
//! Mirrors LiteLLM's own quickstart: DB-backed models, master key for admin
//! calls, `/key/generate` for the application key.

use anyhow::{bail, Context, Result};
use reqwest::Method;
use serde_json::{json, Value};
use std::time::Duration;

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

    /// Admin credential is the master key; fall back to the app key for older
    /// deployments where the same value served both roles.
    pub fn from_env() -> Option<Self> {
        let base = std::env::var("CODASAURUS_BASE_URL").ok()?;
        let admin = std::env::var("LITELLM_MASTER_KEY")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| std::env::var("CODASAURUS_API_KEY").ok())?;
        if base.trim().is_empty() || admin.trim().is_empty() {
            return None;
        }
        Some(Self::new(base, admin))
    }

    async fn request<T: serde::de::DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<T> {
        let url = format!("{}{path}", self.base_url);
        let mut req = self
            .http
            .request(method, &url)
            .header("Authorization", format!("Bearer {}", self.admin_key))
            .header("Accept", "application/json");
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
        let value: Value = self.request(Method::POST, "/model/new", Some(body)).await?;
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
            )
            .await?;
        Ok(())
    }

    pub async fn list_models(&self) -> Result<Vec<GatewayModel>> {
        let value: Value = self.request(Method::GET, "/model/info", None).await?;
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
            .request(Method::POST, "/model/delete", Some(json!({ "id": id })))
            .await?;
        Ok(())
    }

    /// Issue a model-scoped virtual key. Applications never use the master key.
    pub async fn issue_key(&self, alias: &str) -> Result<String> {
        let value: Value = self
            .request(
                Method::POST,
                "/key/generate",
                Some(json!({ "models": [alias] })),
            )
            .await?;
        value
            .get("key")
            .and_then(Value::as_str)
            .map(str::to_string)
            .context("LiteLLM gateway did not return a virtual key")
    }
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

    async fn spawn(captured: Arc<Mutex<Vec<(String, String, String)>>>) -> String {
        let app = Router::new()
            .route(
                "/model/new",
                post({
                    let c = captured.clone();
                    move |req: Request| {
                        let c = c.clone();
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
                                "new".into(),
                                auth,
                                String::from_utf8(body.to_vec()).unwrap(),
                            ));
                            Json(serde_json::json!({"model_id": "m-1"}))
                        }
                    }
                }),
            )
            .route(
                "/model/info",
                get(|| async {
                    Json(serde_json::json!({"data": [{"model_id": "m-1", "model_name": "alias"}]}))
                }),
            )
            .route(
                "/model/delete",
                post({
                    let c = captured.clone();
                    move |req: Request| {
                        let c = c.clone();
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
                                "delete".into(),
                                auth,
                                String::from_utf8(body.to_vec()).unwrap(),
                            ));
                            Json(serde_json::json!({}))
                        }
                    }
                }),
            )
            .route(
                "/key/generate",
                post({
                    let c = captured.clone();
                    move |req: Request| {
                        let c = c.clone();
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
                                "key".into(),
                                auth,
                                String::from_utf8(body.to_vec()).unwrap(),
                            ));
                            Json(serde_json::json!({"key": "sk-virtual"}))
                        }
                    }
                }),
            )
            .route(
                "/model/m-1/update",
                patch(|| async { Json(serde_json::json!({})) }),
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
        let key = mock(&base).issue_key("alias").await.unwrap();
        assert_eq!(key, "sk-virtual");
        let rows = captured.lock().unwrap();
        assert_eq!(rows[0].1, "Bearer sk-master");
        assert_eq!(rows[0].2, r#"{"models":["alias"]}"#);
    }

    #[tokio::test]
    async fn gateway_error_is_contextual() {
        let err = mock("http://127.0.0.1:1").list_models().await.unwrap_err();
        assert!(
            err.to_string().contains("gateway"),
            "unexpected error: {err}"
        );
    }
}
