//! Durable review-queue worker.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::time::timeout;

use super::auth::get_installation_token;
use super::queue;
use super::review::{fetch_pull_request, review_pr_with_options, ReviewOptions};
use super::{
    bot_db_pool, current_bot_config, pr_lock, prune_pr_lock, BotConfig, WebhookPayload,
    REVIEW_PERMITS,
};
use crate::bot_runtime::BotRuntimeConfig;

/// Wake the durable queue worker when a job is enqueued.
pub(crate) static QUEUE_NOTIFY: std::sync::LazyLock<tokio::sync::Notify> =
    std::sync::LazyLock::new(tokio::sync::Notify::new);

static LISTENER_STARTED: AtomicBool = AtomicBool::new(false);

/// Wake workers on committed enqueues. `NOTIFY` is only a latency hint; the
/// poll loop remains the correctness path. A dedicated connection is required
/// because LISTEN is session state and does not work through transaction pooling.
fn start_queue_listener() {
    if LISTENER_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let Ok(raw) = std::env::var("DATABASE_URL") else {
        tracing::warn!("DATABASE_URL not set; queue wake-ups use polling only");
        return;
    };
    let url = crate::db::ensure_connect_timeout(
        &crate::db::ensure_sslmode(&crate::db::normalize_database_url(&raw)),
        30,
    );
    tokio::spawn(async move {
        loop {
            match sqlx::postgres::PgListener::connect(&url).await {
                Ok(mut listener) => {
                    if let Err(e) = listener.listen("review_jobs_ready").await {
                        tracing::warn!(error = %e, "queue LISTEN failed");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        continue;
                    }
                    // Drain anything committed before LISTEN took effect.
                    QUEUE_NOTIFY.notify_waiters();
                    loop {
                        match tokio::time::timeout(Duration::from_secs(30), listener.try_recv())
                            .await
                        {
                            Ok(Ok(Some(_))) => QUEUE_NOTIFY.notify_waiters(),
                            Ok(Ok(None)) => break, // connection closed; reconnect
                            Ok(Err(e)) => {
                                tracing::warn!(error = %e, "queue listener error");
                                break;
                            }
                            Err(_) => QUEUE_NOTIFY.notify_waiters(), // fallback scan
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "queue listener reconnect failed");
                }
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}

/// Start durable review workers, bounded by `CODASAURUS_QUEUE_WORKERS` or the
/// available review permits. Credentials are read per job so wizard updates and
/// key rotations take effect without a restart.
pub fn start_review_worker(pool: crate::db::DbPool) {
    start_queue_listener();
    let default_n = REVIEW_PERMITS.available_permits().clamp(1, 4);
    let n = std::env::var("CODASAURUS_QUEUE_WORKERS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default_n)
        .clamp(1, 8);

    tracing::info!(workers = n, "starting review queue workers");
    for worker_id in 0..n {
        let pool = pool.clone();
        tokio::spawn(async move {
            tracing::info!(worker_id, "review queue worker started");
            let mut idle_ticks: u64 = 0;
            loop {
                let timeout_secs = BotRuntimeConfig::default().review_timeout_secs;
                // Must exceed review timeout or a second worker reclaims mid-run.
                let stale_secs = (timeout_secs as i64).saturating_add(120).max(600);
                let batch_size = std::env::var("CODASAURUS_CLAIM_BATCH")
                    .ok()
                    .and_then(|v| v.parse::<i64>().ok())
                    .unwrap_or(1)
                    .clamp(1, 50);
                match queue::claim_batch(&pool, stale_secs, batch_size).await {
                    Ok(jobs) if !jobs.is_empty() => {
                        idle_ticks = 0;
                        // Run the batch concurrently: processing sequentially would
                        // let later jobs' leases expire while earlier ones run.
                        let mut set = tokio::task::JoinSet::new();
                        for job in jobs {
                            let pool = pool.clone();
                            set.spawn(
                                async move { process_claimed_job(pool, job, timeout_secs).await },
                            );
                        }
                        while set.join_next().await.is_some() {}
                    }
                    Ok(_) => {
                        idle_ticks = idle_ticks.saturating_add(1);
                        // ~every 60s of idle (2s poll × 30) — only worker 0 runs cleanup
                        if worker_id == 0 && idle_ticks.is_multiple_of(30) {
                            crate::bot::maintenance::run_periodic_cleanup(&pool).await;
                        }
                        tokio::select! {
                            _ = QUEUE_NOTIFY.notified() => {}
                            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
                        }
                    }
                    Err(e) => {
                        tracing::warn!(worker_id, error = %e, "queue claim failed");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        });
    }
}

/// Process one claimed job in its own task, with a lease heartbeat.
async fn process_claimed_job(pool: crate::db::DbPool, job: queue::ReviewJob, timeout_secs: u64) {
    let heartbeat = bot_db_pool().map(|pool| {
        let lease = job.lease_id.clone();
        let id = job.id;
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(10)).await;
                match queue::heartbeat(pool, id, &lease).await {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(e) => tracing::warn!(error = %e, "queue heartbeat failed"),
                }
            }
        })
    });
    let Some(cfg) = current_bot_config() else {
        if let Some(task) = heartbeat {
            task.abort();
        }
        tracing::warn!(job_id = job.id, "no bot config; requeueing after delay");
        let _ = queue::mark_failed(
            &pool,
            job.id,
            &job.lease_id,
            "bot config not configured",
            job.attempts,
            job.max_attempts,
        )
        .await;
        tokio::time::sleep(Duration::from_secs(5)).await;
        return;
    };
    if cfg.app_id.is_empty() || cfg.private_key.is_empty() {
        if let Some(task) = heartbeat {
            task.abort();
        }
        tracing::warn!(
            job_id = job.id,
            "incomplete GitHub App credentials; delaying job"
        );
        let _ = queue::mark_failed(
            &pool,
            job.id,
            &job.lease_id,
            "github app credentials incomplete",
            job.attempts,
            job.max_attempts,
        )
        .await;
        tokio::time::sleep(Duration::from_secs(10)).await;
        return;
    }
    process_queued_review(
        &cfg,
        job.id,
        &job.repo,
        job.pr_number,
        &job.head_sha,
        job.installation_id,
        &job.action,
        job.attempts,
        job.max_attempts,
        &job.lease_id,
        heartbeat,
        timeout_secs,
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
async fn process_queued_review(
    cfg: &BotConfig,
    job_id: i64,
    repo: &str,
    pr_number: i64,
    head_sha: &str,
    inst_id: Option<i64>,
    _action: &str,
    attempts: i64,
    max_attempts: i64,
    lease_id: &str,
    heartbeat: Option<tokio::task::JoinHandle<()>>,
    timeout_secs: u64,
) {
    let Ok(_permit) = REVIEW_PERMITS.acquire().await else {
        tracing::error!("review semaphore closed");
        return;
    };

    let pool = bot_db_pool();
    let lock = pr_lock(repo, pr_number).await;
    let _guard = lock.lock().await;
    let started = std::time::Instant::now();

    let opts = ReviewOptions {
        auto_describe: false,
        auto_review_diff: true,
        force_draft: false,
        force_rereview: false,
    };

    let result = timeout(Duration::from_secs(timeout_secs), async {
        let token = get_installation_token(cfg, inst_id).await?;
        let pr_data = fetch_pull_request(&token, repo, pr_number).await?;
        let wrapped = WebhookPayload::for_pull_request(Some(pr_data));
        review_pr_with_options(&token, repo, &wrapped, opts).await
    })
    .await;

    match result {
        Ok(Ok(())) => {
            crate::metrics::record_review_ok(started);
            tracing::info!(job_id, repo, pr = pr_number, "queued review completed");
            if let Some(pool) = pool {
                let _ = queue::mark_done(pool, job_id, lease_id).await;
            }
        }
        Ok(Err(e)) => {
            crate::metrics::record_review_failed();
            tracing::error!(job_id, error = %e, "queued review failed");
            release_claim_best_effort(repo, pr_number, head_sha).await;
            if let Some(pool) = pool {
                match queue::mark_failed(
                    pool,
                    job_id,
                    lease_id,
                    &e.to_string(),
                    attempts,
                    max_attempts,
                )
                .await
                {
                    Ok(queue::Outcome::Dead) => {
                        tracing::error!(job_id, repo, pr = pr_number, error = %e, "review job dead-lettered")
                    }
                    Ok(queue::Outcome::LeaseLost) => {
                        tracing::warn!(job_id, "lease lost while recording failure")
                    }
                    _ => {}
                }
            }
        }
        Err(_) => {
            crate::metrics::record_review_timeout();
            tracing::error!(job_id, "queued review timed out");
            release_claim_best_effort(repo, pr_number, head_sha).await;
            if let Some(pool) = pool {
                match queue::mark_failed(pool, job_id, lease_id, "timeout", attempts, max_attempts)
                    .await
                {
                    Ok(queue::Outcome::Dead) => tracing::error!(
                        job_id,
                        repo,
                        pr = pr_number,
                        "review job dead-lettered after timeout"
                    ),
                    Ok(queue::Outcome::LeaseLost) => {
                        tracing::warn!(job_id, "lease lost while recording timeout")
                    }
                    _ => {}
                }
            }
        }
    }

    if let Some(task) = heartbeat {
        task.abort();
    }
    drop(_guard);
    prune_pr_lock(repo, pr_number, lock).await;
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_webhook_review_inline(
    cfg: BotConfig,
    repo_full_name: String,
    pr_number: i64,
    pr: Option<serde_json::Value>,
    inst_id: Option<i64>,
    _action: String,
    head_sha: String,
    timeout_secs: u64,
) {
    let Ok(_permit) = REVIEW_PERMITS.acquire().await else {
        tracing::error!("review semaphore closed");
        return;
    };
    let lock = pr_lock(&repo_full_name, pr_number).await;
    let _guard = lock.lock().await;
    let started = std::time::Instant::now();
    let opts = ReviewOptions {
        auto_describe: false,
        auto_review_diff: true,
        force_draft: false,
        force_rereview: false,
    };
    let repo_for_claim = repo_full_name.clone();
    let sha_for_claim = head_sha.clone();
    match timeout(Duration::from_secs(timeout_secs), async move {
        let token = get_installation_token(&cfg, inst_id).await?;
        let wrapped = WebhookPayload::for_pull_request(pr);
        review_pr_with_options(&token, &repo_full_name, &wrapped, opts).await
    })
    .await
    {
        Ok(Ok(())) => {
            crate::metrics::record_review_ok(started);
            tracing::info!("review completed");
        }
        Ok(Err(e)) => {
            crate::metrics::record_review_failed();
            tracing::error!(error = %e, "review failed");
            release_claim_best_effort(&repo_for_claim, pr_number, &sha_for_claim).await;
        }
        Err(_) => {
            crate::metrics::record_review_timeout();
            tracing::error!("review timed out");
            release_claim_best_effort(&repo_for_claim, pr_number, &sha_for_claim).await;
        }
    }
    drop(_guard);
    prune_pr_lock(&repo_for_claim, pr_number, lock).await;
}

pub(crate) async fn release_claim_best_effort(repo: &str, pr_number: i64, head_sha: &str) {
    if head_sha.is_empty() {
        return;
    }
    let Some(pool) = bot_db_pool() else {
        return;
    };
    let state = crate::state::ReviewState::from_pool(pool);
    if let Err(e) = state.release_sha_claim(repo, pr_number, head_sha).await {
        tracing::warn!(error = %e, "failed to release SHA claim");
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn enqueue_notifies_ready_listeners() {
        let _guard = crate::db::TEST_POOL_LOCK.lock().await;
        let url = std::env::var("CODASAURUS_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| {
                "postgres://codasaurus:codasaurus@127.0.0.1:5432/codasaurus".into()
            });
        let pool = crate::db::create_pool(&url).await.expect("pool");
        let mut listener = sqlx::postgres::PgListener::connect_with(pool.as_pg())
            .await
            .expect("listener connects");
        listener
            .listen("review_jobs_ready")
            .await
            .expect("LISTEN succeeds");

        let repo = format!("acme/notify-{}", uuid::Uuid::new_v4().simple());
        let _ = sqlx::query("DELETE FROM review_jobs WHERE repo = $1")
            .bind(&repo)
            .execute(pool.as_pg())
            .await;
        let _ = crate::bot::queue::enqueue(&pool, &repo, 100, "notify-sha", None, "opened")
            .await
            .expect("enqueue");

        let notification =
            tokio::time::timeout(std::time::Duration::from_secs(2), listener.try_recv())
                .await
                .expect("notification within 2s");
        assert!(
            notification.expect("notification").is_some(),
            "listener got a notification"
        );
        let _ = sqlx::query("DELETE FROM review_jobs WHERE repo = $1")
            .bind(&repo)
            .execute(pool.as_pg())
            .await;
    }
}
