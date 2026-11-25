use std::path::{Path, PathBuf};

use crate::context::rules::{parse_guidelines_md, ExtractedRule};

/// A discovered contribution guideline file.
#[derive(Debug, Clone)]
pub struct GuidelineFile {
    pub path: PathBuf,
    /// Label describing the source ("CONTRIBUTING.md", "AGENTS.md", "config", etc.)
    pub source: String,
    pub content: String,
    pub rules: Vec<ExtractedRule>,
}

/// Static list of file/directory names to auto-discover, in discovery order.
const AUTO_DISCOVER_NAMES: &[(&str, bool)] = &[
    ("CONTRIBUTING.md", false),
    ("CONTRIBUTING", true),
    ("AGENTS.md", false),
    ("CLAUDE.md", false),
    (".claude/settings.json", false),
];

/// Discover contribution guideline files for the repository at `root`.
/// Env and config overrides short-circuit auto-discovery, which returns known
/// files in deterministic discovery order.
pub fn find_guidelines(root: &Path, config_path: Option<&str>) -> Vec<GuidelineFile> {
    let env_override = std::env::var("CONTRIBUTING_GUIDELINES").ok();
    find_guidelines_inner(root, config_path, env_override.as_deref())
}

fn find_guidelines_inner(
    root: &Path,
    config_path: Option<&str>,
    env_override: Option<&str>,
) -> Vec<GuidelineFile> {
    if let Some(env_path) = env_override {
        let found = resolve_guideline_path(&PathBuf::from(env_path), "env-var");
        if !found.is_empty() {
            return found;
        }
    }

    if let Some(config) = config_path {
        let path = if Path::new(config).is_absolute() {
            PathBuf::from(config)
        } else {
            root.join(config)
        };
        let found = resolve_guideline_path(&path, "config");
        if !found.is_empty() {
            return found;
        }
    }

    let mut results = Vec::new();
    for (name, is_dir) in AUTO_DISCOVER_NAMES {
        let candidate = root.join(name);
        if *is_dir {
            results.extend(load_markdown_directory(&candidate, name));
        } else if candidate.is_file() {
            if let Some(file) = load_guideline_file(&candidate, name) {
                results.push(file);
            }
        }
    }
    results
}

fn resolve_guideline_path(path: &Path, source: &str) -> Vec<GuidelineFile> {
    if path.is_file() {
        load_guideline_file(path, source).into_iter().collect()
    } else if path.is_dir() {
        load_markdown_directory(path, source)
    } else {
        Vec::new()
    }
}

fn load_markdown_directory(path: &Path, source: &str) -> Vec<GuidelineFile> {
    let Ok(entries) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| {
            entry
                .file_type()
                .map(|kind| kind.is_file())
                .unwrap_or(false)
        })
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|file| load_guideline_file(&file, source))
        .collect()
}

fn load_guideline_file(path: &Path, source: &str) -> Option<GuidelineFile> {
    let content = std::fs::read_to_string(path).ok()?;
    GuidelineFile::from_content(path, source, content)
}

impl GuidelineFile {
    /// Build a guideline file from in-memory content (GitHub Contents API / tests).
    pub fn from_content(path: impl Into<PathBuf>, source: &str, content: String) -> Option<Self> {
        if content.trim().is_empty() {
            return None;
        }
        let rules = parse_guidelines_md(&content);
        Some(Self {
            path: path.into(),
            source: source.to_string(),
            content,
            rules,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discover_nothing_in_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let files = find_guidelines_inner(dir.path(), None, None);
        assert!(files.is_empty());
    }

    #[test]
    fn test_discover_contributing_md() {
        let dir = tempfile::tempdir().unwrap();
        let content = "# Contributing\n\n- [ ] Add tests\n- [ ] Sign commits\n";
        std::fs::write(dir.path().join("CONTRIBUTING.md"), content).unwrap();

        let files = find_guidelines_inner(dir.path(), None, None);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].source, "CONTRIBUTING.md");
        assert!(!files[0].rules.is_empty());
    }

    #[test]
    fn test_discover_agents_and_claude() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "# AGENTS\n").unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "# CLAUDE\n").unwrap();

        let files = find_guidelines_inner(dir.path(), None, None);
        assert_eq!(files.len(), 2);
        let sources: Vec<&str> = files.iter().map(|f| f.source.as_str()).collect();
        assert!(sources.contains(&"AGENTS.md"));
        assert!(sources.contains(&"CLAUDE.md"));
    }

    #[test]
    fn test_env_var_override() {
        let dir = tempfile::tempdir().unwrap();
        let custom = tempfile::tempdir().unwrap();
        let custom_file = custom.path().join("MYRULES.md");
        std::fs::write(&custom_file, "# Custom Rules\n- [ ] Do the thing\n").unwrap();
        std::fs::write(dir.path().join("CONTRIBUTING.md"), "# Ignored\n").unwrap();

        // Use find_guidelines_inner to avoid mutating process env vars (unsafe in parallel tests)
        let files = find_guidelines_inner(dir.path(), None, custom_file.to_str());

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].source, "env-var");
        assert!(files[0].path.to_string_lossy().contains("MYRULES.md"));
    }

    #[test]
    fn test_config_path_override() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CONTRIBUTING.md"), "# Ignored\n").unwrap();

        let custom_dir = dir.path().join("docs");
        std::fs::create_dir_all(&custom_dir).unwrap();
        std::fs::write(custom_dir.join("CONTRIBUTE.md"), "# Custom\n- [ ] Test\n").unwrap();

        let files = find_guidelines_inner(dir.path(), Some("docs/CONTRIBUTE.md"), None);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].source, "config");
        assert!(files[0].path.to_string_lossy().contains("CONTRIBUTE.md"));
    }

    #[test]
    fn test_skip_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CONTRIBUTING.md"), "").unwrap();
        let files = find_guidelines_inner(dir.path(), None, None);
        assert!(files.is_empty());
    }

    #[test]
    fn test_contributing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let contrib_dir = dir.path().join("CONTRIBUTING");
        std::fs::create_dir_all(&contrib_dir).unwrap();
        std::fs::write(contrib_dir.join("CODE_OF_CONDUCT.md"), "# Code\n").unwrap();
        std::fs::write(
            contrib_dir.join("STYLE_GUIDE.md"),
            "# Style\n- [ ] Use tabs\n",
        )
        .unwrap();

        let files = find_guidelines_inner(dir.path(), None, None);
        assert_eq!(files.len(), 2);
        assert!(files
            .iter()
            .any(|file| file.path.ends_with("CODE_OF_CONDUCT.md")));
        assert!(files
            .iter()
            .any(|file| file.path.ends_with("STYLE_GUIDE.md")));
    }
}
