//! Shared dependency file parsing logic.

const DEP_CHARS: &[char] = &['=', '<', '>', '~', '!', ';', ' ', '\t'];

/// Extract npm dependencies from a `package.json` manifest.
pub fn extract_npm_deps(content: &str) -> Vec<String> {
    let mut pkgs = Vec::new();
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(content) {
        if let Some(deps) = json.get("dependencies").and_then(|d| d.as_object()) {
            pkgs.reserve(deps.len());
            pkgs.extend(deps.keys().cloned());
        }
        if let Some(dev_deps) = json.get("devDependencies").and_then(|d| d.as_object()) {
            pkgs.reserve(dev_deps.len());
            pkgs.extend(dev_deps.keys().cloned());
        }
    }
    pkgs
}

/// Extract dependencies from a `requirements.txt`, `setup.py`, or `setup.cfg`.
pub fn extract_requirements_deps(content: &str) -> Vec<String> {
    let mut pkgs = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('-') {
            continue;
        }
        let pkg = line
            .split(DEP_CHARS)
            .next()
            .unwrap_or("")
            .trim()
            .to_lowercase();
        if !pkg.is_empty() {
            pkgs.push(pkg);
        }
    }
    pkgs
}

/// Extract dependencies from a `pyproject.toml`.
/// Handles both `[project] dependencies` and `[tool.poetry.dependencies]`.
pub fn extract_pyproject_deps(content: &str) -> Vec<String> {
    let mut pkgs = Vec::new();
    if let Ok(toml_table) = content.parse::<toml::Table>() {
        if let Some(project) = toml_table.get("project") {
            if let Some(deps) = project.get("dependencies").and_then(|d| d.as_array()) {
                for dep in deps {
                    if let Some(s) = dep.as_str() {
                        let pkg = s
                            .split(DEP_CHARS)
                            .next()
                            .unwrap_or("")
                            .trim()
                            .to_lowercase();
                        if !pkg.is_empty() {
                            pkgs.push(pkg);
                        }
                    }
                }
            }
        }
        if let Some(tool) = toml_table.get("tool") {
            if let Some(poetry) = tool.get("poetry") {
                if let Some(deps) = poetry.get("dependencies").and_then(|d| d.as_table()) {
                    pkgs.reserve(deps.len());
                    pkgs.extend(deps.keys().cloned());
                }
            }
        }
    }
    pkgs
}

/// Workspace crate name from `[package].name` in `Cargo.toml`.
pub fn extract_cargo_package_name(content: &str) -> Option<String> {
    let table = content.parse::<toml::Table>().ok()?;
    table
        .get("package")?
        .as_table()?
        .get("name")?
        .as_str()
        .map(str::to_string)
}

/// Extract dependencies from a `Cargo.toml`.
/// Covers `[dependencies]`, `[dev-dependencies]`, and `[build-dependencies]`.
pub fn extract_cargo_deps(content: &str) -> Vec<String> {
    let mut pkgs = Vec::new();
    if let Ok(toml_table) = content.parse::<toml::Table>() {
        for key in &["dependencies", "dev-dependencies", "build-dependencies"] {
            if let Some(deps) = toml_table.get(*key).and_then(|d| d.as_table()) {
                pkgs.reserve(deps.len());
                pkgs.extend(deps.keys().cloned());
            }
        }
    }
    pkgs
}

/// Extract dependencies from a `go.mod`.
pub fn extract_go_mod_deps(content: &str) -> Vec<String> {
    let mut pkgs = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if is_go_mod_noise(line) {
            continue;
        }
        // Handle single-line require: "require pkg v1.0" and block require: "pkg v1.0"
        let rest = line.strip_prefix("require ").unwrap_or(line);
        if rest == "(" || rest.trim().is_empty() {
            continue;
        }
        if let Some(pkg) = rest.split_whitespace().next() {
            if pkg.contains('/') {
                pkgs.push(pkg.to_string());
            }
        }
    }
    pkgs
}

/// A manifest dependency paired with the version declared in that manifest.
pub struct VersionedDep {
    pub name: String,
    pub version: String,
}

/// Strip range prefixes so the declared version can be queried as an exact version.
pub fn clean_version(v: &str) -> String {
    v.trim_start_matches(['^', '~', '=', '>', '<', ' '])
        .to_string()
}

/// Lines in a `go.mod` that never declare a direct dependency.
pub fn is_go_mod_noise(line: &str) -> bool {
    line.is_empty()
        || line.starts_with("//")
        || line.starts_with(')')
        || line.starts_with("module ")
        || line.starts_with("go ")
        || line.starts_with("exclude ")
        || line.starts_with("retract ")
        || line.starts_with("replace ")
}

pub fn extract_npm_versioned(content: &str) -> Vec<VersionedDep> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(content) else {
        return Vec::new();
    };
    let mut deps = Vec::new();
    for key in ["dependencies", "devDependencies"] {
        if let Some(obj) = json.get(key).and_then(|d| d.as_object()) {
            for (name, version) in obj {
                if let Some(v) = version.as_str() {
                    deps.push(VersionedDep {
                        name: name.clone(),
                        version: clean_version(v),
                    });
                }
            }
        }
    }
    deps
}

pub fn extract_cargo_versioned(content: &str) -> Vec<VersionedDep> {
    let Ok(table) = content.parse::<toml::Table>() else {
        return Vec::new();
    };
    let mut deps = Vec::new();
    for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(obj) = table.get(key).and_then(|d| d.as_table()) {
            for (name, value) in obj {
                let version = value.as_str().map(str::to_string).or_else(|| {
                    value
                        .as_table()
                        .and_then(|t| t.get("version"))
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                });
                if let Some(v) = version {
                    deps.push(VersionedDep {
                        name: name.clone(),
                        version: clean_version(&v),
                    });
                }
            }
        }
    }
    deps
}

pub fn extract_go_mod_versioned(content: &str) -> Vec<VersionedDep> {
    let mut deps = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if is_go_mod_noise(line) {
            continue;
        }
        let rest = line.strip_prefix("require ").unwrap_or(line);
        if rest == "(" || rest.trim().is_empty() {
            continue;
        }
        let mut parts = rest.split_whitespace();
        if let (Some(name), Some(version)) = (parts.next(), parts.next()) {
            if name.contains('/') && version.starts_with('v') {
                deps.push(VersionedDep {
                    name: name.to_string(),
                    version: version.to_string(),
                });
            }
        }
    }
    deps
}

pub fn extract_requirements_versioned(content: &str) -> Vec<VersionedDep> {
    let mut deps = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('-') {
            continue;
        }
        if let Some((name, version)) = line.split_once("==") {
            let name = name.trim().to_lowercase();
            let version = version
                .split(|c: char| c == ';' || c.is_whitespace())
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            if !name.is_empty() && !version.is_empty() {
                deps.push(VersionedDep { name, version });
            }
        }
    }
    deps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_npm_deps() {
        let content = r#"{"dependencies": {"react": "^18.0.0", "lodash": "^4.0.0"}}"#;
        let deps = extract_npm_deps(content);
        assert_eq!(deps, vec!["lodash", "react"]);
    }

    #[test]
    fn test_extract_cargo_package_name() {
        let content = "[package]\nname = \"codasaurus\"\nversion = \"0.1.0\"\n";
        assert_eq!(
            extract_cargo_package_name(content).as_deref(),
            Some("codasaurus")
        );
    }

    #[test]
    fn test_extract_cargo_deps() {
        let content = r#"[dependencies]
serde = "1.0"
tokio = { version = "1", features = ["full"] }

[dev-dependencies]
tempfile = "3"
"#;
        let deps = extract_cargo_deps(content);
        assert!(deps.contains(&"serde".to_string()));
        assert!(deps.contains(&"tokio".to_string()));
        assert!(deps.contains(&"tempfile".to_string()));
    }

    #[test]
    fn test_extract_requirements_deps() {
        let content = "requests==2.31.0\nflask>=2.0.0\n# comment\n-e .\n";
        let deps = extract_requirements_deps(content);
        assert!(deps.contains(&"requests".to_string()));
        assert!(deps.contains(&"flask".to_string()));
    }

    #[test]
    fn test_extract_pyproject_deps() {
        let content = r#"[project]
dependencies = ["requests>=2.0", "click"]

[tool.poetry.dependencies]
python = "^3.9"
"#;
        let deps = extract_pyproject_deps(content);
        assert!(deps.contains(&"requests".to_string()));
        assert!(deps.contains(&"click".to_string()));
    }

    #[test]
    fn test_extract_go_mod_deps() {
        let content = r#"module github.com/example/project

go 1.21

require (
	github.com/gorilla/mux v1.8.0
	github.com/sirupsen/logrus v1.9.0
)
"#;
        let deps = extract_go_mod_deps(content);
        assert!(deps.contains(&"github.com/gorilla/mux".to_string()));
        assert!(deps.contains(&"github.com/sirupsen/logrus".to_string()));
        assert!(!deps.contains(&"github.com/example/project".to_string()));
    }

    #[test]
    fn parses_a_dependency_added_after_a_comment() {
        let manifest = r#"
[dependencies]
serde_json = "1"

# OpenAPI
utoipa = "6"
utoipa-swagger-ui = { version = "10", features = ["axum"] }
"#;
        let deps = extract_cargo_deps(manifest);
        assert!(deps.iter().any(|d| d == "utoipa"), "deps: {deps:?}");
        assert!(
            deps.iter().any(|d| d == "utoipa-swagger-ui"),
            "deps: {deps:?}"
        );
    }
}
