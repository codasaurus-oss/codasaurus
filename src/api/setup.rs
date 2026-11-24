use axum::extract::{ConnectInfo, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::net::SocketAddr;
use std::time::Duration;

use crate::bot;
use crate::db;
use crate::github_jwt;
use crate::ssrf;

use super::errors::ApiError;
use super::AppState;

/// Reject mutating setup routes once an owner exists, unless the caller is an owner.
async fn require_setup_open_or_admin(
    state: &AppState,
    headers: &axum::http::HeaderMap,
) -> Result<(), ApiError> {
    let owner_exists = db::users::owner_exists(&state.pool)
        .await
        .map_err(ApiError::internal_error)?;
    if !owner_exists {
        return Ok(());
    }
    super::rbac::require_owner(state, headers).await?;
    Ok(())
}

#[derive(Deserialize)]
pub struct DatabaseBody {
    pub provider: String, // "postgres"
    pub url: String,
}

#[derive(Deserialize)]
pub struct LlmBody {
    pub provider: String, // "litellm" | "disabled"
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub model_cheap: Option<String>,
    pub embedding_model: Option<String>,
    /// Gateway-managed model definition. When present, Codasaurus registers it
    /// in LiteLLM and stores only the alias plus a scoped virtual key.
    #[serde(default)]
    pub upstream_provider: Option<String>,
    #[serde(default)]
    pub upstream_model: Option<String>,
    #[serde(default)]
    pub upstream_model_cheap: Option<String>,
    #[serde(default)]
    pub upstream_model_embedding: Option<String>,
    #[serde(default)]
    pub api_base: Option<String>,
    #[serde(default)]
    pub model_alias: Option<String>,
}

/// LiteLLM needs an explicit provider prefix even for names containing `/`.
fn upstream_model_name(provider: &str, upstream: &str) -> String {
    let prefix = format!("{provider}/");
    if upstream.starts_with(&prefix) {
        upstream.to_string()
    } else {
        format!("{prefix}{upstream}")
    }
}

#[derive(Clone, Debug)]
struct ManagedModel {
    alias: String,
    id: String,
    upstream: String,
}

#[derive(Debug)]
struct ProvisionedModels {
    primary: ManagedModel,
    cheap: Option<ManagedModel>,
    embedding: Option<ManagedModel>,
    virtual_key: String,
}

/// Register each selected role and issue one virtual key scoped to all aliases.
async fn provision_models(
    client: &crate::llm::gateway::GatewayClient,
    body: &LlmBody,
    current_ids: [Option<&str>; 3],
    old_key: Option<&str>,
    daily_budget_usd: f64,
) -> Result<ProvisionedModels, ApiError> {
    let provider = body.upstream_provider.as_deref().unwrap_or("").trim();
    let upstream = body.upstream_model.as_deref().unwrap_or("").trim();
    let key = body.api_key.as_deref().unwrap_or("").trim();
    if provider.is_empty() || upstream.is_empty() {
        return Err(ApiError::bad_request(
            "Provider and review model are required to register a model",
        ));
    }
    if key.is_empty() {
        return Err(ApiError::bad_request("A provider API key is required"));
    }

    let primary_alias = body
        .model_alias
        .as_deref()
        .or(body.model.as_deref())
        .unwrap_or("codasaurus-review")
        .trim();
    let mut selections = vec![(primary_alias.to_string(), upstream.to_string(), "text")];
    if let Some(upstream) = body.upstream_model_cheap.as_deref().map(str::trim) {
        if !upstream.is_empty() {
            let alias = body
                .model_cheap
                .as_deref()
                .unwrap_or("codasaurus-cheap")
                .trim();
            selections.push((alias.to_string(), upstream.to_string(), "text"));
        }
    }
    if let Some(upstream) = body.upstream_model_embedding.as_deref().map(str::trim) {
        if !upstream.is_empty() {
            let alias = body
                .embedding_model
                .as_deref()
                .unwrap_or("codasaurus-embedding")
                .trim();
            selections.push((alias.to_string(), upstream.to_string(), "embedding"));
        }
    }

    let catalog = client
        .discover_models(provider, key, body.api_base.as_deref())
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "gateway model discovery failed");
            ApiError::bad_request("Could not load the selected provider's models")
        })?;
    if selections
        .iter()
        .any(|(alias, _, _)| alias.trim().is_empty())
    {
        return Err(ApiError::bad_request("Model aliases cannot be blank"));
    }
    for (_, upstream, mode) in &selections {
        let name = upstream
            .strip_prefix(provider)
            .and_then(|name| name.strip_prefix('/'))
            .unwrap_or(upstream);
        let selected_mode = catalog
            .iter()
            .find(|model| model.name == name)
            .map(|model| model.mode.as_str());
        let valid = match *mode {
            "embedding" => selected_mode.is_some_and(crate::llm::gateway::is_embedding_model_mode),
            _ => selected_mode.is_some_and(crate::llm::gateway::is_text_model_mode),
        };
        if !valid {
            return Err(ApiError::bad_request(format!(
                "The selected model is not available for this {mode} role"
            )));
        }
    }

    let api_base = body
        .api_base
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let mut created = Vec::new();
    let result: Result<ProvisionedModels, ApiError> = async {
        let mut models = Vec::with_capacity(selections.len());
        for (index, (alias, upstream, _)) in selections.into_iter().enumerate() {
            let request = crate::llm::gateway::ModelRequest {
                alias: alias.clone(),
                upstream: upstream_model_name(provider, &upstream),
                api_key: key.to_string(),
                api_base: api_base.clone(),
            };
            let id = match current_ids[index] {
                Some(id) => {
                    client.update_model(id, request).await.map_err(|error| {
                        tracing::error!(error = %error, "gateway model update failed");
                        ApiError::bad_request("Could not update the model in the gateway")
                    })?;
                    id.to_string()
                }
                None => {
                    let id = client.register_model(request).await.map_err(|error| {
                        tracing::error!(error = %error, "gateway model registration failed");
                        ApiError::bad_request("Could not register the model in the gateway")
                    })?;
                    created.push(id.clone());
                    id
                }
            };
            models.push(ManagedModel {
                alias,
                id,
                upstream: upstream_model_name(provider, &upstream),
            });
        }
        let aliases: Vec<String> = models.iter().map(|model| model.alias.clone()).collect();
        let virtual_key = client
            .issue_key_for_models(&aliases, daily_budget_usd)
            .await
            .map_err(|error| {
                tracing::error!(error = %error, "gateway key issuance failed");
                ApiError::bad_request("Could not issue a gateway key for the models")
            })?;
        let mut models = models.into_iter();
        Ok(ProvisionedModels {
            primary: models
                .next()
                .expect("primary model selection is always present"),
            cheap: models.next(),
            embedding: models.next(),
            virtual_key,
        })
    }
    .await;
    let provisioned = match result {
        Ok(provisioned) => provisioned,
        Err(error) => {
            for id in created {
                if let Err(cleanup_error) = client.delete_model(&id).await {
                    tracing::warn!(error = %cleanup_error, "could not remove a partially registered model");
                }
            }
            return Err(error);
        }
    };
    if let Some(old) = old_key.filter(|key| !key.is_empty() && *key != provisioned.virtual_key) {
        if let Err(error) = client.revoke_key(old).await {
            tracing::warn!(error = %error, "could not revoke the previous gateway key");
        }
    }
    Ok(provisioned)
}

#[derive(Deserialize)]
pub struct LlmDiscoverBody {
    pub provider: String,
    pub api_key: String,
    #[serde(default)]
    pub api_base: Option<String>,
}

#[derive(Deserialize)]
pub struct LlmTestParams {
    pub test: Option<bool>,
}

#[derive(Deserialize)]
pub struct LlmConfigParams {
    pub provider: Option<String>,
}

#[derive(Serialize)]
pub struct SetupResponse {
    pub status: String,
    pub message: Option<String>,
    pub test_passed: Option<bool>,
}

#[derive(Deserialize)]
pub struct GithubBody {
    pub app_id: String,
    pub private_key: String,
    pub webhook_secret: String,
    /// Optional for manual setup — when omitted, resolved from `GET /app`
    /// and persisted so install URLs keep working.
    #[serde(default)]
    pub slug: Option<String>,
}

#[derive(Deserialize)]
pub struct GithubCallbackBody {
    pub code: String,
}

#[derive(Deserialize)]
pub struct GithubCallbackQuery {
    pub code: Option<String>,
}

#[derive(Deserialize)]
pub struct ManifestPageQuery {
    pub org: Option<String>,
}

/// GitHub org login: alphanumeric or hyphens, no leading/trailing hyphen, <= 39 chars.
fn valid_org_login(org: &str) -> bool {
    let org = org.trim();
    !org.is_empty()
        && org.len() <= 39
        && !org.starts_with('-')
        && !org.ends_with('-')
        && org.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

#[derive(Serialize)]
pub struct GithubCallbackResponse {
    pub status: String,
    pub app_id: String,
    pub app_name: String,
    pub slug: String,
    pub install_url: String,
}

#[derive(Deserialize)]
pub struct AdminBody {
    pub email: String,
    pub password: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/status", get(setup_status))
        .route("/database", get(get_database_status).post(setup_database))
        .route("/llm", get(get_llm_config).post(setup_llm))
        .route("/llm/discover", post(discover_llm_models))
        .route("/github/manifest-page", get(github_manifest_page))
        .route("/github/manifest-url", get(github_manifest_url))
        .route("/github", post(setup_github))
        .route("/public-url", get(get_public_url).post(setup_public_url))
        .route(
            "/github/callback",
            get(github_callback_page).post(github_callback),
        )
        .route("/admin", post(setup_admin))
}

#[derive(Serialize)]
pub struct SetupStatus {
    pub database: bool,
    pub llm: bool,
    pub github: bool,
    pub admin: bool,
    pub complete: bool,
    /// Present when a GitHub App slug is known — used by the complete screen before login.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_install_url: Option<String>,
    /// Effective public URL (DB → env → request Host) the manifest uses for
    /// webhook and callback URLs. The wizard shows it for confirmation.
    pub public_url: String,
}

/// `llm_provider=disabled` is a deliberate opt-out and completes the step;
/// every other case needs a config that can actually build (a model alias).
fn llm_step_complete(provider: Option<&str>, configured: bool) -> bool {
    match provider {
        Some(p) if p.eq_ignore_ascii_case("disabled") => true,
        _ => configured,
    }
}

/// GET /api/setup/status — check which setup steps have been completed.
#[utoipa::path(get, path = "/api/setup/status", tag = "setup", responses((status = 200, description = "OK")))]
pub(crate) async fn setup_status(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<SetupStatus>, ApiError> {
    use db::config::get_config;

    let database = true;
    let (github_id, llm_provider, llm_configured, admin, slug, public_url) = tokio::join!(
        get_config(&state.pool, "github_app_id"),
        get_config(&state.pool, "llm_provider"),
        crate::llm::LlmConfig::from_db_or_env(Some(&state.pool)),
        db::users::owner_exists(&state.pool),
        get_config(&state.pool, "github_app_slug"),
        resolve_public_url(&state, &headers),
    );
    let github = github_id.ok().flatten().is_some() || crate::util::env_set("GITHUB_APP_ID");
    let llm_provider = llm_provider.ok().flatten();
    let llm = llm_step_complete(llm_provider.as_deref(), llm_configured.is_some());
    let admin = admin?;
    let complete = database && llm && github && admin;

    let mut github_install_url = slug
        .ok()
        .flatten()
        .or_else(|| {
            std::env::var("GITHUB_APP_SLUG")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .map(|slug| format!("https://github.com/apps/{slug}/installations/new"));
    if admin
        && super::rbac::current_user(&state.pool, &headers)
            .await
            .is_err()
    {
        github_install_url = None;
    }

    Ok(Json(SetupStatus {
        database,
        llm,
        github,
        admin,
        complete,
        github_install_url,
        public_url,
    }))
}

/// GET /api/setup/database — live Postgres connection summary for the wizard.
#[utoipa::path(get, path = "/api/setup/database", tag = "setup", responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn get_database_status(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_setup_open_or_admin(&state, &headers).await?;
    state
        .pool
        .ping()
        .await
        .map_err(|e| ApiError::internal_error(format!("Postgres ping failed: {e}")))?;

    let (version,): (String,) = sqlx::query_as("SELECT version()")
        .fetch_one(state.pool.as_pg())
        .await
        .map_err(ApiError::internal_error)?;

    let short_version = version
        .split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ");

    let url = std::env::var("DATABASE_URL").unwrap_or_default();
    let (host, database) = redact_pg_url(&url);

    Ok(Json(json!({
        "connected": true,
        "provider": "postgres",
        "host": host,
        "database": database,
        "server_version": short_version,
        "hint": "Runtime always uses DATABASE_URL from the Codasaurus process. Compose starts Postgres for you.",
    })))
}

fn redact_pg_url(raw: &str) -> (String, String) {
    let Ok(parsed) = url::Url::parse(raw) else {
        return ("(unknown)".into(), "(unknown)".into());
    };
    let host = parsed
        .host_str()
        .map(|h| {
            if let Some(port) = parsed.port() {
                format!("{h}:{port}")
            } else {
                h.to_string()
            }
        })
        .unwrap_or_else(|| "(socket)".into());
    let database = parsed.path().trim_start_matches('/').to_string();
    let database = if database.is_empty() {
        "(default)".into()
    } else {
        database
    };
    (host, database)
}

/// POST /api/setup/database
#[utoipa::path(post, path = "/api/setup/database", tag = "setup", responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn setup_database(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<DatabaseBody>,
) -> Result<Json<SetupResponse>, ApiError> {
    require_setup_open_or_admin(&state, &headers).await?;
    let provider = body.provider.to_lowercase();

    match provider.as_str() {
        "postgres" | "" => {
            // Always verify the live pool first (Compose / env boots).
            state
                .pool
                .ping()
                .await
                .map_err(|e| ApiError::bad_request(format!("Live Postgres ping failed: {e}")))?;

            let mut message = "Connected to PostgreSQL".to_string();

            if !body.url.is_empty() {
                if !body.url.starts_with("postgres://") && !body.url.starts_with("postgresql://") {
                    return Err(ApiError::bad_request(
                        "Postgres URL must start with postgres:// or postgresql://",
                    ));
                }
                let db_url = crate::db::normalize_database_url(&body.url);
                // SSRF: reject private/metadata hosts before connecting.
                // Loopback (localhost / 127.0.0.1) is allowed for local setup.
                ssrf::validate_postgres_url_resolved(&db_url)
                    .await
                    .map_err(ApiError::bad_request)?;
                let test_pool = sqlx::PgPool::connect(&db_url)
                    .await
                    .map_err(|e| ApiError::bad_request(format!("Cannot connect: {e}")))?;
                sqlx::query_scalar::<_, i32>("SELECT 1")
                    .fetch_one(&test_pool)
                    .await
                    .map_err(|e| ApiError::bad_request(format!("Test query failed: {e}")))?;
                test_pool.close().await;
                // Do not persist credentials in app_config — use DATABASE_URL env.
                message =
                    "Postgres URL validated (not stored; set DATABASE_URL for the server)".into();
            }

            db::config::set_config(&state.pool, "database_provider", "postgres").await?;

            Ok(Json(SetupResponse {
                status: "ok".into(),
                message: Some(message),
                test_passed: Some(true),
            }))
        }
        other => Err(ApiError::bad_request(format!(
            "Unsupported database provider: {other}. Codasaurus requires PostgreSQL."
        ))),
    }
}

/// GET /api/setup/llm — return current LLM config from DB + env vars.
#[utoipa::path(get, path = "/api/setup/llm", tag = "setup", responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn get_llm_config(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Query(params): Query<LlmConfigParams>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_setup_open_or_admin(&state, &headers).await?;
    let entries = db::config::get_all_config(&state.pool)
        .await
        .unwrap_or_default();
    let value = |key: &str| {
        entries
            .iter()
            .find(|entry| entry.key == key)
            .map(|entry| entry.value.as_str())
    };
    let setting = |key: &str, env: &str| {
        value(key)
            .map(str::trim)
            .filter(|value| crate::llm::usable_secret(value))
            .map(str::to_string)
            .or_else(|| {
                std::env::var(env)
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| crate::llm::usable_secret(value))
            })
    };

    let gateway = crate::llm::gateway::GatewayClient::from_env();
    let selected_provider = params
        .provider
        .map(|provider| provider.trim().to_string())
        .filter(|provider| !provider.is_empty())
        .or_else(|| value("llm_upstream_provider").map(str::to_string));
    let (models, providers, catalog) = tokio::join!(
        async {
            match gateway.as_ref() {
                Some(client) => client
                    .list_models()
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|model| json!({"id": model.id, "name": model.name}))
                    .collect(),
                None => Vec::new(),
            }
        },
        async {
            match gateway.as_ref() {
                Some(client) => client
                    .list_providers()
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|provider| json!({"id": provider.id, "name": provider.name}))
                    .collect(),
                None => Vec::new(),
            }
        },
        async {
            match (gateway.as_ref(), selected_provider.as_deref()) {
                (Some(client), Some(provider)) => client
                    .list_model_catalog(provider)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|model| json!({"name": model.name, "mode": model.mode}))
                    .collect(),
                _ => Vec::new(),
            }
        },
    );
    let provider = match value("llm_provider") {
        Some(provider) if provider.eq_ignore_ascii_case("disabled") => Some("disabled".to_string()),
        Some(_) => Some("litellm".to_string()),
        None if crate::util::env_set("CODASAURUS_API_KEY")
            || crate::util::env_set("CODASAURUS_BASE_URL") =>
        {
            Some("litellm".to_string())
        }
        None => None,
    };
    let api_key = value("llm_api_key").is_some_and(crate::llm::usable_secret)
        || std::env::var("CODASAURUS_API_KEY").is_ok_and(|value| crate::llm::usable_secret(&value));
    let api_key = api_key.then(|| "••••••••".to_string());

    Ok(Json(json!({
        "provider": provider,
        "api_key": api_key,
        "model": setting("llm_model", "CODASAURUS_MODEL"),
        "model_cheap": setting("llm_model_cheap", "CODASAURUS_MODEL_CHEAP"),
        "embedding_model": setting("llm_embedding_model", "CODASAURUS_EMBEDDING_MODEL"),
        "upstream_provider": value("llm_upstream_provider"),
        "upstream_model": value("llm_upstream_model"),
        "upstream_model_cheap": value("llm_upstream_model_cheap"),
        "upstream_model_embedding": value("llm_upstream_model_embedding"),
        "models": models,
        "providers": providers,
        "catalog": catalog,
    })))
}

/// POST /api/setup/llm/discover — list the selected provider's models through LiteLLM.
#[utoipa::path(post, path = "/api/setup/llm/discover", tag = "setup", responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn discover_llm_models(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<LlmDiscoverBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_setup_open_or_admin(&state, &headers).await?;
    let provider = body.provider.trim();
    let api_key = body.api_key.trim();
    if provider.is_empty() || api_key.is_empty() {
        return Err(ApiError::bad_request(
            "Provider and API key are required to load models",
        ));
    }
    let client = crate::llm::gateway::GatewayClient::from_env().ok_or_else(|| {
        ApiError::bad_request(
            "Gateway management is not configured. Set LITELLM_MASTER_KEY and CODASAURUS_BASE_URL.",
        )
    })?;
    let models = client
        .discover_models(provider, api_key, body.api_base.as_deref())
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "gateway model discovery failed");
            ApiError::bad_request("Could not load models from the selected provider")
        })?;
    Ok(Json(json!({
        "models": models
            .into_iter()
            .map(|model| json!({"name": model.name, "mode": model.mode}))
            .collect::<Vec<_>>(),
    })))
}

/// POST /api/setup/llm?test=true
#[utoipa::path(post, path = "/api/setup/llm", tag = "setup", responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn setup_llm(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Query(params): Query<LlmTestParams>,
    Json(body): Json<LlmBody>,
) -> Result<Json<SetupResponse>, ApiError> {
    require_setup_open_or_admin(&state, &headers).await?;
    let provider = match body.provider.as_str() {
        "litellm" | "disabled" => body.provider.as_str(),
        other => {
            return Err(ApiError::bad_request(format!(
                "Unsupported LLM provider: {other}. Use litellm or disabled."
            )))
        }
    };
    let api_key = body.api_key.as_deref().unwrap_or("");

    let (stored_model_id, stored_cheap_id, stored_embedding_id, stored_key, daily_budget_usd) = tokio::join!(
        db::config::get_config(&state.pool, "llm_gateway_model_id"),
        db::config::get_config(&state.pool, "llm_gateway_model_id_cheap"),
        db::config::get_config(&state.pool, "llm_gateway_model_id_embedding"),
        db::config::get_config(&state.pool, "llm_api_key"),
        crate::llm::budget::daily_budget_usd(Some(&state.pool)),
    );
    let stored_model_id = stored_model_id
        .ok()
        .flatten()
        .filter(|value| !value.is_empty());
    let stored_cheap_id = stored_cheap_id
        .ok()
        .flatten()
        .filter(|value| !value.is_empty());
    let stored_embedding_id = stored_embedding_id
        .ok()
        .flatten()
        .filter(|value| !value.is_empty());
    let stored_key = stored_key.ok().flatten();
    let wants_managed = body.upstream_provider.is_some()
        || body.upstream_model.is_some()
        || body.upstream_model_cheap.is_some()
        || body.upstream_model_embedding.is_some();

    // Gateway-managed setup: register/update the selected roles and keep only a scoped virtual key.
    let managed = if provider == "litellm" && wants_managed {
        let client = crate::llm::gateway::GatewayClient::from_env().ok_or_else(|| {
            ApiError::bad_request(
                "Gateway management is not configured. Set LITELLM_MASTER_KEY and CODASAURUS_BASE_URL.",
            )
        })?;
        Some(
            provision_models(
                &client,
                &body,
                [
                    stored_model_id.as_deref(),
                    stored_cheap_id.as_deref(),
                    stored_embedding_id.as_deref(),
                ],
                stored_key.as_deref(),
                daily_budget_usd,
            )
            .await?,
        )
    } else {
        None
    };

    if provider == "disabled" {
        let mut deleted = true;
        for id in [&stored_model_id, &stored_cheap_id, &stored_embedding_id]
            .into_iter()
            .flatten()
        {
            if let Some(client) = crate::llm::gateway::GatewayClient::from_env() {
                if let Err(error) = client.delete_model(id).await {
                    tracing::warn!(error = %error, "could not delete managed gateway model");
                    deleted = false;
                }
            }
        }
        if deleted {
            if let (Some(client), Some(key)) = (
                crate::llm::gateway::GatewayClient::from_env(),
                stored_key.as_deref().filter(|key| !key.is_empty()),
            ) {
                if let Err(error) = client.revoke_key(key).await {
                    tracing::warn!(error = %error, "could not revoke gateway key on disable");
                }
            }
            db::config::set_configs(
                &state.pool,
                &[
                    ("llm_gateway_model_id", ""),
                    ("llm_gateway_model_id_cheap", ""),
                    ("llm_gateway_model_id_embedding", ""),
                    ("llm_api_key", ""),
                ],
            )
            .await?;
            db::config::apply_setting_to_env("llm_api_key", "");
        }
    }

    let mut entries = vec![("llm_provider", provider)];
    if let Some(managed) = &managed {
        let cheap_alias = managed
            .cheap
            .as_ref()
            .map(|model| model.alias.as_str())
            .unwrap_or("");
        let embedding_alias = managed
            .embedding
            .as_ref()
            .map(|model| model.alias.as_str())
            .unwrap_or("");
        let cheap_id = managed
            .cheap
            .as_ref()
            .map(|model| model.id.as_str())
            .unwrap_or("");
        let embedding_id = managed
            .embedding
            .as_ref()
            .map(|model| model.id.as_str())
            .unwrap_or("");
        let upstream_provider = body.upstream_provider.as_deref().unwrap_or("").trim();
        entries.extend([
            ("llm_model", managed.primary.alias.as_str()),
            ("llm_model_cheap", cheap_alias),
            ("llm_embedding_model", embedding_alias),
            ("llm_api_key", managed.virtual_key.as_str()),
            ("llm_gateway_model_id", managed.primary.id.as_str()),
            ("llm_gateway_model_id_cheap", cheap_id),
            ("llm_gateway_model_id_embedding", embedding_id),
            ("llm_upstream_provider", upstream_provider),
            ("llm_upstream_model", managed.primary.upstream.as_str()),
            (
                "llm_upstream_model_cheap",
                managed
                    .cheap
                    .as_ref()
                    .map(|model| model.upstream.as_str())
                    .unwrap_or(""),
            ),
            (
                "llm_upstream_model_embedding",
                managed
                    .embedding
                    .as_ref()
                    .map(|model| model.upstream.as_str())
                    .unwrap_or(""),
            ),
        ]);
        db::config::apply_setting_to_env("llm_model", &managed.primary.alias);
        db::config::apply_setting_to_env("llm_model_cheap", cheap_alias);
        db::config::apply_setting_to_env("llm_embedding_model", embedding_alias);
        db::config::apply_setting_to_env("llm_api_key", &managed.virtual_key);
    } else {
        // A masked placeholder from GET /api/setup/llm must not overwrite the real key.
        if crate::llm::usable_secret(api_key) {
            entries.push(("llm_api_key", api_key));
            db::config::apply_setting_to_env("llm_api_key", api_key);
        }
        if let Some(model) = &body.model {
            entries.push(("llm_model", model.as_str()));
            db::config::apply_setting_to_env("llm_model", model);
        }
        if let Some(model_cheap) = &body.model_cheap {
            entries.push(("llm_model_cheap", model_cheap.trim()));
            db::config::apply_setting_to_env("llm_model_cheap", model_cheap.trim());
        }
        if let Some(embedding_model) = &body.embedding_model {
            entries.push(("llm_embedding_model", embedding_model.trim()));
            db::config::apply_setting_to_env("llm_embedding_model", embedding_model.trim());
        }
    }
    db::config::set_configs(&state.pool, &entries).await?;

    if let Some(managed) = &managed {
        let current_ids = [
            Some(managed.primary.id.as_str()),
            managed.cheap.as_ref().map(|model| model.id.as_str()),
            managed.embedding.as_ref().map(|model| model.id.as_str()),
        ];
        for old_id in [&stored_model_id, &stored_cheap_id, &stored_embedding_id]
            .into_iter()
            .flatten()
        {
            if !current_ids.contains(&Some(old_id.as_str())) {
                if let Some(client) = crate::llm::gateway::GatewayClient::from_env() {
                    if let Err(error) = client.delete_model(old_id).await {
                        tracing::warn!(error = %error, "could not remove a replaced gateway model");
                    }
                }
            }
        }
    }

    // If ?test=true, send a lightweight probe to the gateway
    let test_passed = if params.test.unwrap_or(false) {
        test_llm_connection(provider).await?
    } else {
        None
    };

    Ok(Json(SetupResponse {
        status: "ok".into(),
        message: llm_saved_message(test_passed),
        test_passed,
    }))
}

/// Report what the probe actually proved, so a failed probe never reads as success.
fn llm_saved_message(test_passed: Option<bool>) -> Option<String> {
    match test_passed {
        Some(true) => Some("LLM configuration saved and connection verified".into()),
        Some(false) => Some(
            "Saved, but the gateway probe failed. Set CODASAURUS_MODEL (or Settings → LLM) \
             to a model alias you defined in your LiteLLM gateway, then check the gateway URL \
             and API key."
                .into(),
        ),
        None => None,
    }
}

/// Probe the configured gateway with a minimal chat completion.
async fn test_llm_connection(provider: &str) -> Result<Option<bool>, ApiError> {
    if provider == "disabled" {
        return Ok(Some(true));
    }
    let Some(config) = crate::llm::LlmConfig::from_env() else {
        return Ok(Some(false));
    };
    Ok(Some(crate::llm::check_connection(&config).await.is_ok()))
}

#[derive(Deserialize)]
pub struct PublicUrlBody {
    pub url: String,
}

/// GET /api/setup/public-url — effective public URL + where it came from.
#[utoipa::path(get, path = "/api/setup/public-url", tag = "setup", responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn get_public_url(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    require_setup_open_or_admin(&state, &headers).await?;
    let from_db = db::config::get_config(&state.pool, "public_url")
        .await
        .ok()
        .flatten();
    let from_env = std::env::var("PUBLIC_URL").ok();
    let effective = resolve_public_url(&state, &headers).await;
    let source = if from_db.is_some() {
        "dashboard"
    } else if from_env.is_some() {
        "env"
    } else {
        "auto"
    };
    Ok(Json(json!({
        "url": effective,
        "source": source,
        "localhost": is_local_url(&effective),
    })))
}

fn is_local_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("http://localhost")
        || lower.starts_with("http://127.")
        || lower.starts_with("http://[::1]")
        || lower.contains("://localhost:")
}

/// POST /api/setup/public-url — save the canonical HTTPS origin used for
/// GitHub manifest webhook/callback URLs. Open before the first owner exists.
#[utoipa::path(post, path = "/api/setup/public-url", tag = "setup", responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn setup_public_url(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<PublicUrlBody>,
) -> Result<Json<SetupResponse>, ApiError> {
    require_setup_open_or_admin(&state, &headers).await?;
    let url = body.url.trim().to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(ApiError::bad_request("Public URL must be an http(s) URL"));
    }
    let trimmed = url.trim_end_matches('/').to_string();
    db::config::set_config(&state.pool, "public_url", &trimmed).await?;
    db::config::apply_setting_to_env("public_url", &trimmed);
    Ok(Json(SetupResponse {
        status: "ok".into(),
        message: Some(if is_local_url(&trimmed) {
            "Saved. Note: GitHub cannot reach a localhost URL — webhooks need a public HTTPS origin (deployed host or a tunnel)."
                .into()
        } else {
            "Public URL saved".into()
        }),
        test_passed: None,
    }))
}

/// Resolve the public-facing URL for the GitHub App manifest.
/// Checks DB config → `PUBLIC_URL` env var → auto-detects from request Host header.
async fn resolve_public_url(state: &AppState, headers: &axum::http::HeaderMap) -> String {
    if let Some(url) = db::config::get_config(&state.pool, "public_url")
        .await
        .ok()
        .flatten()
    {
        return url;
    }
    if let Ok(url) = std::env::var("PUBLIC_URL") {
        return url;
    }
    // Host is client-controlled and only a last-resort fallback. Require plain
    // `host[:port]` so a spoofed value cannot smuggle a scheme, path, or userinfo
    // into the manifest and redirect the webhook/OAuth callback.
    if let Some(host) = headers.get("host").and_then(|v| v.to_str().ok()) {
        let valid = !host.is_empty()
            && host.len() <= 253
            && host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'));
        if valid {
            let scheme = if host.starts_with("localhost") || host.starts_with("127.") {
                "http"
            } else {
                "https"
            };
            return format!("{scheme}://{host}");
        }
        tracing::warn!(%host, "rejected malformed Host header while resolving public URL");
    }
    "http://localhost:3000".to_string()
}

/// Build the GitHub App manifest JSON shared by the HTML page and JSON endpoints.
fn build_manifest(public_url: &str) -> serde_json::Value {
    let suffix: String = uuid::Uuid::new_v4().to_string().chars().take(4).collect();
    // Events must match `bot::handle_webhook`; installation events keep the repo
    // list in sync. Manifests cannot set a logo, so serve one for manual upload.
    json!({
        "name": format!("codasaurus-{}", suffix),
        "url": public_url,
        "description": "Self-hosted PR review agent — Tier-1 static detectors, optional BYOK LLM, fail-closed offline mode.",
        "hook_attributes": {
            "url": format!("{}/webhook/", public_url),
            "active": true
        },
        "redirect_url": format!("{}/api/setup/github/callback", public_url),
        // Dashboard auth is password + OIDC only (no GitHub OAuth user login).
        "callback_urls": [
            format!("{}/api/setup/github/callback", public_url)
        ],
        "setup_url": format!("{}/#/setup/complete", public_url),
        "setup_on_update": true,
        "public": false,
        "request_oauth_on_install": false,
        "default_permissions": {
            "pull_requests": "write",
            "checks": "write",
            "contents": "write",
            "issues": "write",
            "metadata": "read"
        },
        // `installation`/`installation_repositories` are delivered to every app
        // and cannot be subscribed to. `reaction` is not a webhook event.
        "default_events": [
            "pull_request",
            "issue_comment",
            "pull_request_review_thread"
        ]
    })
}

/// GET /api/setup/github/manifest-page — auto-submitting HTML form that
/// POSTs the manifest to GitHub. This is the officially documented flow:
/// https://docs.github.com/en/apps/sharing-github-apps/registering-a-github-app-from-a-manifest
#[utoipa::path(get, path = "/api/setup/github/manifest-page", tag = "setup", responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn github_manifest_page(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Query(params): Query<ManifestPageQuery>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    use axum::http::{header, StatusCode};
    use axum::response::Html;

    let public_url = resolve_public_url(&state, &headers).await;
    let manifest = build_manifest(&public_url);

    // A private app can only be installed on the account that owns it, so an
    // org install needs the org-scoped creation endpoint.
    let action_url = match params.org.as_deref().map(str::trim) {
        Some(org) if valid_org_login(org) => {
            format!("https://github.com/organizations/{org}/settings/apps/new")
        }
        _ => "https://github.com/settings/apps/new".to_string(),
    };

    let manifest_json = serde_json::to_string(&manifest)
        .map_err(|e| ApiError::internal_error(format!("Failed to serialize manifest: {e}")))?;

    // HTML page that auto-submits the manifest form to GitHub.
    // This matches GitHub's documented POST form flow exactly.
    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <title>Redirecting to GitHub…</title>
  <style>
    body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
           display: flex; justify-content: center; align-items: center;
           height: 100vh; margin: 0; background: #f6f8fa; color: #1f2328; }}
    .card {{ text-align: center; padding: 40px; background: #fff;
             border-radius: 8px; box-shadow: 0 1px 3px rgba(0,0,0,.12); }}
    .spinner {{ border: 3px solid #d0d7de; border-top-color: #0969da;
                border-radius: 50%; width: 24px; height: 24px;
                animation: spin .8s linear infinite; margin: 16px auto; }}
    @keyframes spin {{ to {{ transform: rotate(360deg); }} }}
  </style>
</head>
<body>
  <div class="card">
    <p style="font-size:14px;margin:0 0 8px">Redirecting to GitHub to create your app…</p>
    <div class="spinner"></div>
    <form id="f" method="post" action="{action_url}">
      <input type="hidden" name="manifest" value='{}'>
    </form>
  </div>
  <script>document.getElementById("f").submit();</script>
</body>
</html>"#,
        // Escape single quotes for the HTML attribute
        manifest_json.replace('\'', "&#39;"),
        action_url = action_url
    );

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            // The page auto-submits inline; the global `script-src 'self'` would block it.
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; form-action https://github.com",
            ),
        ],
        Html(html),
    ))
}

/// GET /api/setup/github/manifest — return the manifest JSON for programmatic use.
#[utoipa::path(get, path = "/api/setup/github/manifest-url", tag = "setup", responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn github_manifest_url(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let public_url = resolve_public_url(&state, &headers).await;
    Ok(Json(build_manifest(&public_url)))
}

/// POST /api/setup/github
#[utoipa::path(post, path = "/api/setup/github", tag = "setup", responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn setup_github(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<GithubBody>,
) -> Result<Json<SetupResponse>, ApiError> {
    require_setup_open_or_admin(&state, &headers).await?;

    // Validate credentials by generating an RS256 JWT and hitting the GitHub API
    let token = github_jwt::create_app_jwt(&body.app_id, &body.private_key)
        .map_err(|e| ApiError::bad_request(format!("Invalid GitHub App credentials: {e}")))?;

    let client = http_client(10)?;

    let resp = client
        .get("https://api.github.com/app")
        .header("Authorization", format!("Bearer {token}"))
        .header("User-Agent", "codasaurus")
        .send()
        .await
        .map_err(|e| ApiError::bad_request(format!("GitHub API request failed: {e}")))?;

    let test_passed = resp.status().is_success();
    if !test_passed {
        return Err(ApiError::bad_request(format!(
            "GitHub API rejected credentials (HTTP {})",
            resp.status()
        )));
    }
    let app_info: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);

    // The install button needs the app slug (`github.com/apps/<slug>/...`).
    // Prefer an explicit slug, else whatever `GET /app` returned.
    let slug = body
        .slug
        .clone()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            app_info
                .get("slug")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        });
    let app_name = app_info
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mut entries = vec![
        ("github_app_id", body.app_id.as_str()),
        ("github_private_key", body.private_key.as_str()),
        ("github_webhook_secret", body.webhook_secret.as_str()),
    ];
    if let Some(slug) = slug.as_deref() {
        entries.push(("github_app_slug", slug));
        db::config::apply_setting_to_env("github_app_slug", slug);
    }
    if let Some(name) = app_name.as_deref() {
        entries.push(("github_app_name", name));
    }
    db::config::set_configs(&state.pool, &entries).await?;

    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into());
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);
    bot::set_bot_config(bot::BotConfig {
        app_id: body.app_id.clone(),
        private_key: body.private_key.clone(),
        webhook_secret: body.webhook_secret.clone(),
        host,
        port,
    });
    bot::ensure_review_worker(state.pool.clone());

    Ok(Json(SetupResponse {
        status: "ok".into(),
        message: Some("GitHub App credentials verified and saved".into()),
        test_passed: Some(test_passed),
    }))
}

/// GET /api/setup/github/callback — lightweight HTML page that exchanges
/// the manifest code with GitHub, then redirects to the SPA entry point.
/// This avoids SPA routing issues (hash vs path) because the exchange happens
/// in a plain page before handing control back to the SPA.
#[utoipa::path(get, path = "/api/setup/github/callback", tag = "setup", responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn github_callback_page(
    Query(params): Query<GithubCallbackQuery>,
) -> impl axum::response::IntoResponse {
    use axum::http::StatusCode;
    use axum::response::Html;

    /// Static error page for the manifest callback — both messages are fixed
    /// strings, so no request input is ever reflected into the HTML.
    fn error_page(
        message: &'static str,
    ) -> (
        StatusCode,
        [(axum::http::header::HeaderName, &'static str); 2],
        Html<String>,
    ) {
        use axum::http::header;
        (
            StatusCode::BAD_REQUEST,
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (
                    header::CONTENT_SECURITY_POLICY,
                    "default-src 'none'; style-src 'unsafe-inline'",
                ),
            ],
            Html(format!(
                r#"<html><body style="font:14px sans-serif;padding:40px">
                    <p>{message}</p>
                    <a href="/#/setup/github">Back to setup</a>
                </body></html>"#
            )),
        )
    }

    let code = match params.code {
        Some(ref c) if !c.is_empty() => c.clone(),
        _ => return error_page("Missing authorization code from GitHub."),
    };

    // Reject anything that isn't a safe OAuth code charset to prevent XSS when
    // embedding into the HTML/JS bootstrap page.
    if !code
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~' | '/' | '=' | '+'))
        || code.len() > 512
    {
        return error_page("Invalid authorization code.");
    }

    // Pass code via JSON serialization so it cannot break out of a JS string.
    let code_json = serde_json::to_string(&code).unwrap_or_else(|_| "\"\"".into());

    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <title>Completing GitHub App setup…</title>
  <style>
    body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
           display: flex; justify-content: center; align-items: center;
           height: 100vh; margin: 0; background: #f6f8fa; color: #1f2328; }}
    .card {{ text-align: center; padding: 40px; background: #fff;
             border-radius: 8px; box-shadow: 0 1px 3px rgba(0,0,0,.12); max-width: 480px; }}
    .spinner {{ border: 3px solid #d0d7de; border-top-color: #0969da;
                border-radius: 50%; width: 24px; height: 24px;
                animation: spin .8s linear infinite; margin: 16px auto; }}
    @keyframes spin {{ to {{ transform: rotate(360deg); }} }}
    .error {{ color: #cf222e; }}
    .success {{ color: #1a7f37; }}
    button {{ margin-top: 16px; padding: 8px 20px; font-size: 14px;
              border-radius: 6px; border: 1px solid #d0d7de; background: #f6f8fa; cursor: pointer; }}
  </style>
</head>
<body>
  <div class="card">
    <p id="status">Exchanging code with GitHub…</p>
    <div class="spinner" id="spinner"></div>
    <div id="result"></div>
  </div>
  <script>
    (async () => {{
      try {{
        const code = {code_json};
        const r = await fetch('/api/setup/github/callback', {{
          method: 'POST',
          headers: {{ 'Content-Type': 'application/json' }},
          body: JSON.stringify({{"code": code}})
        }});
        const data = await r.json();
        if (!r.ok) throw new Error(data.error || 'Request failed');
        document.getElementById('spinner').style.display = 'none';
        document.getElementById('status').textContent = 'GitHub App created and credentials saved!';
        document.getElementById('status').className = 'success';
        const btn = document.createElement('button');
        btn.textContent = 'Return to Setup';
        btn.onclick = () => {{ window.location.href = '/#/setup/github'; }};
        document.getElementById('result').appendChild(btn);
        // Also try to update the opener tab
        if (window.opener && !window.opener.closed) {{
          window.opener.location.href = '/#/setup/github';
        }}
      }} catch (err) {{
        document.getElementById('spinner').style.display = 'none';
        document.getElementById('status').textContent = 'Error: ' + err.message;
        document.getElementById('status').className = 'error';
        const btn = document.createElement('button');
        btn.textContent = 'Back to Setup';
        btn.onclick = () => {{ window.location.href = '/#/setup/github'; }};
        document.getElementById('result').appendChild(btn);
      }}
    }})();
  </script>
</body>
</html>"#
    );

    (
        StatusCode::OK,
        [
            (
                axum::http::header::CONTENT_TYPE,
                "text/html; charset=utf-8",
            ),
            // The page exchanges the code with an inline script; the global
            // `script-src 'self'` would block it.
            (
                axum::http::header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'",
            ),
        ],
        Html(html),
    )
}

fn github_conversion_error(status: reqwest::StatusCode, upstream_body: &str) -> ApiError {
    tracing::error!("GitHub manifest conversion failed (HTTP {status}): {upstream_body}");
    ApiError::bad_request(format!("GitHub manifest conversion failed (HTTP {status})"))
}

fn http_client(secs: u64) -> Result<reqwest::Client, ApiError> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(secs))
        .build()
        .map_err(|e| ApiError::internal_error(format!("Failed to build HTTP client: {e}")))
}

/// POST /api/setup/github/callback
#[utoipa::path(post, path = "/api/setup/github/callback", tag = "setup", responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn github_callback(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<GithubCallbackBody>,
) -> Result<Json<GithubCallbackResponse>, ApiError> {
    require_setup_open_or_admin(&state, &headers).await?;

    let client = http_client(30)?;

    let resp = client
        .post(format!(
            "https://api.github.com/app-manifests/{}/conversions",
            body.code
        ))
        .header("User-Agent", "codasaurus")
        .send()
        .await
        .map_err(|e| ApiError::bad_request(format!("GitHub API request failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_else(|_| "unknown error".into());
        return Err(github_conversion_error(status, &text));
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ApiError::bad_request(format!("Invalid GitHub response: {e}")))?;

    let app_id = data["id"]
        .as_i64()
        .ok_or_else(|| ApiError::bad_request("Missing 'id' in GitHub response"))?
        .to_string();
    let pem = data["pem"]
        .as_str()
        .ok_or_else(|| ApiError::bad_request("Missing 'pem' in GitHub response"))?
        .to_string();
    let webhook_secret = data["webhook_secret"]
        .as_str()
        .ok_or_else(|| ApiError::bad_request("Missing 'webhook_secret' in GitHub response"))?
        .to_string();
    let slug = data["slug"]
        .as_str()
        .or_else(|| data["name"].as_str())
        .unwrap_or("codasaurus")
        .to_string();
    let app_name = data["name"].as_str().unwrap_or(&slug).to_string();

    db::config::set_configs(
        &state.pool,
        &[
            ("github_app_id", app_id.as_str()),
            ("github_private_key", pem.as_str()),
            ("github_webhook_secret", webhook_secret.as_str()),
            ("github_app_name", app_name.as_str()),
            ("github_app_slug", slug.as_str()),
        ],
    )
    .await?;
    db::config::apply_setting_to_env("github_app_slug", &slug);

    // Update the in-memory bot config so the webhook handler picks up the
    // new credentials immediately without requiring a server restart.
    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into());
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);
    bot::set_bot_config(bot::BotConfig {
        app_id: app_id.clone(),
        private_key: pem,
        webhook_secret: webhook_secret.clone(),
        host,
        port,
    });
    // Wizard-first installs often boot without credentials; start workers now if not yet running.
    bot::ensure_review_worker(state.pool.clone());

    let install_url = format!("https://github.com/apps/{slug}/installations/new");

    Ok(Json(GithubCallbackResponse {
        status: "ok".into(),
        app_id,
        app_name,
        slug,
        install_url,
    }))
}

fn validate_admin_body(body: &AdminBody) -> Result<(), ApiError> {
    if !body.email.contains('@') {
        return Err(ApiError::bad_request("Invalid email address"));
    }
    if let Err(msg) = db::users::validate_password_policy(&body.password, &body.email) {
        return Err(ApiError::bad_request(msg));
    }
    Ok(())
}

fn setup_db_error(err: sqlx::Error) -> ApiError {
    if let sqlx::Error::Database(ref db_err) = err {
        if db_err.message().contains("UNIQUE") {
            return ApiError::bad_request("A user with that email already exists");
        }
    }
    tracing::error!(error = %err, "database error during setup");
    ApiError::internal("Internal server error")
}

async fn first_owner_gate(
    redis: &crate::redis::StateStore,
    peer: SocketAddr,
    headers: &axum::http::HeaderMap,
    body: &AdminBody,
) -> Result<(), ApiError> {
    let key = format!(
        "bootstrap:{}",
        super::rate_limit::client_ip(peer.ip(), headers, super::rate_limit::trusted_proxies())
    );
    super::rate_limit::check(redis, &key, 5, Duration::from_secs(900))
        .await
        .map_err(|retry| {
            ApiError::too_many_requests_after(
                "Too many bootstrap attempts. Please try again later.",
                retry.as_secs().max(1),
            )
        })?;
    let secret = headers
        .get(super::bootstrap::BOOTSTRAP_SECRET_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    validate_admin_body(body)?;
    super::bootstrap::consume(redis, secret).await
}

/// POST /api/setup/admin
#[utoipa::path(post, path = "/api/setup/admin", tag = "setup", responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn setup_admin(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: axum::http::HeaderMap,
    Json(body): Json<AdminBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // Only allow first owner without auth; subsequent creates require owner.
    let owner_exists = db::users::owner_exists(&state.pool)
        .await
        .map_err(setup_db_error)?;
    if owner_exists {
        super::rbac::require_owner(&state, &headers).await?;
        validate_admin_body(&body)?;
    } else {
        first_owner_gate(&state.redis, peer, &headers, &body).await?;
    }

    let result = if owner_exists {
        // Additional owners via wizard (rare) — not bootstrap.
        db::users::create_user(&state.pool, &body.email, &body.password, "owner").await
    } else {
        // Day-zero onboarding account = instance bootstrap / superuser.
        db::users::create_first_owner(&state.pool, &body.email, &body.password).await
    };
    result.map_err(setup_db_error)?;

    Ok(Json(json!({ "status": "ok" })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::bootstrap;
    use axum::response::IntoResponse;
    use http_body_util::BodyExt;

    #[tokio::test]
    async fn invalid_first_owner_body_does_not_burn_bootstrap_secret() {
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
        let redis = crate::redis::StateStore::connect(&redis_url)
            .await
            .expect("connect redis");
        let secret = format!("bootstrap-test-{}", uuid::Uuid::new_v4());
        bootstrap::arm(&redis, &secret, Duration::from_secs(60))
            .await
            .unwrap();
        let id = uuid::Uuid::new_v4();
        let octets = id.as_bytes();
        let peer: SocketAddr = format!("10.{}.{}.{}:0", octets[0], octets[1], octets[2])
            .parse()
            .unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            bootstrap::BOOTSTRAP_SECRET_HEADER,
            axum::http::HeaderValue::from_str(&secret).unwrap(),
        );

        let bad = AdminBody {
            email: "not-an-email".into(),
            password: "password123".into(),
        };
        let err = first_owner_gate(&redis, peer, &headers, &bad)
            .await
            .unwrap_err();
        assert!(
            matches!(err, ApiError::BadRequest(_)),
            "unexpected: {err:?}"
        );

        let good = AdminBody {
            email: format!("owner-{}@example.com", uuid::Uuid::new_v4()),
            password: "password123".into(),
        };
        first_owner_gate(&redis, peer, &headers, &good)
            .await
            .expect("a rejected body must not consume the one-time bootstrap secret");
    }

    #[tokio::test]
    async fn setup_db_error_body_omits_internal_detail() {
        let err = setup_db_error(sqlx::Error::Protocol("SENTINEL_DB_DETAIL".into()));
        let body = String::from_utf8(
            err.into_response()
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert!(
            !body.contains("SENTINEL_DB_DETAIL"),
            "response leaked DB detail: {body}"
        );
        assert_eq!(body, r#"{"error":"Internal server error"}"#);
    }

    #[test]
    fn llm_step_needs_a_working_config_unless_disabled() {
        assert!(llm_step_complete(Some("disabled"), false));
        assert!(llm_step_complete(Some("DISABLED"), false));
        assert!(llm_step_complete(Some("litellm"), true));
        assert!(!llm_step_complete(Some("litellm"), false));
        assert!(!llm_step_complete(None, false));
        assert!(llm_step_complete(None, true));
    }

    #[test]
    fn failed_llm_probe_message_does_not_claim_verified() {
        assert_eq!(
            llm_saved_message(Some(true)).as_deref(),
            Some("LLM configuration saved and connection verified")
        );
        let failed = llm_saved_message(Some(false)).expect("failed probe must explain itself");
        assert!(failed.contains("CODASAURUS_MODEL"), "{failed}");
        assert!(!failed.contains("verified"), "{failed}");
        assert_eq!(llm_saved_message(None), None);
    }

    #[test]
    fn github_conversion_error_body_omits_upstream_text() {
        let err =
            github_conversion_error(reqwest::StatusCode::BAD_REQUEST, "upstream-secret-token");
        let text = format!("{err:?}");
        assert!(
            !text.contains("upstream-secret-token"),
            "error leaked upstream text: {text}"
        );
        assert!(
            text.contains("GitHub manifest conversion failed"),
            "unexpected error: {text}"
        );
    }

    #[test]
    fn upstream_model_name_is_provider_prefixed() {
        assert_eq!(
            upstream_model_name("openrouter", "liquid/lfm-3-1.7b:free"),
            "openrouter/liquid/lfm-3-1.7b:free"
        );
        assert_eq!(
            upstream_model_name("openrouter", "openrouter/foo"),
            "openrouter/foo"
        );
    }

    async fn provision_mock() -> (
        crate::llm::gateway::GatewayClient,
        std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    ) {
        use std::sync::{Arc, Mutex};

        let captured = Arc::new(Mutex::new(Vec::new()));
        let models = captured.clone();
        let app = Router::new()
            .route(
                "/public/litellm_model_cost_map",
                get(|| async {
                    Json(json!({
                        "gpt-4o": {"litellm_provider": "openai", "mode": "responses"},
                        "gpt-4o-mini": {"litellm_provider": "openai", "mode": "completion"},
                        "text-embedding-3-small": {"litellm_provider": "openai", "mode": "embedding"}
                    }))
                }),
            )
            .route(
                "/v1/models",
                get(|| async {
                    Json(json!({"data": [
                        {"id": "openai/gpt-4o"},
                        {"id": "openai/gpt-4o-mini"},
                        {"id": "openai/text-embedding-3-small"},
                        {"id": "openai/new-text-model"},
                        {"id": "openai/decision-model"}
                    ]}))
                }),
            )
            .route(
                "/model/info",
                get(|| async {
                    Json(json!({"data": [
                        {"model_name": "openai/gpt-4o", "model_info": {"mode": "responses"}},
                        {"model_name": "openai/gpt-4o-mini", "model_info": {"mode": "completion"}},
                        {"model_name": "openai/text-embedding-3-small", "model_info": {"mode": "embedding"}},
                        {"model_name": "openai/new-text-model", "model_info": {}},
                        {"model_name": "openai/decision-model", "model_info": {"mode": "decisions"}}
                    ]}))
                }),
            )
            .route(
                "/model/new",
                post(move |request: axum::extract::Request| {
                    let models = models.clone();
                    async move {
                        let bytes = axum::body::to_bytes(request.into_body(), 1 << 20)
                            .await
                            .unwrap();
                        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                        let alias = body["model_name"].as_str().unwrap();
                        let model_id = format!("id-{alias}");
                        models.lock().unwrap().push(body);
                        Json(json!({"model_id": model_id}))
                    }
                }),
            )
            .route(
                "/model/delete",
                post(|| async { Json(json!({})) }),
            )
            .route(
                "/key/generate",
                post({
                    let captured = captured.clone();
                    move |request: axum::extract::Request| {
                        let captured = captured.clone();
                        async move {
                            let bytes = axum::body::to_bytes(request.into_body(), 1 << 20)
                                .await
                                .unwrap();
                            captured.lock().unwrap().push(
                                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
                            );
                            Json(json!({"key": "sk-virtual"}))
                        }
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (
            crate::llm::gateway::GatewayClient::new(format!("http://{addr}"), "sk-master".into()),
            captured,
        )
    }

    #[tokio::test]
    async fn provision_models_accepts_non_embedding_review_roles() {
        let (client, captured) = provision_mock().await;
        let body = LlmBody {
            provider: "litellm".into(),
            api_key: Some("provider-key".into()),
            model: Some("review".into()),
            model_cheap: Some("cheap".into()),
            embedding_model: Some("embedding".into()),
            upstream_provider: Some("openai".into()),
            upstream_model: Some("gpt-4o".into()),
            upstream_model_cheap: Some("gpt-4o-mini".into()),
            upstream_model_embedding: Some("text-embedding-3-small".into()),
            api_base: None,
            model_alias: Some("review".into()),
        };

        let provisioned = provision_models(&client, &body, [None, None, None], None, 0.0)
            .await
            .unwrap();
        assert_eq!(provisioned.primary.alias, "review");
        assert_eq!(provisioned.primary.id, "id-review");
        assert_eq!(provisioned.primary.upstream, "openai/gpt-4o");
        assert_eq!(provisioned.cheap.unwrap().id, "id-cheap");
        assert_eq!(provisioned.embedding.unwrap().id, "id-embedding");
        assert_eq!(provisioned.virtual_key, "sk-virtual");

        let rows = captured.lock().unwrap();
        let key = rows.iter().find(|row| row.get("models").is_some()).unwrap();
        assert_eq!(key["models"], json!(["review", "cheap", "embedding"]));
    }

    #[tokio::test]
    async fn provision_models_accepts_discovered_model_missing_from_cost_map() {
        let (client, captured) = provision_mock().await;
        let body = LlmBody {
            provider: "litellm".into(),
            api_key: Some("provider-key".into()),
            model: Some("review".into()),
            model_cheap: None,
            embedding_model: None,
            upstream_provider: Some("openai".into()),
            upstream_model: Some("new-text-model".into()),
            upstream_model_cheap: None,
            upstream_model_embedding: None,
            api_base: None,
            model_alias: Some("review".into()),
        };

        let provisioned = provision_models(&client, &body, [None, None, None], None, 0.0)
            .await
            .unwrap();
        assert_eq!(provisioned.primary.upstream, "openai/new-text-model");
        assert_eq!(provisioned.virtual_key, "sk-virtual");

        let rows = captured.lock().unwrap();
        let key = rows.iter().find(|row| row.get("models").is_some()).unwrap();
        assert_eq!(key["models"], json!(["review"]));
    }

    #[tokio::test]
    async fn provision_rejects_blank_provider_key() {
        let body = LlmBody {
            provider: "litellm".into(),
            api_key: Some("   ".into()),
            model: None,
            model_cheap: None,
            embedding_model: None,
            upstream_provider: Some("openrouter".into()),
            upstream_model: Some("liquid/lfm-3-1.7b:free".into()),
            upstream_model_cheap: None,
            upstream_model_embedding: None,
            api_base: None,
            model_alias: Some("codasaurus-e2e".into()),
        };
        let client = crate::llm::gateway::GatewayClient::new(
            "http://127.0.0.1:1".into(),
            "sk-master".into(),
        );
        let err = provision_models(&client, &body, [None, None, None], None, 0.0)
            .await
            .unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");
    }
}
