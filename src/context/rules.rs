use regex::Regex;
use std::sync::LazyLock;

static CHECKBOX_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*[-*]\s+\[([ xX])\]\s+(.+)$").expect("valid checkbox regex"));
static HEADING_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^#{2,4}\s+(.+)$").expect("valid heading regex"));
static BRANCH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)branch\s*(naming|pattern|name|format)?\s*[:]\s*(.+?)(?:[,.\n]|$)")
        .expect("valid branch regex")
});
static BRANCH_INLINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(branches?\s+(?:must|should)\s+(?:start|begin)\s+with\s+`([^`]+)`)")
        .expect("valid inline branch regex")
});
static DCO_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(signed.off.by|DCO|developer.certificate.of.origin)").expect("valid DCO regex")
});
static CONVENTIONAL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(conventional\s*commits?|commit\s*(message)?\s*(convention|format|style))")
        .expect("valid conventional commit regex")
});
static FILE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:required|must\s+have|must\s+include)\s+`([^`]+)`")
        .expect("valid required-file regex")
});

/// A structured rule extracted from a contribution guideline file.
#[derive(Debug, Clone)]
pub enum ExtractedRule {
    /// A markdown checklist item from `- [ ]` / `- [x]`
    ChecklistItem { text: String, checked: bool },
    /// A required file path mentioned in the guidelines
    FileRequired { path: String },
    /// A branch naming pattern mentioned
    BranchPattern { pattern: String },
    /// A commit message convention (conventional commits, sign-off, etc.)
    CommitRule { description: String },
    /// A generic parsed section with heading and content
    SectionRule { heading: String, text: String },
}

/// Parse Markdown contribution guidelines into structured rules.
/// Covers checklists, required files, branch/commit conventions, and sections.
pub fn parse_guidelines_md(content: &str) -> Vec<ExtractedRule> {
    let mut rules: Vec<ExtractedRule> = Vec::new();
    let mut in_requirements = false;
    let mut in_commit_section = false;

    for line in content.lines() {
        if let Some(caps) = HEADING_RE.captures(line) {
            let heading = caps[1].trim().to_string();

            let hl = heading.to_lowercase();
            in_requirements = hl.contains("requirement")
                || hl.contains("prerequisite")
                || hl.contains("checklist")
                || hl.contains("definition of done");
            in_commit_section = hl.contains("commit");

            rules.push(ExtractedRule::SectionRule {
                heading,
                text: String::new(),
            });
            continue;
        }

        if let Some(ExtractedRule::SectionRule { text, .. }) = rules.last_mut() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(line);
        }

        if let Some(caps) = CHECKBOX_RE.captures(line) {
            let checked = matches!(&caps[1], "x" | "X");
            rules.push(ExtractedRule::ChecklistItem {
                text: caps[2].trim().to_string(),
                checked,
            });
            continue;
        }

        if let Some(caps) = FILE_RE.captures(line) {
            rules.push(ExtractedRule::FileRequired {
                path: caps[1].to_string(),
            });
            continue;
        }

        if CONVENTIONAL_RE.is_match(line) || DCO_RE.is_match(line) {
            rules.push(ExtractedRule::CommitRule {
                description: line.trim().to_string(),
            });
            continue;
        }

        if let Some(caps) = BRANCH_RE.captures(line) {
            let pattern = caps[2].trim().to_string();
            if !pattern.is_empty() {
                rules.push(ExtractedRule::BranchPattern { pattern });
                continue;
            }
        }
        if let Some(caps) = BRANCH_INLINE_RE.captures(line) {
            let pattern = caps[2].to_string();
            if !pattern.is_empty() {
                rules.push(ExtractedRule::BranchPattern { pattern });
            }
            continue;
        }

        if in_requirements {
            if let Some(item) = bullet_item(line) {
                rules.push(ExtractedRule::ChecklistItem {
                    text: item.to_string(),
                    checked: false,
                });
            }
        }

        if in_commit_section {
            if let Some(item) = bullet_item(line) {
                rules.push(ExtractedRule::CommitRule {
                    description: item.to_string(),
                });
            }
        }
    }

    dedup_rules(&mut rules);
    rules
}

/// Bullet text from a `- ` / `* ` line, excluding empty items and checkboxes.
fn bullet_item(line: &str) -> Option<&str> {
    let line = line.trim();
    let item = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))?
        .trim();
    (!item.is_empty() && !item.starts_with('[')).then_some(item)
}

fn dedup_rules(rules: &mut Vec<ExtractedRule>) {
    let mut seen = std::collections::HashSet::new();
    rules.retain(|rule| {
        let key = match rule {
            ExtractedRule::ChecklistItem { text, .. } => ("checklist", text.clone()),
            ExtractedRule::FileRequired { path } => ("file", path.clone()),
            ExtractedRule::BranchPattern { pattern } => ("branch", pattern.clone()),
            ExtractedRule::CommitRule { description } => ("commit", description.clone()),
            ExtractedRule::SectionRule { heading, .. } => ("section", heading.clone()),
        };
        seen.insert(key)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_checklist_items() {
        let md = "- [ ] Add tests\n- [x] Sign commits\n- [ ] Update docs\n";
        let rules = parse_guidelines_md(md);
        let items: Vec<&ExtractedRule> = rules
            .iter()
            .filter(|r| matches!(r, ExtractedRule::ChecklistItem { .. }))
            .collect();
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn test_parse_commit_convention() {
        let commit_rules = rule_kind(
            "## Commit Convention\nUse conventional commits: `type(scope): description`\n",
            |r| matches!(r, ExtractedRule::CommitRule { .. }),
        );
        assert!(!commit_rules.is_empty());
    }

    #[test]
    fn test_parse_dco() {
        let md = "All commits must be signed-off-by per DCO guidelines.";
        let rules = parse_guidelines_md(md);
        assert!(rules.iter().any(|r| matches!(r, ExtractedRule::CommitRule { description } if description.to_lowercase().contains("dco"))));
    }

    fn rule_kind(md: &str, pred: impl Fn(&ExtractedRule) -> bool) -> Vec<ExtractedRule> {
        parse_guidelines_md(md).into_iter().filter(pred).collect()
    }

    #[test]
    fn test_parse_branch_pattern() {
        let branches = rule_kind("Branch naming: `feat/`, `fix/`, or `chore/`", |r| {
            matches!(r, ExtractedRule::BranchPattern { .. })
        });
        assert!(!branches.is_empty());
    }

    #[test]
    fn test_parse_required_files() {
        let files = rule_kind(
            "Your PR must include `CHANGELOG.md` and `README.md`.",
            |r| matches!(r, ExtractedRule::FileRequired { .. }),
        );
        assert!(!files.is_empty());
    }

    #[test]
    fn test_parse_sections() {
        let sections = rule_kind(
            "## Testing\nRun tests with `cargo test`.\n## Docs\nUpdate docs.",
            |r| matches!(r, ExtractedRule::SectionRule { .. }),
        );
        assert_eq!(sections.len(), 2);
    }

    #[test]
    fn test_dedup_rules() {
        let checklists = rule_kind(
            "- [ ] Add tests\n- [ ] Add tests\n## Commit Convention\nconventional commits\nconventional commits\n",
            |r| matches!(r, ExtractedRule::ChecklistItem { text, .. } if text == "Add tests"),
        );
        assert_eq!(checklists.len(), 1);
    }

    #[test]
    fn test_requirements_section_lists() {
        let items = rule_kind(
            "## Requirements\n- Node.js 18+\n- Rust 1.70+\n- Docker",
            |r| matches!(r, ExtractedRule::ChecklistItem { .. }),
        );
        assert!(!items.is_empty());
    }

    #[test]
    fn test_branch_inline_rule() {
        let branches = rule_kind("Branches must start with `feat/` or `fix/`.", |r| {
            matches!(r, ExtractedRule::BranchPattern { .. })
        });
        assert!(!branches.is_empty());
    }

    #[test]
    fn test_empty_content() {
        let rules = parse_guidelines_md("");
        assert!(rules.is_empty());
    }

    #[test]
    fn test_noise_only() {
        let md = "Just some random text without any recognizable patterns.";
        let rules = parse_guidelines_md(md);
        assert!(rules.is_empty());
    }
}
