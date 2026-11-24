//! End-to-end QA journey against real Postgres and Redis.

use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::Router;
use hmac::{Hmac, Mac};
use http_body_util::BodyExt;
use sha2::Sha256;
use tower::ServiceExt;

use codasaurus::bot::{self, BotConfig};
use codasaurus::db;
use codasaurus::redis::StateStore;
use std::net::SocketAddr;

const PASSWORD: &str = "qa-password-123";
const WEBHOOK_SECRET: &str = "qa-journey-webhook-secret";

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4().simple())
}

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .or_else(|_| std::env::var("CODASAURUS_TEST_DATABASE_URL"))
        .unwrap_or_else(|_| "postgres://codasaurus:codasaurus@127.0.0.1:5432/codasaurus".into())
}

fn redis_url() -> String {
    std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into())
}

fn request(
    method: &str,
    uri: impl AsRef<str>,
    token: Option<&str>,
    body: Option<serde_json::Value>,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri.as_ref())
        .header("x-forwarded-for", "10.20.30.40");
    if let Some(token) = token {
        builder = builder.header(header::COOKIE, format!("codasaurus_session={token}"));
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

struct Response {
    status: StatusCode,
    headers: HeaderMap,
    body: String,
}

async fn call(app: &Router, request: Request<Body>) -> Response {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    Response {
        status,
        headers,
        body: String::from_utf8_lossy(&bytes).to_string(),
    }
}

fn assert_clean_error(response: &Response, context: &str) {
    assert!(
        response.status.is_client_error(),
        "{context}: expected client error, got {} {}",
        response.status,
        response.body
    );
    let body = response.body.to_ascii_lowercase();
    for forbidden in ["panic", "backtrace", "internal server error", "postgres://"] {
        assert!(!body.contains(forbidden), "{context}: leaked {forbidden}");
    }
}

async fn seed_user(pool: &db::DbPool, role: &str) -> (String, db::models::User) {
    let email = format!("{}@example.com", unique("qa-user"));
    let user = db::users::create_user(pool, &email, PASSWORD, role)
        .await
        .unwrap();
    (email, user)
}

async fn login(app: &Router, email: &str, password: &str) -> Response {
    call(
        app,
        request(
            "POST",
            "/api/auth/login",
            None,
            Some(serde_json::json!({"email": email, "password": password})),
        ),
    )
    .await
}

fn session(response: &Response) -> String {
    let cookie = response
        .headers
        .get(header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    cookie
        .split(';')
        .next()
        .unwrap()
        .trim_start_matches("codasaurus_session=")
        .to_string()
}

async fn seed_repo(pool: &db::DbPool) -> db::models::Repo {
    let name = unique("qa-repo");
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
            pr_number: chrono::Utc::now().timestamp(),
            pr_title: Some("QA journey".into()),
            pr_author: Some("qa-author".into()),
            pr_base_branch: Some("main".into()),
            pr_head_branch: Some("qa".into()),
            pr_head_sha: Some(uuid::Uuid::new_v4().simple().to_string()),
        },
    )
    .await
    .unwrap()
}

async fn queue_recovery_journey(pool: &db::DbPool) -> anyhow::Result<()> {
    let repo = unique("acme/qa-queue");
    let id =
        bot::queue::enqueue_with_priority(pool, &repo, 7, "qa-sha", None, "opened", -100).await?;
    let first = bot::queue::claim_next(pool, 600)
        .await?
        .expect("queued QA job");
    assert_eq!(first.id, id, "QA job must have highest priority");

    let outcome = bot::queue::mark_failed(pool, id, &first.lease_id, "temporary", 1, 5).await?;
    assert!(matches!(outcome, bot::queue::Outcome::Scheduled { .. }));
    sqlx::query("UPDATE review_jobs SET run_at = NOW() WHERE id = $1")
        .bind(id)
        .execute(pool.as_pg())
        .await?;
    let second = bot::queue::claim_next(pool, 600)
        .await?
        .expect("retried QA job");
    assert_eq!(second.id, id);
    let outcome = bot::queue::mark_failed(pool, id, &second.lease_id, "terminal", 5, 5).await?;
    assert_eq!(outcome, bot::queue::Outcome::Dead);
    assert!(bot::queue::replay_dead_job(pool, id).await?);
    let status: String = sqlx::query_scalar("SELECT status FROM review_jobs WHERE id = $1")
        .bind(id)
        .fetch_one(pool.as_pg())
        .await?;
    assert_eq!(status, "pending");
    sqlx::query("DELETE FROM review_jobs WHERE repo = $1")
        .bind(&repo)
        .execute(pool.as_pg())
        .await?;
    Ok(())
}

#[tokio::test]
async fn qa_user_journey() -> anyhow::Result<()> {
    let pool = db::create_pool(&database_url()).await?;
    let redis = StateStore::connect(&redis_url()).await?;
    queue_recovery_journey(&pool).await?;

    let app = codasaurus::serve::build_router(
        pool.clone(),
        redis,
        Some(BotConfig {
            app_id: "123456".into(),
            private_key: String::new(),
            webhook_secret: WEBHOOK_SECRET.into(),
            host: "127.0.0.1".into(),
            port: 1,
        }),
    )
    .layer(MockConnectInfo(
        "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
    ));

    for (method, uri) in [
        ("GET", "/api/settings"),
        ("GET", "/api/repos"),
        ("GET", "/api/reviews"),
        ("POST", "/api/reviews/dismiss"),
    ] {
        let response = call(&app, request(method, uri, None, None)).await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED, "{uri}");
    }

    let (owner_email, _) = seed_user(&pool, "owner").await;
    let (viewer_email, _) = seed_user(&pool, "viewer").await;
    let (maintainer_email, _) = seed_user(&pool, "maintainer").await;

    let wrong = login(&app, &owner_email, "wrong-password").await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    assert_clean_error(&wrong, "wrong password");

    let owner_login = login(&app, &owner_email, PASSWORD).await;
    assert_eq!(owner_login.status, StatusCode::OK, "{}", owner_login.body);
    let owner = session(&owner_login);
    let viewer_login = login(&app, &viewer_email, PASSWORD).await;
    let viewer = session(&viewer_login);
    let maintainer_login = login(&app, &maintainer_email, PASSWORD).await;
    let maintainer = session(&maintainer_login);

    let anonymous = call(&app, request("GET", "/api/auth/me", None, None)).await;
    assert!(anonymous.body.contains("\"authenticated\":false"));
    let me = call(&app, request("GET", "/api/auth/me", Some(&owner), None)).await;
    assert!(me.body.contains("\"authenticated\":true"));
    assert!(me.body.contains(&owner_email));

    let viewer_settings = call(&app, request("GET", "/api/settings", Some(&viewer), None)).await;
    assert_eq!(viewer_settings.status, StatusCode::FORBIDDEN);
    let settings = call(
        &app,
        request("GET", "/api/settings", Some(&maintainer), None),
    )
    .await;
    assert_eq!(settings.status, StatusCode::OK, "{}", settings.body);
    assert!(settings.body.contains("offline_mode_effective"));
    assert!(!settings.body.contains("POSTGRES_PASSWORD"));

    let old_max_warnings = db::config::get_config(&pool, "max_warnings").await?;
    let denied = call(
        &app,
        request(
            "PUT",
            "/api/settings/max_warnings",
            Some(&viewer),
            Some(serde_json::json!({"value": "9"})),
        ),
    )
    .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    let invalid = call(
        &app,
        request(
            "PUT",
            "/api/settings/max_warnings",
            Some(&owner),
            Some(serde_json::json!({"value": "not-a-number"})),
        ),
    )
    .await;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST);
    assert_clean_error(&invalid, "invalid setting");
    let valid = call(
        &app,
        request(
            "PUT",
            "/api/settings/max_warnings",
            Some(&owner),
            Some(serde_json::json!({"value": "7"})),
        ),
    )
    .await;
    assert_eq!(valid.status, StatusCode::OK, "{}", valid.body);
    assert_eq!(
        db::config::get_config(&pool, "max_warnings")
            .await?
            .as_deref(),
        Some("7")
    );
    match old_max_warnings {
        Some(value) => db::config::set_config(&pool, "max_warnings", &value).await?,
        None => db::config::delete_config(&pool, "max_warnings").await?,
    }

    let old_offline = db::config::get_config(&pool, "offline_mode").await?;
    let offline_on = call(
        &app,
        request(
            "PUT",
            "/api/settings/offline_mode",
            Some(&owner),
            Some(serde_json::json!({"value": "true"})),
        ),
    )
    .await;
    assert_eq!(offline_on.status, StatusCode::OK, "{}", offline_on.body);
    let offline_health = call(&app, request("GET", "/health", None, None)).await;
    assert!(offline_health.body.contains("\"offline_mode\":true"));
    let offline_off = call(
        &app,
        request(
            "PUT",
            "/api/settings/offline_mode",
            Some(&owner),
            Some(serde_json::json!({"value": "false"})),
        ),
    )
    .await;
    assert_eq!(offline_off.status, StatusCode::OK);
    match old_offline {
        Some(value) => db::config::set_config(&pool, "offline_mode", &value).await?,
        None => db::config::delete_config(&pool, "offline_mode").await?,
    }

    let old_metrics_token = std::env::var_os("CODASAURUS_METRICS_TOKEN");
    std::env::remove_var("CODASAURUS_METRICS_TOKEN");
    let metrics_disabled = call(&app, request("GET", "/metrics", None, None)).await;
    assert_eq!(metrics_disabled.status, StatusCode::NOT_FOUND);
    let metrics_token = unique("qa-metrics");
    std::env::set_var("CODASAURUS_METRICS_TOKEN", &metrics_token);
    let metrics_denied = call(&app, request("GET", "/metrics", None, None)).await;
    assert_eq!(metrics_denied.status, StatusCode::UNAUTHORIZED);
    let mut metrics_request = request("GET", "/metrics", None, None);
    metrics_request.headers_mut().insert(
        header::AUTHORIZATION,
        format!("Bearer {metrics_token}").parse().unwrap(),
    );
    let metrics = call(&app, metrics_request).await;
    assert_eq!(metrics.status, StatusCode::OK, "{}", metrics.body);
    assert!(metrics.body.contains("codasaurus_up 1"));
    assert!(metrics.body.contains("codasaurus_queue_depth"));
    match old_metrics_token {
        Some(value) => std::env::set_var("CODASAURUS_METRICS_TOKEN", value),
        None => std::env::remove_var("CODASAURUS_METRICS_TOKEN"),
    }

    let repo = seed_repo(&pool).await;
    let repo_list = call(&app, request("GET", "/api/repos", Some(&viewer), None)).await;
    assert_eq!(repo_list.status, StatusCode::OK);
    assert!(repo_list.body.contains(&repo.full_name));
    let repo_detail = call(
        &app,
        request(
            "GET",
            format!("/api/repos/{}", repo.id),
            Some(&viewer),
            None,
        ),
    )
    .await;
    assert_eq!(repo_detail.status, StatusCode::OK);
    let repo_denied = call(
        &app,
        request(
            "PUT",
            format!("/api/repos/{}", repo.id),
            Some(&viewer),
            Some(serde_json::json!({"config_json": "{}", "active": true})),
        ),
    )
    .await;
    assert_eq!(repo_denied.status, StatusCode::FORBIDDEN);
    let repo_update = call(
        &app,
        request(
            "PUT",
            format!("/api/repos/{}", repo.id),
            Some(&maintainer),
            Some(serde_json::json!({"config_json": "{\"review_strictness\":\"strict\"}", "active": true})),
        ),
    )
    .await;
    assert_eq!(repo_update.status, StatusCode::OK, "{}", repo_update.body);

    let review_repo = seed_repo(&pool).await;
    let review = seed_review(&pool, review_repo.id).await;
    let review_list = call(&app, request("GET", "/api/reviews", Some(&viewer), None)).await;
    assert_eq!(review_list.status, StatusCode::OK);
    assert!(review_list.body.contains(&review.id.to_string()));
    for suffix in ["", "/findings", "/sarif"] {
        let uri = format!("/api/reviews/{}{suffix}", review.id);
        let response = call(&app, request("GET", &uri, Some(&viewer), None)).await;
        assert_eq!(response.status, StatusCode::OK, "{uri}: {}", response.body);
    }
    let fingerprint = unique("qa-fingerprint");
    let dismiss_denied = call(
        &app,
        request(
            "POST",
            "/api/reviews/dismiss",
            Some(&viewer),
            Some(serde_json::json!({"fingerprint": fingerprint, "detector": "secrets"})),
        ),
    )
    .await;
    assert_eq!(dismiss_denied.status, StatusCode::FORBIDDEN);
    let dismiss_short = call(
        &app,
        request(
            "POST",
            "/api/reviews/dismiss",
            Some(&maintainer),
            Some(serde_json::json!({"fingerprint": "short"})),
        ),
    )
    .await;
    assert_eq!(dismiss_short.status, StatusCode::BAD_REQUEST);
    let dismiss = call(
        &app,
        request(
            "POST",
            "/api/reviews/dismiss",
            Some(&maintainer),
            Some(serde_json::json!({"fingerprint": fingerprint, "detector": "secrets"})),
        ),
    )
    .await;
    assert_eq!(dismiss.status, StatusCode::OK, "{}", dismiss.body);

    let invalid_state = call(
        &app,
        request(
            "GET",
            "/api/auth/oidc/callback?code=qa&state=not-a-real-state",
            None,
            None,
        ),
    )
    .await;
    assert_eq!(invalid_state.status, StatusCode::BAD_REQUEST);
    assert!(invalid_state.body.contains("invalid or expired OIDC state"));

    let mut bad_signature = Hmac::<Sha256>::new_from_slice(b"wrong-secret")?;
    bad_signature.update(b"{}");
    let bad_webhook = Request::builder()
        .method("POST")
        .uri("/webhook")
        .header("content-type", "application/json")
        .header("x-github-event", "ping")
        .header("x-github-delivery", unique("qa-delivery"))
        .header(
            "x-hub-signature-256",
            format!(
                "sha256={}",
                hex::encode(bad_signature.finalize().into_bytes())
            ),
        )
        .body(Body::from("{}"))
        .unwrap();
    let bad_webhook = call(&app, bad_webhook).await;
    assert_eq!(bad_webhook.status, StatusCode::UNAUTHORIZED);

    let ping_body = serde_json::json!({"zen": "qa-ok"}).to_string();
    let mut signature = Hmac::<Sha256>::new_from_slice(WEBHOOK_SECRET.as_bytes())?;
    signature.update(ping_body.as_bytes());
    let delivery = unique("qa-ping");
    let ping = Request::builder()
        .method("POST")
        .uri("/webhook")
        .header("content-type", "application/json")
        .header("x-github-event", "ping")
        .header("x-github-delivery", &delivery)
        .header(
            "x-hub-signature-256",
            format!("sha256={}", hex::encode(signature.finalize().into_bytes())),
        )
        .body(Body::from(ping_body))
        .unwrap();
    let ping = call(&app, ping).await;
    assert_eq!(ping.status, StatusCode::OK, "{}", ping.body);

    for uri in ["/health", "/health/ready"] {
        let response = call(&app, request("GET", uri, None, None)).await;
        assert_eq!(response.status, StatusCode::OK, "{uri}: {}", response.body);
        assert_eq!(
            response.headers.get("x-content-type-options").unwrap(),
            "nosniff"
        );
        assert!(response.headers.contains_key("content-security-policy"));
    }

    let repo_delete = call(
        &app,
        request(
            "DELETE",
            format!("/api/repos/{}", repo.id),
            Some(&maintainer),
            None,
        ),
    )
    .await;
    assert_eq!(repo_delete.status, StatusCode::OK);
    let repo_gone = call(
        &app,
        request(
            "GET",
            format!("/api/repos/{}", repo.id),
            Some(&viewer),
            None,
        ),
    )
    .await;
    assert_eq!(repo_gone.status, StatusCode::NOT_FOUND);

    let logout = call(
        &app,
        request("POST", "/api/auth/logout", Some(&owner), None),
    )
    .await;
    assert_eq!(logout.status, StatusCode::OK);
    let after_logout = call(&app, request("GET", "/api/auth/me", Some(&owner), None)).await;
    assert!(after_logout.body.contains("\"authenticated\":false"));

    pool.close().await;
    Ok(())
}
