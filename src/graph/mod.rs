use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct SymbolNode {
    pub name: String,
    pub file: String,
    pub kind: SymbolKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SymbolKind {
    Function,
    Class,
    Import,
    Variable,
    Type,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EdgeKind {
    Calls,
    Imports,
}

pub struct CodeGraph {
    graph: DiGraph<SymbolNode, EdgeKind>,
    pub node_indices: HashMap<String, NodeIndex>,
    pub files: HashMap<String, Vec<NodeIndex>>,
    pub file_to_nodes: HashMap<String, Vec<String>>,
}

impl Default for CodeGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeGraph {
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            node_indices: HashMap::new(),
            files: HashMap::new(),
            file_to_nodes: HashMap::new(),
        }
    }

    pub fn add_symbol(&mut self, name: &str, file: &str, kind: SymbolKind) {
        let idx = self.graph.add_node(SymbolNode {
            name: name.to_string(),
            file: file.to_string(),
            kind,
        });
        self.node_indices.insert(name.to_string(), idx);
        self.files.entry(file.to_string()).or_default().push(idx);
        self.file_to_nodes
            .entry(file.to_string())
            .or_default()
            .push(name.to_string());
    }

    pub fn add_edge(&mut self, from: &str, to: &str, kind: EdgeKind) {
        if let (Some(&from_idx), Some(&to_idx)) =
            (self.node_indices.get(from), self.node_indices.get(to))
        {
            self.graph.add_edge(from_idx, to_idx, kind);
        }
    }

    /// Direct out-neighbors count (cheap pre-check before BFS).
    pub fn out_degree(&self, symbol: &str) -> usize {
        self.node_indices
            .get(symbol)
            .map(|&idx| self.graph.neighbors(idx).count())
            .unwrap_or(0)
    }

    /// Find all symbols that could be affected by a change to `symbol`, up to `max_hops` away.
    pub fn blast_radius(&self, symbol: &str, max_hops: usize) -> Vec<&SymbolNode> {
        self.blast_radius_capped(symbol, max_hops, usize::MAX)
    }

    /// Like [`blast_radius`], but stops once `cap` nodes have been collected.
    pub fn blast_radius_capped(
        &self,
        symbol: &str,
        max_hops: usize,
        cap: usize,
    ) -> Vec<&SymbolNode> {
        let mut affected = Vec::new();
        if let Some(&start) = self.node_indices.get(symbol) {
            let mut visited = std::collections::HashSet::new();
            let mut queue = std::collections::VecDeque::new();
            let mut depths = std::collections::HashMap::new();

            visited.insert(start);
            queue.push_back(start);
            depths.insert(start, 0);

            while let Some(nx) = queue.pop_front() {
                let d = depths[&nx];
                affected.push(&self.graph[nx]);
                if affected.len() >= cap {
                    break;
                }

                if d < max_hops {
                    for neighbor in self.graph.neighbors(nx) {
                        if visited.insert(neighbor) {
                            depths.insert(neighbor, d + 1);
                            queue.push_back(neighbor);
                        }
                    }
                }
            }
        }
        affected
    }

    /// Find symbols by file path — returns all symbol names in the given file.
    pub fn symbols_in_file(&self, file: &str) -> Vec<&str> {
        self.file_to_nodes
            .get(file)
            .map(|v| v.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }
}
