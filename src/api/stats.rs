use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use crate::db;

use super::errors::ApiError;
use super::AppState;

#[derive(Default)]
struct StatsCache {
    fetched_at: Option<Instant>,
    value: Option<serde_json::Value>,
}

/// Single-flight cache: concurrent misses must not all run the rollups.
static STATS_CACHE: LazyLock<tokio::sync::Mutex<StatsCache>> =
    LazyLock::new(|| tokio::sync::Mutex::new(StatsCache::default()));

fn stats_cache_ttl() -> Duration {
    Duration::from_secs(crate::env::u64("CODASAURUS_STATS_CACHE_TTL_SECS", 30))
}

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(stats))
}

/// One row of the recent-activity panel.
#[derive(sqlx::FromRow)]
struct RecentReview {
    id: i64,
    repo_name: String,
    pr_number: i64,
    pr_title: Option<String>,
    status: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(sqlx::FromRow)]
struct DayCount {
    day: chrono::NaiveDate,
    count: i64,
}

#[derive(sqlx::FromRow)]
struct DetectorCount {
    detector: String,
    count: i64,
}

#[derive(sqlx::FromRow)]
struct StatusCount {
    status: String,
    count: i64,
}

/// GET /api/stats
#[utoipa::path(get, path = "/api/stats", tag = "stats", responses((status = 200, description = "OK"), (status = 401, description = "Authentication required")))]
pub(crate) async fn stats(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    // Belt and suspenders: middleware already gates protected routes, but the
    // handler must stay closed even if served from the unguarded `router()`.
    super::rbac::require_role(&state.pool, &headers, super::rbac::MinRole::Viewer).await?;
    let mut cache = STATS_CACHE.lock().await;
    if cache
        .fetched_at
        .is_some_and(|at| at.elapsed() < stats_cache_ttl())
    {
        if let Some(cached) = &cache.value {
            return Ok(Json(cached.clone()));
        }
    }

    // Independent reads run together. Core rollups propagate errors; optional
    // panels degrade to empty so one slow table cannot blank the dashboard.
    let pool = &state.pool;
    let (
        core,
        recent_rows,
        dismissals,
        tier1,
        reviews_by_day,
        findings_by_day,
        findings_by_detector,
        outcomes_7d,
        findings_last_7,
        findings_prev_7,
        reviews_prev_7,
        dismissals_prev_7,
        pass_rate_7d,
        pass_rate_prev_7d,
        spend_usd_last_day,
        daily_budget_usd,
    ) = tokio::join!(
        db::reviews::get_stats(pool),
        async {
            crate::db::db_fetch_all!(
                pool,
                RecentReview,
                "SELECT r.id, COALESCE(repo.full_name, '') AS repo_name, r.pr_number, r.pr_title, r.status, r.created_at
                 FROM reviews r
                 LEFT JOIN repos repo ON repo.id = r.repo_id
                 ORDER BY r.created_at DESC
                 LIMIT 10"
            )
        },
        async {
            crate::db::db_scalar!(pool, i64, "SELECT COUNT(*) FROM dismissed_findings").unwrap_or(0)
        },
        async {
            crate::db::db_scalar!(
                pool,
                i64,
                "SELECT COUNT(*) FROM findings WHERE detector IN ('secrets','vulnerabilities','iac','hallucinated-imports','phantom-deps')
                 AND review_id IN (SELECT id FROM reviews WHERE created_at >= NOW() - INTERVAL '30 days')"
            )
            .unwrap_or(0)
        },
        async {
            crate::db::db_fetch_all!(
                pool,
                DayCount,
                "SELECT (created_at AT TIME ZONE 'UTC')::date AS day, COUNT(*)::bigint AS count
                 FROM reviews
                 WHERE created_at >= (NOW() AT TIME ZONE 'UTC')::date - INTERVAL '13 days'
                 GROUP BY 1
                 ORDER BY 1 ASC"
            )
            .unwrap_or_default()
        },
        async {
            crate::db::db_fetch_all!(
                pool,
                DayCount,
                "SELECT (r.created_at AT TIME ZONE 'UTC')::date AS day, COUNT(*)::bigint AS count
                 FROM findings f
                 INNER JOIN reviews r ON r.id = f.review_id
                 WHERE r.created_at >= (NOW() AT TIME ZONE 'UTC')::date - INTERVAL '13 days'
                 GROUP BY 1
                 ORDER BY 1 ASC"
            )
            .unwrap_or_default()
        },
        async {
            crate::db::db_fetch_all!(
                pool,
                DetectorCount,
                "SELECT f.detector, COUNT(*)::bigint AS count
                 FROM findings f
                 INNER JOIN reviews r ON r.id = f.review_id
                 WHERE r.created_at >= NOW() - INTERVAL '30 days'
                 GROUP BY f.detector
                 ORDER BY count DESC
                 LIMIT 12"
            )
            .unwrap_or_default()
        },
        async {
            crate::db::db_fetch_all!(
                pool,
                StatusCount,
                "SELECT status, COUNT(*)::bigint AS count
                 FROM reviews
                 WHERE created_at >= NOW() - INTERVAL '7 days'
                 GROUP BY status"
            )
            .unwrap_or_default()
        },
        async {
            crate::db::db_scalar!(
                pool,
                i64,
                "SELECT COUNT(*) FROM findings f
                 INNER JOIN reviews r ON r.id = f.review_id
                 WHERE r.created_at >= NOW() - INTERVAL '7 days'"
            )
            .unwrap_or(0)
        },
        async {
            crate::db::db_scalar!(
                pool,
                i64,
                "SELECT COUNT(*) FROM findings f
                 INNER JOIN reviews r ON r.id = f.review_id
                 WHERE r.created_at >= NOW() - INTERVAL '14 days'
                   AND r.created_at < NOW() - INTERVAL '7 days'"
            )
            .unwrap_or(0)
        },
        async {
            crate::db::db_scalar!(
                pool,
                i64,
                "SELECT COUNT(*) FROM reviews
                 WHERE created_at >= NOW() - INTERVAL '14 days'
                   AND created_at < NOW() - INTERVAL '7 days'"
            )
            .unwrap_or(0)
        },
        async {
            crate::db::db_scalar!(
                pool,
                i64,
                "SELECT COUNT(*) FROM dismissed_findings
                 WHERE dismissed_at >= NOW() - INTERVAL '14 days'
                   AND dismissed_at < NOW() - INTERVAL '7 days'"
            )
            .unwrap_or(0)
        },
        async {
            crate::db::db_scalar!(
                pool,
                Option<f64>,
                "SELECT AVG(CASE WHEN status = 'passed' THEN 100.0 WHEN status = 'failed' THEN 0.0 ELSE NULL END)::float8
                 FROM reviews
                 WHERE created_at >= NOW() - INTERVAL '7 days'"
            )
            .ok()
            .flatten()
        },
        async {
            crate::db::db_scalar!(
                pool,
                Option<f64>,
                "SELECT AVG(CASE WHEN status = 'passed' THEN 100.0 WHEN status = 'failed' THEN 0.0 ELSE NULL END)::float8
                 FROM reviews
                 WHERE created_at >= NOW() - INTERVAL '14 days'
                   AND created_at < NOW() - INTERVAL '7 days'"
            )
            .ok()
            .flatten()
        },
        crate::db::events::spend_usd_last_day(pool),
        crate::llm::budget::daily_budget_usd(Some(pool)),
    );
    let mut core: serde_json::Value = core?;
    let recent_rows: Vec<RecentReview> = recent_rows?;

    let activity: Vec<serde_json::Value> = recent_rows
        .into_iter()
        .map(|rev| {
            json!({
                "id": rev.id,
                "repo": rev.repo_name,
                "pr_number": rev.pr_number,
                "pr_title": rev.pr_title,
                "status": rev.status,
                "created_at": rev.created_at,
            })
        })
        .collect();

    if let Some(obj) = core.as_object_mut() {
        obj.insert("recent_activity".into(), json!(activity));
    }

    let total_findings = core
        .get("total_findings")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);

    // Trust panel: FP proxy + accept rate from dismissals vs Tier-1 findings.
    let fp_proxy = if tier1 == 0 {
        0.0
    } else {
        dismissals as f64 / tier1 as f64
    };
    // Accept proxy: clamp dismissals so rate stays in [0, 100] even when tables diverge.
    let accept_rate = if total_findings == 0 {
        None
    } else {
        let dismissed_capped = dismissals.min(total_findings);
        Some(((total_findings - dismissed_capped) as f64 / total_findings as f64) * 100.0)
    };
    if let Some(obj) = core.as_object_mut() {
        obj.insert(
            "trust".into(),
            json!({
                "dismissals": dismissals,
                "tier1_findings": tier1,
                "fp_proxy_ratio": fp_proxy,
                "accept_rate": accept_rate,
                "note": "Proxy metrics: dismissals table vs current findings rows — not a lifetime cohort.",
            }),
        );
        obj.insert(
            "llm".into(),
            json!({
                "requests": crate::metrics::llm_request_count(),
                "prompt_chars": crate::metrics::llm_prompt_chars_total(),
                "spend_usd_estimate": crate::metrics::llm_spend_usd_estimate(),
                "spend_usd_last_day": spend_usd_last_day,
                "daily_budget_usd": daily_budget_usd,
                "note": "Estimates from agent_events + process metrics; not billing truth.",
            }),
        );
    }

    // Team analytics: daily series, detector hits, weekly digest + prior-week deltas.
    // (All series were fetched in the join! wave above.)

    if let Some(obj) = core.as_object_mut() {
        let today = chrono::Utc::now().date_naive();
        let start = today - chrono::Duration::days(13);
        let review_map: std::collections::HashMap<_, _> = reviews_by_day
            .into_iter()
            .map(|d| (d.day, d.count))
            .collect();
        let finding_map: std::collections::HashMap<_, _> = findings_by_day
            .into_iter()
            .map(|d| (d.day, d.count))
            .collect();

        let mut series = Vec::with_capacity(14);
        for offset in 0..14 {
            let day = start + chrono::Duration::days(offset);
            series.push(json!({
                "day": day.to_string(),
                "reviews": review_map.get(&day).copied().unwrap_or(0),
                "findings": finding_map.get(&day).copied().unwrap_or(0),
            }));
        }

        let detector_total: i64 = findings_by_detector.iter().map(|d| d.count).sum();
        let mut concern_counts: std::collections::BTreeMap<&'static str, i64> =
            std::collections::BTreeMap::new();
        for d in &findings_by_detector {
            let concern = crate::bot::concern::concern_for_detector(&d.detector);
            *concern_counts.entry(concern).or_default() += d.count;
        }
        let concerns: Vec<serde_json::Value> = concern_counts
            .into_iter()
            .map(|(concern, count)| {
                let share = if detector_total == 0 {
                    0.0
                } else {
                    (count as f64 / detector_total as f64) * 100.0
                };
                json!({
                    "concern": concern,
                    "count": count,
                    "share_pct": share,
                })
            })
            .collect();

        let detectors: Vec<serde_json::Value> = findings_by_detector
            .into_iter()
            .map(|d| {
                let share = if detector_total == 0 {
                    0.0
                } else {
                    (d.count as f64 / detector_total as f64) * 100.0
                };
                json!({
                    "detector": d.detector,
                    "concern": crate::bot::concern::concern_for_detector(&d.detector),
                    "count": d.count,
                    "share_pct": share,
                })
            })
            .collect();

        let mut passed_7d = 0i64;
        let mut failed_7d = 0i64;
        let mut other_7d = 0i64;
        for row in outcomes_7d {
            match row.status.as_str() {
                "passed" => passed_7d = row.count,
                "failed" => failed_7d = row.count,
                _ => other_7d += row.count,
            }
        }

        let dismiss_week = obj
            .get("dismissals_last_7_days")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let reviews_week = obj
            .get("reviews_last_7_days")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let dismiss_rate = if findings_last_7 == 0 {
            None
        } else {
            Some((dismiss_week as f64 / findings_last_7 as f64) * 100.0)
        };

        obj.insert(
            "analytics".into(),
            json!({
                "reviews_by_day": series,
                "findings_by_detector": detectors,
                "findings_by_concern": concerns,
                "findings_last_7_days": findings_last_7,
                "findings_prev_7_days": findings_prev_7,
                "reviews_prev_7_days": reviews_prev_7,
                "dismissals_prev_7_days": dismissals_prev_7,
                "dismiss_rate_last_7_days": dismiss_rate,
                "pass_rate_7d": pass_rate_7d,
                "pass_rate_prev_7d": pass_rate_prev_7d,
                "outcomes_7d": {
                    "passed": passed_7d,
                    "failed": failed_7d,
                    "other": other_7d,
                },
                "weekly_digest": {
                    "reviews": reviews_week,
                    "findings": findings_last_7,
                    "dismissals": dismiss_week,
                    "note": "Postgres-backed rollup for the last 7 days vs prior 7 days.",
                },
            }),
        );
    }

    *cache = StatsCache {
        fetched_at: Some(Instant::now()),
        value: Some(core.clone()),
    };

    Ok(Json(core))
}
