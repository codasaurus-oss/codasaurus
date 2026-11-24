//! Postgres smoke: migrations, queue claim, users/sessions.
//!
//! Uses `DATABASE_URL` or `CODASAURUS_TEST_DATABASE_URL`
//! (defaults to `postgres://codasaurus:codasaurus@127.0.0.1:5432/codasaurus`).

use codasaurus::bot::queue::{self, Outcome};
use codasaurus::db;
use std::sync::LazyLock;

static TEST_SERIAL: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

fn pg_url() -> String {
    std::env::var("CODASAURUS_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| "postgres://codasaurus:codasaurus@127.0.0.1:5432/codasaurus".into())
}

#[tokio::test]
async fn queue_recovers_null_leases_and_reports_lost_leases() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = db::create_pool(&pg_url()).await.expect("pool");
    let _ = sqlx::query("DELETE FROM review_jobs")
        .execute(pool.as_pg())
        .await;

    let repo = format!("acme/lease-null-{}", uuid::Uuid::new_v4().simple());
    let id = queue::enqueue(&pool, &repo, 92, "nulllease", None, "opened")
        .await
        .expect("enqueue");
    sqlx::query("UPDATE review_jobs SET status = 'running', lease_id = NULL, leased_until = NULL WHERE id = $1")
        .bind(id)
        .execute(pool.as_pg())
        .await
        .expect("simulate pre-v28 running row");
    let job = queue::claim_next(&pool, 600)
        .await
        .expect("reclaim")
        .expect("job");
    assert_eq!(job.id, id);

    assert_eq!(
        queue::mark_failed(&pool, id, "not-the-lease", "boom", 1, 5)
            .await
            .expect("mark_failed"),
        Outcome::LeaseLost
    );
    let _ = sqlx::query("DELETE FROM review_jobs WHERE repo = $1")
        .bind(&repo)
        .execute(pool.as_pg())
        .await;
}

#[tokio::test]
async fn schedule_review_repends_finished_job() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = db::create_pool(&pg_url()).await.expect("pool");
    let _ = sqlx::query("DELETE FROM review_jobs")
        .execute(pool.as_pg())
        .await;

    let repo = format!("acme/schedule-{}", uuid::Uuid::new_v4().simple());
    let id = queue::enqueue(&pool, &repo, 5, "sched", None, "opened")
        .await
        .expect("enqueue");
    let job = queue::claim_next(&pool, 600)
        .await
        .expect("claim")
        .expect("job");
    assert!(queue::mark_done(&pool, id, &job.lease_id)
        .await
        .expect("done"));

    let when = chrono::Utc::now() + chrono::Duration::seconds(1);
    let scheduled = queue::schedule_review(&pool, &repo, 5, "sched", when, 7)
        .await
        .expect("schedule");
    assert_eq!(scheduled, id);

    let (status, priority): (String, i16) =
        sqlx::query_as("SELECT status, priority FROM review_jobs WHERE id = $1")
            .bind(id)
            .fetch_one(pool.as_pg())
            .await
            .expect("row");
    assert_eq!(status, "pending");
    assert_eq!(priority, 7);
    assert!(
        queue::claim_next(&pool, 600)
            .await
            .expect("claim")
            .is_none(),
        "scheduled job must not run before run_at"
    );

    let _ = sqlx::query("DELETE FROM review_jobs WHERE repo = $1")
        .bind(&repo)
        .execute(pool.as_pg())
        .await;
}

#[tokio::test]
async fn migration_v27_adds_lease_and_scheduling_columns() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = db::create_pool(&pg_url())
        .await
        .expect("postgres create_pool / migrations");
    let _ = sqlx::query("DELETE FROM review_jobs")
        .execute(pool.as_pg())
        .await;

    let repo = format!("acme/schema-{}", uuid::Uuid::new_v4().simple());
    let id = queue::enqueue(&pool, &repo, 77, "schemasha", None, "opened")
        .await
        .expect("enqueue");
    let row: (
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
        i16,
        String,
        i32,
    ) = sqlx::query_as(
        "SELECT created_at, run_at, priority, queue_name, max_attempts
               FROM review_jobs WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool.as_pg())
    .await
    .expect("new columns exist");
    assert_eq!(row.0, row.1, "run_at backfilled from created_at");
    assert_eq!(row.2, 100);
    assert_eq!(row.3, "reviews");
    assert_eq!(row.4, 5);

    db::migrations::run_migrations(pool.as_pg())
        .await
        .expect("idempotent migrations");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM review_jobs WHERE id = $1")
        .bind(id)
        .fetch_one(pool.as_pg())
        .await
        .expect("count");
    assert_eq!(count, 1);

    let _ = sqlx::query("DELETE FROM review_jobs WHERE repo = $1")
        .bind(&repo)
        .execute(pool.as_pg())
        .await;
}

#[tokio::test]
async fn queue_lease_backoff_and_dead_letter() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = db::create_pool(&pg_url()).await.expect("pool");
    let _ = sqlx::query("DELETE FROM review_jobs")
        .execute(pool.as_pg())
        .await;
    let repo = format!("acme/lease-{}", uuid::Uuid::new_v4().simple());
    let id = queue::enqueue(&pool, &repo, 91, "leasesha", None, "opened")
        .await
        .expect("enqueue");

    let first = queue::claim_next(&pool, 600)
        .await
        .expect("claim")
        .expect("job");
    assert_eq!(first.id, id);
    assert!(queue::claim_next(&pool, 600)
        .await
        .expect("claim")
        .is_none());

    sqlx::query("UPDATE review_jobs SET leased_until = NOW() - INTERVAL '1 second' WHERE id = $1")
        .bind(id)
        .execute(pool.as_pg())
        .await
        .expect("expire lease");
    let second = queue::claim_next(&pool, 600)
        .await
        .expect("reclaim")
        .expect("job");
    assert_ne!(first.lease_id, second.lease_id);
    assert!(
        !queue::mark_done(&pool, id, &first.lease_id)
            .await
            .expect("stale done"),
        "stale lease must not mark done"
    );
    let status: String = sqlx::query_scalar("SELECT status FROM review_jobs WHERE id = $1")
        .bind(id)
        .fetch_one(pool.as_pg())
        .await
        .expect("status");
    assert_eq!(status, "running");

    let outcome = queue::mark_failed(&pool, id, &second.lease_id, "rate limited", 1, 5)
        .await
        .expect("mark_failed");
    assert!(matches!(outcome, Outcome::Scheduled { delay_secs } if delay_secs >= 1));
    let run_at_future: bool =
        sqlx::query_scalar("SELECT run_at > NOW() FROM review_jobs WHERE id = $1")
            .bind(id)
            .fetch_one(pool.as_pg())
            .await
            .expect("run_at");
    assert!(run_at_future, "retry must be scheduled into the future");

    sqlx::query("UPDATE review_jobs SET run_at = NOW() WHERE id = $1")
        .bind(id)
        .execute(pool.as_pg())
        .await
        .expect("make retry due");
    let third = queue::claim_next(&pool, 600)
        .await
        .expect("claim after retry")
        .expect("job");
    let outcome = queue::mark_failed(&pool, id, &third.lease_id, "still failing", 5, 5)
        .await
        .expect("dead letter");
    assert_eq!(outcome, Outcome::Dead);
    assert!(queue::replay_dead_job(&pool, id).await.expect("replay"));
    let status: String = sqlx::query_scalar("SELECT status FROM review_jobs WHERE id = $1")
        .bind(id)
        .fetch_one(pool.as_pg())
        .await
        .expect("status");
    assert_eq!(status, "pending");

    let _ = sqlx::query("DELETE FROM review_jobs WHERE repo = $1")
        .bind(&repo)
        .execute(pool.as_pg())
        .await;
}

#[tokio::test]
async fn postgres_migrations_enqueue_and_claim() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = db::create_pool(&pg_url())
        .await
        .expect("postgres create_pool / migrations");

    pool.ping().await.expect("ping");

    let repo = format!("acme/demo-{}", uuid::Uuid::new_v4().simple());
    let id = queue::enqueue(&pool, &repo, 42, "deadbeef", Some(1), "opened")
        .await
        .expect("enqueue");
    assert!(id > 0);

    let job = queue::claim_next(&pool, 600)
        .await
        .expect("claim")
        .expect("expected a job");
    assert_eq!(job.repo, repo);
    assert_eq!(job.pr_number, 42);

    queue::mark_done(&pool, job.id, &job.lease_id)
        .await
        .expect("mark_done");

    let email = format!(
        "pg-smoke-{}@example.com",
        chrono::Utc::now().timestamp_millis()
    );
    let user = db::users::create_user(&pool, &email, "test-pass-123!", "owner")
        .await
        .expect("create_user");
    assert_eq!(user.email, email);
    let token = db::sessions::create_session(&pool, &user.email)
        .await
        .expect("session");
    assert_eq!(token.len(), 64);

    let _ = sqlx::query("DELETE FROM review_jobs WHERE repo = $1")
        .bind(&repo)
        .execute(pool.as_pg())
        .await;
}
