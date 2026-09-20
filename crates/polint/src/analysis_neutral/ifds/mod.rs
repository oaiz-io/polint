use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use serde::{Deserialize, Serialize};

use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::calls::facts::{CallSiteFact, CallTargetStatus};
use crate::analysis_neutral::cfg::facts::{CfgEdgeFact, CfgFunctionFact, CfgNodeFact};
use crate::analysis_neutral::cfg::ids::CfgNodeId;
use crate::analysis_neutral::data_flow::facts::{
    DataFlowBudgetReason, DataFlowEdgeFact, DataFlowEdgeKind, DataFlowModelKind, DataFlowNodeKind,
    DataFlowStatus,
};
use crate::analysis_neutral::data_flow::store::DataFlowStore;
use crate::analysis_neutral::ids::{
    CallSiteId, DataFlowEdgeId, DataFlowNodeId, DataFlowPathId, RefinedCallEdgeId,
};
use crate::analysis_neutral::refined_calls::facts::RefinedCallEdgeFact;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataFlowPath {
    pub id: DataFlowPathId,
    pub source: DataFlowNodeId,
    pub sink: DataFlowNodeId,
    pub edges: Vec<DataFlowEdgeId>,
    pub status: DataFlowPathStatus,
    pub budget: DataFlowSearchBudget,
    pub budget_reason: Option<DataFlowBudgetReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataFlowSearchBudget {
    pub max_depth: usize,
    pub max_paths: usize,
}

impl Default for DataFlowSearchBudget {
    fn default() -> Self {
        Self {
            max_depth: 32,
            max_paths: 256,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataFlowPathStatus {
    Found,
    NotFound,
    Unknown,
    BudgetExceeded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum IcfgEdgeKind {
    Intra(crate::analysis_neutral::cfg::facts::CfgEdgeKind),
    Call(CallSiteId),
    Return(CallSiteId),
    CallToReturn(CallSiteId, crate::analysis_neutral::cfg::facts::CfgEdgeKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct IcfgEdge {
    pub to: CfgNodeId,
    pub kind: IcfgEdgeKind,
}

#[derive(Debug, Default)]
pub struct Icfg {
    outgoing: BTreeMap<CfgNodeId, Vec<IcfgEdge>>,
    calls: BTreeMap<CallSiteId, BTreeSet<RefinedCallEdgeId>>,
}

impl Icfg {
    pub fn build(db: &impl AnalysisHost) -> Self {
        Self::from_facts(
            db.cfg_functions(),
            db.cfg_nodes(),
            db.cfg_edges(),
            db.call_sites(),
            db.refined_call_edges(),
        )
    }

    fn from_facts(
        cfg_functions: &[CfgFunctionFact],
        cfg_nodes: &[CfgNodeFact],
        cfg_edges: &[CfgEdgeFact],
        call_sites: &[CallSiteFact],
        refined_calls: &[RefinedCallEdgeFact],
    ) -> Self {
        // One pass over the CFG nodes instead of a scan per call site (research
        // doc section 3.2, the `Icfg::build` row: O(sites x nodes), paid in
        // `abstract_domains` and again in data flow). `or_insert` keeps the first
        // node per operation, which is what the `find` returned.
        let mut node_by_operation = HashMap::with_capacity(cfg_nodes.len());
        for node in cfg_nodes {
            if let Some(operation) = node.operation {
                node_by_operation.entry(operation).or_insert(node.id);
            }
        }
        let call_nodes = call_sites
            .iter()
            .filter_map(|site| {
                node_by_operation
                    .get(&site.operation)
                    .map(|node| (site.id, *node))
            })
            .collect::<BTreeMap<_, _>>();
        let call_site_by_node = call_nodes
            .iter()
            .map(|(site, node)| (*node, *site))
            .collect::<BTreeMap<_, _>>();
        let functions = cfg_functions
            .iter()
            .map(|function| (function.function, function))
            .collect::<BTreeMap<_, _>>();
        let mut graph = Self::default();

        for edge in cfg_edges {
            let kind = call_site_by_node
                .get(&edge.from)
                .copied()
                .map_or(IcfgEdgeKind::Intra(edge.kind), |site| {
                    IcfgEdgeKind::CallToReturn(site, edge.kind)
                });
            graph.push_edge(edge.from, edge.to, kind);
        }

        for refined in refined_calls.iter().filter(|edge| {
            edge.status == CallTargetStatus::Resolved && edge.target_function.is_some()
        }) {
            graph
                .calls
                .entry(refined.site)
                .or_default()
                .insert(refined.id);
            let Some(call_node) = call_nodes.get(&refined.site).copied() else {
                continue;
            };
            let Some(callee) = refined
                .target_function
                .and_then(|function| functions.get(&function).copied())
            else {
                continue;
            };
            graph.push_edge(
                call_node,
                callee.entry_node,
                IcfgEdgeKind::Call(refined.site),
            );
            let continuations = graph
                .outgoing
                .get(&call_node)
                .into_iter()
                .flatten()
                .filter_map(|edge| {
                    matches!(edge.kind, IcfgEdgeKind::CallToReturn(site, _) if site == refined.site)
                        .then_some(edge.to)
                })
                .collect::<Vec<_>>();
            for continuation in continuations {
                graph.push_edge(
                    callee.normal_exit_node,
                    continuation,
                    IcfgEdgeKind::Return(refined.site),
                );
            }
        }

        for edges in graph.outgoing.values_mut() {
            edges.sort();
            edges.dedup();
        }
        graph
    }

    fn push_edge(&mut self, from: CfgNodeId, to: CfgNodeId, kind: IcfgEdgeKind) {
        self.outgoing
            .entry(from)
            .or_default()
            .push(IcfgEdge { to, kind });
    }

    pub fn outgoing(&self, node: CfgNodeId) -> &[IcfgEdge] {
        self.outgoing.get(&node).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn has_resolved_call(&self, site: CallSiteId) -> bool {
        self.calls.get(&site).is_some_and(|edges| !edges.is_empty())
    }

    fn accepts_boundary(&self, site: CallSiteId, refined_call: Option<RefinedCallEdgeId>) -> bool {
        let Some(edges) = self.calls.get(&site) else {
            return false;
        };
        refined_call.is_none_or(|edge| edges.contains(&edge))
    }
}

pub fn find_taint_paths(
    db: &impl AnalysisHost,
    store: &DataFlowStore,
    source: DataFlowNodeId,
    sink: DataFlowNodeId,
    sanitizer_sites: &BTreeSet<CallSiteId>,
    budget: DataFlowSearchBudget,
) -> Vec<DataFlowPath> {
    IfdsTaintSolver::new(db, store, source, sink, sanitizer_sites, budget).solve()
}

struct IfdsTaintSolver<'a> {
    icfg: Icfg,
    store: &'a DataFlowStore,
    source: DataFlowNodeId,
    sink: DataFlowNodeId,
    sanitizer_sites: &'a BTreeSet<CallSiteId>,
    budget: DataFlowSearchBudget,
}

impl<'a> IfdsTaintSolver<'a> {
    fn new(
        db: &impl AnalysisHost,
        store: &'a DataFlowStore,
        source: DataFlowNodeId,
        sink: DataFlowNodeId,
        sanitizer_sites: &'a BTreeSet<CallSiteId>,
        budget: DataFlowSearchBudget,
    ) -> Self {
        Self {
            icfg: Icfg::build(db),
            store,
            source,
            sink,
            sanitizer_sites,
            budget,
        }
    }

    fn solve(&self) -> Vec<DataFlowPath> {
        if self.budget.max_paths == 0 {
            return vec![self.status_path(
                DataFlowPathId(0),
                DataFlowPathStatus::BudgetExceeded,
                Some(DataFlowBudgetReason::PathCount),
            )];
        }

        let initial_state = ExplodedState {
            node: self.source,
            fact: IfdsFact::Tainted,
            call_stack: Vec::new(),
        };
        let mut queue = VecDeque::from([PathFrame {
            state: initial_state.clone(),
            edges: Vec::new(),
            visited: BTreeSet::from([initial_state]),
        }]);
        let mut paths = Vec::new();
        let mut budget_exceeded_reason = None;
        let mut saw_uncertain_edge = false;

        while let Some(frame) = queue.pop_front() {
            if frame.state.node == self.sink {
                paths.push(DataFlowPath {
                    id: DataFlowPathId(paths.len() as u64),
                    source: self.source,
                    sink: self.sink,
                    edges: frame.edges,
                    status: DataFlowPathStatus::Found,
                    budget: self.budget,
                    budget_reason: None,
                });
                if paths.len() >= self.budget.max_paths {
                    budget_exceeded_reason =
                        (!queue.is_empty()).then_some(DataFlowBudgetReason::PathCount);
                    break;
                }
                continue;
            }

            let mut outgoing = self.store.outgoing(frame.state.node);
            outgoing.sort_by_key(|edge| edge.id);
            if frame.edges.len() >= self.budget.max_depth {
                self.observe_truncated_edges(
                    &frame,
                    &outgoing,
                    &mut budget_exceeded_reason,
                    &mut saw_uncertain_edge,
                );
                continue;
            }

            for edge in outgoing {
                match edge.status {
                    DataFlowStatus::BudgetExceeded => {
                        budget_exceeded_reason = Some(DataFlowBudgetReason::EdgeLimit);
                        continue;
                    }
                    DataFlowStatus::Present => {}
                    DataFlowStatus::Unknown
                    | DataFlowStatus::Unsupported
                    | DataFlowStatus::SetupMissing
                    | DataFlowStatus::Rejected => {
                        saw_uncertain_edge |= self.transfer(&frame.state, edge).is_some();
                        continue;
                    }
                }
                let Some(next_state) = self.transfer(&frame.state, edge) else {
                    continue;
                };
                if frame.visited.contains(&next_state) {
                    continue;
                }
                let mut next_edges = frame.edges.clone();
                next_edges.push(edge.id);
                let mut next_visited = frame.visited.clone();
                next_visited.insert(next_state.clone());
                queue.push_back(PathFrame {
                    state: next_state,
                    edges: next_edges,
                    visited: next_visited,
                });
            }
        }

        if paths.is_empty() {
            let status = if budget_exceeded_reason.is_some() {
                DataFlowPathStatus::BudgetExceeded
            } else if saw_uncertain_edge {
                DataFlowPathStatus::Unknown
            } else {
                DataFlowPathStatus::NotFound
            };
            paths.push(self.status_path(DataFlowPathId(0), status, budget_exceeded_reason));
        } else if let Some(reason) = budget_exceeded_reason {
            paths.push(self.status_path(
                DataFlowPathId(paths.len() as u64),
                DataFlowPathStatus::BudgetExceeded,
                Some(reason),
            ));
        }
        paths
    }

    fn observe_truncated_edges(
        &self,
        frame: &PathFrame,
        edges: &[&DataFlowEdgeFact],
        budget_reason: &mut Option<DataFlowBudgetReason>,
        saw_uncertain: &mut bool,
    ) {
        let mut has_present_continuation = false;
        for edge in edges {
            match edge.status {
                DataFlowStatus::BudgetExceeded => {
                    *budget_reason = Some(DataFlowBudgetReason::EdgeLimit);
                }
                DataFlowStatus::Present => {
                    has_present_continuation |= self.transfer(&frame.state, edge).is_some();
                }
                DataFlowStatus::Unknown
                | DataFlowStatus::Unsupported
                | DataFlowStatus::SetupMissing
                | DataFlowStatus::Rejected => {
                    *saw_uncertain |= self.transfer(&frame.state, edge).is_some();
                }
            }
        }
        if has_present_continuation && budget_reason.is_none() {
            *budget_reason = Some(DataFlowBudgetReason::PathDepth);
        }
    }

    fn transfer(&self, state: &ExplodedState, edge: &DataFlowEdgeFact) -> Option<ExplodedState> {
        if self.kills_taint(edge) {
            return None;
        }
        let mut call_stack = state.call_stack.clone();
        match boundary(edge, self.store) {
            Boundary::Local => {}
            Boundary::Call(site) => {
                if !self.icfg.accepts_boundary(site, edge.refined_call) {
                    return None;
                }
                call_stack.push(site);
            }
            Boundary::Return(site) => {
                if !self.icfg.accepts_boundary(site, edge.refined_call)
                    || call_stack.pop() != Some(site)
                {
                    return None;
                }
            }
        }
        Some(ExplodedState {
            node: edge.to,
            fact: state.fact,
            call_stack,
        })
    }

    fn kills_taint(&self, edge: &DataFlowEdgeFact) -> bool {
        if edge.status != DataFlowStatus::Present {
            return false;
        }
        if matches!(
            edge.kind,
            DataFlowEdgeKind::Sanitizer | DataFlowEdgeKind::Barrier
        ) {
            return true;
        }
        if edge
            .call_site
            .is_some_and(|site| self.sanitizer_sites.contains(&site))
        {
            return true;
        }
        if edge.model.is_some_and(|id| {
            self.store.models().iter().any(|model| {
                model.id == id
                    && matches!(
                        model.kind,
                        DataFlowModelKind::Sanitizer | DataFlowModelKind::Barrier
                    )
            })
        }) {
            return true;
        }
        self.store.nodes().iter().any(|node| {
            node.id == edge.to
                && matches!(
                    node.kind,
                    DataFlowNodeKind::Sanitizer | DataFlowNodeKind::Barrier
                )
        })
    }

    fn status_path(
        &self,
        id: DataFlowPathId,
        status: DataFlowPathStatus,
        budget_reason: Option<DataFlowBudgetReason>,
    ) -> DataFlowPath {
        DataFlowPath {
            id,
            source: self.source,
            sink: self.sink,
            edges: Vec::new(),
            status,
            budget: self.budget,
            budget_reason,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ExplodedState {
    node: DataFlowNodeId,
    fact: IfdsFact,
    call_stack: Vec<CallSiteId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum IfdsFact {
    Tainted,
}

#[derive(Debug, Clone)]
struct PathFrame {
    state: ExplodedState,
    edges: Vec<DataFlowEdgeId>,
    visited: BTreeSet<ExplodedState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Boundary {
    Local,
    Call(CallSiteId),
    Return(CallSiteId),
}

fn boundary(edge: &DataFlowEdgeFact, store: &DataFlowStore) -> Boundary {
    let Some(site) = edge.call_site else {
        return Boundary::Local;
    };
    match edge.kind {
        DataFlowEdgeKind::CallArgumentToParameter | DataFlowEdgeKind::ReceiverToMethod => {
            Boundary::Call(site)
        }
        DataFlowEdgeKind::CallReturnToUse => Boundary::Return(site),
        DataFlowEdgeKind::SummaryProjected => {
            let from = store.nodes().iter().find(|node| node.id == edge.from);
            let to = store.nodes().iter().find(|node| node.id == edge.to);
            match (from.map(|node| node.kind), to.map(|node| node.kind)) {
                (Some(from), Some(DataFlowNodeKind::SummaryInput))
                    if from != DataFlowNodeKind::SummaryInput =>
                {
                    Boundary::Call(site)
                }
                (Some(DataFlowNodeKind::SummaryOutput), Some(to))
                    if to != DataFlowNodeKind::SummaryOutput =>
                {
                    Boundary::Return(site)
                }
                _ => Boundary::Local,
            }
        }
        _ => Boundary::Local,
    }
}

#[cfg(test)]
mod tests {
    use crate::analysis_api::FunctionFact;
    use crate::analysis_neutral::LocalAnalysisDb;

    use super::*;
    use crate::analysis_neutral::calls::facts::{
        CallAlgorithm, CallCallee, CallEdgeKind, CallPrecision, CallProvenance, CallSiteFact,
        CallSyntaxKind,
    };
    use crate::analysis_neutral::calls::store::CallOutput;
    use crate::analysis_neutral::cfg::facts::{
        CfgEdgeKind, CfgFunctionFact, CfgNodeFact, CfgNodeKind, CfgPrecision, CfgStatus, CfgView,
    };
    use crate::analysis_neutral::cfg::ids::{BasicBlockId, CfgEdgeId, CfgFunctionId};
    use crate::analysis_neutral::data_flow::facts::{
        DataFlowAlgorithm, DataFlowConfidence, DataFlowNodeFact, DataFlowPrecision,
        DataFlowProvenance, DataFlowValidation,
    };
    use crate::analysis_neutral::data_flow::store::DataFlowOutput;
    use crate::analysis_neutral::ids::{CallTargetId, MirBodyId, MirOpId};
    use crate::analysis_neutral::refined_calls::facts::{
        RefinedCallConfidence, RefinedCallEdgeFact, RefinedCallTier, RefinedCallValidation,
    };
    use crate::analysis_neutral::refined_calls::store::RefinedCallOutput;
    use crate::internal_core::{FileId, FunctionId, Language, Span};

    /// W1 commit 3: the per-call-site scan over `cfg_nodes` became one index.
    /// The scan took the first node carrying the operation, so a second node with
    /// the same operation must stay invisible to the call-to-return edge.
    #[test]
    fn icfg_call_nodes_keep_the_first_cfg_node_per_operation() {
        let interner = crate::internal_core::StableKeyInterner::default();
        let sites = vec![call_site(1)];
        let refined = vec![refined_edge(1)];
        let functions = vec![
            cfg_function(&interner, 0, 0, 0, 3),
            cfg_function(&interner, 1, 1, 4, 5),
        ];
        // Two nodes carry `MirOpId(1)`; the scan returned node 1.
        let nodes = vec![
            cfg_node(&interner, 1, 0, Some(MirOpId(1))),
            cfg_node(&interner, 3, 0, Some(MirOpId(1))),
            cfg_node(&interner, 2, 0, None),
        ];
        let edges = vec![cfg_edge(&interner, 1, 2), cfg_edge(&interner, 3, 2)];

        let icfg = Icfg::from_facts(&functions, &nodes, &edges, &sites, &refined);

        let from_first = icfg
            .outgoing
            .get(&CfgNodeId(1))
            .into_iter()
            .flatten()
            .copied()
            .collect::<BTreeSet<_>>();
        assert!(
            from_first.contains(&IcfgEdge {
                to: CfgNodeId(2),
                kind: IcfgEdgeKind::CallToReturn(CallSiteId(1), CfgEdgeKind::Normal),
            }),
            "the first node for the operation owns the call-to-return edge"
        );
        let from_second = icfg
            .outgoing
            .get(&CfgNodeId(3))
            .into_iter()
            .flatten()
            .copied()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            from_second,
            BTreeSet::from([IcfgEdge {
                to: CfgNodeId(2),
                kind: IcfgEdgeKind::Intra(CfgEdgeKind::Normal),
            }]),
            "the shadowed node keeps a plain intraprocedural edge"
        );
    }

    #[test]
    fn icfg_builds_call_to_return_and_matched_return_edges() {
        let interner = crate::internal_core::StableKeyInterner::default();
        let sites = vec![call_site(1)];
        let refined = vec![refined_edge(1)];
        let functions = vec![
            cfg_function(&interner, 0, 0, 0, 3),
            cfg_function(&interner, 1, 1, 4, 5),
        ];
        let nodes = vec![
            cfg_node(&interner, 1, 0, Some(MirOpId(1))),
            cfg_node(&interner, 2, 0, None),
        ];
        let edges = vec![cfg_edge(&interner, 1, 2)];

        let icfg = Icfg::from_facts(&functions, &nodes, &edges, &sites, &refined);
        let actual = [
            icfg.outgoing
                .get(&CfgNodeId(1))
                .into_iter()
                .flatten()
                .copied()
                .collect::<BTreeSet<_>>(),
            icfg.outgoing
                .get(&CfgNodeId(5))
                .into_iter()
                .flatten()
                .copied()
                .collect::<BTreeSet<_>>(),
        ];
        let expected = [
            BTreeSet::from([
                IcfgEdge {
                    to: CfgNodeId(2),
                    kind: IcfgEdgeKind::CallToReturn(CallSiteId(1), CfgEdgeKind::Normal),
                },
                IcfgEdge {
                    to: CfgNodeId(4),
                    kind: IcfgEdgeKind::Call(CallSiteId(1)),
                },
            ]),
            BTreeSet::from([IcfgEdge {
                to: CfgNodeId(2),
                kind: IcfgEdgeKind::Return(CallSiteId(1)),
            }]),
        ];

        assert_eq!(actual, expected);
    }

    #[test]
    fn solver_rejects_unrealizable_path_with_mismatched_return_site() {
        let db = call_graph();
        let store = DataFlowStore::from_output(
            DataFlowOutput {
                nodes: (0..4).map(node).collect(),
                edges: vec![
                    boundary_edge(0, 0, 1, CallSiteId(1), true),
                    local_edge(1, 1, 2),
                    boundary_edge(2, 2, 3, CallSiteId(2), false),
                ],
                models: Vec::new(),
                budgets: Vec::new(),
            },
            &crate::internal_core::test_stable_key_interner(),
        )
        .expect("valid data-flow store");

        let paths = find_taint_paths(
            &db,
            &store,
            DataFlowNodeId(0),
            DataFlowNodeId(3),
            &BTreeSet::new(),
            DataFlowSearchBudget::default(),
        );

        assert_eq!(paths[0].status, DataFlowPathStatus::NotFound);
    }

    #[test]
    fn solver_accepts_call_and_return_for_same_site() {
        let db = call_graph();
        let store = DataFlowStore::from_output(
            DataFlowOutput {
                nodes: (0..4).map(node).collect(),
                edges: vec![
                    boundary_edge(0, 0, 1, CallSiteId(1), true),
                    local_edge(1, 1, 2),
                    boundary_edge(2, 2, 3, CallSiteId(1), false),
                ],
                models: Vec::new(),
                budgets: Vec::new(),
            },
            &crate::internal_core::test_stable_key_interner(),
        )
        .expect("valid data-flow store");

        let paths = find_taint_paths(
            &db,
            &store,
            DataFlowNodeId(0),
            DataFlowNodeId(3),
            &BTreeSet::new(),
            DataFlowSearchBudget::default(),
        );

        assert_eq!(paths[0].status, DataFlowPathStatus::Found);
    }

    #[test]
    fn solver_kills_only_paths_crossing_a_configured_sanitizer() {
        let db = LocalAnalysisDb::new();
        let mut sanitizer = local_edge(0, 0, 1);
        sanitizer.kind = DataFlowEdgeKind::CallArgumentToReturn;
        sanitizer.call_site = Some(CallSiteId(7));
        let store = DataFlowStore::from_output(
            DataFlowOutput {
                nodes: (0..4).map(node).collect(),
                edges: vec![
                    sanitizer,
                    local_edge(1, 1, 2),
                    local_edge(2, 0, 3),
                    local_edge(3, 3, 2),
                ],
                models: Vec::new(),
                budgets: Vec::new(),
            },
            &crate::internal_core::test_stable_key_interner(),
        )
        .expect("valid data-flow store");

        let paths = find_taint_paths(
            &db,
            &store,
            DataFlowNodeId(0),
            DataFlowNodeId(2),
            &BTreeSet::from([CallSiteId(7)]),
            DataFlowSearchBudget::default(),
        );

        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].status, DataFlowPathStatus::Found);
        assert_eq!(paths[0].edges, vec![DataFlowEdgeId(2), DataFlowEdgeId(3)]);
    }

    #[test]
    fn solver_reports_not_found_when_sanitizer_kills_every_tainted_path() {
        let db = LocalAnalysisDb::new();
        let mut sanitizer = local_edge(0, 0, 1);
        sanitizer.kind = DataFlowEdgeKind::CallArgumentToReturn;
        sanitizer.call_site = Some(CallSiteId(7));
        let store = DataFlowStore::from_output(
            DataFlowOutput {
                nodes: (0..3).map(node).collect(),
                edges: vec![sanitizer, local_edge(1, 1, 2)],
                models: Vec::new(),
                budgets: Vec::new(),
            },
            &crate::internal_core::test_stable_key_interner(),
        )
        .expect("valid data-flow store");

        let paths = find_taint_paths(
            &db,
            &store,
            DataFlowNodeId(0),
            DataFlowNodeId(2),
            &BTreeSet::from([CallSiteId(7)]),
            DataFlowSearchBudget::default(),
        );

        assert_eq!(paths[0].status, DataFlowPathStatus::NotFound);
    }

    fn call_graph() -> LocalAnalysisDb {
        let mut db = LocalAnalysisDb::new();
        let file = db.add_file("app.ts".into(), "app.ts".to_string(), String::new());
        for index in 0..=2 {
            db.push_function(FunctionFact::new(
                FunctionId::from_raw(0),
                file,
                format!("function_{index}"),
                Span::point(file, 1, 1),
                Language::TypeScript,
                false,
                false,
                1,
                Vec::new(),
            ));
        }
        db.replace_call_facts(CallOutput {
            sites: vec![call_site(1), call_site(2)],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("valid call facts");
        db.replace_refined_call_facts(RefinedCallOutput {
            edges: vec![refined_edge(1), refined_edge(2)],
        })
        .expect("valid refined calls");
        db
    }

    fn call_site(id: u64) -> CallSiteFact {
        CallSiteFact {
            id: CallSiteId(id),
            language: Language::TypeScript,
            file: FileId::from_raw(0),
            caller: FunctionId::from_raw(0),
            owner_symbol: None,
            body: MirBodyId(0),
            operation: MirOpId(id),
            span: Span::point(FileId::from_raw(0), id as u32, 1),
            kind: CallSyntaxKind::Function,
            callee: CallCallee::Identifier {
                reference: None,
                name: format!("callee{id}"),
            },
            receiver: None,
            arguments: Vec::new(),
            result: None,
            status: CallTargetStatus::Resolved,
            precision: CallPrecision::Exact,
            in_throw: false,
            stable_key: crate::internal_core::StableKeyId(id as u32),
        }
    }

    fn refined_edge(id: u64) -> RefinedCallEdgeFact {
        RefinedCallEdgeFact {
            id: RefinedCallEdgeId(id),
            site: CallSiteId(id),
            base_target: None,
            caller: FunctionId::from_raw(0),
            target_function: Some(FunctionId::from_raw(id)),
            target_symbol: None,
            synthetic_target: None,
            language: Language::TypeScript,
            edge_kind: CallEdgeKind::Direct,
            algorithm: CallAlgorithm::DirectReference,
            tier: RefinedCallTier::DirectOnly,
            status: CallTargetStatus::Resolved,
            reason: None,
            provenance: CallProvenance::Native,
            precision: CallPrecision::Exact,
            validation: RefinedCallValidation::ReferentiallyValidated,
            confidence: RefinedCallConfidence::High,
            evidence: Vec::new(),
            input_stable_keys: Vec::new(),
            stable_key: crate::internal_core::stable_key_for_test(&format!("refined:{id}")),
        }
    }

    fn cfg_function(
        interner: &crate::internal_core::StableKeyInterner,
        id: u64,
        function: u64,
        entry: u64,
        exit: u64,
    ) -> CfgFunctionFact {
        CfgFunctionFact {
            id: CfgFunctionId(id),
            body: MirBodyId(id),
            function: FunctionId::from_raw(function),
            language: Language::TypeScript,
            file: FileId::from_raw(0),
            span: Span::point(FileId::from_raw(0), 1, 1),
            entry_node: CfgNodeId(entry),
            normal_exit_node: CfgNodeId(exit),
            exceptional_exit_node: None,
            stable_key: interner.intern(format!("cfg-function:{id}")),
            status: CfgStatus::Resolved,
            precision: CfgPrecision::ExactLowered,
        }
    }

    fn cfg_node(
        interner: &crate::internal_core::StableKeyInterner,
        id: u64,
        function: u64,
        operation: Option<MirOpId>,
    ) -> CfgNodeFact {
        CfgNodeFact {
            id: CfgNodeId(id),
            cfg_function: CfgFunctionId(function),
            body: MirBodyId(function),
            operation,
            block: BasicBlockId(id),
            kind: CfgNodeKind::CallSite,
            span: None,
            generated: false,
            operation_ordinal: id as u32,
            stable_key: interner.intern(format!("cfg-node:{id}")),
            status: CfgStatus::Resolved,
            precision: CfgPrecision::ExactLowered,
        }
    }

    fn cfg_edge(
        interner: &crate::internal_core::StableKeyInterner,
        from: u64,
        to: u64,
    ) -> CfgEdgeFact {
        CfgEdgeFact {
            id: CfgEdgeId(0),
            cfg_function: CfgFunctionId(0),
            view: CfgView::AbruptAware,
            from: CfgNodeId(from),
            to: CfgNodeId(to),
            from_block: BasicBlockId(from),
            to_block: BasicBlockId(to),
            kind: CfgEdgeKind::Normal,
            label: None,
            stable_key: interner.intern(format!("cfg-edge:{from}:{to}")),
            status: CfgStatus::Resolved,
            precision: CfgPrecision::ExactLowered,
        }
    }

    fn node(id: u64) -> DataFlowNodeFact {
        DataFlowNodeFact {
            id: DataFlowNodeId(id),
            kind: if id == 1 {
                DataFlowNodeKind::SummaryInput
            } else if id == 2 {
                DataFlowNodeKind::SummaryOutput
            } else {
                DataFlowNodeKind::Place
            },
            language: Language::TypeScript,
            file: None,
            function: None,
            body: None,
            operation: None,
            cfg_node: None,
            place: None,
            symbol: None,
            reference: None,
            call_site: None,
            model: None,
            span: None,
            stable_key: crate::internal_core::stable_key_for_test(&format!("node:{id}")),
        }
    }

    fn boundary_edge(
        id: u64,
        from: u64,
        to: u64,
        site: CallSiteId,
        call: bool,
    ) -> DataFlowEdgeFact {
        DataFlowEdgeFact {
            id: DataFlowEdgeId(id),
            from: DataFlowNodeId(from),
            to: DataFlowNodeId(to),
            kind: DataFlowEdgeKind::SummaryProjected,
            algorithm: DataFlowAlgorithm::SummaryProjection,
            status: DataFlowStatus::Present,
            precision: DataFlowPrecision::Exact,
            validation: DataFlowValidation::ReferentiallyValidated,
            confidence: DataFlowConfidence::High,
            provenance: DataFlowProvenance::Summary,
            call_site: Some(site),
            call_target: Some(CallTargetId(site.0)),
            refined_call: Some(RefinedCallEdgeId(site.0)),
            model: None,
            budget: None,
            evidence: vec![if call { "call" } else { "return" }.to_string()],
            input_stable_keys: Vec::new(),
            stable_key: crate::internal_core::stable_key_for_test(&format!("edge:{id}")),
        }
    }

    fn local_edge(id: u64, from: u64, to: u64) -> DataFlowEdgeFact {
        DataFlowEdgeFact {
            call_site: None,
            refined_call: None,
            ..boundary_edge(id, from, to, CallSiteId(0), true)
        }
    }
}
