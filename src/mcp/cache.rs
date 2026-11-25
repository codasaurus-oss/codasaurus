//! TTL cache for `tools/list` results — mirrors `crate::registry`'s cache
//! pattern so reviews don't refetch tool schemas from every MCP server.

use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, LazyLock, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::OnceCell;

const TTL_SECS: u64 = 300;
const MAX_CACHED_SERVERS: usize = 256;

struct CachedTools {
    base_url: String,
    tools: Vec<Value>,
    fetched_at: Instant,
}

impl CachedTools {
    fn is_fresh(&self, base_url: &str) -> bool {
        self.base_url == base_url && self.fetched_at.elapsed() < Duration::from_secs(TTL_SECS)
    }
}

type Cache = HashMap<String, Arc<OnceCell<CachedTools>>>;

static CACHE: LazyLock<RwLock<Cache>> = LazyLock::new(|| RwLock::new(HashMap::new()));

pub async fn get_or_fetch_tools(server_id: &str, base_url: &str, api_key: &str) -> Vec<Value> {
    get_or_fetch_with(server_id, base_url, || {
        super::client::tools_list(base_url, api_key)
    })
    .await
}

async fn get_or_fetch_with<F, Fut>(server_id: &str, base_url: &str, fetch: F) -> Vec<Value>
where
    F: Fn() -> Fut,
    Fut: Future<Output = anyhow::Result<Vec<Value>>>,
{
    loop {
        let cell = {
            let mut cache = CACHE.write().unwrap_or_else(|error| {
                tracing::warn!("MCP tools cache poisoned; recovering");
                error.into_inner()
            });
            let replace = cache
                .get(server_id)
                .is_none_or(|cell| cell.get().is_some_and(|entry| !entry.is_fresh(base_url)));
            if replace {
                let cell = Arc::new(OnceCell::new());
                trim_cache(&mut cache, server_id);
                cache.insert(server_id.to_string(), Arc::clone(&cell));
                cell
            } else {
                Arc::clone(cache.get(server_id).expect("cache entry was just checked"))
            }
        };

        let entry = cell
            .get_or_init(|| async {
                let fetched_at = Instant::now();
                match fetch().await {
                    Ok(tools) => CachedTools {
                        base_url: base_url.to_string(),
                        tools,
                        fetched_at,
                    },
                    Err(error) => {
                        tracing::warn!(server_id, error = %error, "MCP tools/list failed; skipping server");
                        CachedTools {
                            base_url: base_url.to_string(),
                            tools: Vec::new(),
                            fetched_at,
                        }
                    }
                }
            })
            .await;
        if entry.base_url == base_url {
            return entry.tools.clone();
        }
    }
}

fn trim_cache(cache: &mut Cache, server_id: &str) {
    if cache.contains_key(server_id) || cache.len() < MAX_CACHED_SERVERS {
        return;
    }
    let now = Instant::now();
    cache.retain(|_, cell| {
        cell.get().is_none_or(|entry| {
            now.duration_since(entry.fetched_at) < Duration::from_secs(TTL_SECS)
        })
    });
    if cache.len() < MAX_CACHED_SERVERS {
        return;
    }
    let oldest = cache
        .iter()
        .filter_map(|(id, cell)| cell.get().map(|entry| (id.clone(), entry.fetched_at)))
        .min_by_key(|(_, fetched_at)| *fetched_at);
    if let Some((id, _)) = oldest {
        cache.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn server_id() -> String {
        format!("cache-test-{}", uuid::Uuid::new_v4())
    }

    #[tokio::test]
    async fn concurrent_misses_share_one_fetch() {
        let id = server_id();
        let calls = Arc::new(AtomicUsize::new(0));
        let fetch_calls = Arc::clone(&calls);
        let fetch = || async {
            tokio::task::yield_now().await;
            fetch_calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![json!({ "name": "get_issue" })])
        };

        let (first, second) = tokio::join!(
            get_or_fetch_with(&id, "https://example.test/mcp", fetch),
            get_or_fetch_with(&id, "https://example.test/mcp", fetch),
        );

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(first, second);
        assert_eq!(first[0]["name"], "get_issue");
    }

    #[tokio::test]
    async fn changing_the_server_url_invalidates_cached_tools() {
        let id = server_id();
        let calls = Arc::new(AtomicUsize::new(0));
        let first_calls = Arc::clone(&calls);
        let first = get_or_fetch_with(&id, "https://old.example.test/mcp", || async {
            first_calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![json!({ "name": "old_tool" })])
        })
        .await;
        let second_calls = Arc::clone(&calls);
        let second = get_or_fetch_with(&id, "https://new.example.test/mcp", || async {
            second_calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![json!({ "name": "new_tool" })])
        })
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(first[0]["name"], "old_tool");
        assert_eq!(second[0]["name"], "new_tool");
    }

    #[tokio::test]
    async fn failed_fetches_are_negatively_cached() {
        let id = server_id();
        let calls = Arc::new(AtomicUsize::new(0));
        let fetch_calls = Arc::clone(&calls);
        let fetch = || async {
            fetch_calls.fetch_add(1, Ordering::SeqCst);
            Err::<Vec<Value>, _>(anyhow::anyhow!("unavailable"))
        };

        let first = get_or_fetch_with(&id, "https://example.test/mcp", fetch).await;
        let second = get_or_fetch_with(&id, "https://example.test/mcp", fetch).await;

        assert!(first.is_empty());
        assert!(second.is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cache_size_stays_bounded() {
        for index in 0..MAX_CACHED_SERVERS + 1 {
            let id = format!("{}-{index}", server_id());
            let tools = get_or_fetch_with(&id, "https://example.test/mcp", || async {
                Ok(Vec::<Value>::new())
            })
            .await;
            assert!(tools.is_empty());
        }

        let cache = CACHE.read().unwrap();
        assert!(cache.len() <= MAX_CACHED_SERVERS);
    }
}
