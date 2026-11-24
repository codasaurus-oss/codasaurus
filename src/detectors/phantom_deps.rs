use crate::detectors::Finding;
use crate::parser::ParsedFile;
use std::collections::{HashMap, HashSet};

type DepMap = HashMap<&'static str, HashMap<String, HashSet<String>>>;

pub fn detect(parsed_files: &[ParsedFile]) -> Vec<Finding> {
    let mut findings = Vec::new();

    let dep_map = build_dep_map(parsed_files);

    for file in parsed_files {
        if crate::detectors::is_test_or_fixture_path(&file.path) {
            continue;
        }
        // Skip dependency manifest files themselves
        let fname = file.path.rsplit('/').next().unwrap_or("").to_lowercase();
        if matches!(
            fname.as_str(),
            "package.json"
                | "requirements.txt"
                | "pyproject.toml"
                | "cargo.toml"
                | "setup.py"
                | "setup.cfg"
                | "go.mod"
                | "gemfile"
                | "gemfile.lock"
        ) {
            continue;
        }

        let Some(registry) = crate::detectors::registry_for_language(&file.language) else {
            continue;
        };

        let Some(declared) = dep_map
            .get(registry)
            .and_then(|dirs| nearest_manifest_deps(dirs, &file.path))
        else {
            continue;
        };

        for (import, package) in crate::detectors::external_imports(file) {
            // Skip built-in and stdlib imports.
            if crate::detectors::hallucinated_imports::is_builtin(&package, registry) {
                continue;
            }

            if !declared.contains(&package) {
                let codemod = match registry {
                    "npm" => Some(format!("npm install {package}")),
                    "pypi" => Some(format!("pip install {package}")),
                    "crates.io" => Some(format!("cargo add {package}")),
                    "go" => Some(format!("go get {package}")),
                    _ => None,
                };
                findings.push(Finding {
                    detector: "phantom-deps".to_string(),
                    severity: "blocking",
                    file: file.path.clone(),
                    line: import.line,
                    column: import.column,
                    message: format!(
                        "Package `{package}` is used but not declared in your dependency file."
                    ),
                    suggestion: Some(format!(
                        "Add `{package}` to your package manager's dependency file."
                    )),
                    evidence: Some(import.name.clone()),
                    codemod,
                    confidence: None,
                    judge_rationale: None,
                    reachability: None,
                });
            }
        }
    }

    findings
}

fn build_dep_map(files: &[ParsedFile]) -> DepMap {
    let mut map = DepMap::new();

    for file in files {
        let path = file.path.to_lowercase();

        if path.ends_with("package.json") {
            add_deps(
                &mut map,
                "npm",
                &file.path,
                crate::dep_parser::extract_npm_deps(&file.raw_content),
            );
        } else if path.ends_with("requirements.txt")
            || path.ends_with("setup.py")
            || path.ends_with("setup.cfg")
        {
            add_deps(
                &mut map,
                "pypi",
                &file.path,
                crate::dep_parser::extract_requirements_deps(&file.raw_content),
            );
        } else if path.ends_with("pyproject.toml") {
            add_deps(
                &mut map,
                "pypi",
                &file.path,
                crate::dep_parser::extract_pyproject_deps(&file.raw_content),
            );
        } else if path.ends_with("cargo.toml") {
            add_deps(
                &mut map,
                "crates.io",
                &file.path,
                crate::dep_parser::extract_cargo_deps(&file.raw_content),
            );
            if let Some(name) = crate::dep_parser::extract_cargo_package_name(&file.raw_content) {
                map.entry("crates.io")
                    .or_default()
                    .entry(directory_of(&file.path).to_string())
                    .or_default()
                    .insert(name);
            }
        } else if path.ends_with("go.mod") {
            add_deps(
                &mut map,
                "go",
                &file.path,
                crate::dep_parser::extract_go_mod_deps(&file.raw_content),
            );
        }
    }

    map
}

fn add_deps(map: &mut DepMap, registry: &'static str, path: &str, deps: Vec<String>) {
    map.entry(registry)
        .or_default()
        .entry(directory_of(path).to_string())
        .or_default()
        .extend(deps);
}

fn nearest_manifest_deps<'a>(
    dirs: &'a HashMap<String, HashSet<String>>,
    path: &str,
) -> Option<&'a HashSet<String>> {
    let mut dir = directory_of(path);
    loop {
        if let Some(deps) = dirs.get(dir) {
            return Some(deps);
        }
        if dir.is_empty() {
            return None;
        }
        dir = directory_of(dir);
    }
}

fn directory_of(path: &str) -> &str {
    path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_file;

    fn parse(path: &str, content: &str) -> ParsedFile {
        parse_file(path, content).unwrap()
    }

    #[test]
    fn python_stdlib_and_dotted_imports_are_not_phantom() {
        let files = [
            parse("requirements.txt", "requests==2.31.0\n"),
            parse(
                "main.py",
                "import os\nfrom requests.sessions import Session\n",
            ),
        ];
        assert!(detect(&files).is_empty());
    }

    #[test]
    fn go_stdlib_and_full_module_paths_are_not_phantom() {
        let files = [
            parse(
                "go.mod",
                "module example.com/app\n\nrequire github.com/gorilla/mux v1.8.0\n",
            ),
            parse(
                "main.go",
                "package main\nimport \"fmt\"\nimport \"net/http\"\nimport \"github.com/gorilla/mux\"\n",
            ),
        ];
        assert!(detect(&files).is_empty());
    }

    #[test]
    fn dependencies_are_scoped_to_the_nearest_manifest() {
        let files = [
            parse(
                "packages/a/package.json",
                r#"{"dependencies": {"lodash": "^4.0.0"}}"#,
            ),
            parse(
                "packages/b/package.json",
                r#"{"dependencies": {"express": "^4.0.0"}}"#,
            ),
            parse("packages/b/src/index.js", "import lodash from 'lodash';\n"),
        ];
        let findings = detect(&files);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].file, "packages/b/src/index.js");
        assert!(findings[0].message.contains("lodash"));
    }

    #[test]
    fn parent_manifest_supplies_workspace_dependencies() {
        let files = [
            parse("package.json", r#"{"dependencies": {"lodash": "^4.0.0"}}"#),
            parse(
                "packages/app/src/index.js",
                "import lodash from 'lodash';\n",
            ),
        ];
        assert!(detect(&files).is_empty());
    }
}
