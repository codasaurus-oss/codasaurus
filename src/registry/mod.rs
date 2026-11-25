//! Package registry + OSV lookups.
//!
//! Async is the source of truth (org-scale reviews). Sync wrappers use
//! `block_on` for sync callers; the bot already runs detectors in
//! `spawn_blocking`, so sync wrappers there are safe.

use crate::retry::{is_reqwest_error_retryable, retry_async, RetryConfig};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

pub(crate) use crate::util::block_on;

const CACHE_MAX_SIZE: usize = 10_000;
/// Bound concurrent registry/OSV fan-out when warming caches for large PRs.
const PREFETCH_CONCURRENCY: usize = 12;
const MAX_PREFETCH_PACKAGES: usize = 200;
const OSV_DETAIL_CONCURRENCY: usize = 8;
const OSV_API_URL: &str = "https://api.osv.dev";
/// Short TTL for transient failures so we don't stampede npm/OSV on outages.
const SOFT_FAIL_TTL_SECS: u64 = 120;

mod crates_io;
mod go;
mod npm;
mod pypi;

#[derive(Clone, Copy)]
enum PkgCacheVal {
    Exists(bool),
    /// Network/unknown failure — do not treat as "package missing".
    SoftFail,
}

static CACHE_TTL: AtomicU64 = AtomicU64::new(3600);

static CACHE: LazyLock<RwLock<HashMap<String, (PkgCacheVal, Instant)>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// When true, registry/OSV never open sockets — cache hits only (air-gap).
static OFFLINE_MODE: AtomicBool = AtomicBool::new(false);

pub fn set_offline_mode(enabled: bool) {
    OFFLINE_MODE.store(enabled, Ordering::Relaxed);
}

pub fn offline_mode() -> bool {
    if OFFLINE_MODE.load(Ordering::Relaxed) {
        return true;
    }
    std::env::var("CODASAURUS_OFFLINE")
        .map(|v| crate::util::flag_truthy(&v))
        .unwrap_or(false)
}

type OsvCacheEntry = (Vec<OsvVulnerability>, Instant);
type OsvCacheMap = HashMap<String, OsvCacheEntry>;

static OSV_CACHE: LazyLock<RwLock<OsvCacheMap>> = LazyLock::new(|| RwLock::new(HashMap::new()));

#[derive(Debug, Clone)]
pub struct Metadata {
    pub name: String,
    pub license: Option<String>,
}

type MetadataCacheEntry = (Option<Metadata>, Instant);
type MetadataCacheMap = HashMap<String, MetadataCacheEntry>;

static METADATA_CACHE: LazyLock<RwLock<MetadataCacheMap>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Shared async HTTP client used by all registry lookups (npm, PyPI, crates.io, OSV).
static ASYNC_CLIENT: LazyLock<Option<reqwest::Client>> = LazyLock::new(|| {
    match reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .connect_timeout(Duration::from_secs(5))
        .pool_max_idle_per_host(16)
        .pool_idle_timeout(Duration::from_secs(90))
        .build()
    {
        Ok(c) => Some(c),
        Err(e) => {
            tracing::warn!(error = %e, "failed to build async registry HTTP client");
            None
        }
    }
});

/// Shared registry GET with the user agent Codasaurus uses for every registry.
pub(super) async fn fetch_json(
    url: &str,
    accept: Option<&str>,
) -> Result<Option<serde_json::Value>> {
    let client = async_client().context("registry HTTP client not available")?;
    let mut request = client.get(url).header(
        "User-Agent",
        concat!("codasaurus/", env!("CARGO_PKG_VERSION")),
    );
    if let Some(accept) = accept {
        request = request.header("Accept", accept);
    }
    let response = request.send().await?;
    if response.status().as_u16() != 200 {
        return Ok(None);
    }
    Ok(Some(response.json().await?))
}

/// Status-only registry probe; the caller maps codes to exists/missing/soft-fail.
pub(super) async fn status_code(
    url: &str,
    method: reqwest::Method,
    accept: Option<&str>,
) -> Result<u16> {
    let client = async_client().context("registry HTTP client not available")?;
    let mut request = client.request(method, url).header(
        "User-Agent",
        concat!("codasaurus/", env!("CARGO_PKG_VERSION")),
    );
    if let Some(accept) = accept {
        request = request.header("Accept", accept);
    }
    Ok(request.send().await?.status().as_u16())
}

pub fn async_client() -> Result<&'static reqwest::Client> {
    ASYNC_CLIENT
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("async registry HTTP client not available"))
}

pub(super) fn registry_url(base: &str, package: &str) -> String {
    format!("{base}/{}", crate::util::url_encode(package))
}

pub(super) fn classify_status(status: u16, found: &[u16], missing: &[u16]) -> Option<bool> {
    if found.contains(&status) {
        Some(true)
    } else if missing.contains(&status) {
        Some(false)
    } else {
        None
    }
}

pub(super) async fn check_status(
    url: &str,
    method: reqwest::Method,
    accept: Option<&str>,
    found: &[u16],
    missing: &[u16],
) -> Result<Option<bool>> {
    let status = status_code(url, method, accept).await?;
    Ok(classify_status(status, found, missing))
}

pub(super) async fn fetch_metadata(
    url: &str,
    accept: Option<&str>,
    package: &str,
    extract_license: fn(&serde_json::Value) -> Option<String>,
) -> Result<Option<Metadata>> {
    let Some(data) = fetch_json(url, accept).await? else {
        return Ok(None);
    };
    Ok(Some(Metadata {
        name: package.to_string(),
        license: extract_license(&data),
    }))
}

pub fn set_cache_ttl(secs: u64) {
    CACHE_TTL.store(secs, Ordering::Relaxed);
}

pub fn get_cache_ttl() -> u64 {
    CACHE_TTL.load(Ordering::Relaxed)
}

pub fn init_cache_from_config(config: &crate::config::Config) {
    set_cache_ttl(config.registry.cache_ttl_secs);
}

/// Sync API for detectors running under `spawn_blocking` (wraps async).
pub fn check_package(registry: &str, package: &str) -> Result<Option<bool>> {
    block_on(check_package_async(registry, package))
}

pub async fn check_package_async(registry: &str, package: &str) -> Result<Option<bool>> {
    let cache_key = format!("{registry}:{package}");
    let ttl_secs = get_cache_ttl();
    {
        let cache = CACHE.read().unwrap_or_else(|e| {
            tracing::warn!("registry cache RwLock poisoned; recovering");
            e.into_inner()
        });
        if let Some((result, time)) = cache.get(&cache_key) {
            let age = time.elapsed();
            match result {
                PkgCacheVal::Exists(exists) => {
                    if age < Duration::from_secs(ttl_secs) || offline_mode() {
                        crate::metrics::record_registry_cache_hit();
                        return Ok(Some(*exists));
                    }
                }
                PkgCacheVal::SoftFail => {
                    if age < Duration::from_secs(SOFT_FAIL_TTL_SECS) {
                        crate::metrics::record_registry_cache_hit();
                        return Ok(None);
                    }
                }
            }
        }
    }

    crate::metrics::record_registry_cache_miss();

    if offline_mode() {
        return Ok(None);
    }

    let result = match registry {
        "npm" => npm::check_async(package).await,
        "pypi" => pypi::check_async(package).await,
        "crates.io" => crates_io::check_async(package).await,
        "go" => go::check_async(package).await,
        _ => return Ok(None),
    };

    {
        let mut cache = CACHE.write().unwrap_or_else(|e| {
            tracing::warn!("registry cache RwLock poisoned; recovering");
            e.into_inner()
        });
        match &result {
            Ok(Some(exists)) => {
                cache.insert(cache_key, (PkgCacheVal::Exists(*exists), Instant::now()));
            }
            Ok(None) | Err(_) => {
                cache.insert(cache_key, (PkgCacheVal::SoftFail, Instant::now()));
            }
        }
        if cache.len() > CACHE_MAX_SIZE {
            evict_cache(&mut cache);
        }
    }
    result
}

pub fn check_vulnerabilities(registry: &str, package: &str) -> Result<Vec<OsvVulnerability>> {
    block_on(check_vulnerabilities_async(registry, package))
}

pub fn get_metadata(registry: &str, package: &str) -> Result<Option<Metadata>> {
    block_on(get_metadata_async(registry, package))
}

pub async fn get_metadata_async(registry: &str, package: &str) -> Result<Option<Metadata>> {
    if offline_mode() {
        return Ok(None);
    }
    let cache_key = format!("{registry}:{package}");
    let ttl_secs = get_cache_ttl();
    {
        let cache = METADATA_CACHE.read().unwrap_or_else(|e| e.into_inner());
        if let Some((meta, time)) = cache.get(&cache_key) {
            if time.elapsed() < Duration::from_secs(ttl_secs) {
                return Ok(meta.clone());
            }
        }
    }
    let meta = match registry {
        "npm" => npm::metadata_async(package).await,
        "pypi" => pypi::metadata_async(package).await,
        "crates.io" => crates_io::metadata_async(package).await,
        "go" => go::metadata_async(package).await,
        _ => Ok(None),
    };
    if let Ok(ref m) = meta {
        let mut cache = METADATA_CACHE.write().unwrap_or_else(|e| e.into_inner());
        cache.insert(cache_key, (m.clone(), Instant::now()));
        evict_oldest(&mut cache, ttl_secs);
    }
    meta
}

pub async fn check_vulnerabilities_async(
    registry: &str,
    package: &str,
) -> Result<Vec<OsvVulnerability>> {
    let ecosystem = match registry {
        "npm" => "npm",
        "pypi" => "PyPI",
        "crates.io" => "crates.io",
        "go" => "Go",
        _ => return Ok(vec![]),
    };
    check_osv_async(ecosystem, package).await
}

/// Batch-query OSV for (ecosystem, package, version) triples via `querybatch`.
/// No caching here — callers supply all queries for a review in one call and
/// fail open on error. Version is optional; omit to get all known advisories.
pub async fn query_osv_batch(
    queries: &[(String, String, Option<String>)],
) -> Result<Vec<Vec<OsvVulnerability>>> {
    if queries.is_empty() || offline_mode() {
        return Ok(vec![Vec::new(); queries.len()]);
    }
    query_osv_batch_at(OSV_API_URL, queries).await
}

async fn query_osv_batch_at(
    base_url: &str,
    queries: &[(String, String, Option<String>)],
) -> Result<Vec<Vec<OsvVulnerability>>> {
    let client = async_client()?;
    let body = serde_json::json!({
        "queries": queries.iter().map(|(ecosystem, name, version)| {
            let mut query = serde_json::json!({
                "package": { "name": name, "ecosystem": ecosystem }
            });
            if let Some(version) = version {
                query["version"] = serde_json::Value::String(version.clone());
            }
            query
        }).collect::<Vec<_>>()
    });
    let endpoint = format!("{}/v1/querybatch", base_url.trim_end_matches('/'));
    let data: serde_json::Value = retry_async(
        &RetryConfig::api_default(),
        "osv_querybatch",
        &is_reqwest_error_retryable,
        || async {
            client
                .post(&endpoint)
                .json(&body)
                .send()
                .await?
                .error_for_status()?
                .json::<serde_json::Value>()
                .await
                .map_err(Into::into)
        },
    )
    .await?;

    let ids_by_query = parse_osv_batch(&data, queries.len())?;
    let unique_ids = ids_by_query
        .iter()
        .flatten()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let details = fetch_osv_details(client, base_url, &unique_ids).await;
    Ok(queries
        .iter()
        .zip(ids_by_query)
        .map(|((ecosystem, package, _), ids)| {
            ids.iter()
                .map(|id| {
                    details
                        .get(id)
                        .and_then(|data| extract_osv_vuln_for(data, ecosystem, package))
                        .unwrap_or_else(|| id_only_vulnerability(id))
                })
                .collect()
        })
        .collect())
}

fn parse_osv_batch(data: &serde_json::Value, expected: usize) -> Result<Vec<Vec<String>>> {
    let results = data
        .get("results")
        .and_then(serde_json::Value::as_array)
        .context("OSV querybatch response is missing results")?;
    if results.len() != expected {
        anyhow::bail!(
            "OSV querybatch result count {} did not match {expected} queries",
            results.len()
        );
    }
    Ok(results
        .iter()
        .map(|result| {
            result["vulns"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|vuln| vuln["id"].as_str().map(str::to_string))
                .collect()
        })
        .collect())
}

async fn fetch_osv_details(
    client: &reqwest::Client,
    base_url: &str,
    ids: &HashSet<&str>,
) -> HashMap<String, serde_json::Value> {
    let semaphore = Arc::new(Semaphore::new(OSV_DETAIL_CONCURRENCY));
    let mut tasks = JoinSet::new();
    for id in ids {
        let id = (*id).to_string();
        let url = format!(
            "{}/v1/vulns/{}",
            base_url.trim_end_matches('/'),
            crate::util::url_encode(&id)
        );
        let client = client.clone();
        let semaphore = Arc::clone(&semaphore);
        tasks.spawn(async move {
            let Ok(_permit) = semaphore.acquire().await else {
                return None;
            };
            let result = retry_async(
                &RetryConfig::api_default(),
                "osv_vuln_detail",
                &is_reqwest_error_retryable,
                || async {
                    client
                        .get(&url)
                        .send()
                        .await?
                        .error_for_status()?
                        .json::<serde_json::Value>()
                        .await
                        .map_err(Into::into)
                },
            )
            .await;
            match result {
                Ok(data) => {
                    if data["id"].as_str() == Some(id.as_str()) {
                        Some((id, data))
                    } else {
                        tracing::warn!(%id, "OSV vulnerability detail id did not match");
                        None
                    }
                }
                Err(error) => {
                    tracing::warn!(%id, error = %error, "OSV vulnerability detail lookup failed");
                    None
                }
            }
        });
    }

    let mut details = HashMap::with_capacity(ids.len());
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Some((id, data))) => {
                details.insert(id, data);
            }
            Ok(None) => {}
            Err(error) => tracing::warn!(error = %error, "OSV vulnerability detail task failed"),
        }
    }
    details
}

fn id_only_vulnerability(id: &str) -> OsvVulnerability {
    OsvVulnerability {
        id: id.to_string(),
        summary: String::new(),
        severity: "UNKNOWN".to_string(),
        fixed_version: None,
    }
}

/// Concurrently warm package-existence + OSV caches for a large PR (org-scale).
pub async fn prefetch_packages(pairs: &[(String, String)]) {
    if pairs.is_empty() || offline_mode() {
        return;
    }
    let pairs = unique_prefetch_pairs(pairs, MAX_PREFETCH_PACKAGES);
    let semaphore = Arc::new(Semaphore::new(PREFETCH_CONCURRENCY));
    let mut tasks = JoinSet::new();
    for (registry, package) in pairs {
        let registry = registry.clone();
        let package = package.clone();
        let semaphore = Arc::clone(&semaphore);
        tasks.spawn(async move {
            let ((), ()) = tokio::join!(
                async {
                    let Ok(_permit) = semaphore.acquire().await else {
                        return;
                    };
                    let _ = check_package_async(&registry, &package).await;
                },
                async {
                    let Ok(_permit) = semaphore.acquire().await else {
                        return;
                    };
                    let _ = check_vulnerabilities_async(&registry, &package).await;
                }
            );
        });
    }
    while let Some(result) = tasks.join_next().await {
        if let Err(error) = result {
            tracing::warn!(error = %error, "registry prefetch task failed");
        }
    }
}

fn unique_prefetch_pairs(pairs: &[(String, String)], limit: usize) -> Vec<&(String, String)> {
    let mut seen = HashSet::with_capacity(pairs.len().min(limit));
    pairs
        .iter()
        .filter(|(registry, package)| seen.insert((registry.as_str(), package.as_str())))
        .take(limit)
        .collect()
}

/// Drop expired entries and then the oldest half when a simple cache is full.
fn evict_oldest<V>(cache: &mut HashMap<String, (V, Instant)>, ttl_secs: u64) {
    if cache.len() <= CACHE_MAX_SIZE {
        return;
    }
    let ttl = Duration::from_secs(ttl_secs);
    let now = Instant::now();
    cache.retain(|_, (_, time)| now.duration_since(*time) < ttl);
    if cache.len() <= CACHE_MAX_SIZE {
        return;
    }
    let target = CACHE_MAX_SIZE / 2;
    let mut keys: Vec<(String, Instant)> =
        cache.iter().map(|(k, (_, t))| (k.clone(), *t)).collect();
    keys.sort_unstable_by_key(|(_, t)| *t);
    for (k, _) in keys.into_iter().take(cache.len().saturating_sub(target)) {
        cache.remove(&k);
    }
}

fn evict_cache(cache: &mut HashMap<String, (PkgCacheVal, Instant)>) {
    let ttl = Duration::from_secs(get_cache_ttl());
    let now = Instant::now();
    cache.retain(|_, &mut (val, time)| match val {
        PkgCacheVal::SoftFail => now.duration_since(time) < Duration::from_secs(SOFT_FAIL_TTL_SECS),
        PkgCacheVal::Exists(_) => now.duration_since(time) < ttl,
    });

    let target_len = CACHE_MAX_SIZE / 2;
    if cache.len() <= target_len {
        return;
    }

    let mut entries: Vec<(String, Instant)> = cache
        .iter()
        .map(|(key, &(_, timestamp))| (key.clone(), timestamp))
        .collect();
    entries.sort_unstable_by_key(|(_, timestamp)| *timestamp);
    for (key, _) in entries.into_iter().take(cache.len() - target_len) {
        cache.remove(&key);
    }
}

#[derive(Debug, Clone)]
pub struct OsvVulnerability {
    pub id: String,
    pub summary: String,
    pub severity: String,
    pub fixed_version: Option<String>,
}

async fn check_osv_async(ecosystem: &str, package: &str) -> Result<Vec<OsvVulnerability>> {
    let cache_key = format!("{ecosystem}:{package}");
    let ttl_secs = get_cache_ttl();
    {
        let cache = OSV_CACHE.read().unwrap_or_else(|e| {
            tracing::warn!("OSV cache RwLock poisoned; recovering");
            e.into_inner()
        });
        if let Some((vulns, time)) = cache.get(&cache_key) {
            if time.elapsed() < Duration::from_secs(ttl_secs) {
                crate::metrics::record_osv_cache_hit();
                return Ok(vulns.clone());
            }
            if offline_mode() {
                crate::metrics::record_osv_cache_hit();
                return Ok(vulns.clone());
            }
        }
    }

    crate::metrics::record_osv_cache_miss();

    if offline_mode() {
        return Ok(vec![]);
    }

    let client = async_client()?;
    let body = serde_json::json!({
        "package": {
            "name": package,
            "ecosystem": ecosystem
        }
    });

    let data: serde_json::Value = retry_async(
        &RetryConfig::api_default(),
        "osv_query",
        &is_reqwest_error_retryable,
        || async {
            client
                .post("https://api.osv.dev/v1/query")
                .json(&body)
                .send()
                .await?
                .error_for_status()?
                .json::<serde_json::Value>()
                .await
                .map_err(Into::into)
        },
    )
    .await?;

    let vulns = data["vulns"].as_array().map_or_else(Vec::new, |arr| {
        arr.iter()
            .filter_map(|vuln| extract_osv_vuln_for(vuln, ecosystem, package))
            .collect()
    });

    {
        let mut cache = OSV_CACHE.write().unwrap_or_else(|e| {
            tracing::warn!("OSV cache RwLock poisoned; recovering");
            e.into_inner()
        });
        cache.insert(cache_key, (vulns.clone(), Instant::now()));
        evict_oldest(&mut cache, ttl_secs);
    }

    Ok(vulns)
}

fn ecosystem_severity(affected: &serde_json::Value) -> Option<&str> {
    affected
        .get("ecosystem_specific")
        .and_then(|data| data.get("severity"))
        .and_then(|severity| severity.as_str())
}

fn extract_severity(value: &serde_json::Value, preferred: Option<&serde_json::Value>) -> String {
    value
        .get("database_specific")
        .and_then(|data| data.get("severity"))
        .and_then(|severity| severity.as_str())
        .or_else(|| preferred.and_then(ecosystem_severity))
        .or_else(|| {
            value["affected"]
                .as_array()
                .and_then(|affected| affected.iter().find_map(ecosystem_severity))
        })
        .unwrap_or("UNKNOWN")
        .to_string()
}

fn extract_osv_vuln_for(
    value: &serde_json::Value,
    ecosystem: &str,
    package: &str,
) -> Option<OsvVulnerability> {
    let id = value["id"].as_str()?.to_string();
    let summary = value["summary"].as_str().unwrap_or("").to_string();
    let affected = value["affected"].as_array().and_then(|affected| {
        affected.iter().find(|entry| {
            let candidate = entry["package"]["name"].as_str().unwrap_or_default();
            let same_package = if ecosystem == "PyPI" {
                candidate
                    .replace(['_', '.'], "-")
                    .eq_ignore_ascii_case(&package.replace(['_', '.'], "-"))
            } else {
                candidate == package
            };
            same_package && entry["package"]["ecosystem"].as_str() == Some(ecosystem)
        })
    });
    let severity = extract_severity(value, affected);
    let fixed = affected
        .and_then(|entry| entry.get("ranges"))
        .and_then(|ranges| ranges.as_array())
        .and_then(|ranges| ranges.first())
        .and_then(|range| range.get("events"))
        .and_then(|events| events.as_array())
        .and_then(|events| {
            events
                .iter()
                .find_map(|event| event["fixed"].as_str().map(str::to_string))
        });

    Some(OsvVulnerability {
        id,
        summary,
        severity,
        fixed_version: fixed,
    })
}

#[cfg(test)]
pub(crate) fn seed_osv_cache(ecosystem: &str, package: &str, vulns: Vec<OsvVulnerability>) {
    let mut cache = OSV_CACHE.write().unwrap_or_else(|e| e.into_inner());
    cache.insert(format!("{ecosystem}:{package}"), (vulns, Instant::now()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Path, State};
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct OsvMockState {
        batch_calls: Arc<AtomicUsize>,
        detail_calls: Arc<Mutex<HashMap<String, usize>>>,
    }

    async fn start_osv_mock() -> (String, OsvMockState, tokio::task::JoinHandle<()>) {
        let state = OsvMockState {
            batch_calls: Arc::new(AtomicUsize::new(0)),
            detail_calls: Arc::new(Mutex::new(HashMap::new())),
        };
        let app = Router::new()
            .route(
                "/v1/querybatch",
                post(|State(state): State<OsvMockState>, Json(_): Json<serde_json::Value>| async move {
                    state.batch_calls.fetch_add(1, Ordering::SeqCst);
                    Json(json!({
                        "results": [
                            { "vulns": [{ "id": "GHSA-1" }, { "id": "GHSA-2" }] },
                            { "vulns": [{ "id": "GHSA-1" }] },
                            { "vulns": [{ "id": "GHSA-missing" }] }
                        ]
                    }))
                }),
            )
            .route(
                "/v1/vulns/{id}",
                get(|State(state): State<OsvMockState>, Path(id): Path<String>| async move {
                    *state
                        .detail_calls
                        .lock()
                        .unwrap()
                        .entry(id.clone())
                        .or_default() += 1;
                    if id == "GHSA-missing" {
                        return StatusCode::NOT_FOUND.into_response();
                    }
                    let mut record = json!({
                        "id": id,
                        "summary": format!("summary-{id}")
                    });
                    if id == "GHSA-1" {
                        record["database_specific"] = json!({ "severity": "HIGH" });
                        record["affected"] = json!([
                            {
                                "package": { "name": "other", "ecosystem": "npm" },
                                "ranges": [{ "events": [{ "fixed": "9.0.0" }] }]
                            },
                            {
                                "package": { "name": "one", "ecosystem": "npm" },
                                "ranges": [{ "events": [{ "fixed": "2.0.0" }] }]
                            }
                        ]);
                    } else {
                        record["affected"] = json!([
                            {
                                "package": { "name": "unrelated", "ecosystem": "PyPI" },
                                "ranges": [{ "events": [{ "fixed": "8.0.0" }] }],
                                "ecosystem_specific": { "severity": "HIGH" }
                            },
                            {
                                "package": { "name": "one", "ecosystem": "npm" },
                                "ranges": [{ "events": [{ "fixed": "3.0.0" }] }],
                                "ecosystem_specific": { "severity": "MODERATE" }
                            }
                        ]);
                    }
                    Json(record).into_response()
                }),
            )
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{address}"), state, server)
    }

    #[tokio::test]
    async fn osv_batch_hydrates_unique_ids_and_keeps_fallbacks() {
        let (base_url, state, server) = start_osv_mock().await;
        let queries = vec![
            (
                "npm".to_string(),
                "one".to_string(),
                Some("1.0.0".to_string()),
            ),
            (
                "npm".to_string(),
                "two".to_string(),
                Some("1.0.0".to_string()),
            ),
            (
                "pypi".to_string(),
                "missing".to_string(),
                Some("1.0.0".to_string()),
            ),
        ];

        let results = query_osv_batch_at(&base_url, &queries).await.unwrap();
        server.abort();

        assert_eq!(state.batch_calls.load(Ordering::SeqCst), 1);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].len(), 2);
        assert_eq!(results[0][0].id, "GHSA-1");
        assert_eq!(results[0][0].summary, "summary-GHSA-1");
        assert_eq!(results[0][0].severity, "HIGH");
        assert_eq!(results[0][0].fixed_version.as_deref(), Some("2.0.0"));
        assert_eq!(results[0][1].severity, "MODERATE");
        assert_eq!(results[0][1].fixed_version.as_deref(), Some("3.0.0"));
        assert_eq!(results[1][0].id, "GHSA-1");
        assert_eq!(results[1][0].fixed_version, None);
        assert_eq!(results[2][0].id, "GHSA-missing");
        assert_eq!(results[2][0].severity, "UNKNOWN");
        let calls = state.detail_calls.lock().unwrap();
        assert_eq!(calls.get("GHSA-1"), Some(&1));
        assert_eq!(calls.get("GHSA-2"), Some(&1));
        assert_eq!(calls.get("GHSA-missing"), Some(&1));
    }

    #[test]
    fn malformed_osv_batch_result_count_is_an_error() {
        let data = json!({ "results": [{ "vulns": [] }] });
        let error = parse_osv_batch(&data, 2).unwrap_err();

        assert!(error.to_string().contains("result count"), "{error}");
    }

    #[test]
    fn prefetch_pairs_are_deduplicated_in_first_seen_order() {
        let pairs = vec![
            ("npm".to_string(), "one".to_string()),
            ("npm".to_string(), "one".to_string()),
            ("pypi".to_string(), "two".to_string()),
            ("npm".to_string(), "three".to_string()),
        ];

        let unique = unique_prefetch_pairs(&pairs, 3);

        assert_eq!(
            unique
                .iter()
                .map(|(registry, package)| (registry.as_str(), package.as_str()))
                .collect::<Vec<_>>(),
            vec![("npm", "one"), ("pypi", "two"), ("npm", "three")]
        );
    }

    #[test]
    fn status_classification_preserves_registry_semantics() {
        assert_eq!(
            classify_status(200, &[200, 301, 302], &[404, 410]),
            Some(true)
        );
        assert_eq!(
            classify_status(302, &[200, 301, 302], &[404, 410]),
            Some(true)
        );
        assert_eq!(
            classify_status(404, &[200, 301, 302], &[404, 410]),
            Some(false)
        );
        assert_eq!(classify_status(410, &[200], &[404]), None);
        assert_eq!(classify_status(503, &[200], &[404]), None);
    }

    #[test]
    fn registry_urls_encode_package_path_components() {
        assert_eq!(
            registry_url("https://registry.npmjs.org", "@scope/package"),
            "https://registry.npmjs.org/%40scope%2Fpackage"
        );
        assert_eq!(
            registry_url("https://pypi.org/pypi", "package?x=1"),
            "https://pypi.org/pypi/package%3Fx%3D1"
        );
    }

    #[test]
    fn registry_metadata_extractors_handle_documented_shapes() {
        let npm = serde_json::json!({ "license": { "type": "MIT" } });
        let pypi = serde_json::json!({ "info": { "license": "Apache-2.0" } });
        let crates = serde_json::json!({ "crate": { "license": "BSD-3-Clause" } });

        assert_eq!(npm::extract_license(&npm).as_deref(), Some("MIT"));
        assert_eq!(pypi::extract_license(&pypi).as_deref(), Some("Apache-2.0"));
        assert_eq!(
            crates_io::extract_license(&crates).as_deref(),
            Some("BSD-3-Clause")
        );
        assert_eq!(crates_io::extract_license(&serde_json::json!({})), None);
    }

    #[test]
    fn osv_severity_uses_database_and_ecosystem_fields() {
        let database = serde_json::json!({ "database_specific": { "severity": "HIGH" } });
        let ecosystem = serde_json::json!({
            "affected": [{ "ecosystem_specific": { "severity": "MODERATE" } }]
        });

        assert_eq!(extract_severity(&database, None), "HIGH");
        assert_eq!(extract_severity(&ecosystem, None), "MODERATE");
        assert_eq!(extract_severity(&serde_json::json!({}), None), "UNKNOWN");
    }

    #[test]
    fn test_cache_ttl_default() {
        assert_eq!(get_cache_ttl(), 3600);
    }

    #[test]
    fn test_cache_ttl_set_and_get() {
        set_cache_ttl(600);
        assert_eq!(get_cache_ttl(), 600);
        set_cache_ttl(3600);
    }

    #[test]
    fn test_cache_eviction_policy() {
        set_cache_ttl(3600);
        let mut cache = CACHE.write().unwrap();
        for i in 0..CACHE_MAX_SIZE + 100 {
            let key = format!("test:pkg-{i}");
            cache.insert(key, (PkgCacheVal::Exists(true), Instant::now()));
        }
        evict_cache(&mut cache);
        drop(cache);
        let cache = CACHE.read().unwrap();
        assert!(cache.len() <= CACHE_MAX_SIZE);
        assert!(cache.len() >= CACHE_MAX_SIZE / 2);
    }

    #[test]
    fn test_extract_package_name() {
        assert_eq!(
            crate::detectors::extract_package_name("react", "javascript").as_deref(),
            Some("react")
        );
        assert_eq!(
            crate::detectors::extract_package_name("@scope/package", "javascript").as_deref(),
            Some("@scope/package")
        );
        assert_eq!(
            crate::detectors::extract_package_name("lodash/fp", "javascript").as_deref(),
            Some("lodash")
        );
        assert_eq!(
            crate::detectors::extract_package_name("requests.sessions", "python").as_deref(),
            Some("requests")
        );
        assert_eq!(
            crate::detectors::extract_package_name("github.com/gorilla/mux", "go").as_deref(),
            Some("github.com/gorilla/mux")
        );
    }

    #[tokio::test]
    async fn offline_returns_none_without_cache() {
        set_offline_mode(true);
        let r = check_package_async("npm", "codasaurus-offline-miss-xyz")
            .await
            .unwrap();
        set_offline_mode(false);
        assert!(r.is_none());
    }
}

// end of module
