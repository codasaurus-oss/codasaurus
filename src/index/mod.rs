//! Whole-repo symbol graph index: tree-sitter extraction + Postgres store.

pub mod embed;
pub mod extract;
pub mod semantic;
pub mod store;

use crate::config::IndexConfig;

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

fn eligible_paths(paths: &[String], config: &IndexConfig) -> Vec<String> {
    paths
        .iter()
        .filter(|path| {
            extract::language_name(path)
                .is_some_and(|language| config.languages.iter().any(|allowed| allowed == language))
        })
        .take(config.max_files)
        .cloned()
        .collect()
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
) {
    let eligible = eligible_paths(paths, config);
    if eligible.is_empty() {
        return;
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
        return;
    }

    if let Err(error) = store::replace_files_index(pool, repo_full_name, &indices).await {
        tracing::warn!(error = %error, repo = repo_full_name, "batch index update failed");
        return;
    }
    if let Some(llm_cfg) = llm_cfg {
        if let Err(error) =
            semantic::reindex_repo_embeddings(pool, llm_cfg, repo_full_name, &indices).await
        {
            tracing::warn!(error = %error, repo = repo_full_name, "semantic index embedding failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eligible_paths_respect_language_and_file_caps() {
        let config = IndexConfig {
            enabled: true,
            languages: vec!["python".into()],
            max_files: 1,
            ..Default::default()
        };
        let paths = ["a.py".into(), "b.py".into(), "c.rs".into()];
        assert_eq!(eligible_paths(&paths, &config), ["a.py"]);
        let disabled = IndexConfig {
            max_files: 0,
            ..config
        };
        assert!(eligible_paths(&paths, &disabled).is_empty());
    }
}
