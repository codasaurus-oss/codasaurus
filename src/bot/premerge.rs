//! Natural-language pre-merge checks (Phase 5): LLM-judged, fail-closed offline.

use crate::config::PreMergeCheck;
use crate::db::DbPool;

pub struct PreMergeCheckResult {
    pub name: String,
    pub mode: String,
    pub status: String,
    pub reasoning: String,
}

/// Run configured checks against the diff. Offline or missing LLM config →
/// every check is `inconclusive` (never blocks). Scope-globbed checks only run
/// when a changed path matches.
pub async fn run_pre_merge_checks(
    pool: Option<&DbPool>,
    repo: &str,
    pr_number: i64,
    diff: &str,
    changed_paths: &[String],
    checks: &[PreMergeCheck],
    offline: bool,
) -> Vec<PreMergeCheckResult> {
    let applicable: Vec<&PreMergeCheck> = checks
        .iter()
        .filter(|check| {
            check.mode != "off"
                && check
                    .scope
                    .as_deref()
                    .is_none_or(|scope| changed_paths.iter().any(|path| glob_match(scope, path)))
        })
        .collect();
    if applicable.is_empty() {
        return Vec::new();
    }
    let llm_cfg = if offline {
        None
    } else {
        crate::llm::LlmConfig::from_db_or_env(pool).await
    };
    let mut results = Vec::with_capacity(applicable.len());
    for check in applicable {
        let (status, reasoning) = if offline {
            (
                "inconclusive".to_string(),
                "LLM disabled (offline mode); inconclusive checks never block".to_string(),
            )
        } else if let Some(llm_cfg) = &llm_cfg {
            match crate::llm::premerge_check(&check.name, &check.instructions, diff, llm_cfg).await
            {
                Ok((status, reasoning)) => (status, reasoning),
                Err(error) => ("inconclusive".to_string(), format!("LLM error: {error}")),
            }
        } else {
            (
                "inconclusive".to_string(),
                "No LLM configured; pre-merge checks need a BYOK model".to_string(),
            )
        };

        results.push(PreMergeCheckResult {
            name: check.name.clone(),
            mode: check.mode.clone(),
            status,
            reasoning,
        });
    }
    persist(pool, repo, pr_number, &results).await;
    results
}

/// Best-effort batch record of the run for the dashboard.
async fn persist(
    pool: Option<&DbPool>,
    repo: &str,
    pr_number: i64,
    results: &[PreMergeCheckResult],
) {
    let (Some(pool), false) = (pool, results.is_empty()) else {
        return;
    };
    let mut latest = std::collections::BTreeMap::new();
    for result in results {
        latest.insert(result.name.as_str(), result);
    }
    let names: Vec<&str> = latest.keys().copied().collect();
    let modes: Vec<&str> = latest.values().map(|result| result.mode.as_str()).collect();
    let statuses: Vec<&str> = latest
        .values()
        .map(|result| result.status.as_str())
        .collect();
    let reasoning: Vec<&str> = latest
        .values()
        .map(|result| result.reasoning.as_str())
        .collect();
    let _ = sqlx::query(
        "INSERT INTO pre_merge_check_runs (repo_full_name, pr_number, check_name, mode, status, reasoning, evaluated_at)
         SELECT $1, $2, name, mode, status, reasoning, NOW()
         FROM UNNEST($3::text[], $4::text[], $5::text[], $6::text[]) AS t(name, mode, status, reasoning)
         ON CONFLICT (repo_full_name, pr_number, check_name)
         DO UPDATE SET status = EXCLUDED.status, reasoning = EXCLUDED.reasoning, evaluated_at = NOW()",
    )
    .bind(repo)
    .bind(pr_number)
    .bind(names)
    .bind(modes)
    .bind(statuses)
    .bind(reasoning)
    .execute(pool.as_pg())
    .await;
}

pub fn premerge_markdown(results: &[PreMergeCheckResult]) -> String {
    if results.is_empty() {
        return String::new();
    }
    let mut md = String::from("### Pre-merge checks\n\n| Check | Mode | Result |\n|---|---|---|\n");
    for r in results {
        let mark = match r.status.as_str() {
            "passed" => "✅ passed",
            "failed" => "❌ failed",
            _ => "➖ inconclusive",
        };
        md.push_str(&format!("| {} | {} | {} |\n", r.name, r.mode, mark));
        if !r.reasoning.is_empty() {
            md.push_str(&format!(
                "| | | {}\n",
                r.reasoning.chars().take(120).collect::<String>()
            ));
        }
    }
    md.push('\n');
    md
}

/// Minimal glob: `*` matches within a path segment, `**` matches any depth.
fn glob_match(pattern: &str, path: &str) -> bool {
    let pat = pattern.trim_start_matches('/').trim_end_matches('/');
    let p = path.trim_start_matches('/');
    if pat.contains("**") {
        let parts: Vec<&str> = pat.split("**").collect();
        let head = parts.first().unwrap_or(&"").trim_end_matches('/');
        let tail = parts.last().unwrap_or(&"").trim_start_matches('/');
        (head.is_empty() || p.starts_with(head)) && (tail.is_empty() || p.ends_with(tail))
    } else if pat.contains('*') {
        let re_pat = format!("^{}$", regex::escape(pat).replace("\\*", "[^/]*"));
        regex::Regex::new(&re_pat).is_ok_and(|re| re.is_match(p))
    } else {
        p == pat
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_matches_paths() {
        assert!(glob_match("tests/**", "tests/auth.rs"));
        assert!(glob_match("tests/**", "tests/deep/unit.rs"));
        assert!(!glob_match("tests/**", "src/auth.rs"));
        assert!(glob_match("*.rs", "main.rs"));
        assert!(!glob_match("*.rs", "src/main.rs"));
        assert!(glob_match("src/main.rs", "src/main.rs"));
    }

    #[test]
    fn scope_filters_checks() {
        let checks = vec![PreMergeCheck {
            name: "no secrets".into(),
            mode: "error".into(),
            scope: Some("tests/**".into()),
            instructions: "fail on secrets".into(),
        }];
        // No matching path → no results at all.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let results = rt.block_on(run_pre_merge_checks(
            None,
            "acme/repo",
            1,
            "",
            &["src/app.rs".to_string()],
            &checks,
            true,
        ));
        assert!(results.is_empty());
    }

    #[test]
    fn offline_is_inconclusive() {
        let checks = vec![PreMergeCheck {
            name: "no secrets".into(),
            mode: "error".into(),
            scope: None,
            instructions: "fail on secrets".into(),
        }];
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let results = rt.block_on(run_pre_merge_checks(
            None,
            "acme/repo",
            1,
            "diff",
            &["tests/app.rs".to_string()],
            &checks,
            true,
        ));
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, "inconclusive");
    }

    #[tokio::test]
    async fn batch_persist_keeps_the_latest_result_per_check() {
        let _guard = crate::db::TEST_POOL_LOCK.lock().await;
        let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://codasaurus:codasaurus@127.0.0.1:5432/codasaurus".into()
        });
        let pool = crate::db::create_pool(&url).await.expect("pool");
        let repo = format!("acme/premerge-{}", uuid::Uuid::new_v4().simple());
        let checks = vec![
            PreMergeCheck {
                name: "same".into(),
                mode: "error".into(),
                scope: None,
                instructions: "first".into(),
            },
            PreMergeCheck {
                name: "other".into(),
                mode: "warning".into(),
                scope: None,
                instructions: "other".into(),
            },
            PreMergeCheck {
                name: "same".into(),
                mode: "warning".into(),
                scope: None,
                instructions: "last".into(),
            },
        ];

        let results = run_pre_merge_checks(Some(&pool), &repo, 1, "diff", &[], &checks, true).await;
        assert_eq!(results.len(), 3);
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT check_name, mode FROM pre_merge_check_runs
             WHERE repo_full_name = $1 AND pr_number = 1
             ORDER BY check_name",
        )
        .bind(&repo)
        .fetch_all(pool.as_pg())
        .await
        .expect("premerge rows");
        assert_eq!(
            rows,
            vec![
                ("other".into(), "warning".into()),
                ("same".into(), "warning".into())
            ]
        );
        sqlx::query("DELETE FROM pre_merge_check_runs WHERE repo_full_name = $1")
            .bind(&repo)
            .execute(pool.as_pg())
            .await
            .expect("cleanup");
    }
}
