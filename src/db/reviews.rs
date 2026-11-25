use crate::db::models::*;
use crate::db::{db_fetch_all, db_fetch_one, db_fetch_optional, DbPool};
use sqlx::FromRow;

#[derive(Debug, sqlx::FromRow)]
pub struct ReviewWithRepo {
    #[sqlx(flatten)]
    pub review: Review,
    pub repo_full_name: String,
}

pub async fn list_reviews(
    pool: &DbPool,
    repo_id: Option<i64>,
    status: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<ReviewWithRepo>, sqlx::Error> {
    let mut query = sqlx::QueryBuilder::new(
        "SELECT r.*, COALESCE(repo.full_name, '') AS repo_full_name
         FROM reviews r LEFT JOIN repos repo ON repo.id = r.repo_id",
    );
    let mut filtered = false;
    if let Some(repo_id) = repo_id {
        query.push(" WHERE r.repo_id = ").push_bind(repo_id);
        filtered = true;
    }
    if let Some(status) = status {
        query
            .push(if filtered {
                " AND r.status = "
            } else {
                " WHERE r.status = "
            })
            .push_bind(status);
    }
    query
        .push(" ORDER BY r.created_at DESC LIMIT ")
        .push_bind(limit)
        .push(" OFFSET ")
        .push_bind(offset);
    let rows = query.build().fetch_all(pool.as_pg()).await?;
    rows.into_iter()
        .map(|row| ReviewWithRepo::from_row(&row))
        .collect()
}

pub async fn get_review_with_repo(
    pool: &DbPool,
    id: i64,
) -> Result<Option<ReviewWithRepo>, sqlx::Error> {
    db_fetch_optional!(
        pool,
        ReviewWithRepo,
        "SELECT r.*, repo.full_name AS repo_full_name
         FROM reviews r JOIN repos repo ON repo.id = r.repo_id
         WHERE r.id = ?",
        id
    )
}

pub async fn create_review(pool: &DbPool, review: &ReviewCreate) -> Result<Review, sqlx::Error> {
    db_fetch_one!(
        pool,
        Review,
        "INSERT INTO reviews (repo_id, pr_number, pr_title, pr_author, pr_base_branch, pr_head_branch, pr_head_sha)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(repo_id, pr_number, pr_head_sha) DO UPDATE SET
           pr_title = excluded.pr_title,
           pr_author = excluded.pr_author,
           pr_base_branch = excluded.pr_base_branch,
           pr_head_branch = excluded.pr_head_branch,
           status = 'pending',
           started_at = NULL,
           completed_at = NULL,
           summary_json = NULL
         RETURNING *",
        review.repo_id,
        review.pr_number,
        &review.pr_title,
        &review.pr_author,
        &review.pr_base_branch,
        &review.pr_head_branch,
        review.pr_head_sha.as_deref().unwrap_or("")
    )
}

async fn delete_findings_on(
    conn: &mut sqlx::PgConnection,
    review_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM findings WHERE review_id = $1")
        .bind(review_id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

pub async fn replace_findings(
    pool: &DbPool,
    review_id: i64,
    findings: &[FindingCreate],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.as_pg().begin().await?;
    delete_findings_on(&mut tx, review_id).await?;
    insert_findings_on(&mut tx, findings).await?;
    // An uncommitted transaction rolls back on drop, preserving prior findings on error.
    tx.commit().await
}

pub async fn update_review(
    pool: &DbPool,
    id: i64,
    update: &ReviewUpdate,
) -> Result<(), sqlx::Error> {
    if update.status.is_none() && update.summary_json.is_none() && update.completed_at.is_none() {
        return Ok(());
    }
    let mut query = sqlx::QueryBuilder::new("UPDATE reviews SET ");
    let mut first = true;
    if let Some(status) = update.status.as_deref() {
        query
            .push(if first { "status = " } else { ", status = " })
            .push_bind(status);
        first = false;
    }
    if let Some(summary) = update.summary_json.as_deref() {
        query
            .push(if first {
                "summary_json = "
            } else {
                ", summary_json = "
            })
            .push_bind(summary);
        first = false;
    }
    if let Some(completed_at) = update.completed_at {
        query
            .push(if first {
                "completed_at = "
            } else {
                ", completed_at = "
            })
            .push_bind(completed_at);
    }
    query.push(" WHERE id = ").push_bind(id);
    query.build().execute(pool.as_pg()).await?;
    Ok(())
}

pub async fn get_findings_for_review(
    pool: &DbPool,
    review_id: i64,
) -> Result<Vec<Finding>, sqlx::Error> {
    db_fetch_all!(
        pool,
        Finding,
        "SELECT * FROM findings WHERE review_id = ? ORDER BY severity, file_path",
        review_id
    )
}

/// Findings from the most recent completed review for a PR, in one query.
pub async fn get_latest_completed_findings_for_pr(
    pool: &DbPool,
    repo_id: i64,
    pr_number: i64,
) -> Result<Vec<Finding>, sqlx::Error> {
    db_fetch_all!(
        pool,
        Finding,
        "SELECT f.* FROM findings f
         WHERE f.review_id = (
           SELECT r.id FROM reviews r
           WHERE r.repo_id = ? AND r.pr_number = ?
             AND r.status IN ('passed', 'failed')
           ORDER BY r.completed_at DESC NULLS LAST, r.created_at DESC
           LIMIT 1
         )
         ORDER BY f.severity, f.file_path",
        repo_id,
        pr_number
    )
}

pub async fn create_findings_batch(
    pool: &DbPool,
    findings: &[FindingCreate],
) -> Result<(), sqlx::Error> {
    let mut conn = pool.as_pg().acquire().await?;
    insert_findings_on(&mut conn, findings).await
}

async fn insert_findings_on(
    conn: &mut sqlx::PgConnection,
    findings: &[FindingCreate],
) -> Result<(), sqlx::Error> {
    if findings.is_empty() {
        return Ok(());
    }

    // Single multi-row INSERT via UNNEST — one round-trip per review.
    let mut review_ids = Vec::with_capacity(findings.len());
    let mut fingerprints: Vec<Option<String>> = Vec::with_capacity(findings.len());
    let mut file_paths = Vec::with_capacity(findings.len());
    let mut line_starts: Vec<Option<i32>> = Vec::with_capacity(findings.len());
    let mut line_ends: Vec<Option<i32>> = Vec::with_capacity(findings.len());
    let mut column_starts: Vec<Option<i32>> = Vec::with_capacity(findings.len());
    let mut column_ends: Vec<Option<i32>> = Vec::with_capacity(findings.len());
    let mut severities = Vec::with_capacity(findings.len());
    let mut detectors = Vec::with_capacity(findings.len());
    let mut rule_ids: Vec<Option<String>> = Vec::with_capacity(findings.len());
    let mut messages = Vec::with_capacity(findings.len());
    let mut suggested_fixes: Vec<Option<String>> = Vec::with_capacity(findings.len());
    let mut code_snippets: Vec<Option<String>> = Vec::with_capacity(findings.len());
    let mut contexts: Vec<Option<String>> = Vec::with_capacity(findings.len());
    let mut categories: Vec<Option<String>> = Vec::with_capacity(findings.len());
    let mut confidences: Vec<Option<i32>> = Vec::with_capacity(findings.len());
    let mut judge_rationales: Vec<Option<String>> = Vec::with_capacity(findings.len());

    for f in findings {
        review_ids.push(f.review_id);
        fingerprints.push(f.fingerprint.clone());
        file_paths.push(f.file_path.clone());
        line_starts.push(f.line_start);
        line_ends.push(f.line_end);
        column_starts.push(f.column_start);
        column_ends.push(f.column_end);
        severities.push(f.severity.clone());
        detectors.push(f.detector.clone());
        rule_ids.push(f.rule_id.clone());
        messages.push(f.message.clone());
        suggested_fixes.push(f.suggested_fix.clone());
        code_snippets.push(f.code_snippet.clone());
        contexts.push(f.context.clone());
        categories.push(f.category.clone());
        confidences.push(f.confidence);
        judge_rationales.push(f.judge_rationale.clone());
    }

    sqlx::query(
        "INSERT INTO findings (
            review_id, fingerprint, file_path, line_start, line_end,
            column_start, column_end, severity, detector, rule_id,
            message, suggested_fix, code_snippet, context, category,
            confidence, judge_rationale
         )
         SELECT * FROM UNNEST(
            $1::bigint[], $2::text[], $3::text[], $4::int4[], $5::int4[],
            $6::int4[], $7::int4[], $8::text[], $9::text[], $10::text[],
            $11::text[], $12::text[], $13::text[], $14::text[], $15::text[],
            $16::int4[], $17::text[]
         )
         ON CONFLICT DO NOTHING",
    )
    .bind(&review_ids)
    .bind(&fingerprints)
    .bind(&file_paths)
    .bind(&line_starts)
    .bind(&line_ends)
    .bind(&column_starts)
    .bind(&column_ends)
    .bind(&severities)
    .bind(&detectors)
    .bind(&rule_ids)
    .bind(&messages)
    .bind(&suggested_fixes)
    .bind(&code_snippets)
    .bind(&contexts)
    .bind(&categories)
    .bind(&confidences)
    .bind(&judge_rationales)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

pub async fn get_stats(pool: &DbPool) -> Result<serde_json::Value, sqlx::Error> {
    #[derive(sqlx::FromRow)]
    struct StatsRow {
        total_repos: i64,
        total_reviews_today: i64,
        pass_rate: Option<f64>,
        total_findings: i64,
        reviews_last_7_days: i64,
        dismissals_last_7_days: i64,
    }

    let stats: StatsRow = sqlx::query_as(
        "SELECT
           (SELECT COUNT(*) FROM repos WHERE active = $1) AS total_repos,
           (SELECT COUNT(*) FROM reviews
            WHERE created_at >= CURRENT_DATE
              AND created_at < CURRENT_DATE + INTERVAL '1 day') AS total_reviews_today,
           (SELECT AVG(CASE WHEN status = 'passed' THEN 100.0 WHEN status = 'failed' THEN 0.0 ELSE NULL END)::float8
            FROM reviews
            WHERE created_at >= NOW() - INTERVAL '30 days') AS pass_rate,
           (SELECT COUNT(*) FROM findings f
            INNER JOIN reviews r ON r.id = f.review_id
            WHERE r.created_at >= NOW() - INTERVAL '30 days') AS total_findings,
           (SELECT COUNT(*) FROM reviews
            WHERE created_at >= NOW() - INTERVAL '7 days') AS reviews_last_7_days,
           (SELECT COUNT(*) FROM dismissed_findings
            WHERE dismissed_at >= NOW() - INTERVAL '7 days') AS dismissals_last_7_days",
    )
    .bind(true)
    .fetch_one(pool.as_pg())
    .await?;

    Ok(serde_json::json!({
        "total_repos": stats.total_repos,
        "total_reviews_today": stats.total_reviews_today,
        "pass_rate": stats.pass_rate,
        "total_findings": stats.total_findings,
        "reviews_last_7_days": stats.reviews_last_7_days,
        "dismissals_last_7_days": stats.dismissals_last_7_days,
    }))
}
