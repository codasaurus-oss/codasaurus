//! In-memory symbol graph for review-time blast-radius detection.

use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use std::collections::HashMap;

/// A symbol and the file that defines it.
#[derive(Debug, Clone)]
pub struct SymbolNode {
    pub name: String,
    pub file: String,
    pub kind: SymbolKind,
}

/// The kind of symbol stored in a [`CodeGraph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Function,
    Class,
    Import,
    Variable,
    Type,
}

/// The relationship represented by a graph edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    Calls,
    Imports,
}

/// A directed graph keyed by symbol name.
#[derive(Debug)]
pub struct CodeGraph {
    graph: DiGraph<SymbolNode, EdgeKind>,
    node_indices: HashMap<String, NodeIndex>,
    pub file_to_nodes: HashMap<String, Vec<String>>,
}

impl Default for CodeGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeGraph {
    #[must_use]
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            node_indices: HashMap::new(),
            file_to_nodes: HashMap::new(),
        }
    }

    /// Add a symbol once; later insertions with the same name are ignored.
    pub fn add_symbol(&mut self, name: &str, file: &str, kind: SymbolKind) {
        if self.node_indices.contains_key(name) {
            return;
        }
        let idx = self.graph.add_node(SymbolNode {
            name: name.to_string(),
            file: file.to_string(),
            kind,
        });
        self.node_indices.insert(name.to_string(), idx);
        self.file_to_nodes
            .entry(file.to_string())
            .or_default()
            .push(name.to_string());
    }

    /// Add a directed edge when both symbols exist; duplicate edges are ignored.
    pub fn add_edge(&mut self, from: &str, to: &str, kind: EdgeKind) {
        if let (Some(&from_idx), Some(&to_idx)) =
            (self.node_indices.get(from), self.node_indices.get(to))
        {
            let exists = self
                .graph
                .edges(from_idx)
                .any(|edge| edge.target() == to_idx && *edge.weight() == kind);
            if !exists {
                self.graph.add_edge(from_idx, to_idx, kind);
            }
        }
    }

    /// Direct out-neighbors count (cheap pre-check before BFS).
    #[must_use]
    pub fn out_degree(&self, symbol: &str) -> usize {
        self.node_indices
            .get(symbol)
            .map_or(0, |&idx| self.graph.neighbors(idx).count())
    }

    /// Find other symbols affected by a change to `symbol`, up to `max_hops` away.
    #[must_use]
    pub fn blast_radius(&self, symbol: &str, max_hops: usize) -> Vec<&SymbolNode> {
        self.blast_radius_capped(symbol, max_hops, usize::MAX)
    }

    /// Like [`Self::blast_radius`], but stops once `cap` nodes have been collected.
    #[must_use]
    pub fn blast_radius_capped(
        &self,
        symbol: &str,
        max_hops: usize,
        cap: usize,
    ) -> Vec<&SymbolNode> {
        if cap == 0 || max_hops == 0 {
            return Vec::new();
        }
        let Some(&start) = self.node_indices.get(symbol) else {
            return Vec::new();
        };

        let mut affected = Vec::new();
        let mut visited = std::collections::HashSet::from([start]);
        let mut queue = std::collections::VecDeque::new();
        for neighbor in self.graph.neighbors(start) {
            if visited.insert(neighbor) {
                queue.push_back((neighbor, 1));
            }
        }

        while let Some((node, depth)) = queue.pop_front() {
            affected.push(&self.graph[node]);
            if affected.len() >= cap {
                break;
            }
            if depth < max_hops {
                for neighbor in self.graph.neighbors(node) {
                    if visited.insert(neighbor) {
                        queue.push_back((neighbor, depth + 1));
                    }
                }
            }
        }
        affected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph() -> CodeGraph {
        let mut graph = CodeGraph::new();
        graph.add_symbol("a", "src/a.rs", SymbolKind::Function);
        graph.add_symbol("b", "src/b.rs", SymbolKind::Function);
        graph.add_symbol("c", "src/c.rs", SymbolKind::Function);
        graph.add_edge("a", "b", EdgeKind::Calls);
        graph.add_edge("b", "c", EdgeKind::Calls);
        graph
    }

    #[test]
    fn duplicate_symbols_and_edges_are_idempotent() {
        let mut graph = graph();
        graph.add_symbol("a", "src/a.rs", SymbolKind::Function);
        graph.add_edge("a", "b", EdgeKind::Calls);

        assert_eq!(graph.file_to_nodes["src/a.rs"].len(), 1);
        assert_eq!(graph.out_degree("a"), 1);
    }

    #[test]
    fn blast_radius_excludes_the_changed_symbol() {
        let graph = graph();
        let affected = graph.blast_radius("a", 2);
        let names: Vec<&str> = affected.iter().map(|node| node.name.as_str()).collect();
        assert_eq!(names, ["b", "c"]);
    }

    #[test]
    fn blast_radius_honors_hop_and_result_caps() {
        let graph = graph();
        assert_eq!(graph.blast_radius_capped("a", 1, 10).len(), 1);
        assert!(graph.blast_radius_capped("a", 2, 0).is_empty());
        assert_eq!(graph.blast_radius_capped("a", 2, 1).len(), 1);
    }
}
