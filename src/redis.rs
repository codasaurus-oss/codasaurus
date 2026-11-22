//! Shared Redis state: a cheap cloneable handle used by the API and webhook.

use ::redis::aio::ConnectionManager;
use ::redis::AsyncCommands;
use std::time::Duration;

/// Cloneable handle to a multiplexed Redis connection.
#[derive(Clone)]
pub struct StateStore {
    conn: ConnectionManager,
}

// The Lua script gives every counter a TTL, including the first increment.
const INCR_EXPIRES: &str = "local n = redis.call('INCR', KEYS[1])
if n == 1 then redis.call('EXPIRE', KEYS[1], ARGV[1]) end
return n";

impl StateStore {
    /// Connect to Redis, failing at startup rather than silently degrading.
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        let client = ::redis::Client::open(url)?;
        let conn = ConnectionManager::new(client).await?;
        Ok(Self { conn })
    }

    /// Increment `key` and set its TTL on the first increment.
    pub async fn incr_expires(&self, key: &str, ttl: Duration) -> Result<u64, ::redis::RedisError> {
        let mut conn = self.conn.clone();
        ::redis::cmd("EVAL")
            .arg(INCR_EXPIRES)
            .arg(1)
            .arg(key)
            .arg(ttl.as_secs().max(1))
            .query_async::<u64>(&mut conn)
            .await
    }

    /// Set `key` to `value` with a TTL in seconds.
    pub async fn set_ex(
        &self,
        key: &str,
        value: &str,
        ttl: Duration,
    ) -> Result<(), ::redis::RedisError> {
        let mut conn = self.conn.clone();
        conn.set_ex(key, value, ttl.as_secs().max(1)).await
    }

    /// Atomically read and delete `key`, returning its value if it existed.
    pub async fn getdel(&self, key: &str) -> Result<Option<String>, ::redis::RedisError> {
        let mut conn = self.conn.clone();
        ::redis::cmd("GETDEL")
            .arg(key)
            .query_async::<Option<String>>(&mut conn)
            .await
    }
}
