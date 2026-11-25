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

#[tokio::test]
async fn bulk_repo_upsert_preserves_local_config_and_active_state() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = db::create_pool(&pg_url()).await.expect("pool");
    let suffix = uuid::Uuid::new_v4().simple();
    let first_github_id = (uuid::Uuid::new_v4().as_u128() & 0x7fff_ffff_ffff_ffff) as i64;
    let second_github_id = (uuid::Uuid::new_v4().as_u128() & 0x7fff_ffff_ffff_ffff) as i64;
    let third_github_id = (uuid::Uuid::new_v4().as_u128() & 0x7fff_ffff_ffff_ffff) as i64;
    let existing_name = format!("acme/bulk-existing-{suffix}");
    let new_name = format!("acme/bulk-new-{suffix}");
    let existing = db::repos::create_repo(
        &pool,
        &db::models::RepoCreate {
            github_id: Some(first_github_id),
            full_name: existing_name.clone(),
            owner: "acme".into(),
            name: format!("bulk-existing-{suffix}"),
            default_branch: Some("main".into()),
            installation_id: 1,
            private: false,
        },
    )
    .await
    .expect("create existing repo");
    let config = r#"{"checks":{"secrets":false}}"#;
    db::repos::update_repo(&pool, existing.id, config, true)
        .await
        .expect("configure existing repo");

    let affected = db::repos::upsert_repos(
        &pool,
        &[
            db::models::RepoCreate {
                github_id: Some(second_github_id),
                full_name: existing_name.clone(),
                owner: "acme".into(),
                name: format!("bulk-existing-{suffix}"),
                default_branch: None,
                installation_id: 3,
                private: true,
            },
            db::models::RepoCreate {
                github_id: Some(third_github_id),
                full_name: new_name.clone(),
                owner: "acme".into(),
                name: format!("bulk-new-{suffix}"),
                default_branch: Some("trunk".into()),
                installation_id: 3,
                private: false,
            },
        ],
    )
    .await
    .expect("bulk upsert");
    assert_eq!(affected, 2);

    let updated = db::repos::get_repo_by_full_name(&pool, &existing_name)
        .await
        .expect("lookup updated repo")
        .expect("updated repo");
    assert_eq!(updated.github_id, Some(second_github_id));
    assert_eq!(updated.installation_id, 3);
    assert!(updated.private);
    assert!(updated.active);
    assert_eq!(updated.default_branch.as_deref(), Some("main"));
    assert_eq!(updated.config_json.as_deref(), Some(config));

    let inserted = db::repos::get_repo_by_full_name(&pool, &new_name)
        .await
        .expect("lookup inserted repo")
        .expect("inserted repo");
    assert!(!inserted.active);

    db::repos::delete_repo(&pool, updated.id)
        .await
        .expect("cleanup");
    db::repos::delete_repo(&pool, inserted.id)
        .await
        .expect("cleanup");
}

#[tokio::test]
async fn bulk_installation_upsert_activates_new_repos() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = db::create_pool(&pg_url()).await.expect("pool");
    let suffix = uuid::Uuid::new_v4().simple();
    let full_name = format!("acme/installed-{suffix}");
    let affected = db::repos::upsert_repos_from_installation(
        &pool,
        &[db::models::RepoCreate {
            github_id: Some((uuid::Uuid::new_v4().as_u128() & 0x7fff_ffff_ffff_ffff) as i64),
            full_name: full_name.clone(),
            owner: "acme".into(),
            name: format!("installed-{suffix}"),
            default_branch: Some("main".into()),
            installation_id: 7,
            private: false,
        }],
    )
    .await
    .expect("installation upsert");
    assert_eq!(affected, 1);

    let repo = db::repos::get_repo_by_full_name(&pool, &full_name)
        .await
        .expect("lookup repo")
        .expect("repo");
    assert!(repo.active);
    db::repos::delete_repo(&pool, repo.id)
        .await
        .expect("cleanup");
}

#[tokio::test]
async fn config_snapshot_reads_only_requested_keys() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = db::create_pool(&pg_url()).await.expect("pool");
    let suffix = uuid::Uuid::new_v4().simple();
    let first = format!("test_config_first_{suffix}");
    let second = format!("test_config_second_{suffix}");
    db::config::set_configs(&pool, &[(&first, "one"), (&second, "two")])
        .await
        .expect("set configs");

    let values = db::config::get_configs(&pool, &[&first, &second])
        .await
        .expect("read configs");
    assert_eq!(values.get(&first).map(String::as_str), Some("one"));
    assert_eq!(values.get(&second).map(String::as_str), Some("two"));
    assert_eq!(values.len(), 2);

    db::config::delete_configs(&pool, &[&first, &second])
        .await
        .expect("cleanup");
}

#[tokio::test]
async fn baseline_batch_persists_all_changed_files() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = db::create_pool(&pg_url()).await.expect("pool");
    let repo = format!("acme/baseline-{}", uuid::Uuid::new_v4().simple());
    let pr_number = 42;
    let first = std::collections::HashSet::from([1usize, 3]);
    let second = std::collections::HashSet::from([2usize]);

    codasaurus::baseline::save_pr_diff_lines_batch(
        &pool,
        &repo,
        pr_number,
        &[("src/first.rs", &first), ("src/second.rs", &second)],
    )
    .await
    .expect("save baseline batch");

    let rows: Vec<(String, i32)> = sqlx::query_as(
        "SELECT file_path, line FROM pr_diff_lines
         WHERE repo_full_name = $1 AND pr_number = $2
         ORDER BY file_path, line",
    )
    .bind(&repo)
    .bind(pr_number)
    .fetch_all(pool.as_pg())
    .await
    .expect("baseline rows");
    assert_eq!(
        rows,
        vec![
            ("src/first.rs".into(), 1),
            ("src/first.rs".into(), 3),
            ("src/second.rs".into(), 2),
        ]
    );
    codasaurus::baseline::clear_pr_diff_lines(&pool, &repo, pr_number)
        .await
        .expect("clear baseline");
}

#[tokio::test]
async fn index_batch_replaces_files_and_removes_stale_edges() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = db::create_pool(&pg_url()).await.expect("pool");
    let repo = format!("acme/index-{}", uuid::Uuid::new_v4().simple());
    use codasaurus::index::extract::{ExtractedEdge, ExtractedSymbol, FileIndex};
    use codasaurus::index::store;

    let files = vec![
        FileIndex {
            file_path: "src/a.rs".into(),
            symbols: vec![ExtractedSymbol {
                name: "old".into(),
                kind: "function".into(),
                signature: None,
                line: 1,
            }],
            edges: vec![ExtractedEdge {
                from_symbol: "old".into(),
                to_symbol: "entry".into(),
                edge_kind: "CALLS".into(),
            }],
        },
        FileIndex {
            file_path: "src/b.rs".into(),
            symbols: vec![ExtractedSymbol {
                name: "entry".into(),
                kind: "function".into(),
                signature: None,
                line: 2,
            }],
            edges: vec![],
        },
    ];
    store::replace_files_index(&pool, &repo, &files)
        .await
        .expect("replace index batch");

    let symbols = store::symbols_in_files(&pool, &repo, &["src/a.rs".into(), "src/b.rs".into()])
        .await
        .expect("symbols batch");
    assert_eq!(symbols.len(), 2);
    let callers = store::callers_of_many(&pool, &repo, &["entry".to_string()])
        .await
        .expect("callers batch");
    assert_eq!(callers.len(), 1);
    assert_eq!(callers[0].0, "old");

    store::replace_files_index(
        &pool,
        &repo,
        &[FileIndex {
            file_path: "src/a.rs".into(),
            symbols: vec![],
            edges: vec![],
        }],
    )
    .await
    .expect("replace emptied file");
    let callers = store::callers_of_many(&pool, &repo, &["entry".to_string()])
        .await
        .expect("callers after replacement");
    assert!(callers.is_empty());

    sqlx::query("DELETE FROM repo_symbols WHERE repo_full_name = $1")
        .bind(&repo)
        .execute(pool.as_pg())
        .await
        .expect("cleanup symbols");
    sqlx::query("DELETE FROM repo_edges WHERE repo_full_name = $1")
        .bind(&repo)
        .execute(pool.as_pg())
        .await
        .expect("cleanup edges");
    sqlx::query("DELETE FROM index_status WHERE repo_full_name = $1")
        .bind(&repo)
        .execute(pool.as_pg())
        .await
        .expect("cleanup status");
}

#[tokio::test]
async fn index_replacement_removes_incoming_edges_to_removed_symbols() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = db::create_pool(&pg_url()).await.expect("pool");
    let repo = format!("acme/index-incoming-{}", uuid::Uuid::new_v4().simple());
    use codasaurus::index::extract::{ExtractedEdge, ExtractedSymbol, FileIndex};
    use codasaurus::index::store;

    let files = vec![
        FileIndex {
            file_path: "src/a.rs".into(),
            symbols: vec![ExtractedSymbol {
                name: "entry".into(),
                kind: "function".into(),
                signature: None,
                line: 1,
            }],
            edges: vec![],
        },
        FileIndex {
            file_path: "src/b.rs".into(),
            symbols: vec![ExtractedSymbol {
                name: "caller".into(),
                kind: "function".into(),
                signature: None,
                line: 1,
            }],
            edges: vec![ExtractedEdge {
                from_symbol: "caller".into(),
                to_symbol: "entry".into(),
                edge_kind: "CALLS".into(),
            }],
        },
    ];
    store::replace_files_index(&pool, &repo, &files)
        .await
        .expect("replace index batch");
    assert_eq!(
        store::callers_of_many(&pool, &repo, &["entry".to_string()])
            .await
            .expect("callers before replacement")
            .len(),
        1
    );

    store::replace_files_index(
        &pool,
        &repo,
        &[FileIndex {
            file_path: "src/a.rs".into(),
            symbols: vec![],
            edges: vec![],
        }],
    )
    .await
    .expect("replace removed symbol");
    assert!(store::callers_of_many(&pool, &repo, &["entry".to_string()])
        .await
        .expect("callers after replacement")
        .is_empty());

    sqlx::query("DELETE FROM repo_symbols WHERE repo_full_name = $1")
        .bind(&repo)
        .execute(pool.as_pg())
        .await
        .expect("cleanup symbols");
    sqlx::query("DELETE FROM repo_edges WHERE repo_full_name = $1")
        .bind(&repo)
        .execute(pool.as_pg())
        .await
        .expect("cleanup edges");
    sqlx::query("DELETE FROM index_status WHERE repo_full_name = $1")
        .bind(&repo)
        .execute(pool.as_pg())
        .await
        .expect("cleanup status");
}

#[tokio::test]
async fn semantic_reindex_removes_embeddings_for_removed_symbols() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = db::create_pool(&pg_url()).await.expect("pool");
    let has_embeddings: bool =
        sqlx::query_scalar("SELECT to_regclass('repo_symbol_embeddings') IS NOT NULL")
            .fetch_one(pool.as_pg())
            .await
            .expect("embedding table probe");
    if !has_embeddings {
        tracing::warn!("pgvector unavailable; semantic embedding cleanup test not applicable");
        return;
    }

    let repo = format!("acme/index-embedding-{}", uuid::Uuid::new_v4().simple());
    let file_path = "src/removed.py";
    sqlx::query(
        "INSERT INTO repo_symbol_embeddings
            (repo_full_name, file_path, symbol_name, line, content_hash, embedding, model)
         VALUES ($1, $2, 'removed', 1, 'hash', array_fill(0::real, ARRAY[1536])::vector, 'test')",
    )
    .bind(&repo)
    .bind(file_path)
    .execute(pool.as_pg())
    .await
    .expect("insert stale embedding");

    let llm_cfg = codasaurus::llm::LlmConfig {
        api_key: String::new(),
        model: String::new(),
        text_model: String::new(),
        max_tokens: 0,
        temperature: 0.0,
        base_url: String::new(),
        embedding_model: String::new(),
    };
    let files = vec![codasaurus::index::extract::FileIndex {
        file_path: file_path.into(),
        symbols: vec![],
        edges: vec![],
    }];
    codasaurus::index::semantic::reindex_repo_embeddings(&pool, &llm_cfg, &repo, &files)
        .await
        .expect("semantic reindex");

    let remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM repo_symbol_embeddings
         WHERE repo_full_name = $1 AND file_path = $2",
    )
    .bind(&repo)
    .bind(file_path)
    .fetch_one(pool.as_pg())
    .await
    .expect("remaining embeddings");
    assert_eq!(remaining, 0);

    sqlx::query("DELETE FROM repo_symbol_embeddings WHERE repo_full_name = $1")
        .bind(&repo)
        .execute(pool.as_pg())
        .await
        .expect("cleanup embeddings");
}

#[tokio::test]
async fn recent_maintainer_dismissals_are_ordered_by_recency() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = db::create_pool(&pg_url()).await.expect("pool");
    let suffix = uuid::Uuid::new_v4().simple();
    let repo = format!("acme/learning-recent-{suffix}");
    let old_detector = format!("a-old-{suffix}");
    let new_detector = format!("z-new-{suffix}");
    for (fingerprint, detector, message, age) in [
        ("old", old_detector.as_str(), "old noise", "2 days"),
        ("new", new_detector.as_str(), "new noise", "1 hour"),
    ] {
        sqlx::query(
            "INSERT INTO dismissed_findings
                (fingerprint, repo_full_name, detector, file, line, message, dismissed_at, is_maintainer)
             VALUES ($1, $2, $3, '', 0, $4, NOW() - $5::interval, TRUE)",
        )
        .bind(fingerprint)
        .bind(&repo)
        .bind(detector)
        .bind(message)
        .bind(age)
        .execute(pool.as_pg())
        .await
        .expect("insert dismissal");
    }

    let store = codasaurus::learning::store::LearningStore::from_pool(&pool);
    let rows = store
        .recent_maintainer_dismissals(Some(&repo), 2)
        .await
        .expect("recent dismissals");
    assert!(rows.contains(&(new_detector, "new noise".to_string())));
    assert!(!rows
        .iter()
        .any(|(detector, _)| detector.starts_with("a-old-")));

    sqlx::query("DELETE FROM dismissed_findings WHERE repo_full_name = $1")
        .bind(&repo)
        .execute(pool.as_pg())
        .await
        .expect("cleanup dismissals");
}

#[tokio::test]
async fn latest_completed_findings_ignore_newer_pending_reviews() {
    let _guard = TEST_SERIAL.lock().await;
    let pool = db::create_pool(&pg_url()).await.expect("pool");
    let suffix = uuid::Uuid::new_v4().simple();
    let full_name = format!("acme/prior-{suffix}");
    let repo = db::repos::create_repo(
        &pool,
        &db::models::RepoCreate {
            github_id: Some((uuid::Uuid::new_v4().as_u128() & 0x7fff_ffff_ffff_ffff) as i64),
            full_name: full_name.clone(),
            owner: "acme".into(),
            name: format!("prior-{suffix}"),
            default_branch: Some("main".into()),
            installation_id: 1,
            private: false,
        },
    )
    .await
    .expect("repo");
    let create_review = |sha: String| db::models::ReviewCreate {
        repo_id: repo.id,
        pr_number: 9,
        pr_title: None,
        pr_author: None,
        pr_base_branch: None,
        pr_head_branch: None,
        pr_head_sha: Some(sha),
    };
    let completed = db::reviews::create_review(&pool, &create_review(format!("done-{suffix}")))
        .await
        .expect("completed review");
    db::reviews::update_review(
        &pool,
        completed.id,
        &db::models::ReviewUpdate {
            status: Some("passed".into()),
            summary_json: None,
            completed_at: Some(chrono::Utc::now()),
        },
    )
    .await
    .expect("complete review");
    let pending = db::reviews::create_review(&pool, &create_review(format!("pending-{suffix}")))
        .await
        .expect("pending review");
    let finding = |review_id: i64, message: &str| db::models::FindingCreate {
        review_id,
        fingerprint: Some(format!("{review_id}:{message}")),
        file_path: "src/main.rs".into(),
        line_start: None,
        line_end: None,
        column_start: None,
        column_end: None,
        severity: "warning".into(),
        detector: "test".into(),
        rule_id: None,
        message: message.into(),
        suggested_fix: None,
        code_snippet: None,
        context: None,
        category: None,
        confidence: Some(4),
        judge_rationale: Some("grounded".into()),
    };
    db::reviews::create_findings_batch(
        &pool,
        &[
            finding(completed.id, "completed"),
            finding(pending.id, "pending"),
        ],
    )
    .await
    .expect("findings");

    let prior = db::reviews::get_latest_completed_findings_for_pr(&pool, repo.id, 9)
        .await
        .expect("prior findings");
    assert_eq!(prior.len(), 1);
    assert_eq!(prior[0].message, "completed");
    assert_eq!(prior[0].confidence, Some(4));
    assert_eq!(prior[0].judge_rationale.as_deref(), Some("grounded"));
    db::repos::delete_repo(&pool, repo.id)
        .await
        .expect("cleanup");
}
