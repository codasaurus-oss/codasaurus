//! End-to-end HTTP harness: every backend route against real Postgres + Redis.
//!
//! Run with `DATABASE_URL` and `REDIS_URL` set. All rows use unique suffixes so
//! the suite can share one database. Tests are serialized because the app sets
//! process-wide OIDC/GitHub configuration.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::LazyLock;

use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::Router;
use hmac::{Hmac, Mac};
use http_body_util::BodyExt;
use sha2::Sha256;
use tower::ServiceExt;

use codasaurus::bot::BotConfig;
use codasaurus::db;
use codasaurus::learning::store::LearningStore;
use codasaurus::learning::{LearnedRule, RuleAction};
use codasaurus::redis::StateStore;

const TEST_WEBHOOK_SECRET: &str = "full-backend-harness-webhook-secret";
const PASSWORD: &str = "password123";

static TEST_SERIAL: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));
static REQ_SEQ: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4().simple())
}

fn unique_email(prefix: &str) -> String {
    format!("{}@example.com", unique(prefix))
}

fn redis_url() -> String {
    std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into())
}

async fn test_pool() -> db::DbPool {
    let url = std::env::var("DATABASE_URL")
        .or_else(|_| std::env::var("CODASAURUS_TEST_DATABASE_URL"))
        .unwrap_or_else(|_| "postgres://codasaurus:codasaurus@127.0.0.1:5432/codasaurus".into());
    db::create_pool(&url)
        .await
        .expect("create postgres test pool")
}

fn test_bot_config() -> BotConfig {
    BotConfig {
        app_id: "123456".into(),
        private_key: String::new(),
        webhook_secret: TEST_WEBHOOK_SECRET.into(),
        host: "127.0.0.1".into(),
        port: 1,
    }
}

async fn app(pool: db::DbPool) -> Router {
    let redis = StateStore::connect(&redis_url())
        .await
        .expect("connect redis test store");
    codasaurus::serve::build_router(pool, redis, Some(test_bot_config())).layer(MockConnectInfo(
        "127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap(),
    ))
}

fn req(
    method: &str,
    uri: impl AsRef<str>,
    token: Option<&str>,
    body: Option<serde_json::Value>,
) -> Request<Body> {
    let n = REQ_SEQ.fetch_add(1, Ordering::Relaxed);
    let mut builder = Request::builder().method(method).uri(uri.as_ref()).header(
        "x-forwarded-for",
        format!("10.0.{}.{}", (n >> 8) & 255, n & 255),
    );
    if let Some(token) = token {
        builder = builder.header("cookie", format!("codasaurus_session={token}"));
    }
    let body = match body {
        Some(value) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    builder.body(body).unwrap()
}

struct Resp {
    status: StatusCode,
    headers: HeaderMap,
    body: String,
}

async fn call(app: &Router, request: Request<Body>) -> Resp {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    Resp {
        status,
        headers,
        body: String::from_utf8_lossy(&bytes).to_string(),
    }
}

async fn assert_unauthorized(app: &Router, method: &str, uri: &str) {
    let resp = call(app, req(method, uri, None, None)).await;
    assert_eq!(
        resp.status,
        StatusCode::UNAUTHORIZED,
        "{method} {uri} -> {} {}",
        resp.status,
        resp.body
    );
    assert!(
        resp.body.contains("Authentication required"),
        "{method} {uri} body: {}",
        resp.body
    );
}

fn assert_clean_error(resp: &Resp, context: &str) {
    assert!(
        resp.status.is_client_error(),
        "{context}: unexpected status {} {}",
        resp.status,
        resp.body
    );
    let lower = resp.body.to_ascii_lowercase();
    assert!(
        !lower.contains("panicked") && !lower.contains("stack backtrace"),
        "{context}: stack trace leaked: {}",
        resp.body
    );
    assert!(
        !lower.contains("internal server error"),
        "{context}: internal detail leaked: {}",
        resp.body
    );
}

async fn seed_user(pool: &db::DbPool, role: &str) -> (String, String, i64) {
    let email = unique_email(role);
    let user = db::users::create_user(pool, &email, PASSWORD, role)
        .await
        .unwrap();
    let token = db::sessions::create_session(pool, &email).await.unwrap();
    (email, token, user.id)
}

async fn seed_repo(pool: &db::DbPool) -> db::models::Repo {
    let name = unique("repo");
    db::repos::create_repo(
        pool,
        &db::models::RepoCreate {
            github_id: Some((uuid::Uuid::new_v4().as_u128() & 0x7fff_ffff_ffff_ffff) as i64),
            full_name: format!("acme/{name}"),
            owner: "acme".into(),
            name,
            default_branch: Some("main".into()),
            installation_id: 42,
            private: false,
        },
    )
    .await
    .unwrap()
}

async fn seed_review(pool: &db::DbPool, repo_id: i64) -> db::models::Review {
    db::reviews::create_review(
        pool,
        &db::models::ReviewCreate {
            repo_id,
            pr_number: REQ_SEQ.fetch_add(1, Ordering::Relaxed) as i64 + 10_000,
            pr_title: Some("Add feature".into()),
            pr_author: Some("dev".into()),
            pr_base_branch: Some("main".into()),
            pr_head_branch: Some("feature".into()),
            pr_head_sha: Some(uuid::Uuid::new_v4().simple().to_string()),
        },
    )
    .await
    .unwrap()
}

async fn seed_rule(pool: &db::DbPool, status: &str) -> String {
    let id = unique("rule");
    let rule = LearnedRule {
        id: id.clone(),
        detector: "secrets".into(),
        file_pattern: None,
        message_pattern: None,
        action: RuleAction::Ignore,
        reason: "harness fixture".into(),
        created_at: chrono::Utc::now(),
        repo_full_name: None,
        status: status.into(),
        source_count: 3,
        match_count: 0,
        last_matched_at: None,
    };
    LearningStore::from_pool(pool)
        .add_rule_async(&rule)
        .await
        .unwrap();
    id
}

async fn scalar_int(pool: &db::DbPool, sql: &str, value: &str) -> i64 {
    let prepared = pool.prepare_sql(sql);
    sqlx::query_scalar::<_, i64>(&prepared)
        .bind(value)
        .fetch_one(pool.as_pg())
        .await
        .unwrap()
}

async fn scalar_text(pool: &db::DbPool, sql: &str, value: &str) -> String {
    let prepared = pool.prepare_sql(sql);
    sqlx::query_scalar::<_, String>(&prepared)
        .bind(value)
        .fetch_one(pool.as_pg())
        .await
        .unwrap()
}

#[tokio::test]
async fn qa_pagination_and_filter_edges() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = test_pool().await;
    let (_email, token, _id) = seed_user(&pool, "maintainer").await;
    let app = app(pool).await;

    for (uri, expected) in [
        ("/api/reviews?limit=-5&offset=-1", "\"limit\":1"),
        ("/api/reviews?limit=0", "\"limit\":1"),
        ("/api/reviews?limit=100000", "\"limit\":200"),
        ("/api/reviews?status=definitely-not-a-status", "\"total\":0"),
        ("/api/audit?page=0&per_page=0", "\"limit\":1"),
        ("/api/audit?page=-3&limit=-9", "\"limit\":1"),
    ] {
        let resp = call(&app, req("GET", uri, Some(&token), None)).await;
        assert!(
            resp.status.is_success(),
            "{uri} -> {} {}",
            resp.status,
            resp.body
        );
        assert!(
            resp.body.contains(expected),
            "{uri}: expected {expected} in {}",
            resp.body
        );
    }
}

#[tokio::test]
async fn qa_malformed_inputs_are_rejected_without_leaks() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = test_pool().await;
    let app = app(pool).await;

    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/api/auth/login")
        .header("content-type", "application/json")
        .body(axum::body::Body::from("{not json"))
        .unwrap();
    let resp = call(&app, request).await;
    assert!(
        resp.status.is_client_error(),
        "malformed JSON -> {} {}",
        resp.status,
        resp.body
    );
    assert_clean_error(&resp, "malformed login JSON");

    let missing = call(&app, req("GET", "/api/reviews/not-a-number", None, None)).await;
    assert!(
        missing.status.is_client_error(),
        "bad id -> {} {}",
        missing.status,
        missing.body
    );
    assert_clean_error(&missing, "non numeric review id");
}

#[tokio::test]
async fn qa_nonexistent_resources_return_not_found() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = test_pool().await;
    let (_email, token, _id) = seed_user(&pool, "owner").await;
    let app = app(pool).await;

    for uri in ["/api/reviews/999999999", "/api/repos/999999999"] {
        let resp = call(&app, req("GET", uri, Some(&token), None)).await;
        assert_eq!(resp.status, StatusCode::NOT_FOUND, "{uri} -> {}", resp.body);
        assert_clean_error(&resp, uri);
    }
    let resp = call(
        &app,
        req("DELETE", "/api/users/999999999", Some(&token), None),
    )
    .await;
    assert_eq!(resp.status, StatusCode::NOT_FOUND, "{}", resp.body);
    assert_clean_error(&resp, "missing user");
}

#[tokio::test]
async fn qa_webhook_edge_cases() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = test_pool().await;
    let app = app(pool).await;
    let body = serde_json::json!({"action": "opened"}).to_string();

    let signed = |delivery: &str, secret: &str| {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body.as_bytes());
        let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        axum::http::Request::builder()
            .method("POST")
            .uri("/webhook")
            .header("content-type", "application/json")
            .header("x-github-event", "pull_request")
            .header("x-github-delivery", delivery)
            .header("x-hub-signature-256", sig)
            .body(axum::body::Body::from(body.clone()))
            .unwrap()
    };

    let bad = signed("qa-bad-signature", "wrong-secret");
    let resp = call(&app, bad).await;
    assert_eq!(resp.status, StatusCode::UNAUTHORIZED, "{}", resp.body);

    let mut no_delivery = signed("qa-no-delivery", TEST_WEBHOOK_SECRET);
    no_delivery.headers_mut().remove("x-github-delivery");
    let resp = call(&app, no_delivery).await;
    assert_eq!(resp.status, StatusCode::BAD_REQUEST, "{}", resp.body);

    let ping_body = serde_json::json!({"zen": "ok"}).to_string();
    let mut mac = Hmac::<Sha256>::new_from_slice(TEST_WEBHOOK_SECRET.as_bytes()).unwrap();
    mac.update(ping_body.as_bytes());
    let ping = axum::http::Request::builder()
        .method("POST")
        .uri("/webhook")
        .header("content-type", "application/json")
        .header("x-github-event", "ping")
        .header(
            "x-github-delivery",
            format!("qa-ping-{}", uuid::Uuid::new_v4().simple()),
        )
        .header(
            "x-hub-signature-256",
            format!("sha256={}", hex::encode(mac.finalize().into_bytes())),
        )
        .body(axum::body::Body::from(ping_body))
        .unwrap();
    let resp = call(&app, ping).await;
    assert_eq!(resp.status, StatusCode::OK, "{}", resp.body);
    assert!(resp.body.contains("ping"), "{}", resp.body);
}

#[tokio::test]
async fn public_endpoints_health_docs_branding() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = test_pool().await;
    let app = app(pool).await;

    let health = call(&app, req("GET", "/health", None, None)).await;
    assert_eq!(health.status, StatusCode::OK, "{}", health.body);
    assert!(health.body.contains("\"status\":\"ok\""), "{}", health.body);
    assert!(health.body.contains("\"db\":true"), "{}", health.body);

    let ready = call(&app, req("GET", "/health/ready", None, None)).await;
    assert_eq!(ready.status, StatusCode::OK, "{}", ready.body);
    assert!(ready.body.contains("\"ready\":true"), "{}", ready.body);

    let mut docs = call(&app, req("GET", "/api/docs", None, None)).await;
    if docs.status.is_redirection() {
        let location = docs
            .headers
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        docs = call(&app, req("GET", location, None, None)).await;
    }
    assert_eq!(docs.status, StatusCode::OK, "{}", docs.body);
    assert!(
        docs.body.to_ascii_lowercase().contains("swagger"),
        "{}",
        docs.body
    );

    let openapi = call(&app, req("GET", "/api/openapi.json", None, None)).await;
    assert_eq!(openapi.status, StatusCode::OK, "{}", openapi.body);
    assert!(openapi.body.contains("Codasaurus API"), "{}", openapi.body);
    assert!(openapi.body.contains("/api/repos"), "{}", openapi.body);

    let logo = call(&app, req("GET", "/branding/logo.png", None, None)).await;
    assert_eq!(logo.status, StatusCode::OK, "{}", logo.body);
    assert!(
        logo.headers
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("image"),
        "{:?}",
        logo.headers
    );

    let banner = call(&app, req("GET", "/branding/review-banner.svg", None, None)).await;
    assert_eq!(banner.status, StatusCode::OK, "{}", banner.body);
    assert!(banner.body.contains("<svg"), "{}", banner.body);
}

#[tokio::test]
async fn public_endpoints_spa_fallback_and_api_404() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = test_pool().await;
    let app = app(pool).await;

    let spa = call(&app, req("GET", "/some/unknown/path", None, None)).await;
    assert_eq!(spa.status, StatusCode::NOT_FOUND, "{}", spa.body);

    let api_404 = call(&app, req("GET", "/api/some/unknown/path", None, None)).await;
    assert_eq!(api_404.status, StatusCode::NOT_FOUND, "{}", api_404.body);
    assert!(
        api_404.body.contains("\"error\":\"not_found\""),
        "{}",
        api_404.body
    );
}

#[tokio::test]
async fn public_endpoints_metrics_token_gate() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = test_pool().await;
    let app = app(pool).await;

    std::env::remove_var("CODASAURUS_METRICS_TOKEN");
    let disabled = call(&app, req("GET", "/metrics", None, None)).await;
    assert_eq!(disabled.status, StatusCode::NOT_FOUND, "{}", disabled.body);

    let token = unique("metrics-token");
    std::env::set_var("CODASAURUS_METRICS_TOKEN", &token);
    let no_auth = call(&app, req("GET", "/metrics", None, None)).await;
    assert_eq!(no_auth.status, StatusCode::UNAUTHORIZED, "{}", no_auth.body);

    let mut authorized = req("GET", "/metrics", None, None);
    authorized.headers_mut().insert(
        header::AUTHORIZATION,
        format!("Bearer {token}").parse().unwrap(),
    );
    let ok = call(&app, authorized).await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.body);
    assert!(ok.body.contains("codasaurus_up 1"), "{}", ok.body);
    std::env::remove_var("CODASAURUS_METRICS_TOKEN");
}

#[tokio::test]
async fn public_endpoints_setup_status_and_github_manifest() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = test_pool().await;
    let app = app(pool).await;

    let status = call(&app, req("GET", "/api/setup/status", None, None)).await;
    assert_eq!(status.status, StatusCode::OK, "{}", status.body);
    for key in ["database", "llm", "github", "admin", "complete"] {
        assert!(
            status.body.contains(&format!("\"{key}\"")),
            "{}",
            status.body
        );
    }

    let page = call(
        &app,
        req("GET", "/api/setup/github/manifest-page", None, None),
    )
    .await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(
        page.body.contains("github.com/settings/apps/new"),
        "{}",
        page.body
    );

    let org_page = call(
        &app,
        req(
            "GET",
            "/api/setup/github/manifest-page?org=valid-org",
            None,
            None,
        ),
    )
    .await;
    assert_eq!(org_page.status, StatusCode::OK, "{}", org_page.body);
    assert!(
        org_page
            .body
            .contains("github.com/organizations/valid-org/settings/apps/new"),
        "{}",
        org_page.body
    );

    let manifest = call(
        &app,
        req("GET", "/api/setup/github/manifest-url", None, None),
    )
    .await;
    assert_eq!(manifest.status, StatusCode::OK, "{}", manifest.body);
    assert!(
        manifest.body.contains("hook_attributes"),
        "{}",
        manifest.body
    );
    assert!(manifest.body.contains("/webhook/"), "{}", manifest.body);

    let missing_code = call(&app, req("GET", "/api/setup/github/callback", None, None)).await;
    assert_eq!(
        missing_code.status,
        StatusCode::BAD_REQUEST,
        "{}",
        missing_code.body
    );
    assert!(
        missing_code.body.contains("Missing authorization code"),
        "{}",
        missing_code.body
    );

    let with_code = call(
        &app,
        req("GET", "/api/setup/github/callback?code=abc123", None, None),
    )
    .await;
    assert_eq!(with_code.status, StatusCode::OK, "{}", with_code.body);
    assert!(
        with_code.body.contains("Exchanging code with GitHub"),
        "{}",
        with_code.body
    );
}

#[tokio::test]
async fn setup_endpoints_database_llm_public_url() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = test_pool().await;
    let (_email, token, _id) = seed_user(&pool, "owner").await;
    let app = app(pool.clone()).await;

    assert_unauthorized(&app, "GET", "/api/setup/database").await;
    assert_unauthorized(&app, "GET", "/api/setup/llm").await;
    assert_unauthorized(&app, "GET", "/api/setup/public-url").await;

    let db_status = call(&app, req("GET", "/api/setup/database", Some(&token), None)).await;
    assert_eq!(db_status.status, StatusCode::OK, "{}", db_status.body);
    assert!(
        db_status.body.contains("\"connected\":true"),
        "{}",
        db_status.body
    );

    let db_post = call(
        &app,
        req(
            "POST",
            "/api/setup/database",
            Some(&token),
            Some(serde_json::json!({"provider": "postgres", "url": ""})),
        ),
    )
    .await;
    assert_eq!(db_post.status, StatusCode::OK, "{}", db_post.body);
    assert!(
        db_post.body.contains("Connected to PostgreSQL"),
        "{}",
        db_post.body
    );

    let old_llm = db::config::get_config(&pool, "llm_provider").await.unwrap();
    let llm_get = call(&app, req("GET", "/api/setup/llm", Some(&token), None)).await;
    assert_eq!(llm_get.status, StatusCode::OK, "{}", llm_get.body);
    assert!(llm_get.body.contains("\"provider\""), "{}", llm_get.body);

    let llm_post = call(
        &app,
        req(
            "POST",
            "/api/setup/llm",
            Some(&token),
            Some(serde_json::json!({"provider": "litellm"})),
        ),
    )
    .await;
    assert_eq!(llm_post.status, StatusCode::OK, "{}", llm_post.body);
    assert!(
        llm_post.body.contains("\"status\":\"ok\""),
        "{}",
        llm_post.body
    );
    assert_eq!(
        db::config::get_config(&pool, "llm_provider")
            .await
            .unwrap()
            .as_deref(),
        Some("litellm")
    );

    let old_url = db::config::get_config(&pool, "public_url").await.unwrap();
    let url_get = call(
        &app,
        req("GET", "/api/setup/public-url", Some(&token), None),
    )
    .await;
    assert_eq!(url_get.status, StatusCode::OK, "{}", url_get.body);
    assert!(url_get.body.contains("\"url\""), "{}", url_get.body);

    let url_post = call(
        &app,
        req(
            "POST",
            "/api/setup/public-url",
            Some(&token),
            Some(serde_json::json!({"url": "http://localhost:3000"})),
        ),
    )
    .await;
    assert_eq!(url_post.status, StatusCode::OK, "{}", url_post.body);
    assert_eq!(
        db::config::get_config(&pool, "public_url")
            .await
            .unwrap()
            .as_deref(),
        Some("http://localhost:3000")
    );

    match old_llm {
        Some(v) => db::config::set_config(&pool, "llm_provider", &v)
            .await
            .unwrap(),
        None => db::config::delete_config(&pool, "llm_provider")
            .await
            .unwrap(),
    }
    match old_url {
        Some(v) => db::config::set_config(&pool, "public_url", &v)
            .await
            .unwrap(),
        None => db::config::delete_config(&pool, "public_url")
            .await
            .unwrap(),
    }
}

#[tokio::test]
async fn setup_endpoints_github_and_admin_freeze() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = test_pool().await;
    let (_email, token, _id) = seed_user(&pool, "owner").await;
    let app = app(pool.clone()).await;

    let setup_no_auth = call(
        &app,
        req(
            "POST",
            "/api/setup/github",
            None,
            Some(serde_json::json!({
                "app_id": "123456",
                "private_key": "not-a-pem",
                "webhook_secret": "secret"
            })),
        ),
    )
    .await;
    assert_eq!(
        setup_no_auth.status,
        StatusCode::UNAUTHORIZED,
        "{}",
        setup_no_auth.body
    );
    assert!(
        setup_no_auth.body.contains("Authentication required"),
        "{}",
        setup_no_auth.body
    );

    let bad_key = call(
        &app,
        req(
            "POST",
            "/api/setup/github",
            Some(&token),
            Some(serde_json::json!({
                "app_id": "123456",
                "private_key": "not-a-pem",
                "webhook_secret": "secret"
            })),
        ),
    )
    .await;
    assert_clean_error(&bad_key, "POST /api/setup/github");
    assert!(
        bad_key.body.contains("Invalid GitHub App credentials"),
        "{}",
        bad_key.body
    );

    let new_admin = unique_email("wizard-admin");
    let no_auth = call(
        &app,
        req(
            "POST",
            "/api/setup/admin",
            None,
            Some(serde_json::json!({"email": new_admin, "password": PASSWORD})),
        ),
    )
    .await;
    assert_eq!(no_auth.status, StatusCode::UNAUTHORIZED, "{}", no_auth.body);

    let with_owner = call(
        &app,
        req(
            "POST",
            "/api/setup/admin",
            Some(&token),
            Some(serde_json::json!({"email": new_admin, "password": PASSWORD})),
        ),
    )
    .await;
    assert_eq!(with_owner.status, StatusCode::OK, "{}", with_owner.body);
    assert!(
        with_owner.body.contains("\"status\":\"ok\""),
        "{}",
        with_owner.body
    );
    assert!(
        db::users::get_user_by_email(&pool, &new_admin)
            .await
            .unwrap()
            .is_some(),
        "wizard-created owner was not persisted"
    );
}

#[tokio::test]
async fn auth_endpoints_login_me_logout() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = test_pool().await;
    let (email, _token, _id) = seed_user(&pool, "viewer").await;
    let app = app(pool).await;

    let wrong = call(
        &app,
        req(
            "POST",
            "/api/auth/login",
            None,
            Some(serde_json::json!({"email": email, "password": "wrongpassword"})),
        ),
    )
    .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED, "{}", wrong.body);
    assert!(
        wrong.body.contains("Invalid email or password"),
        "{}",
        wrong.body
    );

    let login = call(
        &app,
        req(
            "POST",
            "/api/auth/login",
            None,
            Some(serde_json::json!({"email": email, "password": PASSWORD})),
        ),
    )
    .await;
    assert_eq!(login.status, StatusCode::OK, "{}", login.body);
    assert!(login.body.contains(&email), "{}", login.body);
    assert!(login.body.contains("\"role\":\"viewer\""), "{}", login.body);
    let set_cookie = login
        .headers
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(set_cookie.contains("codasaurus_session="), "{set_cookie}");
    let session = set_cookie
        .split(';')
        .next()
        .unwrap()
        .trim_start_matches("codasaurus_session=");

    let anonymous = call(&app, req("GET", "/api/auth/me", None, None)).await;
    assert_eq!(anonymous.status, StatusCode::OK, "{}", anonymous.body);
    assert!(
        anonymous.body.contains("\"authenticated\":false"),
        "{}",
        anonymous.body
    );

    let me = call(&app, req("GET", "/api/auth/me", Some(session), None)).await;
    assert_eq!(me.status, StatusCode::OK, "{}", me.body);
    assert!(me.body.contains("\"authenticated\":true"), "{}", me.body);
    assert!(me.body.contains(&email), "{}", me.body);

    let logout = call(&app, req("POST", "/api/auth/logout", Some(session), None)).await;
    assert_eq!(logout.status, StatusCode::OK, "{}", logout.body);
    assert!(logout.body.contains("\"status\":\"ok\""), "{}", logout.body);

    let after = call(&app, req("GET", "/api/auth/me", Some(session), None)).await;
    assert!(
        after.body.contains("\"authenticated\":false"),
        "{}",
        after.body
    );
}

#[tokio::test]
async fn auth_endpoints_oidc_login_callback_and_status() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = test_pool().await;
    let redis = StateStore::connect(&redis_url()).await.unwrap();
    let app = app(pool).await;

    let status = call(&app, req("GET", "/api/auth/oidc/status", None, None)).await;
    assert_eq!(status.status, StatusCode::OK, "{}", status.body);
    assert!(status.body.contains("\"enabled\""), "{}", status.body);

    let invalid_state = call(
        &app,
        req(
            "GET",
            "/api/auth/oidc/callback?code=abc&state=not-a-real-state",
            None,
            None,
        ),
    )
    .await;
    assert_eq!(
        invalid_state.status,
        StatusCode::BAD_REQUEST,
        "{}",
        invalid_state.body
    );
    assert!(
        invalid_state.body.contains("invalid or expired OIDC state"),
        "{}",
        invalid_state.body
    );

    let discovery = Router::new().route(
        "/.well-known/openid-configuration",
        axum::routing::get(|| async {
            axum::Json(serde_json::json!({
                "authorization_endpoint": "https://idp.example.com/authorize",
                "token_endpoint": "https://idp.example.com/token",
                "jwks_uri": "https://idp.example.com/jwks"
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, discovery).await.unwrap();
    });

    std::env::set_var("OIDC_ISSUER", format!("http://{addr}"));
    std::env::set_var("OIDC_CLIENT_ID", unique("oidc-client"));
    std::env::set_var("OIDC_CLIENT_SECRET", "harness-secret");

    let login = call(&app, req("GET", "/api/auth/oidc/login", None, None)).await;
    assert_eq!(
        login.status,
        StatusCode::TEMPORARY_REDIRECT,
        "{}",
        login.body
    );
    let location = login
        .headers
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(location.contains("state="), "{location}");
    let state = location
        .split("state=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap();
    assert!(
        !state.is_empty() && state != "not-a-real-state",
        "unexpected state: {state}"
    );
    let pending = redis
        .getdel(&format!("oidc:state:{state}"))
        .await
        .unwrap()
        .expect("OIDC state must be stored in Redis");
    assert!(!pending.is_empty(), "empty PKCE verifier in Redis");

    std::env::remove_var("OIDC_ISSUER");
    std::env::remove_var("OIDC_CLIENT_ID");
    std::env::remove_var("OIDC_CLIENT_SECRET");
    server.abort();
}

#[tokio::test]
async fn auth_endpoints_invite_info_and_invalid_token() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = test_pool().await;
    let (owner_email, _token, _id) = seed_user(&pool, "owner").await;
    let guest = unique_email("guest");
    let (_invite, raw) = db::invites::create_invite(&pool, Some(&guest), "viewer", &owner_email, 7)
        .await
        .unwrap();
    let app = app(pool.clone()).await;

    let info = call(
        &app,
        req("GET", format!("/api/auth/invite/{raw}"), None, None),
    )
    .await;
    assert_eq!(info.status, StatusCode::OK, "{}", info.body);
    assert!(info.body.contains(&guest), "{}", info.body);
    assert!(info.body.contains("\"role\":\"viewer\""), "{}", info.body);

    let missing = call(&app, req("GET", "/api/auth/invite/not-a-token", None, None)).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND, "{}", missing.body);
    assert!(
        missing.body.contains("Invite not found or expired"),
        "{}",
        missing.body
    );

    let accept = call(
        &app,
        req(
            "POST",
            "/api/auth/invite/not-a-token/accept",
            None,
            Some(serde_json::json!({"password": PASSWORD})),
        ),
    )
    .await;
    assert_eq!(accept.status, StatusCode::NOT_FOUND, "{}", accept.body);
    assert!(
        accept.body.contains("Invite not found or expired"),
        "{}",
        accept.body
    );
    assert!(
        db::invites::get_pending_by_token(&pool, &raw)
            .await
            .unwrap()
            .is_some(),
        "fixture invite vanished after invalid accept"
    );
}

#[tokio::test]
async fn repo_endpoints_list_detail_update_sync_and_delete() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = test_pool().await;
    let (_owner, owner_token, _) = seed_user(&pool, "owner").await;
    let (_maint, maint_token, _) = seed_user(&pool, "maintainer").await;
    let (_viewer, viewer_token, _) = seed_user(&pool, "viewer").await;
    let repo = seed_repo(&pool).await;
    let app = app(pool.clone()).await;

    assert_unauthorized(&app, "GET", "/api/repos").await;
    assert_unauthorized(&app, "POST", "/api/repos/sync").await;

    let list = call(&app, req("GET", "/api/repos", Some(&viewer_token), None)).await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.body);
    assert!(list.body.contains(&repo.full_name), "{}", list.body);

    let detail = call(
        &app,
        req(
            "GET",
            format!("/api/repos/{}", repo.id),
            Some(&viewer_token),
            None,
        ),
    )
    .await;
    assert_eq!(detail.status, StatusCode::OK, "{}", detail.body);
    assert!(detail.body.contains(&repo.full_name), "{}", detail.body);

    let missing = call(
        &app,
        req("GET", "/api/repos/999999999", Some(&viewer_token), None),
    )
    .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND, "{}", missing.body);

    let forbidden = call(
        &app,
        req(
            "PUT",
            format!("/api/repos/{}", repo.id),
            Some(&viewer_token),
            Some(serde_json::json!({"config_json": "{}", "active": true})),
        ),
    )
    .await;
    assert_eq!(
        forbidden.status,
        StatusCode::FORBIDDEN,
        "{}",
        forbidden.body
    );

    let update = call(
        &app,
        req(
            "PUT",
            format!("/api/repos/{}", repo.id),
            Some(&maint_token),
            Some(serde_json::json!({"config_json": "{\"review_strictness\":\"strict\"}", "active": true})),
        ),
    )
    .await;
    assert_eq!(update.status, StatusCode::OK, "{}", update.body);
    assert!(update.body.contains("\"status\":\"ok\""), "{}", update.body);
    let updated = db::repos::get_repo(&pool, repo.id).await.unwrap().unwrap();
    assert_eq!(
        updated.config_json.as_deref(),
        Some("{\"review_strictness\":\"strict\"}")
    );

    let github_keys = ["github_app_id", "github_private_key", "github_app_slug"];
    let mut saved = Vec::new();
    for key in github_keys {
        saved.push((key, db::config::get_config(&pool, key).await.unwrap()));
        db::config::delete_config(&pool, key).await.unwrap();
    }
    let sync = call(
        &app,
        req("POST", "/api/repos/sync", Some(&owner_token), None),
    )
    .await;
    for (key, value) in saved {
        match value {
            Some(value) => db::config::set_config(&pool, key, &value).await.unwrap(),
            None => db::config::delete_config(&pool, key).await.unwrap(),
        }
    }
    assert_clean_error(&sync, "POST /api/repos/sync");
    assert!(
        sync.body.contains("No GitHub App configured"),
        "{}",
        sync.body
    );

    let doomed = seed_repo(&pool).await;
    let delete = call(
        &app,
        req(
            "DELETE",
            format!("/api/repos/{}", doomed.id),
            Some(&maint_token),
            None,
        ),
    )
    .await;
    assert_eq!(delete.status, StatusCode::OK, "{}", delete.body);
    assert!(db::repos::get_repo(&pool, doomed.id)
        .await
        .unwrap()
        .is_none());
    let gone = call(
        &app,
        req(
            "GET",
            format!("/api/repos/{}", doomed.id),
            Some(&maint_token),
            None,
        ),
    )
    .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND, "{}", gone.body);
}

#[tokio::test]
async fn review_endpoints_list_detail_findings_sarif_and_dismiss() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = test_pool().await;
    let (_maint, maint_token, _) = seed_user(&pool, "maintainer").await;
    let (_viewer, viewer_token, _) = seed_user(&pool, "viewer").await;
    let repo = seed_repo(&pool).await;
    let review = seed_review(&pool, repo.id).await;
    let app = app(pool.clone()).await;

    assert_unauthorized(&app, "GET", "/api/reviews").await;
    assert_unauthorized(&app, "GET", &format!("/api/reviews/{}", review.id)).await;
    assert_unauthorized(&app, "POST", "/api/reviews/dismiss").await;

    let list = call(&app, req("GET", "/api/reviews", Some(&viewer_token), None)).await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.body);
    for key in ["reviews", "total", "total_pages", "limit", "offset"] {
        assert!(list.body.contains(&format!("\"{key}\"")), "{}", list.body);
    }

    let detail = call(
        &app,
        req(
            "GET",
            format!("/api/reviews/{}", review.id),
            Some(&viewer_token),
            None,
        ),
    )
    .await;
    assert_eq!(detail.status, StatusCode::OK, "{}", detail.body);
    for key in ["review", "findings", "summary", "finding_count"] {
        assert!(
            detail.body.contains(&format!("\"{key}\"")),
            "{}",
            detail.body
        );
    }

    let findings = call(
        &app,
        req(
            "GET",
            format!("/api/reviews/{}/findings", review.id),
            Some(&viewer_token),
            None,
        ),
    )
    .await;
    assert_eq!(findings.status, StatusCode::OK, "{}", findings.body);
    assert!(
        findings.body.contains("\"findings_by_file\""),
        "{}",
        findings.body
    );
    assert!(
        findings.body.contains("\"total_findings\""),
        "{}",
        findings.body
    );

    let sarif = call(
        &app,
        req(
            "GET",
            format!("/api/reviews/{}/sarif", review.id),
            Some(&viewer_token),
            None,
        ),
    )
    .await;
    assert_eq!(sarif.status, StatusCode::OK, "{}", sarif.body);
    assert_eq!(
        sarif
            .headers
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "application/sarif+json"
    );
    assert!(sarif.body.contains("\"runs\""), "{}", sarif.body);

    let fingerprint = unique("fingerprint");
    let denied = call(
        &app,
        req(
            "POST",
            "/api/reviews/dismiss",
            Some(&viewer_token),
            Some(serde_json::json!({"fingerprint": fingerprint})),
        ),
    )
    .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN, "{}", denied.body);

    let short = call(
        &app,
        req(
            "POST",
            "/api/reviews/dismiss",
            Some(&maint_token),
            Some(serde_json::json!({"fingerprint": "short"})),
        ),
    )
    .await;
    assert_eq!(short.status, StatusCode::BAD_REQUEST, "{}", short.body);
    assert!(
        short.body.contains("fingerprint too short"),
        "{}",
        short.body
    );

    let dismiss = call(
        &app,
        req(
            "POST",
            "/api/reviews/dismiss",
            Some(&maint_token),
            Some(serde_json::json!({"fingerprint": fingerprint, "detector": "secrets"})),
        ),
    )
    .await;
    assert_eq!(dismiss.status, StatusCode::OK, "{}", dismiss.body);
    assert!(
        dismiss.body.contains("\"status\":\"ok\""),
        "{}",
        dismiss.body
    );
    assert_eq!(
        scalar_int(
            &pool,
            "SELECT COUNT(*) FROM dismissed_findings WHERE fingerprint = ?",
            &fingerprint
        )
        .await,
        1
    );
}

#[tokio::test]
async fn settings_endpoints_read_update_and_connection_tests() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = test_pool().await;
    let (_owner, owner_token, _) = seed_user(&pool, "owner").await;
    let (_maint, maint_token, _) = seed_user(&pool, "maintainer").await;
    let (_viewer, viewer_token, _) = seed_user(&pool, "viewer").await;
    let app = app(pool.clone()).await;

    assert_unauthorized(&app, "GET", "/api/settings").await;
    assert_unauthorized(&app, "GET", "/api/settings/github").await;
    assert_unauthorized(&app, "POST", "/api/settings/jira/test").await;

    let viewer = call(&app, req("GET", "/api/settings", Some(&viewer_token), None)).await;
    assert_eq!(viewer.status, StatusCode::FORBIDDEN, "{}", viewer.body);

    let settings = call(&app, req("GET", "/api/settings", Some(&maint_token), None)).await;
    assert_eq!(settings.status, StatusCode::OK, "{}", settings.body);
    assert!(
        settings.body.contains("\"offline_mode_effective\""),
        "{}",
        settings.body
    );
    assert!(
        !settings.body.contains("POSTGRES_PASSWORD"),
        "{}",
        settings.body
    );

    let github_settings = call(
        &app,
        req("GET", "/api/settings/github", Some(&maint_token), None),
    )
    .await;
    assert_eq!(
        github_settings.status,
        StatusCode::OK,
        "{}",
        github_settings.body
    );
    assert!(
        github_settings.body.contains("\"configured\""),
        "{}",
        github_settings.body
    );

    let key = "max_warnings";
    let old_value = db::config::get_config(&pool, key).await.unwrap();
    let viewer_write = call(
        &app,
        req(
            "PUT",
            format!("/api/settings/{key}"),
            Some(&viewer_token),
            Some(serde_json::json!({"value": "9"})),
        ),
    )
    .await;
    assert_eq!(
        viewer_write.status,
        StatusCode::FORBIDDEN,
        "{}",
        viewer_write.body
    );

    let write = call(
        &app,
        req(
            "PUT",
            format!("/api/settings/{key}"),
            Some(&owner_token),
            Some(serde_json::json!({"value": "9"})),
        ),
    )
    .await;
    assert_eq!(write.status, StatusCode::OK, "{}", write.body);
    assert!(write.body.contains("\"status\":\"ok\""), "{}", write.body);
    assert_eq!(
        db::config::get_config(&pool, key).await.unwrap().as_deref(),
        Some("9")
    );

    let unknown = call(
        &app,
        req(
            "PUT",
            format!("/api/settings/{}", unique("bogus_key")),
            Some(&owner_token),
            Some(serde_json::json!({"value": "9"})),
        ),
    )
    .await;
    assert_eq!(unknown.status, StatusCode::BAD_REQUEST, "{}", unknown.body);
    assert!(unknown.body.contains("Unknown setting"), "{}", unknown.body);

    let github_test = call(
        &app,
        req(
            "POST",
            "/api/settings/github/test",
            Some(&maint_token),
            None,
        ),
    )
    .await;
    assert_clean_error(&github_test, "POST /api/settings/github/test");

    db::config::set_config(&pool, "oidc_issuer", "http://127.0.0.1:1")
        .await
        .unwrap();
    let oidc_test = call(
        &app,
        req("POST", "/api/settings/oidc/test", Some(&maint_token), None),
    )
    .await;
    assert_clean_error(&oidc_test, "POST /api/settings/oidc/test");

    for (key, value) in [
        ("jira_base_url", "http://127.0.0.1:1"),
        ("jira_email", "dev@example.com"),
        ("jira_api_token", "token"),
    ] {
        db::config::set_config(&pool, key, value).await.unwrap();
    }
    let jira_test = call(
        &app,
        req("POST", "/api/settings/jira/test", Some(&maint_token), None),
    )
    .await;
    assert_clean_error(&jira_test, "POST /api/settings/jira/test");

    let old_linear = db::config::get_config(&pool, "linear_api_key")
        .await
        .unwrap();
    if old_linear.is_some() {
        db::config::set_config(&pool, "linear_api_key", "")
            .await
            .unwrap();
    }
    let linear_test = call(
        &app,
        req(
            "POST",
            "/api/settings/linear/test",
            Some(&maint_token),
            None,
        ),
    )
    .await;
    assert_clean_error(&linear_test, "POST /api/settings/linear/test");
    assert!(
        linear_test
            .body
            .contains("Linear API key is not configured"),
        "{}",
        linear_test.body
    );

    match old_value {
        Some(v) => db::config::set_config(&pool, key, &v).await.unwrap(),
        None => db::config::delete_config(&pool, key).await.unwrap(),
    }
    db::config::delete_config(&pool, "oidc_issuer")
        .await
        .unwrap();
    for key in ["jira_base_url", "jira_email", "jira_api_token"] {
        db::config::delete_config(&pool, key).await.unwrap();
    }
    if let Some(v) = old_linear {
        db::config::set_config(&pool, "linear_api_key", &v)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn settings_endpoints_destructive_connection_clears() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = test_pool().await;
    let (_owner, owner_token, _) = seed_user(&pool, "owner").await;
    let (_viewer, viewer_token, _) = seed_user(&pool, "viewer").await;
    let app = app(pool.clone()).await;

    for (key, value) in [
        ("github_app_id", unique("app-id")),
        ("github_webhook_secret", unique("secret")),
        ("oidc_client_id", unique("oidc-id")),
        ("oidc_client_secret", unique("oidc-secret")),
        ("jira_api_token", unique("jira-token")),
        ("linear_api_key", unique("linear-key")),
    ] {
        db::config::set_config(&pool, key, &value).await.unwrap();
    }

    assert_unauthorized(&app, "DELETE", "/api/settings/github").await;
    let denied = call(
        &app,
        req("DELETE", "/api/settings/oidc", Some(&viewer_token), None),
    )
    .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN, "{}", denied.body);

    let github = call(
        &app,
        req("DELETE", "/api/settings/github", Some(&owner_token), None),
    )
    .await;
    assert_eq!(github.status, StatusCode::OK, "{}", github.body);
    assert!(github.body.contains("config cleared"), "{}", github.body);
    assert!(db::config::get_config(&pool, "github_app_id")
        .await
        .unwrap()
        .is_none());
    assert!(db::config::get_config(&pool, "github_webhook_secret")
        .await
        .unwrap()
        .is_none());

    let oidc = call(
        &app,
        req("DELETE", "/api/settings/oidc", Some(&owner_token), None),
    )
    .await;
    assert_eq!(oidc.status, StatusCode::OK, "{}", oidc.body);
    assert!(oidc.body.contains("cleared"), "{}", oidc.body);
    assert!(db::config::get_config(&pool, "oidc_client_id")
        .await
        .unwrap()
        .is_none());

    let tickets = call(
        &app,
        req("DELETE", "/api/settings/tickets", Some(&owner_token), None),
    )
    .await;
    assert_eq!(tickets.status, StatusCode::OK, "{}", tickets.body);
    assert!(db::config::get_config(&pool, "jira_api_token")
        .await
        .unwrap()
        .is_none());
    assert!(db::config::get_config(&pool, "linear_api_key")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn learning_endpoints_rule_lifecycle() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = test_pool().await;
    let (_maint, maint_token, _) = seed_user(&pool, "maintainer").await;
    let (_viewer, viewer_token, _) = seed_user(&pool, "viewer").await;
    let rule_id = seed_rule(&pool, "suggested").await;
    let app = app(pool.clone()).await;

    assert_unauthorized(&app, "GET", "/api/learning/rules").await;

    let list = call(
        &app,
        req("GET", "/api/learning/rules", Some(&maint_token), None),
    )
    .await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.body);
    assert!(list.body.contains(&rule_id), "{}", list.body);
    assert!(list.body.contains("\"rules\""), "{}", list.body);

    let denied = call(
        &app,
        req(
            "POST",
            format!("/api/learning/rules/{rule_id}/approve"),
            Some(&viewer_token),
            None,
        ),
    )
    .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN, "{}", denied.body);

    let approve = call(
        &app,
        req(
            "POST",
            format!("/api/learning/rules/{rule_id}/approve"),
            Some(&maint_token),
            None,
        ),
    )
    .await;
    assert_eq!(approve.status, StatusCode::OK, "{}", approve.body);
    assert_eq!(
        scalar_text(
            &pool,
            "SELECT status FROM learned_rules WHERE id = ?",
            &rule_id
        )
        .await,
        "approved"
    );

    let archive = call(
        &app,
        req(
            "POST",
            format!("/api/learning/rules/{rule_id}/archive"),
            Some(&maint_token),
            None,
        ),
    )
    .await;
    assert_eq!(archive.status, StatusCode::OK, "{}", archive.body);
    assert_eq!(
        scalar_text(
            &pool,
            "SELECT status FROM learned_rules WHERE id = ?",
            &rule_id
        )
        .await,
        "archived"
    );

    let delete = call(
        &app,
        req(
            "DELETE",
            format!("/api/learning/rules/{rule_id}"),
            Some(&maint_token),
            None,
        ),
    )
    .await;
    assert_eq!(delete.status, StatusCode::OK, "{}", delete.body);
    assert_eq!(
        scalar_int(
            &pool,
            "SELECT COUNT(*) FROM learned_rules WHERE id = ?",
            &rule_id
        )
        .await,
        0
    );
    let after = call(
        &app,
        req("GET", "/api/learning/rules", Some(&maint_token), None),
    )
    .await;
    assert!(!after.body.contains(&rule_id), "{}", after.body);
    let missing = call(
        &app,
        req(
            "DELETE",
            format!("/api/learning/rules/{rule_id}"),
            Some(&maint_token),
            None,
        ),
    )
    .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND, "{}", missing.body);
}

#[tokio::test]
async fn mcp_endpoints_server_lifecycle_and_test() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = test_pool().await;
    let (_owner, owner_token, _) = seed_user(&pool, "owner").await;
    let (_maint, maint_token, _) = seed_user(&pool, "maintainer").await;
    let (_viewer, viewer_token, _) = seed_user(&pool, "viewer").await;
    let id = unique("custom-mcp");
    let app = app(pool.clone()).await;

    assert_unauthorized(&app, "GET", "/api/mcp/servers").await;

    let list = call(
        &app,
        req("GET", "/api/mcp/servers", Some(&maint_token), None),
    )
    .await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.body);
    assert!(list.body.contains("\"servers\""), "{}", list.body);

    let denied = call(
        &app,
        req(
            "PUT",
            format!("/api/mcp/servers/{id}"),
            Some(&viewer_token),
            Some(serde_json::json!({"enabled": true, "base_url": "http://192.0.2.1:1"})),
        ),
    )
    .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN, "{}", denied.body);

    let upsert = call(
        &app,
        req(
            "PUT",
            format!("/api/mcp/servers/{id}"),
            Some(&owner_token),
            Some(serde_json::json!({
                "enabled": true,
                "base_url": "http://192.0.2.1:1",
                "name": "Harness MCP",
                "api_key": "test-key"
            })),
        ),
    )
    .await;
    assert_eq!(upsert.status, StatusCode::OK, "{}", upsert.body);
    assert!(upsert.body.contains("\"status\":\"ok\""), "{}", upsert.body);
    assert_eq!(
        db::config::get_config(&pool, &format!("mcp_{id}_base_url"))
            .await
            .unwrap()
            .as_deref(),
        Some("http://192.0.2.1:1")
    );

    let listed = call(
        &app,
        req("GET", "/api/mcp/servers", Some(&maint_token), None),
    )
    .await;
    assert!(listed.body.contains(&id), "{}", listed.body);

    let test = call(
        &app,
        req(
            "POST",
            format!("/api/mcp/{id}/test"),
            Some(&maint_token),
            None,
        ),
    )
    .await;
    assert_clean_error(&test, "POST /api/mcp/{id}/test");

    let delete = call(
        &app,
        req(
            "DELETE",
            format!("/api/mcp/servers/{id}"),
            Some(&owner_token),
            None,
        ),
    )
    .await;
    assert_eq!(delete.status, StatusCode::OK, "{}", delete.body);
    assert!(db::config::get_config(&pool, &format!("mcp_{id}_enabled"))
        .await
        .unwrap()
        .is_none());
    let after = call(
        &app,
        req("GET", "/api/mcp/servers", Some(&maint_token), None),
    )
    .await;
    assert!(!after.body.contains(&id), "{}", after.body);
}

#[tokio::test]
async fn github_endpoints_urls_and_resolve() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = test_pool().await;
    let (_maint, maint_token, _) = seed_user(&pool, "maintainer").await;
    let (_viewer, viewer_token, _) = seed_user(&pool, "viewer").await;
    let app = app(pool.clone()).await;

    assert_unauthorized(&app, "GET", "/api/github/install-url").await;
    assert_unauthorized(&app, "GET", "/api/github/manage-url").await;
    assert_unauthorized(&app, "POST", "/api/github/resolve").await;

    let install = call(
        &app,
        req("GET", "/api/github/install-url", Some(&viewer_token), None),
    )
    .await;
    assert_eq!(install.status, StatusCode::OK, "{}", install.body);
    assert!(install.body.contains("\"url\""), "{}", install.body);

    let manage = call(
        &app,
        req("GET", "/api/github/manage-url", Some(&viewer_token), None),
    )
    .await;
    assert_eq!(manage.status, StatusCode::OK, "{}", manage.body);
    assert!(manage.body.contains("\"url\""), "{}", manage.body);

    let denied = call(
        &app,
        req("POST", "/api/github/resolve", Some(&viewer_token), None),
    )
    .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN, "{}", denied.body);

    let github_keys = ["github_app_id", "github_private_key", "github_app_slug"];
    let mut saved = Vec::new();
    for key in github_keys {
        saved.push((key, db::config::get_config(&pool, key).await.unwrap()));
        db::config::delete_config(&pool, key).await.unwrap();
    }
    let resolve = call(
        &app,
        req("POST", "/api/github/resolve", Some(&maint_token), None),
    )
    .await;
    for (key, value) in saved {
        match value {
            Some(value) => db::config::set_config(&pool, key, &value).await.unwrap(),
            None => db::config::delete_config(&pool, key).await.unwrap(),
        }
    }
    assert_clean_error(&resolve, "POST /api/github/resolve");
    assert!(
        resolve.body.contains("Could not resolve the GitHub App"),
        "{}",
        resolve.body
    );
}

#[tokio::test]
async fn audit_endpoints_list() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = test_pool().await;
    let (_maint, maint_token, _) = seed_user(&pool, "maintainer").await;
    let (_viewer, viewer_token, _) = seed_user(&pool, "viewer").await;
    let app = app(pool).await;

    assert_unauthorized(&app, "GET", "/api/audit").await;
    let denied = call(&app, req("GET", "/api/audit", Some(&viewer_token), None)).await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN, "{}", denied.body);

    let list = call(&app, req("GET", "/api/audit", Some(&maint_token), None)).await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.body);
    for key in ["entries", "total", "total_pages", "retention_days"] {
        assert!(list.body.contains(&format!("\"{key}\"")), "{}", list.body);
    }
}

#[tokio::test]
async fn users_endpoints_team_and_invites() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = test_pool().await;
    let actor_email = unique_email("bootstrap");
    let _actor = db::users::create_bootstrap_owner(&pool, &actor_email, PASSWORD)
        .await
        .unwrap();
    let actor_token = db::sessions::create_session(&pool, &actor_email)
        .await
        .unwrap();
    let target_email = unique_email("target-owner");
    let target = db::users::create_user(&pool, &target_email, PASSWORD, "owner")
        .await
        .unwrap();
    let removable_email = unique_email("removable");
    let removable = db::users::create_user(&pool, &removable_email, PASSWORD, "viewer")
        .await
        .unwrap();
    let (_viewer, viewer_token, _) = seed_user(&pool, "viewer").await;
    let app = app(pool.clone()).await;

    assert_unauthorized(&app, "GET", "/api/users").await;

    let roster = call(&app, req("GET", "/api/users", Some(&viewer_token), None)).await;
    assert_eq!(roster.status, StatusCode::OK, "{}", roster.body);
    assert!(roster.body.contains(&target_email), "{}", roster.body);

    let bad_password = call(
        &app,
        req(
            "POST",
            "/api/users/me/password",
            Some(&actor_token),
            Some(serde_json::json!({"current_password": "wrongpassword", "new_password": "newpassword123"})),
        ),
    )
    .await;
    assert_eq!(
        bad_password.status,
        StatusCode::UNAUTHORIZED,
        "{}",
        bad_password.body
    );
    assert!(
        bad_password.body.contains("Current password is incorrect"),
        "{}",
        bad_password.body
    );

    let change = call(
        &app,
        req(
            "POST",
            "/api/users/me/password",
            Some(&actor_token),
            Some(
                serde_json::json!({"current_password": PASSWORD, "new_password": "newpassword123"}),
            ),
        ),
    )
    .await;
    assert_eq!(change.status, StatusCode::OK, "{}", change.body);
    assert!(
        db::users::verify_password(&pool, &actor_email, "newpassword123")
            .await
            .unwrap()
            .is_some()
    );

    let invite_email = unique_email("invited");
    let create = call(
        &app,
        req(
            "POST",
            "/api/users/invites",
            Some(&actor_token),
            Some(serde_json::json!({"email": invite_email, "role": "viewer"})),
        ),
    )
    .await;
    assert_eq!(create.status, StatusCode::OK, "{}", create.body);
    assert!(create.body.contains("\"token\""), "{}", create.body);
    let invite_id: i64 = serde_json::from_str::<serde_json::Value>(&create.body).unwrap()["id"]
        .as_i64()
        .unwrap();

    let invites = call(
        &app,
        req("GET", "/api/users/invites", Some(&actor_token), None),
    )
    .await;
    assert_eq!(invites.status, StatusCode::OK, "{}", invites.body);
    assert!(invites.body.contains(&invite_email), "{}", invites.body);

    let revoke = call(
        &app,
        req(
            "DELETE",
            format!("/api/users/invites/{invite_id}"),
            Some(&actor_token),
            None,
        ),
    )
    .await;
    assert_eq!(revoke.status, StatusCode::OK, "{}", revoke.body);
    let after = call(
        &app,
        req("GET", "/api/users/invites", Some(&actor_token), None),
    )
    .await;
    assert!(!after.body.contains(&invite_email), "{}", after.body);

    let patch = call(
        &app,
        req(
            "PATCH",
            format!("/api/users/{}", target.id),
            Some(&actor_token),
            Some(serde_json::json!({"role": "maintainer"})),
        ),
    )
    .await;
    assert_eq!(patch.status, StatusCode::OK, "{}", patch.body);
    assert!(
        patch.body.contains("\"role\":\"maintainer\""),
        "{}",
        patch.body
    );

    let restore = call(
        &app,
        req(
            "PATCH",
            format!("/api/users/{}", target.id),
            Some(&actor_token),
            Some(serde_json::json!({"role": "owner"})),
        ),
    )
    .await;
    assert_eq!(restore.status, StatusCode::OK, "{}", restore.body);

    let transfer = call(
        &app,
        req(
            "POST",
            format!("/api/users/{}/transfer-bootstrap", target.id),
            Some(&actor_token),
            None,
        ),
    )
    .await;
    assert_eq!(transfer.status, StatusCode::OK, "{}", transfer.body);
    assert!(
        transfer.body.contains("\"is_bootstrap\":true"),
        "{}",
        transfer.body
    );
    assert!(
        db::users::get_user_by_id(&pool, target.id)
            .await
            .unwrap()
            .unwrap()
            .is_bootstrap
    );

    let remove = call(
        &app,
        req(
            "DELETE",
            format!("/api/users/{}", removable.id),
            Some(&actor_token),
            None,
        ),
    )
    .await;
    assert_eq!(remove.status, StatusCode::OK, "{}", remove.body);
    assert!(db::users::get_user_by_id(&pool, removable.id)
        .await
        .unwrap()
        .is_none());
    let roster_after = call(&app, req("GET", "/api/users", Some(&actor_token), None)).await;
    assert!(
        !roster_after.body.contains(&removable_email),
        "{}",
        roster_after.body
    );
}

#[tokio::test]
async fn stats_endpoint() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = test_pool().await;
    let (_viewer, viewer_token, _) = seed_user(&pool, "viewer").await;
    let app = app(pool).await;

    assert_unauthorized(&app, "GET", "/api/stats").await;
    let stats = call(&app, req("GET", "/api/stats", Some(&viewer_token), None)).await;
    assert_eq!(stats.status, StatusCode::OK, "{}", stats.body);
    for key in ["recent_activity", "trust", "llm", "analytics"] {
        assert!(stats.body.contains(&format!("\"{key}\"")), "{}", stats.body);
    }
}

fn webhook_request(body: &str, event: &str, signature: &str) -> Request<Body> {
    let n = REQ_SEQ.fetch_add(1, Ordering::Relaxed);
    Request::builder()
        .method("POST")
        .uri("/webhook")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-github-event", event)
        .header(
            "x-github-delivery",
            format!("delivery-{n}-{}", uuid::Uuid::new_v4()),
        )
        .header("x-hub-signature-256", signature)
        .header(
            "x-forwarded-for",
            format!("10.1.{}.{}", (n >> 8) & 255, n & 255),
        )
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn sign(body: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(TEST_WEBHOOK_SECRET.as_bytes()).unwrap();
    mac.update(body.as_bytes());
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

#[tokio::test]
async fn webhook_endpoints_signature_validation() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = test_pool().await;
    let app = app(pool).await;
    let body = r#"{"zen":"Keep it logically awesome."}"#;

    let valid = call(&app, webhook_request(body, "ping", &sign(body))).await;
    assert_eq!(valid.status, StatusCode::OK, "{}", valid.body);
    assert!(valid.body.contains("\"event\":\"ping\""), "{}", valid.body);

    let invalid = call(&app, webhook_request(body, "ping", "sha256=deadbeef")).await;
    assert_eq!(invalid.status, StatusCode::UNAUTHORIZED, "{}", invalid.body);

    let pr_body = r#"{"action":"opened","number":1}"#;
    let invalid_pr = call(
        &app,
        webhook_request(pr_body, "pull_request", "sha256=deadbeef"),
    )
    .await;
    assert_eq!(
        invalid_pr.status,
        StatusCode::UNAUTHORIZED,
        "{}",
        invalid_pr.body
    );

    let mut missing_delivery = webhook_request(body, "ping", &sign(body));
    missing_delivery.headers_mut().remove("x-github-delivery");
    let bad_delivery = call(&app, missing_delivery).await;
    assert_eq!(
        bad_delivery.status,
        StatusCode::BAD_REQUEST,
        "{}",
        bad_delivery.body
    );
}

#[test]
fn cli_smoke_version_and_health() {
    let binary = env!("CARGO_BIN_EXE_codasaurus");

    let version = std::process::Command::new(binary)
        .arg("version")
        .output()
        .expect("run codasaurus version");
    assert!(
        version.status.success(),
        "version stderr: {}",
        String::from_utf8_lossy(&version.stderr)
    );
    let stdout = String::from_utf8_lossy(&version.stdout);
    assert!(stdout.contains("codasaurus v"), "version stdout: {stdout}");

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://codasaurus:codasaurus@127.0.0.1:5432/codasaurus".into());
    let health = std::process::Command::new(binary)
        .args(["health", "--host", "127.0.0.1", "--port", "1"])
        .env("DATABASE_URL", database_url)
        .output()
        .expect("run codasaurus health");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&health.stdout),
        String::from_utf8_lossy(&health.stderr)
    );
    assert!(
        health.status.success()
            || output.contains("Health check error")
            || output.contains("Health check failed"),
        "health output: {output}"
    );
    assert!(!output.contains("panicked"), "health panicked: {output}");
}
