//! Whole-repo symbol graph index: tree-sitter extraction + Postgres store.

pub mod embed;
pub mod extract;
pub mod semantic;
pub mod store;

use crate::config::IndexConfig;

/// List every file path in the repo at `git_ref` via the Git Trees API
/// (`GET /repos/{repo}/git/trees/{sha}?recursive=1`), capped by max_files.
pub async fn list_repo_files(
    client: &reqwest::Client,
    headers: &reqwest::header::HeaderMap,
    repo_full_name: &str,
    git_ref: &str,
    max_files: usize,
) -> Result<Vec<String>, anyhow::Error> {
    let url =
        format!("https://api.github.com/repos/{repo_full_name}/git/trees/{git_ref}?recursive=1");
    let resp = crate::retry::retry_async(
        &crate::retry::RetryConfig::api_default(),
        "index_tree",
        &crate::retry::is_reqwest_error_retryable,
        || async {
            client
                .get(&url)
                .headers(headers.clone())
                .send()
                .await
                .map_err(Into::into)
        },
    )
    .await?;
    if !resp.status().is_success() {
        anyhow::bail!("git trees API returned {}", resp.status());
    }
    let json: serde_json::Value = resp.json().await?;
    let mut paths: Vec<String> = json["tree"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|t| t["type"].as_str() == Some("blob"))
                .filter_map(|t| t["path"].as_str().map(|p| p.to_string()))
                .filter(|p| extract::language_name(p).is_some())
                .collect()
        })
        .unwrap_or_default();
    paths.truncate(max_files);
    Ok(paths)
}

/// Whole-repo callers of symbols defined in `paths`, as compact markdown.
/// Empty when the repo has no index rows (callers fall back to diff-only impact).
pub async fn callers_markdown(
    pool: &crate::db::DbPool,
    repo_full_name: &str,
    paths: &[String],
) -> String {
    let Ok(rows) = store::symbols_in_files(pool, repo_full_name, paths).await else {
        return String::new();
    };
    let symbols: Vec<(String, String)> = rows
        .into_iter()
        .filter(|(_, _, kind, _)| kind != extract::SYMBOL_IMPORT)
        .map(|(path, symbol, _, _)| (symbol, path))
        .collect();
    if symbols.is_empty() {
        return String::new();
    }

    let mut seen = std::collections::HashSet::new();
    let names: Vec<String> = symbols
        .iter()
        .filter(|(name, _)| seen.insert(name.as_str()))
        .map(|(name, _)| name.clone())
        .collect();
    let Ok(edges) = store::callers_of_many(pool, repo_full_name, &names).await else {
        return String::new();
    };
    let mut callers_by_symbol: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for (from, to, kind) in edges {
        if kind == extract::EDGE_CALLS || kind == extract::EDGE_EXTENDS {
            callers_by_symbol.entry(to).or_default().push(from);
        }
    }

    let mut sections = Vec::new();
    for (symbol, path) in symbols.iter().take(30) {
        let Some(callers) = callers_by_symbol.get_mut(symbol) else {
            continue;
        };
        callers.sort();
        callers.dedup();
        if callers.is_empty() {
            continue;
        }
        let listed = callers
            .iter()
            .take(8)
            .map(|caller| format!("`{caller}`"))
            .collect::<Vec<_>>()
            .join(", ");
        sections.push(format!("- `{symbol}` ({path}) <- {listed}"));
    }
    if sections.is_empty() {
        return String::new();
    }
    format!(
        "**Indexed callers of changed symbols:**\n\n{}\n",
        sections.join("\n")
    )
}

/// Incremental: fetch and re-index changed files in bounded parallel batches.
#[allow(clippy::too_many_arguments)]
pub async fn reindex_files(
    pool: &crate::db::DbPool,
    client: &reqwest::Client,
    headers: &reqwest::header::HeaderMap,
    repo_full_name: &str,
    git_ref: &str,
    paths: &[String],
    config: &IndexConfig,
    llm_cfg: Option<&crate::llm::LlmConfig>,
) -> Result<(), anyhow::Error> {
    let eligible: Vec<String> = paths
        .iter()
        .filter(|path| {
            extract::language_name(path)
                .is_some_and(|lang| config.languages.iter().any(|l| l == lang))
        })
        .cloned()
        .collect();
    if eligible.is_empty() {
        return Ok(());
    }
    let fetched = crate::bot::github_files::fetch_repo_files_parallel(
        client,
        headers,
        repo_full_name,
        &eligible,
        git_ref,
        5,
    )
    .await;
    if fetched.len() < eligible.len() {
        tracing::warn!(
            repo = repo_full_name,
            missing = eligible.len() - fetched.len(),
            "incremental index skipped unavailable files"
        );
    }
    let indices: Vec<extract::FileIndex> = fetched
        .into_iter()
        .filter_map(|(path, content)| extract::extract_file(&path, &content))
        .collect();
    if indices.is_empty() {
        return Ok(());
    }

    if let Err(error) = store::replace_files_index(pool, repo_full_name, &indices).await {
        tracing::warn!(error = %error, repo = repo_full_name, "batch index update failed; retrying files");
        for index in &indices {
            if let Err(error) =
                store::replace_files_index(pool, repo_full_name, std::slice::from_ref(index)).await
            {
                tracing::warn!(error = %error, repo = repo_full_name, path = index.file_path, "incremental index failed");
            }
        }
    }
    if let Some(llm_cfg) = llm_cfg {
        if let Err(error) =
            semantic::reindex_repo_embeddings(pool, llm_cfg, repo_full_name, &indices).await
        {
            tracing::warn!(error = %error, repo = repo_full_name, "batch embedding update failed; retrying files");
            for index in &indices {
                if let Err(error) = semantic::reindex_repo_embeddings(
                    pool,
                    llm_cfg,
                    repo_full_name,
                    std::slice::from_ref(index),
                )
                .await
                {
                    tracing::warn!(error = %error, repo = repo_full_name, path = index.file_path, "semantic index embedding failed");
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_gating_respects_config() {
        let config = IndexConfig {
            enabled: true,
            languages: vec!["python".into()],
            max_files: 1000,
            ..Default::default()
        };
        assert_eq!(extract::language_name("a.py"), Some("python"));
        assert_eq!(extract::language_name("a.rs"), Some("rust"));
        assert!(config.languages.iter().any(|l| l == "python"));
        assert!(!config.languages.iter().any(|l| l == "rust"));
    }
}
