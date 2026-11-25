//! `PostgreSQL` persistence for the whole-repo symbol index.

use crate::db::DbPool;
use crate::index::extract::FileIndex;
use sqlx::Row;

const INDEX_READY: &str = "ready";

/// Incremental update for changed files, replacing only their rows in one transaction.
///
/// # Errors
/// Returns an error when a database operation fails.
pub async fn replace_files_index(
    pool: &DbPool,
    repo_full_name: &str,
    files: &[FileIndex],
) -> Result<(), sqlx::Error> {
    if files.is_empty() {
        return Ok(());
    }
    let paths: Vec<&str> = files.iter().map(|file| file.file_path.as_str()).collect();
    let mut tx = pool.as_pg().begin().await?;
    // Edges first: the subquery reads the very rows the symbol DELETE removes,
    // so deleting symbols first would leave every stale edge behind forever.
    sqlx::query(
        "WITH changed_symbols AS MATERIALIZED (
            SELECT symbol_name FROM repo_symbols
            WHERE repo_full_name = $1 AND file_path = ANY($2)
         )
         DELETE FROM repo_edges
         WHERE repo_full_name = $1
           AND (from_symbol = ANY($2)
                OR from_symbol IN (SELECT symbol_name FROM changed_symbols)
                OR to_symbol IN (SELECT symbol_name FROM changed_symbols))",
    )
    .bind(repo_full_name)
    .bind(&paths)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM repo_symbols WHERE repo_full_name = $1 AND file_path = ANY($2)")
        .bind(repo_full_name)
        .bind(&paths)
        .execute(&mut *tx)
        .await?;
    finish_index_write(tx, repo_full_name, files).await
}

type Tx<'a> = sqlx::Transaction<'a, sqlx::Postgres>;

async fn finish_index_write(
    mut tx: Tx<'_>,
    repo: &str,
    files: &[FileIndex],
) -> Result<(), sqlx::Error> {
    insert_files(&mut tx, repo, files).await?;
    upsert_status(&mut tx, repo, INDEX_READY).await?;
    tx.commit().await
}

/// Bulk-insert via `UNNEST` arrays: one round-trip for the whole index instead
/// of one per symbol/edge (a full repo build is tens of thousands of rows).
async fn insert_files(tx: &mut Tx<'_>, repo: &str, files: &[FileIndex]) -> Result<(), sqlx::Error> {
    let sym_total: usize = files.iter().map(|f| f.symbols.len()).sum();
    let mut paths: Vec<&str> = Vec::with_capacity(sym_total);
    let mut names: Vec<&str> = Vec::with_capacity(sym_total);
    let mut kinds: Vec<&str> = Vec::with_capacity(sym_total);
    let mut signatures: Vec<Option<&str>> = Vec::with_capacity(sym_total);
    let mut lines = Vec::with_capacity(sym_total);

    let edge_total: usize = files.iter().map(|f| f.edges.len()).sum();
    let mut from_symbols: Vec<&str> = Vec::with_capacity(edge_total);
    let mut to_symbols: Vec<&str> = Vec::with_capacity(edge_total);
    let mut edge_kinds: Vec<&str> = Vec::with_capacity(edge_total);

    for file in files {
        for sym in &file.symbols {
            paths.push(file.file_path.as_str());
            names.push(sym.name.as_str());
            kinds.push(sym.kind.as_str());
            signatures.push(sym.signature.as_deref());
            lines.push(sym.line);
        }
        for edge in &file.edges {
            from_symbols.push(edge.from_symbol.as_str());
            to_symbols.push(edge.to_symbol.as_str());
            edge_kinds.push(edge.edge_kind.as_str());
        }
    }

    if !names.is_empty() {
        sqlx::query(
            "INSERT INTO repo_symbols (repo_full_name, file_path, symbol_name, kind, signature, line)
             SELECT $1, * FROM UNNEST($2::text[], $3::text[], $4::text[], $5::text[], $6::bigint[])
             ON CONFLICT (repo_full_name, file_path, symbol_name, line) DO NOTHING",
        )
        .bind(repo)
        .bind(&paths)
        .bind(&names)
        .bind(&kinds)
        .bind(&signatures)
        .bind(&lines)
        .execute(&mut **tx)
        .await?;
    }

    if !from_symbols.is_empty() {
        sqlx::query(
            "INSERT INTO repo_edges (repo_full_name, from_symbol, to_symbol, edge_kind)
             SELECT $1, * FROM UNNEST($2::text[], $3::text[], $4::text[])
             ON CONFLICT (repo_full_name, from_symbol, to_symbol, edge_kind) DO NOTHING",
        )
        .bind(repo)
        .bind(&from_symbols)
        .bind(&to_symbols)
        .bind(&edge_kinds)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn upsert_status(tx: &mut Tx<'_>, repo: &str, status: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO index_status (repo_full_name, status, built_at, error)
         VALUES ($1, $2, NOW(), NULL)
         ON CONFLICT (repo_full_name)
         DO UPDATE SET status = EXCLUDED.status,
                       built_at = EXCLUDED.built_at,
                       error = NULL",
    )
    .bind(repo)
    .bind(status)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn index_status(pool: &DbPool, repo: &str) -> Option<(String, String)> {
    sqlx::query("SELECT status, COALESCE(error, '') FROM index_status WHERE repo_full_name = $1")
        .bind(repo)
        .fetch_optional(pool.as_pg())
        .await
        .ok()
        .flatten()
        .map(|row| (row.get(0), row.get(1)))
}

/// Callers of many symbols, capped per symbol.
///
/// # Errors
/// Returns an error when the caller query fails.
pub async fn callers_of_many(
    pool: &DbPool,
    repo: &str,
    symbols: &[String],
) -> Result<Vec<(String, String, String)>, sqlx::Error> {
    if symbols.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as(
        "SELECT from_symbol, to_symbol, edge_kind FROM (
           SELECT from_symbol, to_symbol, edge_kind,
                  ROW_NUMBER() OVER (PARTITION BY to_symbol ORDER BY from_symbol) AS rn
           FROM repo_edges
           WHERE repo_full_name = $1 AND to_symbol = ANY($2)
         ) ranked
         WHERE rn <= 50
         ORDER BY to_symbol, from_symbol",
    )
    .bind(repo)
    .bind(symbols)
    .fetch_all(pool.as_pg())
    .await
}

/// Symbols defined in many files, ordered by path and line.
///
/// # Errors
/// Returns an error when the symbol query fails.
pub async fn symbols_in_files(
    pool: &DbPool,
    repo: &str,
    file_paths: &[String],
) -> Result<Vec<(String, String, String, i64)>, sqlx::Error> {
    if file_paths.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as(
        "SELECT file_path, symbol_name, kind, COALESCE(line, 0)::bigint FROM repo_symbols
         WHERE repo_full_name = $1 AND file_path = ANY($2)
         ORDER BY file_path, line",
    )
    .bind(repo)
    .bind(file_paths)
    .fetch_all(pool.as_pg())
    .await
}
