//! Periodic DB maintenance (sessions, webhooks, finished jobs).

use crate::db::{db_execute, DbPool};

/// Opportunistic cleanup — safe to call often; each statement is cheap with indexes.
pub async fn run_periodic_cleanup(pool: &DbPool) {
    let (sessions, webhooks, jobs, _, _) = tokio::join!(
        async { db_execute!(pool, "DELETE FROM sessions WHERE expires_at < NOW()") },
        async {
            db_execute!(
                pool,
                "DELETE FROM webhook_deliveries WHERE received_at < NOW() - INTERVAL '14 days'"
            )
        },
        async {
            db_execute!(
                pool,
                "DELETE FROM review_jobs
                 WHERE status IN ('done', 'failed')
                   AND updated_at < NOW() - INTERVAL '30 days'"
            )
        },
        crate::db::events::prune_older_than_days(pool, 30),
        crate::db::audit::prune_older_than_days(pool, crate::db::audit::retention_days()),
    );
    if let Err(error) = sessions {
        tracing::debug!(error = %error, "session cleanup skipped");
    }
    if let Err(error) = webhooks {
        tracing::debug!(error = %error, "webhook cleanup skipped");
    }
    if let Err(error) = jobs {
        tracing::debug!(error = %error, "review_jobs cleanup skipped");
    }
}
