//! Tree-sitter symbol + edge extraction for the whole-repo index.
//!
//! Parses one source file and yields definitions (symbols) plus
//! CALLS / IMPORTS / EXTENDS / DEFINES edges. Unresolvable names are kept as
//! plain text — the graph is best-effort and query time does the matching.

use tree_sitter::{Language, Node, Parser};

pub const SYMBOL_FUNCTION: &str = "function";
pub const SYMBOL_CLASS: &str = "class";
pub const SYMBOL_METHOD: &str = "method";
pub const SYMBOL_CONST: &str = "const";
pub const SYMBOL_IMPORT: &str = "import";

pub const EDGE_CALLS: &str = "CALLS";
pub const EDGE_IMPORTS: &str = "IMPORTS";
pub const EDGE_EXTENDS: &str = "EXTENDS";
pub const EDGE_DEFINES: &str = "DEFINES";

/// A definition or import found in one source file.
#[derive(Debug, Clone)]
pub struct ExtractedSymbol {
    pub name: String,
    pub kind: String,
    pub signature: Option<String>,
    pub line: i64,
}

/// A best-effort relationship between two symbol names.
#[derive(Debug, Clone)]
pub struct ExtractedEdge {
    pub from_symbol: String,
    pub to_symbol: String,
    pub edge_kind: String,
}

/// Symbols and edges extracted from a single file.
#[derive(Debug, Clone)]
pub struct FileIndex {
    pub file_path: String,
    pub symbols: Vec<ExtractedSymbol>,
    pub edges: Vec<ExtractedEdge>,
}

static RUST: std::sync::LazyLock<Language> =
    std::sync::LazyLock::new(|| Language::new(tree_sitter_rust::LANGUAGE));
static GO: std::sync::LazyLock<Language> =
    std::sync::LazyLock::new(|| Language::new(tree_sitter_go::LANGUAGE));
static PYTHON: std::sync::LazyLock<Language> =
    std::sync::LazyLock::new(|| Language::new(tree_sitter_python::LANGUAGE));
static JAVASCRIPT: std::sync::LazyLock<Language> =
    std::sync::LazyLock::new(|| Language::new(tree_sitter_javascript::LANGUAGE));
static TYPESCRIPT: std::sync::LazyLock<Language> =
    std::sync::LazyLock::new(|| Language::new(tree_sitter_typescript::LANGUAGE_TYPESCRIPT));

fn language_for_path(path: &str) -> Option<(&'static str, &'static Language)> {
    let extension = path.rsplit('.').next()?.to_ascii_lowercase();
    let (name, language) = match extension.as_str() {
        "rs" => ("rust", &RUST),
        "go" => ("go", &GO),
        "py" => ("python", &PYTHON),
        "js" | "jsx" | "mjs" => ("javascript", &JAVASCRIPT),
        "ts" | "tsx" => ("typescript", &TYPESCRIPT),
        _ => return None,
    };
    Some((name, language))
}

/// Return the indexed language name for a source path.
#[must_use]
pub fn language_name(path: &str) -> Option<&'static str> {
    language_for_path(path).map(|(name, _)| name)
}

/// Parse a source file into symbols + edges. `None` when the language is not
/// indexed or the tree fails to build (e.g. binary bytes).
#[must_use]
pub fn extract_file(file_path: &str, content: &str) -> Option<FileIndex> {
    let (language_name, language) = language_for_path(file_path)?;
    let mut parser = Parser::new();
    parser.set_language(language).ok()?;

    let tree = parser.parse(content, None)?;
    let mut collector = Collector {
        file_path,
        language: language_name,
        content,
        symbols: Vec::new(),
        edges: Vec::new(),
        scope: Vec::new(),
    };
    collector.walk(tree.root_node());
    Some(FileIndex {
        file_path: file_path.to_string(),
        symbols: collector.symbols,
        edges: collector.edges,
    })
}

struct Collector<'a> {
    file_path: &'a str,
    language: &'static str,
    content: &'a str,
    symbols: Vec<ExtractedSymbol>,
    edges: Vec<ExtractedEdge>,
    /// Enclosing definition names, innermost last. Methods/consts get
    /// `Parent::name` qualification from the top of the stack.
    scope: Vec<String>,
}

impl Collector<'_> {
    fn text(&self, node: Node) -> Option<String> {
        node.utf8_text(self.content.as_bytes())
            .ok()
            .map(ToString::to_string)
    }

    fn node_name(&self, node: Node) -> Option<String> {
        node.child_by_field_name("name").and_then(|n| self.text(n))
    }

    fn enclosing(&self) -> Option<&str> {
        self.scope.last().map(String::as_str)
    }

    fn line_number(node: Node) -> i64 {
        i64::try_from(node.start_position().row)
            .unwrap_or(i64::MAX)
            .saturating_add(1)
    }

    fn definition_kind(&self) -> &'static str {
        if self.enclosing().is_some() {
            SYMBOL_METHOD
        } else {
            SYMBOL_FUNCTION
        }
    }

    /// Record a symbol (+ DEFINES edge) and return its (possibly qualified) name.
    fn add_symbol(&mut self, name: &str, kind: &str, node: Node) -> String {
        let qualified = match (self.enclosing(), kind) {
            (Some(parent), SYMBOL_METHOD) => format!("{parent}::{name}"),
            _ => name.to_string(),
        };
        let signature = self
            .text(node)
            .map(|t| t.lines().next().unwrap_or_default().trim().to_string())
            .map(|t| t.chars().take(160).collect());
        self.symbols.push(ExtractedSymbol {
            name: qualified.clone(),
            kind: kind.to_string(),
            signature,
            line: Self::line_number(node),
        });
        self.edges.push(ExtractedEdge {
            from_symbol: self.file_path.to_string(),
            to_symbol: qualified.clone(),
            edge_kind: EDGE_DEFINES.to_string(),
        });
        qualified
    }

    /// Record a symbol and push it as the enclosing scope for its children.
    fn add_scoped_symbol(&mut self, name: &str, kind: &str, node: Node) -> bool {
        let qualified = self.add_symbol(name, kind, node);
        self.scope.push(qualified);
        true
    }

    /// Record a class plus `extends` edges read from its grammar-specific base field.
    fn add_class_with_bases(&mut self, name: &str, node: Node, bases_field: &str) -> bool {
        let qualified = self.add_symbol(name, SYMBOL_CLASS, node);
        if let Some(bases) = node.child_by_field_name(bases_field) {
            for child in bases.named_children(&mut bases.walk()) {
                if let Some(base) = self.text(child) {
                    self.edges.push(ExtractedEdge {
                        from_symbol: qualified.clone(),
                        to_symbol: base,
                        edge_kind: EDGE_EXTENDS.to_string(),
                    });
                }
            }
        }
        self.scope.push(qualified);
        true
    }

    fn walk(&mut self, node: Node) {
        let kind = node.kind();
        let pushed = self.record_definition(node, kind);
        self.record_call(node, kind);

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.walk(child);
        }
        if pushed {
            self.scope.pop();
        }
    }

    fn record_definition(&mut self, node: Node, kind: &str) -> bool {
        match kind {
            "function_item" | "function_definition" => {
                if let Some(name) = self.node_name(node) {
                    return self.add_scoped_symbol(&name, self.definition_kind(), node);
                }
            }
            "struct_item" | "enum_item" | "trait_item" | "union_item" => {
                if let Some(name) = self.node_name(node) {
                    return self.add_scoped_symbol(&name, SYMBOL_CLASS, node);
                }
            }
            "impl_item" => {
                if let Some(name) = node.child_by_field_name("type").and_then(|n| self.text(n)) {
                    self.scope.push(name);
                    return true;
                }
            }
            "const_item" | "static_item" | "const_declaration" | "var_declaration" => {
                if let Some(name) = self.node_name(node) {
                    self.add_symbol(&name, SYMBOL_CONST, node);
                }
            }
            "use_declaration" => {
                if let Some(text) = self.text(node) {
                    let clean = text.trim_start_matches("use").trim().trim_end_matches(';');
                    self.push_import(clean.to_string(), node);
                }
            }
            "function_declaration" => {
                if let Some(name) = self.node_name(node) {
                    return self.add_scoped_symbol(&name, SYMBOL_FUNCTION, node);
                }
            }
            "method_declaration" | "method_definition" => {
                if let Some(name) = self.node_name(node) {
                    return self.add_scoped_symbol(&name, SYMBOL_METHOD, node);
                }
            }
            "type_declaration" => {
                if let Some(spec) = node.child_by_field_name("type") {
                    if let Some(name) = self.node_name(spec) {
                        return self.add_scoped_symbol(&name, SYMBOL_CLASS, node);
                    }
                }
            }
            "import_declaration" => self.push_go_imports(node),
            "class_definition" => {
                if let Some(name) = self.node_name(node) {
                    return self.add_class_with_bases(&name, node, "superclasses");
                }
            }
            "import_statement" => {
                if self.language == "python" {
                    self.push_python_imports(node);
                } else {
                    self.push_javascript_import(node);
                }
            }
            "import_from_statement" | "future_import_statement" => self.push_python_imports(node),
            "class_declaration" | "interface_declaration" => {
                if let Some(name) = self.node_name(node) {
                    return self.add_class_with_bases(&name, node, "heritage");
                }
            }
            "variable_declarator" => {
                let is_const = node
                    .parent()
                    .and_then(|parent| parent.child(0))
                    .and_then(|token| self.text(token))
                    .is_some_and(|text| text == "const");
                if is_const {
                    if let Some(name) = self.node_name(node) {
                        self.add_symbol(&name, SYMBOL_CONST, node);
                    }
                }
            }
            _ => {}
        }
        false
    }

    fn record_call(&mut self, node: Node, kind: &str) {
        if !matches!(kind, "call_expression" | "call") {
            return;
        }
        let Some(callee) = node.child_by_field_name("function") else {
            return;
        };
        let callee_name = if callee.kind() == "member_expression" {
            callee
                .child_by_field_name("property")
                .and_then(|property| self.text(property))
        } else if callee.kind() == "field_expression" {
            callee
                .child_by_field_name("field")
                .and_then(|field| self.text(field))
        } else {
            callee
                .child_by_field_name("name")
                .and_then(|name| self.text(name))
                .or_else(|| self.text(callee))
        };
        let (Some(target), Some(caller)) = (callee_name, self.enclosing()) else {
            return;
        };
        self.edges.push(ExtractedEdge {
            from_symbol: caller.to_string(),
            to_symbol: target,
            edge_kind: EDGE_CALLS.to_string(),
        });
    }

    fn push_go_imports(&mut self, node: Node) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() != "import_spec" {
                continue;
            }
            if let Some(path) = child.child_by_field_name("path") {
                if let Some(name) = self
                    .text(path)
                    .map(|text| text.trim_matches('"').to_string())
                    .filter(|text| !text.is_empty())
                {
                    self.push_import(name, child);
                }
            }
        }
    }

    fn push_python_imports(&mut self, node: Node) {
        if node.kind() == "future_import_statement" {
            self.push_import("__future__".to_string(), node);
            return;
        }
        if let Some(module) = node
            .child_by_field_name("module_name")
            .and_then(|module| self.text(module))
        {
            self.push_import(module, node);
            return;
        }

        let mut cursor = node.walk();
        for child in node.children_by_field_name("name", &mut cursor) {
            let import = child.child_by_field_name("name").unwrap_or(child);
            if let Some(name) = self.text(import) {
                self.push_import(name, child);
            }
        }
    }

    fn push_javascript_import(&mut self, node: Node) {
        if let Some(source) = node
            .child_by_field_name("source")
            .and_then(|source| self.text(source))
        {
            let name = source.trim_matches(['\'', '"']).to_string();
            if !name.is_empty() {
                self.push_import(name, node);
            }
        }
    }

    fn push_import(&mut self, name: String, node: Node) {
        self.symbols.push(ExtractedSymbol {
            name: name.clone(),
            kind: SYMBOL_IMPORT.to_string(),
            signature: None,
            line: Self::line_number(node),
        });
        self.edges.push(ExtractedEdge {
            from_symbol: self.file_path.to_string(),
            to_symbol: name,
            edge_kind: EDGE_IMPORTS.to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_extracts_functions_and_calls() {
        let src = r#"
fn greet(name: &str) -> String {
    format!("hi {}", name)
}

fn main() {
    let g = greet("x");
}
"#;
        let idx = extract_file("src/main.rs", src).unwrap();
        let names: Vec<&str> = idx.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"greet"), "{names:?}");
        assert!(names.contains(&"main"), "{names:?}");
        let calls = idx
            .edges
            .iter()
            .filter(|e| e.edge_kind == EDGE_CALLS)
            .collect::<Vec<_>>();
        assert!(
            calls
                .iter()
                .any(|e| e.from_symbol == "main" && e.to_symbol == "greet"),
            "{calls:?}"
        );
    }

    #[test]
    fn python_extracts_class_and_extends() {
        let src = "import os\nclass Animal:\n    pass\nclass Dog(Animal):\n    pass\n";
        let idx = extract_file("app.py", src).unwrap();
        let names: Vec<&str> = idx.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Animal"), "{names:?}");
        assert!(names.contains(&"Dog"), "{names:?}");
        assert!(idx.edges.iter().any(|e| {
            e.edge_kind == EDGE_EXTENDS && e.from_symbol == "Dog" && e.to_symbol == "Animal"
        }));
    }

    #[test]
    fn go_extracts_imports() {
        let src = "package main\nimport \"fmt\"\nfunc main() { fmt.Println(\"hi\") }\n";
        let idx = extract_file("main.go", src).unwrap();
        assert!(idx
            .edges
            .iter()
            .any(|e| e.edge_kind == EDGE_IMPORTS && e.to_symbol == "fmt"));
        assert!(idx.symbols.iter().any(|s| s.name == "main"));
    }

    #[test]
    fn javascript_extracts_methods() {
        let src = "class User {\n  async load() { return fetch('/u'); }\n}\nconst MAX = 5;\n";
        let idx = extract_file("user.js", src).unwrap();
        let names: Vec<&str> = idx.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"User"), "{names:?}");
        assert!(names.contains(&"User::load"), "{names:?}");
        assert!(names.contains(&"MAX"), "{names:?}");
    }

    #[test]
    fn rust_impl_methods_qualified() {
        let src = "struct User {}\nimpl User {\n    fn load() -> u8 { 1 }\n}\n";
        let idx = extract_file("src/user.rs", src).unwrap();
        let names: Vec<&str> = idx.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"User"), "{names:?}");
        assert!(names.contains(&"User::load"), "{names:?}");
    }

    #[test]
    fn python_class_methods_qualified() {
        let src = "class A:\n    def go(self):\n        pass\n";
        let idx = extract_file("a.py", src).unwrap();
        let names: Vec<&str> = idx.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"A"), "{names:?}");
        assert!(names.contains(&"A::go"), "{names:?}");
    }

    #[test]
    fn rust_impl_blocks_do_not_duplicate_type_symbols() {
        let src = "struct User {}\nimpl User { fn one() {} }\nimpl User { fn two() {} }\n";
        let idx = extract_file("user.rs", src).unwrap();
        assert_eq!(
            idx.symbols
                .iter()
                .filter(|symbol| symbol.name == "User")
                .count(),
            1
        );
        assert_eq!(
            idx.edges
                .iter()
                .filter(|edge| edge.edge_kind == EDGE_DEFINES && edge.to_symbol == "User")
                .count(),
            1
        );
    }

    #[test]
    fn python_imports_use_module_names() {
        let src = "import importlib\nimport os, sys\nimport numpy as np\nfrom collections.abc import Mapping\nfrom .helpers import thing\n";
        let idx = extract_file("imports.py", src).unwrap();
        let imports: Vec<&str> = idx
            .symbols
            .iter()
            .filter(|symbol| symbol.kind == SYMBOL_IMPORT)
            .map(|symbol| symbol.name.as_str())
            .collect();
        assert_eq!(
            imports,
            [
                "importlib",
                "os",
                "sys",
                "numpy",
                "collections.abc",
                ".helpers"
            ]
        );
    }

    #[test]
    fn javascript_imports_are_extracted() {
        let src = "import React from 'react';\nimport { value } from \"./value.js\";\n";
        let idx = extract_file("imports.js", src).unwrap();
        let imports: Vec<&str> = idx
            .edges
            .iter()
            .filter(|edge| edge.edge_kind == EDGE_IMPORTS)
            .map(|edge| edge.to_symbol.as_str())
            .collect();
        assert_eq!(imports, ["react", "./value.js"]);
    }

    #[test]
    fn javascript_method_calls_use_field_name() {
        let src = "class A {\n  go() { return this.b(); }\n}\n";
        let idx = extract_file("a.js", src).unwrap();
        let calls: Vec<String> = idx
            .edges
            .iter()
            .filter(|e| e.edge_kind == EDGE_CALLS)
            .map(|e| e.to_symbol.clone())
            .collect();
        assert!(calls.contains(&"b".to_string()), "{calls:?}");
        assert!(idx.edges.iter().any(|e| {
            e.edge_kind == EDGE_CALLS && e.from_symbol == "A::go" && e.to_symbol == "b"
        }));
    }

    #[test]
    fn unsupported_extension_returns_none() {
        assert!(extract_file("data.csv", "a,b\n").is_none());
    }
}
