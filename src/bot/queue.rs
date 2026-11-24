//! Durable review job queue (Postgres `FOR UPDATE SKIP LOCKED`).
//!
//! Jobs carry an ownership lease and a `run_at` timestamp. A claim is a single
//! atomic statement; finishing a job is only valid while the lease still
//! matches. This keeps at-least-once execution safe when a worker stalls.

use crate::db::{db_fetch_one, DbPool};
use anyhow::Result;

const QUEUE_NAME: &str = "reviews";

#[derive(Debug, Clone)]
pub struct ReviewJob {
    pub id: i64,
    pub repo: String,
    pub pr_number: i64,
    pub head_sha: String,
    pub installation_id: Option<i64>,
    pub action: String,
    pub attempts: i64,
    pub max_attempts: i64,
    pub lease_id: String,
    pub priority: i16,
    pub queue_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Scheduled { delay_secs: i64 },
    Dead,
    LeaseLost,
}

#[derive(Debug, Clone, Copy)]
pub struct QueueStats {
    pub pending: i64,
    pub running: i64,
    pub failed: i64,
    pub dead: i64,
    pub oldest_ready_age_secs: i64,
}

/// Enqueue (or refresh) a review job for this SHA. Idempotent per (repo, pr, sha).
pub async fn enqueue(
    pool: &DbPool,
    repo: &str,
    pr_number: i64,
    head_sha: &str,
    installation_id: Option<i64>,
    action: &str,
) -> Result<i64> {
    let priority = std::env::var("CODASAURUS_QUEUE_DEFAULT_PRIORITY")
        .ok()
        .and_then(|v| v.parse::<i16>().ok())
        .unwrap_or(100)
        .clamp(-100, 100);
    enqueue_with_priority(
        pool,
        repo,
        pr_number,
        head_sha,
        installation_id,
        action,
        priority,
    )
    .await
}

/// Enqueue with an explicit priority (smaller runs first).
pub async fn enqueue_with_priority(
    pool: &DbPool,
    repo: &str,
    pr_number: i64,
    head_sha: &str,
    installation_id: Option<i64>,
    action: &str,
    priority: i16,
) -> Result<i64> {
    let row: (i64,) = db_fetch_one!(
        pool,
        (i64,),
        "INSERT INTO review_jobs (repo, pr_number, head_sha, installation_id, action, status, attempts, priority, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, 'pending', 0, ?, NOW(), NOW())
         ON CONFLICT(repo, pr_number, head_sha) DO UPDATE SET
           status = CASE
             -- Retry failures only. Do not re-pend a successful or dead job for
             -- the same SHA (duplicate deliveries must not post twice).
             WHEN review_jobs.status = 'failed' THEN 'pending'
             ELSE review_jobs.status
           END,
           installation_id = COALESCE(excluded.installation_id, review_jobs.installation_id),
           action = CASE
             WHEN review_jobs.status = 'failed' THEN excluded.action
             ELSE review_jobs.action
           END,
           updated_at = NOW()
         RETURNING id",
        repo,
        pr_number,
        head_sha,
        installation_id,
        action,
        priority
    )?;
    // Wake-up hint only; polling is the correctness path.
    let _ = sqlx::query("SELECT pg_notify('review_jobs_ready', '')")
        .execute(pool.as_pg())
        .await;
    crate::metrics::record_queue_enqueued();
    Ok(row.0)
}

/// Atomically claim up to `limit` due jobs. Expired leases are reclaimed.
pub async fn claim_batch(pool: &DbPool, stale_secs: i64, limit: i64) -> Result<Vec<ReviewJob>> {
    let lease_id = uuid::Uuid::new_v4();
    let rows = sqlx::query_as::<
        _,
        (
            i64,
            String,
            i64,
            String,
            Option<i64>,
            String,
            i32,
            i16,
            i32,
            String,
            bool,
        ),
    >(
        "WITH picked AS (
           SELECT id, status = 'running' AS reclaimed
             FROM review_jobs
            WHERE queue_name = $4
              AND ((status = 'pending' AND run_at <= NOW())
                OR (status = 'failed' AND run_at <= NOW())
                OR (status = 'running'
                    AND (leased_until IS NULL OR leased_until < NOW())))
            ORDER BY priority ASC, run_at ASC, id ASC
            FOR UPDATE SKIP LOCKED
            LIMIT $3
         )
         UPDATE review_jobs SET
           status = 'running',
           attempts = attempts + 1,
           lease_id = $1::uuid,
           leased_until = NOW() + ($2::bigint * INTERVAL '1 second'),
           updated_at = NOW()
         FROM picked
         WHERE review_jobs.id = picked.id
         RETURNING review_jobs.id, repo, pr_number, head_sha, installation_id, action,
                   attempts, priority, max_attempts, lease_id::text, picked.reclaimed",
    )
    .bind(lease_id.to_string())
    .bind(stale_secs)
    .bind(limit)
    .bind(QUEUE_NAME)
    .fetch_all(pool.as_pg())
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(
                id,
                repo,
                pr_number,
                head_sha,
                installation_id,
                action,
                attempts,
                priority,
                max_attempts,
                lease,
                reclaimed,
            )| {
                if reclaimed {
                    crate::metrics::record_queue_reclaim();
                }
                ReviewJob {
                    id,
                    repo,
                    pr_number,
                    head_sha,
                    installation_id,
                    action,
                    attempts: i64::from(attempts),
                    max_attempts: i64::from(max_attempts),
                    lease_id: lease,
                    priority,
                    queue_name: QUEUE_NAME.to_string(),
                }
            },
        )
        .collect())
}

pub async fn claim_next(pool: &DbPool, stale_secs: i64) -> Result<Option<ReviewJob>> {
    Ok(claim_batch(pool, stale_secs, 1).await?.into_iter().next())
}

/// Refresh the lease while a job is running. False means ownership was lost.
pub async fn heartbeat(pool: &DbPool, id: i64, lease_id: &str) -> Result<bool> {
    let n = sqlx::query(
        "UPDATE review_jobs SET leased_until = NOW() + INTERVAL '600 seconds'
          WHERE id = $1 AND lease_id::text = $2 AND status = 'running'",
    )
    .bind(id)
    .bind(lease_id)
    .execute(pool.as_pg())
    .await?
    .rows_affected();
    Ok(n > 0)
}

/// Mark success only if this worker still owns the lease. False means a stale
/// worker tried to overwrite a reclaimed run.
pub async fn mark_done(pool: &DbPool, id: i64, lease_id: &str) -> Result<bool> {
    let n = sqlx::query(
        "UPDATE review_jobs SET status = 'done', updated_at = NOW(), last_error = NULL,
           lease_id = NULL, leased_until = NULL
          WHERE id = $1 AND lease_id::text = $2 AND status = 'running'",
    )
    .bind(id)
    .bind(lease_id)
    .execute(pool.as_pg())
    .await?
    .rows_affected();
    if n > 0 {
        crate::metrics::record_queue_completed();
    }
    Ok(n > 0)
}

/// Record a failure with exponential backoff and jitter, or dead-letter it.
pub async fn mark_failed(
    pool: &DbPool,
    id: i64,
    lease_id: &str,
    err: &str,
    attempts: i64,
    max_attempts: i64,
) -> Result<Outcome> {
    let msg: String = err.chars().take(500).collect();
    if attempts >= max_attempts {
        let n = sqlx::query(
            "UPDATE review_jobs SET status = 'dead', updated_at = NOW(), last_error = $1,
               lease_id = NULL, leased_until = NULL
             WHERE id = $2 AND lease_id::text = $3 AND status = 'running'",
        )
        .bind(&msg)
        .bind(id)
        .bind(lease_id)
        .execute(pool.as_pg())
        .await?
        .rows_affected();
        if n > 0 {
            crate::metrics::record_queue_dead();
            return Ok(Outcome::Dead);
        }
        return Ok(Outcome::LeaseLost);
    }
    let base = 2_i64.pow(attempts.min(10) as u32).min(3600);
    let jitter = ((id % 41) - 20) as f64 / 100.0;
    let delay_secs = ((base as f64) * (1.0 + jitter)).round().max(1.0) as i64;
    let n = sqlx::query(
        "UPDATE review_jobs SET status = 'failed', updated_at = NOW(), last_error = $1,
           run_at = NOW() + ($2::bigint * INTERVAL '1 second'),
           lease_id = NULL, leased_until = NULL
         WHERE id = $3 AND lease_id::text = $4 AND status = 'running'",
    )
    .bind(&msg)
    .bind(delay_secs)
    .bind(id)
    .bind(lease_id)
    .execute(pool.as_pg())
    .await?
    .rows_affected();
    if n == 0 {
        return Ok(Outcome::LeaseLost);
    }
    crate::metrics::record_queue_retry();
    Ok(Outcome::Scheduled { delay_secs })
}

/// Schedule a review at a future time (cron-like use from the CLI).
pub async fn schedule_review(
    pool: &DbPool,
    repo: &str,
    pr_number: i64,
    head_sha: &str,
    run_at: chrono::DateTime<chrono::Utc>,
    priority: i16,
) -> Result<i64> {
    let row: (i64,) = db_fetch_one!(
        pool,
        (i64,),
        "INSERT INTO review_jobs (repo, pr_number, head_sha, action, status, attempts, priority, run_at, created_at, updated_at)
         VALUES (?, ?, ?, 'scheduled', 'pending', 0, ?, ?, NOW(), NOW())
         ON CONFLICT(repo, pr_number, head_sha) DO UPDATE SET
           run_at = excluded.run_at,
           priority = excluded.priority,
           -- An explicit schedule must run even if this SHA was already reviewed;
           -- leave an in-flight run alone.
           status = CASE WHEN review_jobs.status = 'running' THEN 'running' ELSE 'pending' END,
           attempts = CASE WHEN review_jobs.status = 'running' THEN review_jobs.attempts ELSE 0 END,
           lease_id = CASE WHEN review_jobs.status = 'running' THEN review_jobs.lease_id ELSE NULL END,
           leased_until = CASE WHEN review_jobs.status = 'running' THEN review_jobs.leased_until ELSE NULL END,
           last_error = CASE WHEN review_jobs.status = 'running' THEN review_jobs.last_error ELSE NULL END,
           updated_at = NOW()
         RETURNING id",
        repo,
        pr_number,
        head_sha,
        priority,
        run_at
    )?;
    Ok(row.0)
}

/// Put a dead job back in the queue for an operator-driven retry.
pub async fn replay_dead_job(pool: &DbPool, id: i64) -> Result<bool> {
    let n = sqlx::query(
        "UPDATE review_jobs SET status = 'pending', attempts = 0, run_at = NOW(),
           last_error = NULL, updated_at = NOW()
         WHERE id = $1 AND status = 'dead'",
    )
    .bind(id)
    .execute(pool.as_pg())
    .await?
    .rows_affected();
    Ok(n > 0)
}

pub async fn queue_depth(pool: &DbPool) -> Result<QueueStats, sqlx::Error> {
    let row: (i64, i64, i64, i64, Option<i64>) = sqlx::query_as(
        "SELECT
           COUNT(*) FILTER (WHERE status = 'pending'),
           COUNT(*) FILTER (WHERE status = 'running'),
           COUNT(*) FILTER (WHERE status = 'failed'),
           COUNT(*) FILTER (WHERE status = 'dead'),
           MAX(EXTRACT(EPOCH FROM (NOW() - run_at))::bigint) FILTER (WHERE status IN ('pending', 'failed'))
         FROM review_jobs WHERE queue_name = $1",
    )
    .bind(QUEUE_NAME)
    .fetch_one(pool.as_pg())
    .await?;
    Ok(QueueStats {
        pending: row.0,
        running: row.1,
        failed: row.2,
        dead: row.3,
        oldest_ready_age_secs: row.4.unwrap_or(0).max(0),
    })
}
