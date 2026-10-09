//! The call-graph fact view: resolved call edges between functions, each with
//! the precision and the algorithm behind it.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::analysis_neutral::calls::facts::{CallAlgorithm, CallPrecision};
use crate::core::view_index::IndexedCallEdge;
use crate::core::{AnalysisDb, FileId, FunctionId, Span, SymbolId};

/// Call-graph fact view. Requesting this view maps to the `call_graph`
/// capability.
///
/// Every edge is one resolved candidate callee of one call site. For Go, the
/// typed frontend resolves static calls exactly and narrows dynamic calls
/// (interface methods, function values) by variable-type analysis, falling back
/// to the class hierarchy; a call whose candidates the hierarchy cannot list
/// names its abstract callee instead. Edges between functions in the scanned
/// sources use the same [`FunctionId`]s as the `Functions` view.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct CallGraph<'a> {
    pub(crate) db: &'a AnalysisDb,
}

/// What a call edge calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum CallGraphCallee<'a> {
    /// A function declared in the scanned sources.
    Function(FunctionId),
    /// A symbol the call resolves to when no function fact backs it.
    Symbol(SymbolId),
    /// A callee outside the scanned sources, by label: a dependency's function
    /// or method (`go:func:fmt.Println`), a builtin (`go:builtin:panic`), or the
    /// abstract callee a type-hierarchy edge names (`go:interface-method:...`,
    /// `go:func-value:...`).
    External(&'a str),
}

/// How certain a call edge is, from most to least precise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum CallEdgePrecision {
    /// The callee is fixed by the program text and its types.
    Exact,
    /// A complete answer for the analyzed configuration (build tags, loaded
    /// packages), such as every type variable-type analysis found flowing to
    /// the call.
    SetupAware,
    /// One of the candidates a hierarchy-based analysis could not narrow.
    Conservative,
    /// An answer from names rather than types.
    Heuristic,
}

impl CallEdgePrecision {
    /// Whether this precision is at least as high as `minimum`.
    pub fn at_least(self, minimum: CallEdgePrecision) -> bool {
        self <= minimum
    }
}

/// How a call edge was resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum CallEdgeAlgorithm {
    /// A static call: the callee is named directly.
    Static,
    /// Variable-type analysis: the concrete types that flow to a dynamic call.
    VariableTypeAnalysis,
    /// Class-hierarchy analysis: the types that implement the called method.
    ClassHierarchy,
    /// The type hierarchy only: the edge names the abstract callee because the
    /// class hierarchy had too many candidates to list.
    TypeHierarchy,
    /// Resolution from names, imports and declarations in the source text.
    Syntactic,
}

/// One resolved call edge.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct CallGraphEdge<'a> {
    /// The calling function.
    pub caller: FunctionId,
    /// What the call calls.
    pub callee: CallGraphCallee<'a>,
    /// The file of the call expression.
    pub file: FileId,
    /// The call expression.
    pub span: &'a Span,
    /// How certain the edge is.
    pub precision: CallEdgePrecision,
    /// How the edge was resolved.
    pub algorithm: CallEdgeAlgorithm,
}

/// Bounds and filters for a reachability or path query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct CallGraphWalk {
    /// The most call edges a path may take.
    pub max_depth: usize,
    /// The least precise edge a path may take.
    pub min_precision: CallEdgePrecision,
}

impl CallGraphWalk {
    /// Walks paths of up to `max_depth` edges over edges of any precision.
    pub fn new(max_depth: usize) -> Self {
        Self {
            max_depth,
            min_precision: CallEdgePrecision::Heuristic,
        }
    }

    /// Restricts the walk to edges at least as precise as `minimum`.
    pub fn with_min_precision(mut self, minimum: CallEdgePrecision) -> Self {
        self.min_precision = minimum;
        self
    }
}

/// The functions and external callees a walk reached from its root.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CallGraphReach<'a> {
    root: FunctionId,
    /// Each reached function and the edge it was first reached by.
    reached: BTreeMap<FunctionId, Option<CallGraphEdge<'a>>>,
    externals: BTreeSet<&'a str>,
}

impl<'a> CallGraphReach<'a> {
    /// Whether the walk reached `function` (the root reaches itself).
    pub fn contains(&self, function: FunctionId) -> bool {
        self.reached.contains_key(&function)
    }

    /// The reached functions in ascending id order, the root included.
    pub fn functions(&self) -> impl Iterator<Item = FunctionId> + '_ {
        self.reached.keys().copied()
    }

    /// The external callees the walk reached, in label order.
    pub fn externals(&self) -> impl Iterator<Item = &'a str> + '_ {
        self.externals.iter().copied()
    }

    /// One shortest path from the root to `function`, as the edges it takes;
    /// empty for the root itself and `None` when `function` was not reached.
    pub fn path_to(&self, function: FunctionId) -> Option<Vec<CallGraphEdge<'a>>> {
        let mut edges = Vec::new();
        let mut current = function;
        while current != self.root {
            let edge = (*self.reached.get(&current)?)?;
            edges.push(edge);
            current = edge.caller;
        }
        edges.reverse();
        Some(edges)
    }
}

impl<'a> CallGraph<'a> {
    /// Every call edge out of `caller`, in deterministic order.
    pub fn callees(self, caller: FunctionId) -> impl Iterator<Item = CallGraphEdge<'a>> + 'a {
        let db = self.db;
        let index = db.call_graph_index();
        index
            .by_caller
            .get(&caller)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(move |edge| edge_at(db, *edge))
    }

    /// Every call edge into `callee`, in deterministic order.
    pub fn callers(self, callee: FunctionId) -> impl Iterator<Item = CallGraphEdge<'a>> + 'a {
        let db = self.db;
        let index = db.call_graph_index();
        index
            .by_callee
            .get(&callee)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(move |edge| edge_at(db, *edge))
    }

    /// Every call edge, in deterministic order.
    pub fn edges(self) -> impl Iterator<Item = CallGraphEdge<'a>> + 'a {
        let db = self.db;
        let count = db.call_graph_index().edges.len();
        (0..count)
            .map(move |edge| edge_at(db, u32::try_from(edge).expect("fewer than 2^32 call edges")))
    }

    /// The functions and external callees reachable from `root` within `walk`,
    /// breadth first, so every recorded path is a shortest one.
    pub fn reachable(self, root: FunctionId, walk: CallGraphWalk) -> CallGraphReach<'a> {
        let mut reach = CallGraphReach {
            root,
            reached: BTreeMap::from([(root, None)]),
            externals: BTreeSet::new(),
        };
        let mut queue = VecDeque::from([(root, 0_usize)]);
        while let Some((function, depth)) = queue.pop_front() {
            if depth == walk.max_depth {
                continue;
            }
            for edge in self.callees(function) {
                if !edge.precision.at_least(walk.min_precision) {
                    continue;
                }
                match edge.callee {
                    CallGraphCallee::Function(callee) => {
                        if let std::collections::btree_map::Entry::Vacant(entry) =
                            reach.reached.entry(callee)
                        {
                            entry.insert(Some(edge));
                            queue.push_back((callee, depth + 1));
                        }
                    }
                    CallGraphCallee::External(label) => {
                        reach.externals.insert(label);
                    }
                    CallGraphCallee::Symbol(_) => {}
                }
            }
        }
        reach
    }

    /// The acyclic call paths from `from` to `to` within `walk`, at most
    /// `max_paths` of them, in depth-first order of the deterministic edge
    /// order.
    pub fn paths(
        self,
        from: FunctionId,
        to: FunctionId,
        walk: CallGraphWalk,
        max_paths: usize,
    ) -> Vec<Vec<CallGraphEdge<'a>>> {
        let mut paths = Vec::new();
        let mut path = Vec::new();
        let mut on_path = BTreeSet::from([from]);
        self.extend_paths(
            from,
            to,
            walk,
            max_paths,
            &mut path,
            &mut on_path,
            &mut paths,
        );
        paths
    }

    #[allow(clippy::too_many_arguments)]
    fn extend_paths(
        self,
        function: FunctionId,
        to: FunctionId,
        walk: CallGraphWalk,
        max_paths: usize,
        path: &mut Vec<CallGraphEdge<'a>>,
        on_path: &mut BTreeSet<FunctionId>,
        paths: &mut Vec<Vec<CallGraphEdge<'a>>>,
    ) {
        if paths.len() >= max_paths || path.len() == walk.max_depth {
            return;
        }
        for edge in self.callees(function) {
            if paths.len() >= max_paths {
                return;
            }
            if !edge.precision.at_least(walk.min_precision) {
                continue;
            }
            let CallGraphCallee::Function(callee) = edge.callee else {
                continue;
            };
            if callee == to {
                path.push(edge);
                paths.push(path.clone());
                path.pop();
                continue;
            }
            if !on_path.insert(callee) {
                continue;
            }
            path.push(edge);
            self.extend_paths(callee, to, walk, max_paths, path, on_path, paths);
            path.pop();
            on_path.remove(&callee);
        }
    }
}

fn edge_at(db: &AnalysisDb, edge: u32) -> CallGraphEdge<'_> {
    let edge = db.call_graph_index().edge(db, edge);
    CallGraphEdge {
        caller: edge.caller,
        callee: callee(&edge),
        file: edge.file,
        span: edge.span,
        precision: precision(edge.precision),
        algorithm: algorithm(edge.algorithm),
    }
}

fn callee<'a>(edge: &IndexedCallEdge<'a>) -> CallGraphCallee<'a> {
    if let Some(function) = edge.function {
        return CallGraphCallee::Function(function);
    }
    if let Some(symbol) = edge.symbol {
        return CallGraphCallee::Symbol(symbol);
    }
    CallGraphCallee::External(edge.label)
}

fn precision(precision: CallPrecision) -> CallEdgePrecision {
    match precision {
        CallPrecision::Exact => CallEdgePrecision::Exact,
        CallPrecision::SetupAware => CallEdgePrecision::SetupAware,
        CallPrecision::Conservative => CallEdgePrecision::Conservative,
        CallPrecision::Heuristic
        | CallPrecision::Ambiguous
        | CallPrecision::Unknown
        | CallPrecision::Unsupported => CallEdgePrecision::Heuristic,
    }
}

fn algorithm(algorithm: CallAlgorithm) -> CallEdgeAlgorithm {
    match algorithm {
        CallAlgorithm::GoStatic => CallEdgeAlgorithm::Static,
        CallAlgorithm::GoVta => CallEdgeAlgorithm::VariableTypeAnalysis,
        CallAlgorithm::GoCha | CallAlgorithm::GoRta => CallEdgeAlgorithm::ClassHierarchy,
        CallAlgorithm::TypeHierarchy => CallEdgeAlgorithm::TypeHierarchy,
        // Only the refinement tiers label edges with these, and the view reads
        // the call layer's targets, which carry none of them.
        CallAlgorithm::PointsTo => CallEdgeAlgorithm::VariableTypeAnalysis,
        CallAlgorithm::FrameworkModel | CallAlgorithm::RepoModel => CallEdgeAlgorithm::Syntactic,
        CallAlgorithm::SyntaxOnly
        | CallAlgorithm::DirectReference
        | CallAlgorithm::ImportBinding
        | CallAlgorithm::ConstructorBinding
        | CallAlgorithm::StaticMember
        | CallAlgorithm::DirectMember
        | CallAlgorithm::SummaryAssisted
        | CallAlgorithm::Unsupported => CallEdgeAlgorithm::Syntactic,
    }
}
