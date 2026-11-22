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

#[tokio::test]
async fn oidc_state_from_first_replica_is_consumed_by_second_replica() {
    let a = StateStore::connect(&redis_url())
        .await
        .expect("connect first redis state store");
    let b = StateStore::connect(&redis_url())
        .await
        .expect("connect second redis state store");
    let state = format!("oidc-test-{}", uuid::Uuid::new_v4());

    codasaurus::oidc::store_pending(&a, &state, "verifier-1")
        .await
        .expect("store pending oidc state");
    assert_eq!(
        codasaurus::oidc::take_pending(&b, &state).await,
        Some("verifier-1".to_string())
    );
}

#[tokio::test]
async fn oidc_state_is_single_use() {
    let a = StateStore::connect(&redis_url())
        .await
        .expect("connect first redis state store");
    let b = StateStore::connect(&redis_url())
        .await
        .expect("connect second redis state store");
    let state = format!("oidc-test-{}", uuid::Uuid::new_v4());

    codasaurus::oidc::store_pending(&a, &state, "verifier-1")
        .await
        .expect("store pending oidc state");
    assert_eq!(
        codasaurus::oidc::take_pending(&a, &state).await,
        Some("verifier-1".to_string())
    );
    assert_eq!(codasaurus::oidc::take_pending(&b, &state).await, None);
}
