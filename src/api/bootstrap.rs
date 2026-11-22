//! Single-use bootstrap secret for day-zero owner creation.
//!
//! The operator arms a secret at startup via `CODASAURUS_BOOTSTRAP_SECRET`;
//! the setup handler consumes it atomically when the first owner is created.

use sha2::{Digest, Sha256};
use std::time::Duration;

use super::errors::ApiError;
use crate::redis::StateStore;

pub const BOOTSTRAP_SECRET_HEADER: &str = "x-bootstrap-secret";

/// Redis key namespaced by a hash of the secret so the raw value never leaves
/// process memory or appears in key listings.
pub fn secret_key(secret: &str) -> String {
    let digest = Sha256::digest(secret.as_bytes());
    format!("codasaurus:bootstrap:{}", hex::encode(digest))
}

/// Arm the secret for `ttl`. Idempotent so a restart inside the window does not
/// discard an already-issued single-use secret.
pub async fn arm(store: &StateStore, secret: &str, ttl: Duration) -> anyhow::Result<()> {
    store.set_nx_ex(&secret_key(secret), "1", ttl).await?;
    Ok(())
}

/// Consume the secret. GETDEL is atomic, so exactly one concurrent caller wins.
pub async fn consume(store: &StateStore, secret: &str) -> Result<(), ApiError> {
    match store.getdel(&secret_key(secret)).await {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(ApiError::unauthorized("Invalid or expired bootstrap secret")),
        Err(err) => {
            tracing::error!(error = %err, "bootstrap secret lookup failed");
            Err(ApiError::internal("bootstrap secret lookup failed"))
        }
    }
}
