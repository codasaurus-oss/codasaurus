//! Version-aware dependency CVE/advisory detector via OSV.dev `querybatch`.
//!
//! Unlike `vulnerabilities` (which checks *imported* packages without a
//! resolved version), this detector reads the actual manifest version so
//! findings can be scored blocking/warning by real severity instead of
//! always falling back to "info".

use crate::detectors::Finding;
use crate::parser::ParsedFile;
use crate::registry::OsvVulnerability;

use crate::dep_parser::{
    extract_cargo_versioned, extract_go_mod_versioned, extract_npm_versioned,
    extract_requirements_versioned,
};

type ManifestDep = crate::dep_parser::VersionedDep;

pub async fn detect(parsed_files: &[ParsedFile]) -> Vec<Finding> {
    let mut findings = Vec::new();

    for file in parsed_files {
        let path = file.path.to_lowercase();
        let (ecosystem, deps) = if path.ends_with("package.json") {
            ("npm", extract_npm_versioned(&file.raw_content))
        } else if path.ends_with("cargo.toml") {
            ("crates.io", extract_cargo_versioned(&file.raw_content))
        } else if path.ends_with("go.mod") {
            ("Go", extract_go_mod_versioned(&file.raw_content))
        } else if path.ends_with("requirements.txt") {
            ("PyPI", extract_requirements_versioned(&file.raw_content))
        } else {
            continue;
        };
        if deps.is_empty() {
            continue;
        }

        let queries: Vec<(String, String, Option<String>)> = deps
            .iter()
            .map(|d| {
                (
                    ecosystem.to_string(),
                    d.name.clone(),
                    Some(d.version.clone()),
                )
            })
            .collect();

        let results = match crate::registry::query_osv_batch(&queries).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(file = %file.path, error = %e, "OSV querybatch failed, skipping (fail-open)");
                continue;
            }
        };

        for (dep, vulns) in deps.iter().zip(results.iter()) {
            for vuln in vulns {
                findings.push(finding_for(&file.path, dep, vuln));
            }
        }
    }

    findings
}

fn finding_for(file_path: &str, dep: &ManifestDep, vuln: &OsvVulnerability) -> Finding {
    let severity = match vuln.severity.to_ascii_uppercase().as_str() {
        "CRITICAL" | "HIGH" => "blocking",
        _ => "warning",
    };
    let summary = if vuln.summary.is_empty() {
        String::new()
    } else {
        format!(" — {}", vuln.summary)
    };
    Finding {
        detector: "dependency-vulns".to_string(),
        severity,
        file: file_path.to_string(),
        line: 0,
        column: 0,
        message: format!(
            "{} affects `{}` {}{}",
            vuln.id, dep.name, dep.version, summary
        ),
        suggestion: vuln
            .fixed_version
            .as_ref()
            .map(|v| format!("Upgrade `{}` to {v} or later.", dep.name)),
        evidence: Some(format!("{}: {}", dep.name, dep.version)),
        codemod: None,
        confidence: None,
        judge_rationale: None,
        reachability: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npm_versioned_extraction() {
        let content = r#"{"dependencies": {"lodash": "^4.17.20"}}"#;
        let deps = extract_npm_versioned(content);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "lodash");
        assert_eq!(deps[0].version, "4.17.20");
    }

    #[test]
    fn cargo_versioned_extraction() {
        let content = "[dependencies]\nserde = \"1.0.100\"\ntokio = { version = \"1.20.0\", features = [\"full\"] }\n";
        let deps = extract_cargo_versioned(content);
        assert!(deps
            .iter()
            .any(|d| d.name == "serde" && d.version == "1.0.100"));
        assert!(deps
            .iter()
            .any(|d| d.name == "tokio" && d.version == "1.20.0"));
    }

    #[test]
    fn go_mod_versioned_extraction() {
        let content = "module example\n\nrequire (\n\tgithub.com/gorilla/mux v1.8.0\n)\n";
        let deps = extract_go_mod_versioned(content);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "github.com/gorilla/mux");
        assert_eq!(deps[0].version, "v1.8.0");
    }

    #[test]
    fn requirements_versioned_extraction() {
        let content = "requests==2.31.0\nflask>=2.0.0\n# comment\n";
        let deps = extract_requirements_versioned(content);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "requests");
        assert_eq!(deps[0].version, "2.31.0");
    }
}
