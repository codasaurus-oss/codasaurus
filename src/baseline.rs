use crate::db::DbPool;
use crate::detectors::Finding;
use anyhow::Result;
use std::collections::HashSet;

/// Parse a GitHub diff `patch` and return the set of new-file line numbers that
/// were added or modified (i.e., "new code" lines).
///
/// GitHub hunks look like `@@ -a,b +c,d @@` where lines starting with `+` are
/// new lines, `-` are removed, and ` ` are context. Removed lines do not have
/// a line number in the new file.
pub fn parse_patch_changed_lines(patch: &str) -> HashSet<usize> {
    let mut lines = HashSet::new();
    let mut new_line: usize = 0;
    for line in patch.lines() {
        if let Some(header) = line.strip_prefix("@@") {
            if let Some(rest) = header.split_once(" +") {
                if let Some(rhs) = rest.1.split(" @@").next() {
                    let start = parse_hunk_start(rhs);
                    new_line = start;
                }
            }
        } else if line.starts_with('+') {
            lines.insert(new_line);
            new_line += 1;
        } else if line.starts_with('-') {
            // Removed lines have no line number in the new file.
        } else if line.starts_with(' ') {
            new_line += 1;
        } else if line == "\\ No newline at end of file" {
            // no-op
        }
    }
    lines
}

fn parse_hunk_start(s: &str) -> usize {
    s.split(',').next().unwrap_or(s).trim().parse().unwrap_or(1)
}

/// Persist all changed-line rows for a PR in one statement.
pub async fn save_pr_diff_lines_batch(
    pool: &DbPool,
    repo_full_name: &str,
    pr_number: i64,
    files: &[(&str, &HashSet<usize>)],
) -> Result<()> {
    let total = files.iter().map(|(_, lines)| lines.len()).sum();
    if total == 0 {
        return Ok(());
    }
    let mut paths = Vec::with_capacity(total);
    let mut changed = Vec::with_capacity(total);
    for (path, lines) in files {
        for line in *lines {
            paths.push(*path);
            changed.push(*line as i64);
        }
    }
    sqlx::query(
        "INSERT INTO pr_diff_lines (repo_full_name, pr_number, file_path, line)
         SELECT $1, $2, file_path, line
         FROM UNNEST($3::text[], $4::bigint[]) AS t(file_path, line)
         ON CONFLICT (repo_full_name, pr_number, file_path, line) DO NOTHING",
    )
    .bind(repo_full_name)
    .bind(pr_number)
    .bind(paths)
    .bind(changed)
    .execute(pool.as_pg())
    .await?;
    Ok(())
}

/// Remove any previously stored diff lines for this PR so that re-reviews are
/// idempotent.
pub async fn clear_pr_diff_lines(
    pool: &DbPool,
    repo_full_name: &str,
    pr_number: i64,
) -> Result<()> {
    sqlx::query("DELETE FROM pr_diff_lines WHERE repo_full_name = $1 AND pr_number = $2")
        .bind(repo_full_name)
        .bind(pr_number)
        .execute(pool.as_pg())
        .await?;
    Ok(())
}

/// Filter findings to only those on new code lines.
///
/// Findings that land on pre-existing (non-new) lines are recorded into the
/// baseline and then suppressed. This stops detectors like `boilerplate` from
/// re-flagging the same issue every time a file is touched.
///
/// `repo_full_name` is the `owner/repo` slug; `pr_number` is the PR id.
pub async fn filter_new_code_findings(
    pool: &DbPool,
    repo_full_name: &str,
    pr_number: i64,
    findings: &[Finding],
) -> Result<Vec<Finding>> {
    use std::collections::HashMap;

    let (new_rows, baseline_rows) = tokio::join!(
        sqlx::query_as::<_, (String, i32)>(
            "SELECT file_path, line FROM pr_diff_lines WHERE repo_full_name = $1 AND pr_number = $2",
        )
        .bind(repo_full_name)
        .bind(pr_number)
        .fetch_all(pool.as_pg()),
        sqlx::query_scalar("SELECT fingerprint FROM finding_baseline WHERE repo_full_name = $1")
            .bind(repo_full_name)
            .fetch_all(pool.as_pg()),
    );
    let new_by_file: HashMap<String, HashSet<i32>> =
        new_rows?
            .into_iter()
            .fold(HashMap::new(), |mut acc, (file, line)| {
                acc.entry(file).or_default().insert(line);
                acc
            });
    let mut baseline: HashSet<String> = baseline_rows?.into_iter().collect();

    let mut out = Vec::new();
    let mut new_baselines: Vec<String> = Vec::new();
    for f in findings {
        let is_new = new_by_file
            .get(&f.file)
            .is_some_and(|lines| lines.contains(&(f.line as i32)));
        if is_new {
            out.push(f.clone());
            continue;
        }
        let fp = f.fingerprint();
        if baseline.contains(&fp) {
            continue;
        }
        baseline.insert(fp.clone());
        new_baselines.push(fp);
    }
    record_baselines(pool, repo_full_name, &new_baselines).await?;
    Ok(out)
}

async fn record_baselines(
    pool: &DbPool,
    repo_full_name: &str,
    fingerprints: &[String],
) -> Result<()> {
    if fingerprints.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO finding_baseline (repo_full_name, fingerprint, first_seen_at, last_seen_at)
         SELECT $1, unnest($2::text[]), NOW(), NOW()
         ON CONFLICT (repo_full_name, fingerprint) DO UPDATE SET last_seen_at = NOW()",
    )
    .bind(repo_full_name)
    .bind(fingerprints)
    .execute(pool.as_pg())
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_hunk() {
        let patch = "@@ -1,3 +1,5 @@\n line1\n-line2\n+line2new\n+line3new\n line4\n";
        let lines = parse_patch_changed_lines(patch);
        assert!(lines.contains(&2));
        assert!(lines.contains(&3));
        assert!(!lines.contains(&1));
        assert!(!lines.contains(&4));
    }

    #[test]
    fn parse_no_newline_marker() {
        let patch = "@@ -1 +1,2 @@\n-old\n+new\n\\ No newline at end of file\n";
        let lines = parse_patch_changed_lines(patch);
        assert!(lines.contains(&1));
    }

    #[test]
    fn parse_multiple_hunks() {
        let patch = "@@ -1,2 +1,3 @@\n a\n+b\n c\n@@ -10,2 +11,3 @@\n x\n+y\n z\n";
        let lines = parse_patch_changed_lines(patch);
        assert!(lines.contains(&2));
        assert!(lines.contains(&12));
    }
}
