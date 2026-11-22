//! Redis-backed state: the limiter counter must be visible across connections.

use codasaurus::api::rate_limit::check;
use codasaurus::redis::StateStore;
use std::time::Duration;

fn redis_url() -> String {
    std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into())
}

#[tokio::test]
async fn rate_limit_counter_shared_across_two_state_stores() {
    let a = StateStore::connect(&redis_url())
        .await
        .expect("connect first redis state store");
    let b = StateStore::connect(&redis_url())
        .await
        .expect("connect second redis state store");
    let key = format!("shared-key-{}", uuid::Uuid::new_v4());

    assert!(check(&a, &key, 1, Duration::from_secs(60)).await.is_ok());
    assert!(check(&b, &key, 1, Duration::from_secs(60)).await.is_err());
}
