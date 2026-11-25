use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use crate::db;
use crate::db::DbPool;
use crate::learning::store::LearningStore;

use super::errors::ApiError;
use super::AppState;

#[derive(Deserialize, Default)]
pub struct ListReviewsParams {
    pub repo_id: Option<i64>,
    pub status: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Deserialize)]
pub struct DismissBody {
    pub fingerprint: String,
    pub detector: Option<String>,
    pub file: Option<String>,
    pub message: Option<String>,
    pub pr_number: Option<i64>,
    pub repo: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_reviews))
        .route("/{id}", get(get_review))
        .route("/{id}/findings", get(get_review_findings))
        .route("/{id}/sarif", get(get_review_sarif))
        .route("/dismiss", post(dismiss_finding))
}

/// GET /api/reviews
#[utoipa::path(get, path = "/api/reviews", tag = "reviews", responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn list_reviews(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Query(params): Query<ListReviewsParams>,
) -> Result<Json<serde_json::Value>, ApiError> {
    super::rbac::require_role(&state.pool, &headers, super::rbac::MinRole::Viewer).await?;
    let limit = params.limit.unwrap_or(20).clamp(1, 200);
    let offset = params.offset.unwrap_or(0).max(0);
    let repo_id = params.repo_id;
    let status = params.status.as_deref();

    let (reviews, total) = tokio::join!(
        db::reviews::list_reviews(&state.pool, repo_id, status, limit, offset),
        count_reviews(&state.pool, repo_id, status),
    );
    let reviews = reviews?;
    let total = total?;
    let total_pages = total / limit + i64::from(total % limit != 0);

    let enriched: Vec<serde_json::Value> = reviews
        .into_iter()
        .map(|row| {
            let mut v = serde_json::to_value(row.review).unwrap_or_default();
            if let Some(obj) = v.as_object_mut() {
                obj.insert("repo_name".into(), json!(row.repo_full_name.clone()));
                obj.insert("repo_full_name".into(), json!(row.repo_full_name));
            }
            v
        })
        .collect();

    Ok(Json(json!({
        "reviews": enriched,
        "total": total,
        "total_pages": total_pages,
        "limit": limit,
        "offset": offset,
    })))
}

/// Helper: COUNT query matching list_reviews filters.
async fn count_reviews(
    pool: &DbPool,
    repo_id: Option<i64>,
    status: Option<&str>,
) -> Result<i64, ApiError> {
    let count = match (repo_id, status) {
        (Some(rid), Some(st)) => crate::db::db_scalar!(
            pool,
            i64,
            "SELECT COUNT(*) FROM reviews WHERE repo_id = ? AND status = ?",
            rid,
            st
        )?,
        (Some(rid), None) => crate::db::db_scalar!(
            pool,
            i64,
            "SELECT COUNT(*) FROM reviews WHERE repo_id = ?",
            rid
        )?,
        (None, Some(st)) => crate::db::db_scalar!(
            pool,
            i64,
            "SELECT COUNT(*) FROM reviews WHERE status = ?",
            st
        )?,
        (None, None) => crate::db::db_scalar!(pool, i64, "SELECT COUNT(*) FROM reviews")?,
    };
    Ok(count)
}

/// GET /api/reviews/:id
#[utoipa::path(get, path = "/api/reviews/{id}", tag = "reviews", params(("id" = i64, Path, description = "Resource id")), responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn get_review(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (_user, detail) = super::rbac::require_review_read(&state, &headers, id).await?;
    let findings = db::reviews::get_findings_for_review(&state.pool, id).await?;
    let review = detail.review;
    let repo_full_name = (!detail.repo_full_name.is_empty()).then_some(detail.repo_full_name);

    let mut by_severity: std::collections::BTreeMap<String, i64> =
        std::collections::BTreeMap::new();
    let mut by_detector: std::collections::BTreeMap<String, i64> =
        std::collections::BTreeMap::new();
    let mut files: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for f in &findings {
        files.insert(f.file_path.clone());
        *by_severity.entry(f.severity.clone()).or_insert(0) += 1;
        *by_detector.entry(f.detector.clone()).or_insert(0) += 1;
    }

    let duration_secs = match (review.started_at, review.completed_at) {
        (Some(s), Some(c)) => Some((c - s).num_seconds().max(0)),
        _ => None,
    };

    let mut review_val = serde_json::to_value(&review).unwrap_or_default();
    if let Some(obj) = review_val.as_object_mut() {
        obj.insert(
            "repo_full_name".into(),
            json!(repo_full_name.clone().unwrap_or_default()),
        );
        obj.insert("finding_count".into(), json!(findings.len()));
        obj.insert("file_count".into(), json!(files.len()));
        obj.insert("by_severity".into(), json!(by_severity));
        obj.insert("by_detector".into(), json!(by_detector));
        obj.insert("duration_secs".into(), json!(duration_secs));
    }

    Ok(Json(json!({
        "review": review_val,
        "findings": findings,
        "summary": {
            "finding_count": findings.len(),
            "file_count": files.len(),
            "by_severity": by_severity,
            "by_detector": by_detector,
            "duration_secs": duration_secs,
        },
    })))
}

/// GET /api/reviews/:id/findings
#[utoipa::path(get, path = "/api/reviews/{id}/findings", tag = "reviews", params(("id" = i64, Path, description = "Resource id")), responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn get_review_findings(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    super::rbac::require_review_read(&state, &headers, id).await?;

    let findings = db::reviews::get_findings_for_review(&state.pool, id).await?;

    let mut grouped: std::collections::BTreeMap<String, Vec<serde_json::Value>> =
        std::collections::BTreeMap::new();
    for f in findings {
        let entry = grouped.entry(f.file_path.clone()).or_default();
        entry.push(json!(f));
    }

    Ok(Json(json!({
        "review_id": id,
        "findings_by_file": grouped,
        "total_findings": grouped.values().map(|v| v.len()).sum::<usize>(),
    })))
}

/// GET /api/reviews/:id/sarif — SARIF 2.1.0 export for external tooling (e.g. GitHub code scanning).
#[utoipa::path(get, path = "/api/reviews/{id}/sarif", tag = "reviews", params(("id" = i64, Path, description = "Resource id")), responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn get_review_sarif(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<i64>,
) -> Result<axum::response::Response, ApiError> {
    let (_user, detail) = super::rbac::require_review_read(&state, &headers, id).await?;
    let findings = db::reviews::get_findings_for_review(&state.pool, id).await?;
    let review = detail.review;
    let repo_full_name = detail.repo_full_name;

    let commit_sha = review.pr_head_sha.clone().unwrap_or_default();
    let sarif =
        crate::detectors::sarif::db_findings_to_sarif(&findings, &repo_full_name, &commit_sha);

    axum::response::Response::builder()
        .header("content-type", "application/sarif+json")
        .body(axum::body::Body::from(
            serde_json::to_vec(&sarif).unwrap_or_default(),
        ))
        .map_err(ApiError::internal_error)
}

/// POST /api/reviews/dismiss — dismiss a finding into the learning store
#[utoipa::path(post, path = "/api/reviews/dismiss", tag = "reviews", responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn dismiss_finding(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<DismissBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = super::rbac::require_maintainer(&state, &headers).await?;
    let fp = body.fingerprint.trim();
    if fp.len() < 8 {
        return Err(ApiError::bad_request("fingerprint too short"));
    }
    // Strip review_id: prefix if present from DB storage
    let fp = fp.rsplit(':').next().unwrap_or(fp);
    let store = LearningStore::from_pool(&state.pool);
    store
        .dismiss_fingerprint_for_repo(
            fp,
            body.detector.as_deref().unwrap_or("manual"),
            body.file.as_deref().unwrap_or(""),
            body.message.as_deref().unwrap_or("dismissed via dashboard"),
            body.repo.as_deref(),
            body.pr_number,
            Some(user.email.as_str()),
            true, // dashboard dismiss requires maintainer
        )
        .await
        .map_err(ApiError::internal_error)?;
    Ok(Json(json!({ "status": "ok", "fingerprint": fp })))
}
