use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::lattice::{Changed, TopReason};
use super::results::DomainResults;
pub use super::results::SolverStatus;
use super::state::ProductState;
use super::transfer::{BranchSense, EdgeTransfer, OperationTransfer, TransferCx};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::LocalAnalysisDb;
use crate::analysis_neutral::cfg::facts::{BasicBlockFact, CfgEdgeKind, CfgNodeFact};
use crate::analysis_neutral::cfg::ids::{BasicBlockId, CfgFunctionId, CfgNodeId};
use crate::analysis_neutral::ids::{CallSiteId, MirBodyId, MirOpId, PlaceId};
use crate::analysis_neutral::ifds::{Icfg, IcfgEdgeKind};
use crate::analysis_neutral::mir_body::MirBody;
use crate::analysis_neutral::mir_op::{BranchNilTest, MirOperation, MirOperationKind, MirValue};
use crate::analysis_neutral::places::PlaceRoot;
use crate::internal_core::{FunctionId, StableKeyId, StableKeyInterner};

#[derive(Debug)]
pub struct SolverInput<'a, H: AnalysisHost = LocalAnalysisDb> {
    db: &'a H,
}

impl<H: AnalysisHost> Clone for SolverInput<'_, H> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<H: AnalysisHost> Copy for SolverInput<'_, H> {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SolverBudget {
    pub max_iterations: u32,
    pub widening_fuel: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SolverPolicy {
    pub budget: SolverBudget,
    pub reduction_rounds: u32,
}

#[derive(Clone, Debug)]
pub struct SolverResult {
    results: DomainResults,
    interner: StableKeyInterner,
}

#[derive(Clone, Debug)]
pub struct IdeDomainSolver {
    policy: SolverPolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SolverOutputMode {
    Full,
    SummaryInputs,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ExplodedPoint {
    node: CfgNodeId,
    call_stack: Vec<CallSiteId>,
}

struct IdeMaterialization {
    block_entries: BTreeMap<BasicBlockId, ProductState>,
    block_exits: BTreeMap<BasicBlockId, ProductState>,
    operation_states: BTreeMap<MirOpId, (BasicBlockId, StableKeyId, ProductState, ProductState)>,
    statuses: BTreeMap<MirBodyId, SolverStatus>,
}

impl<'a, H: AnalysisHost> From<&'a H> for SolverInput<'a, H> {
    fn from(db: &'a H) -> Self {
        Self { db }
    }
}

impl SolverPolicy {
    pub fn deterministic() -> Self {
        Self {
            budget: SolverBudget {
                max_iterations: 10_000,
                widening_fuel: 8,
            },
            reduction_rounds: 4,
        }
    }

    pub fn for_test(budget: SolverBudget) -> Self {
        Self {
            budget,
            reduction_rounds: 4,
        }
    }
}

impl IdeDomainSolver {
    pub fn new(policy: SolverPolicy) -> Self {
        Self { policy }
    }

    pub fn solve<H: AnalysisHost>(&self, input: SolverInput<'_, H>) -> SolverResult {
        self.solve_with_output_mode(input, SolverOutputMode::Full)
    }

    pub fn solve_summary_inputs<H: AnalysisHost>(&self, input: SolverInput<'_, H>) -> SolverResult {
        self.solve_with_output_mode(input, SolverOutputMode::SummaryInputs)
    }

    fn solve_with_output_mode<H: AnalysisHost>(
        &self,
        input: SolverInput<'_, H>,
        output_mode: SolverOutputMode,
    ) -> SolverResult {
        let facts = SolverFactIndex::new(input.db);
        let transfer_cx = TransferCx::from_db(input.db);
        let icfg = Icfg::build(input.db);
        let mut states = BTreeMap::<ExplodedPoint, ProductState>::new();
        let mut queue = VecDeque::new();
        let mut block_entries = BTreeMap::<BasicBlockId, ProductState>::new();
        let mut block_exits = BTreeMap::<BasicBlockId, ProductState>::new();
        let mut operation_states =
            BTreeMap::<MirOpId, (BasicBlockId, StableKeyId, ProductState, ProductState)>::new();
        let mut statuses = facts
            .functions
            .iter()
            .map(|function| (function.body, SolverStatus::Solved))
            .collect::<BTreeMap<_, _>>();
        let mut widening_fuel = self.policy.budget.widening_fuel;
        let mut results = DomainResults::new();

        for function in &facts.functions {
            enqueue(
                &mut states,
                &mut queue,
                ExplodedPoint {
                    node: function.entry_node,
                    call_stack: Vec::new(),
                },
                ProductState::entry(),
            );
        }

        let mut iterations = 0_u32;
        while let Some(point) = queue.pop_front() {
            if iterations >= self.policy.budget.max_iterations {
                mark_ide_budget_exceeded(
                    &facts,
                    &mut states,
                    &mut statuses,
                    &mut results,
                    point.node,
                );
                break;
            }
            iterations += 1;
            let Some(node) = facts.node_by_id.get(&point.node).copied() else {
                continue;
            };
            let Some(function) = facts.function_by_cfg.get(&node.cfg_function).copied() else {
                continue;
            };
            let Some(body) = facts.body_by_id.get(&function.body).copied() else {
                continue;
            };
            let state = states
                .get(&point)
                .cloned()
                .unwrap_or_else(ProductState::bottom);
            let visible_before = facts.visible_state(function.function, &state);
            join_state(&mut block_entries, node.block, &visible_before);

            let operation = node
                .operation
                .and_then(|id| facts.operations_by_id.get(&id).copied());
            let is_resolved_call = operation.is_some_and(|operation| {
                matches!(
                    operation.kind,
                    MirOperationKind::Call { site, .. } if icfg.has_resolved_call(site)
                )
            });
            let mut after = state.clone();
            if let Some(operation) = operation
                && !is_resolved_call
            {
                OperationTransfer::apply(&transfer_cx, operation, &mut after);
                after.reduce_value_only(self.policy.reduction_rounds);
            }
            let visible_after = facts.visible_state(function.function, &after);
            if output_mode.records_operation_states()
                && let Some(operation) = operation
            {
                if is_resolved_call {
                    join_operation_state(
                        &facts.interner,
                        &mut operation_states,
                        node.block,
                        operation,
                        &visible_before,
                        &ProductState::bottom(),
                    );
                } else {
                    join_operation_state(
                        &facts.interner,
                        &mut operation_states,
                        node.block,
                        operation,
                        &visible_before,
                        &visible_after,
                    );
                }
            }
            if facts.last_node_by_block.get(&node.block) == Some(&node.id) {
                join_state(&mut block_exits, node.block, &visible_after);
            }

            for edge in icfg.outgoing(node.id) {
                let mut next_point = ExplodedPoint {
                    node: edge.to,
                    call_stack: point.call_stack.clone(),
                };
                let mut candidate = match edge.kind {
                    IcfgEdgeKind::Intra(kind) => {
                        let mut candidate = after.clone();
                        apply_branch(&transfer_cx, kind, operation, &mut candidate);
                        candidate
                    }
                    IcfgEdgeKind::CallToReturn(site, kind) => {
                        if icfg.has_resolved_call(site) {
                            continue;
                        }
                        let mut candidate = after.clone();
                        apply_branch(&transfer_cx, kind, operation, &mut candidate);
                        candidate
                    }
                    IcfgEdgeKind::Call(site) => {
                        let Some(call) = facts.call_operation_by_site.get(&site).copied() else {
                            continue;
                        };
                        let Some(callee) = facts.function_for_node(edge.to) else {
                            continue;
                        };
                        let mut candidate = state.clone();
                        facts.map_call_arguments(call, callee.function, &mut candidate);
                        next_point.call_stack.push(site);
                        candidate
                    }
                    IcfgEdgeKind::Return(site) => {
                        if next_point.call_stack.pop() != Some(site) {
                            continue;
                        }
                        let Some(call) = facts.call_operation_by_site.get(&site).copied() else {
                            continue;
                        };
                        let Some(caller) = facts.function_for_node(edge.to) else {
                            continue;
                        };
                        let mut candidate = after.clone();
                        facts.map_return(body.id, call, caller.function, &mut candidate);
                        if output_mode.records_operation_states() {
                            let caller_visible = facts.visible_state(caller.function, &candidate);
                            let before_call = facts.visible_state(caller.function, &state);
                            let call_block = facts
                                .node_by_operation
                                .get(&call.id)
                                .map_or(node.block, |node| node.block);
                            join_operation_state(
                                &facts.interner,
                                &mut operation_states,
                                call_block,
                                call,
                                &before_call,
                                &caller_visible,
                            );
                        }
                        candidate
                    }
                };

                let control_kind = match edge.kind {
                    IcfgEdgeKind::Intra(kind) | IcfgEdgeKind::CallToReturn(_, kind) => Some(kind),
                    IcfgEdgeKind::Call(_) | IcfgEdgeKind::Return(_) => None,
                };
                if control_kind == Some(CfgEdgeKind::LoopBack) {
                    if widening_fuel == 0 {
                        statuses.insert(body.id, SolverStatus::Widened);
                        candidate.mark_reachability_top(TopReason::Widened);
                        results.record_top_event(
                            body.id,
                            Some(node.block),
                            None,
                            TopReason::Widened,
                            facts.interner.intern(format!(
                                "body={};block={};reason=widened",
                                body.id.0, node.block.0
                            )),
                        );
                    } else {
                        widening_fuel -= 1;
                    }
                }
                enqueue(&mut states, &mut queue, next_point, candidate);
            }
        }

        materialize_results(
            &facts,
            &states,
            IdeMaterialization {
                block_entries,
                block_exits,
                operation_states,
                statuses,
            },
            output_mode,
            &mut results,
        );
        SolverResult {
            results,
            interner: facts.interner.clone(),
        }
    }
}

impl SolverOutputMode {
    fn records_operation_states(self) -> bool {
        matches!(self, Self::Full)
    }
}

struct SolverFactIndex<'a> {
    interner: crate::internal_core::StableKeyInterner,
    body_by_id: BTreeMap<MirBodyId, &'a MirBody>,
    operations_by_id: BTreeMap<MirOpId, &'a MirOperation>,
    functions: Vec<&'a crate::analysis_neutral::cfg::facts::CfgFunctionFact>,
    function_by_cfg:
        BTreeMap<CfgFunctionId, &'a crate::analysis_neutral::cfg::facts::CfgFunctionFact>,
    node_by_id: BTreeMap<CfgNodeId, &'a CfgNodeFact>,
    node_by_operation: BTreeMap<MirOpId, &'a CfgNodeFact>,
    blocks: Vec<&'a BasicBlockFact>,
    last_node_by_block: BTreeMap<BasicBlockId, CfgNodeId>,
    visible_places_by_function: BTreeMap<FunctionId, BTreeSet<PlaceId>>,
    parameters_by_function: BTreeMap<FunctionId, Vec<PlaceId>>,
    call_operation_by_site: BTreeMap<CallSiteId, &'a MirOperation>,
    return_values_by_body: BTreeMap<MirBodyId, Vec<Option<&'a MirValue>>>,
}

impl<'a> SolverFactIndex<'a> {
    fn new(db: &'a impl AnalysisHost) -> Self {
        let interner = db.stable_key_interner();
        let body_by_id = db
            .mir_bodies()
            .iter()
            .map(|body| (body.id, body))
            .collect::<BTreeMap<_, _>>();
        let operations_by_id = db
            .mir_operations()
            .iter()
            .map(|operation| (operation.id, operation))
            .collect::<BTreeMap<_, _>>();

        let mut functions = db.cfg_functions().iter().collect::<Vec<_>>();
        functions.sort_by(|left, right| {
            let left_body = body_by_id.get(&left.body).map_or_else(String::new, |body| {
                interner.resolve(body.stable_key).to_string()
            });
            let right_body = body_by_id
                .get(&right.body)
                .map_or_else(String::new, |body| {
                    interner.resolve(body.stable_key).to_string()
                });
            left_body
                .as_str()
                .cmp(right_body.as_str())
                .then_with(|| interner.compare_canonical(left.stable_key, right.stable_key))
        });

        let function_by_cfg = functions
            .iter()
            .map(|function| (function.id, *function))
            .collect();
        let mut blocks = db.cfg_blocks().iter().collect::<Vec<_>>();
        blocks.sort_by_key(|block| (block.reverse_postorder, block.id));
        let mut node_by_id = BTreeMap::new();
        let mut node_by_operation = BTreeMap::new();
        let mut last_node_by_block = BTreeMap::new();
        for node in db.cfg_nodes() {
            node_by_id.insert(node.id, node);
            if let Some(operation) = node.operation {
                node_by_operation.insert(operation, node);
            }
            last_node_by_block
                .entry(node.block)
                .and_modify(|current: &mut CfgNodeId| {
                    let current_ordinal = node_by_id[current].operation_ordinal;
                    if current_ordinal <= node.operation_ordinal {
                        *current = node.id;
                    }
                })
                .or_insert(node.id);
        }

        let mut visible_places_by_function = BTreeMap::<FunctionId, BTreeSet<PlaceId>>::new();
        let mut global_places = BTreeSet::new();
        let mut parameters_by_function = BTreeMap::<FunctionId, Vec<(u32, PlaceId)>>::new();
        for place in db.mir_places() {
            if let Some(function) = place.function {
                visible_places_by_function
                    .entry(function)
                    .or_default()
                    .insert(place.id);
            }
            match place.root {
                PlaceRoot::Parameter {
                    function, index, ..
                } => parameters_by_function
                    .entry(function)
                    .or_default()
                    .push((index, place.id)),
                PlaceRoot::Global { .. } => {
                    global_places.insert(place.id);
                }
                _ => {}
            }
        }
        for places in visible_places_by_function.values_mut() {
            places.extend(global_places.iter().copied());
        }
        let parameters_by_function = parameters_by_function
            .into_iter()
            .map(|(function, mut parameters)| {
                parameters.sort();
                (
                    function,
                    parameters.into_iter().map(|(_, place)| place).collect(),
                )
            })
            .collect();
        let call_operation_by_site = db
            .mir_operations()
            .iter()
            .filter_map(|operation| match operation.kind {
                MirOperationKind::Call { site, .. } => Some((site, operation)),
                _ => None,
            })
            .collect();
        let mut return_values_by_body = BTreeMap::<MirBodyId, Vec<Option<&MirValue>>>::new();
        for operation in db.mir_operations() {
            if let MirOperationKind::Return { ref value } = operation.kind {
                return_values_by_body
                    .entry(operation.body)
                    .or_default()
                    .push(value.as_ref());
            }
        }

        Self {
            interner,
            body_by_id,
            operations_by_id,
            functions,
            function_by_cfg,
            node_by_id,
            node_by_operation,
            blocks,
            last_node_by_block,
            visible_places_by_function,
            parameters_by_function,
            call_operation_by_site,
            return_values_by_body,
        }
    }

    fn function_for_node(
        &self,
        node: CfgNodeId,
    ) -> Option<&'a crate::analysis_neutral::cfg::facts::CfgFunctionFact> {
        self.node_by_id
            .get(&node)
            .and_then(|node| self.function_by_cfg.get(&node.cfg_function))
            .copied()
    }

    fn visible_state(&self, function: FunctionId, state: &ProductState) -> ProductState {
        let mut visible = state.clone();
        visible.retain_places(
            self.visible_places_by_function
                .get(&function)
                .unwrap_or(&BTreeSet::new()),
        );
        visible
    }

    fn map_call_arguments(
        &self,
        call: &MirOperation,
        callee: FunctionId,
        state: &mut ProductState,
    ) {
        let MirOperationKind::Call { arguments, .. } = &call.kind else {
            return;
        };
        let source = state.clone();
        for (argument, parameter) in arguments.iter().zip(
            self.parameters_by_function
                .get(&callee)
                .into_iter()
                .flatten(),
        ) {
            state.copy_place_from(&source, *parameter, *argument);
        }
    }

    fn map_return(
        &self,
        callee_body: MirBodyId,
        call: &MirOperation,
        caller: FunctionId,
        state: &mut ProductState,
    ) {
        let MirOperationKind::Call { return_place, .. } = call.kind else {
            return;
        };
        let source = state.clone();
        state.retain_places(
            self.visible_places_by_function
                .get(&caller)
                .unwrap_or(&BTreeSet::new()),
        );
        let values = self
            .return_values_by_body
            .get(&callee_body)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if values.is_empty() {
            state.mark_place_top(return_place, TopReason::UnknownValue);
            return;
        }
        let base = state.clone();
        let mut joined = ProductState::bottom();
        for value in values {
            let mut returned = base.clone();
            match value {
                Some(MirValue::Place(source_place)) => {
                    returned.copy_place_from(&source, return_place, *source_place);
                }
                Some(value) => {
                    OperationTransfer::apply_return_value(&mut returned, return_place, value);
                }
                None => returned.mark_place_top(return_place, TopReason::UnknownValue),
            }
            joined.join_into(&returned);
        }
        *state = joined;
    }
}

impl SolverResult {
    pub fn results(&self) -> &DomainResults {
        &self.results
    }

    pub fn statuses(&self) -> impl Iterator<Item = SolverStatus> + '_ {
        self.results.statuses()
    }

    pub fn has_top_reason(&self, reason: TopReason) -> bool {
        self.results.has_top_reason(reason)
    }

    pub fn stable_digest_parts(&self) -> Vec<String> {
        self.results.stable_digest_parts(&self.interner)
    }
}

fn branch_assumption(
    edge: CfgEdgeKind,
    operation: Option<&MirOperation>,
) -> Option<(Option<PlaceId>, Option<BranchNilTest>, BranchSense)> {
    let sense = match edge {
        CfgEdgeKind::True => BranchSense::True,
        CfgEdgeKind::False => BranchSense::False,
        _ => return None,
    };
    operation.and_then(|operation| {
        if let MirOperationKind::Branch {
            predicate_place,
            nil_test,
            ..
        } = operation.kind
        {
            Some((predicate_place, nil_test, sense))
        } else {
            None
        }
    })
}

fn apply_branch(
    transfer_cx: &TransferCx<'_>,
    kind: CfgEdgeKind,
    operation: Option<&MirOperation>,
    state: &mut ProductState,
) {
    if let Some((predicate, nil_test, sense)) = branch_assumption(kind, operation) {
        EdgeTransfer::apply_branch_assumption(transfer_cx, predicate, nil_test, sense, state);
    }
}

fn enqueue(
    states: &mut BTreeMap<ExplodedPoint, ProductState>,
    queue: &mut VecDeque<ExplodedPoint>,
    point: ExplodedPoint,
    incoming: ProductState,
) {
    let changed = states
        .entry(point.clone())
        .or_insert_with(ProductState::bottom)
        .join_into(&incoming);
    if changed == Changed::Yes {
        queue.push_back(point);
    }
}

fn join_state<K: Ord + Copy>(
    states: &mut BTreeMap<K, ProductState>,
    key: K,
    incoming: &ProductState,
) {
    states
        .entry(key)
        .or_insert_with(ProductState::bottom)
        .join_into(incoming);
}

fn join_operation_state(
    _interner: &crate::internal_core::StableKeyInterner,
    states: &mut BTreeMap<MirOpId, (BasicBlockId, StableKeyId, ProductState, ProductState)>,
    block: BasicBlockId,
    operation: &MirOperation,
    before: &ProductState,
    after: &ProductState,
) {
    let row = states.entry(operation.id).or_insert_with(|| {
        (
            block,
            operation.stable_key,
            ProductState::bottom(),
            ProductState::bottom(),
        )
    });
    row.2.join_into(before);
    row.3.join_into(after);
}

fn mark_ide_budget_exceeded(
    facts: &SolverFactIndex<'_>,
    states: &mut BTreeMap<ExplodedPoint, ProductState>,
    statuses: &mut BTreeMap<MirBodyId, SolverStatus>,
    results: &mut DomainResults,
    node: CfgNodeId,
) {
    for state in states.values_mut() {
        state.mark_reachability_top(TopReason::BudgetExceeded);
    }
    for status in statuses.values_mut() {
        *status = SolverStatus::BudgetExceeded;
    }
    let Some(cfg_node) = facts.node_by_id.get(&node) else {
        return;
    };
    let Some(function) = facts.function_by_cfg.get(&cfg_node.cfg_function) else {
        return;
    };
    results.record_top_event(
        function.body,
        Some(cfg_node.block),
        None,
        TopReason::BudgetExceeded,
        facts.interner.intern(format!(
            "body={};block={};reason=budget_exceeded",
            function.body.0, cfg_node.block.0
        )),
    );
}

fn materialize_results(
    facts: &SolverFactIndex<'_>,
    states: &BTreeMap<ExplodedPoint, ProductState>,
    materialization: IdeMaterialization,
    output_mode: SolverOutputMode,
    results: &mut DomainResults,
) {
    for function in &facts.functions {
        let mut entry = ProductState::bottom();
        for (point, state) in states {
            if point.node == function.entry_node {
                entry.join_into(&facts.visible_state(function.function, state));
            }
        }
        let body_key = facts
            .body_by_id
            .get(&function.body)
            .map_or(function.stable_key, |body| body.stable_key);
        results.insert_function(
            function.body,
            body_key,
            materialization
                .statuses
                .get(&function.body)
                .copied()
                .unwrap_or(SolverStatus::Solved),
            entry,
        );
    }

    for block in &facts.blocks {
        let Some(function) = facts.function_by_cfg.get(&block.cfg_function) else {
            continue;
        };
        results.insert_block_state(
            function.body,
            block.id,
            block.stable_key,
            materialization
                .block_entries
                .get(&block.id)
                .cloned()
                .unwrap_or_else(ProductState::bottom),
            materialization
                .block_exits
                .get(&block.id)
                .cloned()
                .unwrap_or_else(ProductState::bottom),
        );
    }

    if output_mode.records_operation_states() {
        for (operation, (block, stable_key, before, after)) in materialization.operation_states {
            let Some(mir_operation) = facts.operations_by_id.get(&operation) else {
                continue;
            };
            results.insert_operation_state(
                mir_operation.body,
                block,
                operation,
                stable_key,
                before,
                after,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::analysis_api::FunctionFact;
    use crate::analysis_neutral::LocalAnalysisDb;

    use super::*;
    use crate::analysis_neutral::calls::facts::{
        CallAlgorithm, CallCallee, CallEdgeKind, CallPrecision, CallProvenance, CallSiteFact,
        CallSyntaxKind, CallTargetStatus,
    };
    use crate::analysis_neutral::calls::store::CallOutput;
    use crate::analysis_neutral::cfg::facts::{
        BasicBlockFact, BasicBlockKind, CfgEdgeFact, CfgEdgeKind, CfgFunctionFact, CfgNodeFact,
        CfgNodeKind, CfgPrecision, CfgStatus, CfgView,
    };
    use crate::analysis_neutral::cfg::ids::{BasicBlockId, CfgEdgeId, CfgFunctionId, CfgNodeId};
    use crate::analysis_neutral::cfg::store::CfgOutput;
    use crate::analysis_neutral::domains::core::{
        ConstantDomain, ConstantLiteral, ReachabilityDomain,
    };
    use crate::analysis_neutral::domains::lattice::TopReason;
    use crate::analysis_neutral::domains::store::{DomainMaterialization, DomainOutput};
    use crate::analysis_neutral::ids::{MirBodyId, MirOpId, PlaceId, RefinedCallEdgeId};
    use crate::analysis_neutral::mir_body::{MirBody, MirOutput, MirStatus};
    use crate::analysis_neutral::mir_op::{AssignMode, MirOperation, MirOperationKind, MirValue};
    use crate::analysis_neutral::places::{PlaceFact, PlaceRoot, PlaceStatus};
    use crate::analysis_neutral::refined_calls::facts::{
        RefinedCallConfidence, RefinedCallEdgeFact, RefinedCallTier, RefinedCallValidation,
    };
    use crate::analysis_neutral::refined_calls::store::RefinedCallOutput;
    use crate::internal_core::{FileId, FunctionId, Language, Span};

    #[test]
    fn deterministic_shuffled_rows_produce_byte_identical_result_digests() {
        let first = test_fixture::solver_input(false);
        let second = test_fixture::solver_input(true);
        let solver = IdeDomainSolver::new(SolverPolicy::for_test(SolverBudget {
            max_iterations: 16,
            widening_fuel: 2,
        }));

        let first_digest = solver.solve(first).stable_digest_parts();
        let second_digest = solver.solve(second).stable_digest_parts();

        assert_eq!(first_digest, second_digest);
    }

    #[test]
    fn loop_back_edges_consume_widening_fuel_and_record_widening_top() {
        let input = test_fixture::looping_solver_input();
        let solver = IdeDomainSolver::new(SolverPolicy::for_test(SolverBudget {
            max_iterations: 16,
            widening_fuel: 0,
        }));

        let result = solver.solve(input);

        assert!(result.has_top_reason(TopReason::Widened));
    }

    #[test]
    fn budget_exhaustion_marks_function_result_budget_top() {
        let input = test_fixture::looping_solver_input();
        let solver = IdeDomainSolver::new(SolverPolicy::for_test(SolverBudget {
            max_iterations: 1,
            widening_fuel: 4,
        }));

        let result = solver.solve(input);

        assert_eq!(result.statuses().next(), Some(SolverStatus::BudgetExceeded));
        assert!(result.has_top_reason(TopReason::BudgetExceeded));
    }

    #[test]
    fn cursor_after_operation_exposes_transfer_materialized_state() {
        let input = test_fixture::solver_input(false);
        let solver = IdeDomainSolver::new(SolverPolicy::for_test(SolverBudget {
            max_iterations: 16,
            widening_fuel: 2,
        }));

        let result = solver.solve(input);
        let before = result
            .results()
            .before_operation(MirOpId(0))
            .expect("before operation state");
        let after = result
            .results()
            .after_operation(MirOpId(0))
            .expect("after operation state");

        assert!(!before.core.constants.contains_key(&PlaceId(0)));
        assert_eq!(
            after.core.constants[&PlaceId(0)],
            ConstantDomain::from_literal(ConstantLiteral::String("ready".to_string()))
        );
    }

    #[test]
    fn unreachable_blocks_expose_unreachable_without_value_facts() {
        let input = test_fixture::unreachable_solver_input();
        let solver = IdeDomainSolver::new(SolverPolicy::for_test(SolverBudget {
            max_iterations: 16,
            widening_fuel: 2,
        }));

        let result = solver.solve(input);
        let unreachable_entry = result
            .results()
            .block_entry(BasicBlockId(4))
            .expect("unreachable block state");

        assert_eq!(
            unreachable_entry.core.reachability,
            ReachabilityDomain::Unreachable
        );
        assert!(unreachable_entry.observed_places().is_empty());
    }

    #[test]
    fn solving_same_function_twice_returns_identical_stable_result_rows() {
        let input = test_fixture::solver_input(false);
        let solver = IdeDomainSolver::new(SolverPolicy::for_test(SolverBudget {
            max_iterations: 16,
            widening_fuel: 2,
        }));

        let first = solver.solve(input).stable_digest_parts();
        let second = solver.solve(input).stable_digest_parts();

        assert_eq!(first, second);
    }

    #[test]
    fn summary_input_solver_skips_operation_states_without_changing_output_rows() {
        let input = test_fixture::solver_input(false);
        let solver = IdeDomainSolver::new(SolverPolicy::for_test(SolverBudget {
            max_iterations: 16,
            widening_fuel: 2,
        }));

        let full = solver.solve(input);
        let summary = solver.solve_summary_inputs(input);
        let full_output = DomainOutput::from_results_with_materialization(
            &LocalAnalysisDb::new().stable_key_interner(),
            full.results(),
            None,
            DomainMaterialization::SummaryInputs,
        );
        let summary_output = DomainOutput::from_results_with_materialization(
            &LocalAnalysisDb::new().stable_key_interner(),
            summary.results(),
            None,
            DomainMaterialization::SummaryInputs,
        );

        assert_eq!(full_output, summary_output);
        assert!(summary.results().before_operation(MirOpId(0)).is_none());
    }

    #[test]
    fn ide_solver_maps_constant_and_nilness_through_matched_call_and_return() {
        let input = test_fixture::interprocedural_solver_input();
        let solver = IdeDomainSolver::new(SolverPolicy::for_test(SolverBudget {
            max_iterations: 64,
            widening_fuel: 2,
        }));

        let result = solver.solve(input);
        let after_call = result
            .results()
            .after_operation(MirOpId(1))
            .expect("matched return should materialize the call result");

        assert_eq!(
            *after_call
                .core
                .constants
                .get(&PlaceId(1))
                .unwrap_or_else(|| {
                    panic!(
                        "missing return constant in {after_call:?}; callee={:?}",
                        result.results().after_operation(MirOpId(2))
                    )
                }),
            ConstantDomain::from_literal(ConstantLiteral::String("ready".to_string()))
        );
        assert_eq!(
            after_call.core.nilness.get(&PlaceId(1)),
            Some(&crate::analysis_neutral::domains::core::NilnessDomain::NonNil)
        );
    }

    mod test_fixture {
        use super::*;

        pub(super) fn solver_input(shuffled: bool) -> SolverInput<'static, LocalAnalysisDb> {
            let mut db = LocalAnalysisDb::new();
            let interner = db.stable_key_interner();
            db.replace_semantic_mir(mir_output(&interner, shuffled))
                .expect("semantic MIR should store");
            db.replace_cfg_facts(cfg_output(&interner, shuffled, false))
                .expect("CFG should store");
            let db = Box::leak(Box::new(db));
            (&*db).into()
        }

        pub(super) fn looping_solver_input() -> SolverInput<'static, LocalAnalysisDb> {
            let mut db = LocalAnalysisDb::new();
            let interner = db.stable_key_interner();
            db.replace_semantic_mir(mir_output(&interner, false))
                .expect("semantic MIR should store");
            db.replace_cfg_facts(cfg_output(&interner, false, true))
                .expect("CFG should store");
            let db = Box::leak(Box::new(db));
            (&*db).into()
        }

        pub(super) fn unreachable_solver_input() -> SolverInput<'static, LocalAnalysisDb> {
            let mut db = LocalAnalysisDb::new();
            let interner = db.stable_key_interner();
            db.replace_semantic_mir(mir_output(&interner, false))
                .expect("semantic MIR should store");
            let mut cfg = cfg_output(&interner, false, false);
            cfg.blocks.push(BasicBlockFact {
                id: BasicBlockId(4),
                cfg_function: CfgFunctionId(1),
                kind: BasicBlockKind::Unreachable,
                first_node: Some(CfgNodeId(4)),
                last_node: Some(CfgNodeId(4)),
                reachable: false,
                reverse_postorder: 3,
                stable_key: interner.intern("block:unreachable"),
                status: CfgStatus::Resolved,
                precision: CfgPrecision::ExactLowered,
            });
            cfg.nodes
                .push(node(&interner, 4, 4, None, 3, "node:unreachable"));
            db.replace_cfg_facts(cfg).expect("CFG should store");
            let db = Box::leak(Box::new(db));
            (&*db).into()
        }

        pub(super) fn interprocedural_solver_input() -> SolverInput<'static, LocalAnalysisDb> {
            let caller = FunctionId::from_raw(1);
            let callee = FunctionId::from_raw(2);
            let argument = PlaceId(0);
            let call_result = PlaceId(1);
            let parameter = PlaceId(2);
            let site = CallSiteId(1);
            let mut db = LocalAnalysisDb::new();
            db.add_file("padding.go".into(), "padding.go".to_string(), String::new());
            let file = db.add_file("app.go".into(), "app.go".to_string(), String::new());
            for (name, is_exported) in [("padding", false), ("caller", true), ("callee", true)] {
                db.push_function(FunctionFact::new(
                    FunctionId::from_raw(0),
                    file,
                    name.to_string(),
                    Span::point(file, 1, 1),
                    Language::Go,
                    is_exported,
                    true,
                    1,
                    Vec::new(),
                ));
            }
            let interner = db.stable_key_interner();
            db.replace_semantic_mir(MirOutput {
                bodies: vec![
                    mir_body(&interner, MirBodyId(0), caller, "body:0:caller"),
                    mir_body(&interner, MirBodyId(1), callee, "body:1:callee"),
                ],
                places: vec![
                    local_place(&interner, argument, caller, "argument", "place:0:argument"),
                    local_place(&interner, call_result, caller, "result", "place:1:result"),
                    PlaceFact {
                        id: parameter,
                        language: Language::Go,
                        file: Some(FileId::from_raw(1)),
                        function: Some(callee),
                        root: PlaceRoot::Parameter {
                            function: callee,
                            index: 0,
                            name: Some("value".to_string()),
                        },
                        projections: Vec::new(),
                        stable_key: interner.intern("place:2:callee:value".to_string()),
                        status: PlaceStatus::Resolved,
                    },
                ],
                operations: vec![
                    mir_operation(
                        &interner,
                        MirOpId(0),
                        MirBodyId(0),
                        MirOperationKind::Assign {
                            place: argument,
                            value: MirValue::Literal {
                                value: "\"ready\"".to_string(),
                            },
                            mode: AssignMode::Overwrite,
                        },
                        "op:0:caller:assign",
                    ),
                    mir_operation(
                        &interner,
                        MirOpId(1),
                        MirBodyId(0),
                        MirOperationKind::Call {
                            site,
                            callee: MirValue::Unknown {
                                evidence: "callee".to_string(),
                            },
                            arguments: vec![argument],
                            return_place: call_result,
                        },
                        "op:1:caller:call",
                    ),
                    mir_operation(
                        &interner,
                        MirOpId(2),
                        MirBodyId(1),
                        MirOperationKind::Return {
                            value: Some(MirValue::Place(parameter)),
                        },
                        "op:2:callee:return",
                    ),
                ],
                unsupported: Vec::new(),
                ..MirOutput::default()
            })
            .expect("semantic MIR should store");
            db.replace_cfg_facts(interprocedural_cfg(&interner, caller, callee))
                .expect("CFG should store");
            db.replace_call_facts(CallOutput {
                sites: vec![call_site(site, caller)],
                targets: Vec::new(),
                unresolved: Vec::new(),
            })
            .expect("call facts should store");
            db.replace_refined_call_facts(RefinedCallOutput {
                edges: vec![refined_call(site, caller, callee)],
            })
            .expect("refined calls should store");
            let db = Box::leak(Box::new(db));
            (&*db).into()
        }

        fn mir_body(
            interner: &crate::internal_core::StableKeyInterner,
            id: MirBodyId,
            function: FunctionId,
            stable_key: &str,
        ) -> MirBody {
            MirBody {
                id,
                language: Language::Go,
                file: FileId::from_raw(1),
                function,
                package: None,
                module: None,
                owner_stable_key: interner.intern(format!("owner:{}", function.0)),
                span: span(),
                stable_key: interner.intern(stable_key.to_string()),
                status: MirStatus::Resolved,
            }
        }

        fn local_place(
            interner: &crate::internal_core::StableKeyInterner,
            id: PlaceId,
            function: FunctionId,
            name: &str,
            stable_key: &str,
        ) -> PlaceFact {
            PlaceFact {
                id,
                language: Language::Go,
                file: Some(FileId::from_raw(1)),
                function: Some(function),
                root: PlaceRoot::Local {
                    function,
                    name: name.to_string(),
                },
                projections: Vec::new(),
                stable_key: interner.intern(stable_key.to_string()),
                status: PlaceStatus::Resolved,
            }
        }

        fn mir_operation(
            interner: &crate::internal_core::StableKeyInterner,
            id: MirOpId,
            body: MirBodyId,
            kind: MirOperationKind,
            stable_key: &str,
        ) -> MirOperation {
            MirOperation {
                id,
                body,
                ordinal: id.0 as u32,
                span: span(),
                kind,
                stable_key: interner.intern(stable_key.to_string()),
                status: MirStatus::Resolved,
            }
        }

        fn interprocedural_cfg(
            interner: &crate::internal_core::StableKeyInterner,
            caller: FunctionId,
            callee: FunctionId,
        ) -> CfgOutput {
            let functions = vec![
                cfg_function_for(interner, 1, 0, caller, 1, 4, "cfg:caller"),
                cfg_function_for(interner, 2, 1, callee, 10, 12, "cfg:callee"),
            ];
            let blocks = vec![
                cfg_block(interner, 1, 1, BasicBlockKind::Entry, 0),
                cfg_block(interner, 2, 1, BasicBlockKind::StraightLine, 1),
                cfg_block(interner, 3, 1, BasicBlockKind::StraightLine, 2),
                cfg_block(interner, 4, 1, BasicBlockKind::ExitNormal, 3),
                cfg_block(interner, 10, 2, BasicBlockKind::Entry, 0),
                cfg_block(interner, 11, 2, BasicBlockKind::StraightLine, 1),
                cfg_block(interner, 12, 2, BasicBlockKind::ExitNormal, 2),
            ];
            let nodes = vec![
                cfg_node_for(interner, 1, 1, 1, 0, None),
                cfg_node_for(interner, 2, 1, 2, 1, Some(MirOpId(0))),
                cfg_node_for(interner, 3, 1, 3, 2, Some(MirOpId(1))),
                cfg_node_for(interner, 4, 1, 4, 3, None),
                cfg_node_for(interner, 10, 2, 10, 0, None),
                cfg_node_for(interner, 11, 2, 11, 1, Some(MirOpId(2))),
                cfg_node_for(interner, 12, 2, 12, 2, None),
            ];
            let edges = vec![
                cfg_edge_for(interner, 1, 1, 1, 2),
                cfg_edge_for(interner, 2, 1, 2, 3),
                cfg_edge_for(interner, 3, 1, 3, 4),
                cfg_edge_for(interner, 4, 2, 10, 11),
                cfg_edge_for(interner, 5, 2, 11, 12),
            ];
            CfgOutput {
                functions,
                blocks,
                nodes,
                edges,
                ..CfgOutput::empty()
            }
        }

        fn cfg_function_for(
            interner: &crate::internal_core::StableKeyInterner,
            id: u64,
            body: u64,
            function: FunctionId,
            entry: u64,
            exit: u64,
            stable_key: &str,
        ) -> CfgFunctionFact {
            CfgFunctionFact {
                id: CfgFunctionId(id),
                body: MirBodyId(body),
                function,
                language: Language::Go,
                file: FileId::from_raw(1),
                span: span(),
                entry_node: CfgNodeId(entry),
                normal_exit_node: CfgNodeId(exit),
                exceptional_exit_node: None,
                stable_key: interner.intern(stable_key),
                status: CfgStatus::Resolved,
                precision: CfgPrecision::ExactLowered,
            }
        }

        fn cfg_block(
            interner: &crate::internal_core::StableKeyInterner,
            id: u64,
            function: u64,
            kind: BasicBlockKind,
            reverse_postorder: u32,
        ) -> BasicBlockFact {
            BasicBlockFact {
                id: BasicBlockId(id),
                cfg_function: CfgFunctionId(function),
                kind,
                first_node: Some(CfgNodeId(id)),
                last_node: Some(CfgNodeId(id)),
                reachable: true,
                reverse_postorder,
                stable_key: interner.intern(format!("block:{function}:{id}")),
                status: CfgStatus::Resolved,
                precision: CfgPrecision::ExactLowered,
            }
        }

        fn cfg_node_for(
            interner: &crate::internal_core::StableKeyInterner,
            id: u64,
            function: u64,
            block: u64,
            ordinal: u32,
            operation: Option<MirOpId>,
        ) -> CfgNodeFact {
            CfgNodeFact {
                id: CfgNodeId(id),
                cfg_function: CfgFunctionId(function),
                body: MirBodyId(function - 1),
                operation,
                block: BasicBlockId(block),
                kind: if operation.is_some() {
                    CfgNodeKind::Operation
                } else {
                    CfgNodeKind::Synthetic
                },
                span: Some(span()),
                generated: operation.is_none(),
                operation_ordinal: ordinal,
                stable_key: interner.intern(format!("node:{function}:{id}")),
                status: CfgStatus::Resolved,
                precision: CfgPrecision::ExactLowered,
            }
        }

        fn cfg_edge_for(
            interner: &crate::internal_core::StableKeyInterner,
            id: u64,
            function: u64,
            from: u64,
            to: u64,
        ) -> CfgEdgeFact {
            CfgEdgeFact {
                id: CfgEdgeId(id),
                cfg_function: CfgFunctionId(function),
                view: CfgView::NormalControl,
                from: CfgNodeId(from),
                to: CfgNodeId(to),
                from_block: BasicBlockId(from),
                to_block: BasicBlockId(to),
                kind: CfgEdgeKind::Normal,
                label: None,
                stable_key: interner.intern(format!("edge:{function}:{from}:{to}")),
                status: CfgStatus::Resolved,
                precision: CfgPrecision::ExactLowered,
            }
        }

        fn call_site(site: CallSiteId, caller: FunctionId) -> CallSiteFact {
            CallSiteFact {
                id: site,
                language: Language::Go,
                file: FileId::from_raw(1),
                caller,
                owner_symbol: None,
                body: MirBodyId(0),
                operation: MirOpId(1),
                span: span(),
                kind: CallSyntaxKind::Function,
                callee: CallCallee::Identifier {
                    reference: None,
                    name: "callee".to_string(),
                },
                receiver: None,
                arguments: vec![PlaceId(0)],
                result: Some(PlaceId(1)),
                status: CallTargetStatus::Resolved,
                precision: CallPrecision::Exact,
                in_throw: false,
                stable_key: crate::internal_core::StableKeyId(0),
            }
        }

        fn refined_call(
            site: CallSiteId,
            caller: FunctionId,
            callee: FunctionId,
        ) -> RefinedCallEdgeFact {
            RefinedCallEdgeFact {
                id: RefinedCallEdgeId(1),
                site,
                base_target: None,
                caller,
                target_function: Some(callee),
                target_symbol: None,
                synthetic_target: None,
                language: Language::Go,
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
                stable_key: crate::internal_core::stable_key_for_test("refined:caller:callee"),
            }
        }

        fn span() -> Span {
            Span::new(FileId::from_raw(1), 1, 2, 1, 1, 1, 2)
        }

        fn mir_output(
            interner: &crate::internal_core::StableKeyInterner,
            shuffled: bool,
        ) -> MirOutput {
            let body = MirBody {
                id: MirBodyId(0),
                language: Language::Go,
                file: FileId::from_raw(1),
                function: FunctionId::from_raw(1),
                package: None,
                module: None,
                owner_stable_key: interner.intern("owner:test".to_string()),
                span: span(),
                stable_key: interner.intern("body:test".to_string()),
                status: MirStatus::Resolved,
            };
            let place = PlaceFact {
                id: PlaceId(0),
                language: Language::Go,
                file: Some(FileId::from_raw(1)),
                function: Some(FunctionId::from_raw(1)),
                root: PlaceRoot::Local {
                    function: FunctionId::from_raw(1),
                    name: "value".to_string(),
                },
                projections: Vec::new(),
                stable_key: interner.intern("place:value".to_string()),
                status: PlaceStatus::Resolved,
            };
            let op = MirOperation {
                id: MirOpId(0),
                body: MirBodyId(0),
                ordinal: 1,
                span: span(),
                kind: MirOperationKind::Assign {
                    place: PlaceId(0),
                    value: MirValue::Literal {
                        value: "\"ready\"".to_string(),
                    },
                    mode: AssignMode::Overwrite,
                },
                stable_key: interner.intern("op:assign".to_string()),
                status: MirStatus::Resolved,
            };
            let mut operations = vec![op];
            if shuffled {
                operations.reverse();
            }
            MirOutput {
                bodies: vec![body],
                places: vec![place],
                operations,
                unsupported: Vec::new(),
                ..MirOutput::default()
            }
        }

        fn cfg_output(
            interner: &crate::internal_core::StableKeyInterner,
            shuffled: bool,
            loop_back: bool,
        ) -> CfgOutput {
            let function = CfgFunctionFact {
                id: CfgFunctionId(1),
                body: MirBodyId(0),
                function: FunctionId::from_raw(1),
                language: Language::Go,
                file: FileId::from_raw(1),
                span: span(),
                entry_node: CfgNodeId(1),
                normal_exit_node: CfgNodeId(3),
                exceptional_exit_node: None,
                stable_key: interner.intern("cfg:function:test"),
                status: CfgStatus::Resolved,
                precision: CfgPrecision::ExactLowered,
            };
            let entry = block(interner, 1, BasicBlockKind::Entry, 0, "block:entry");
            let body = block(interner, 2, BasicBlockKind::LoopHeader, 1, "block:body");
            let exit = block(interner, 3, BasicBlockKind::ExitNormal, 2, "block:exit");
            let nodes = vec![
                node(interner, 1, 1, None, 0, "node:entry"),
                node(interner, 2, 2, Some(MirOpId(0)), 1, "node:op"),
                node(interner, 3, 3, None, u32::MAX - 1, "node:exit"),
            ];
            let mut edges = vec![
                edge(interner, 1, 1, 2, CfgEdgeKind::Normal, "edge:entry-body"),
                edge(interner, 2, 2, 3, CfgEdgeKind::Return, "edge:body-exit"),
            ];
            if loop_back {
                edges.push(edge(
                    interner,
                    3,
                    2,
                    2,
                    CfgEdgeKind::LoopBack,
                    "edge:loop-back",
                ));
            }
            if shuffled {
                edges.reverse();
            }
            CfgOutput {
                functions: vec![function],
                nodes,
                blocks: vec![exit, body, entry],
                edges,
                ..CfgOutput::empty()
            }
        }

        fn block(
            interner: &crate::internal_core::StableKeyInterner,
            id: u64,
            kind: BasicBlockKind,
            reverse_postorder: u32,
            stable_key: &str,
        ) -> BasicBlockFact {
            BasicBlockFact {
                id: BasicBlockId(id),
                cfg_function: CfgFunctionId(1),
                kind,
                first_node: Some(CfgNodeId(id)),
                last_node: Some(CfgNodeId(id)),
                reachable: true,
                reverse_postorder,
                stable_key: interner.intern(stable_key),
                status: CfgStatus::Resolved,
                precision: CfgPrecision::ExactLowered,
            }
        }

        fn node(
            interner: &crate::internal_core::StableKeyInterner,
            id: u64,
            block: u64,
            operation: Option<MirOpId>,
            ordinal: u32,
            stable_key: &str,
        ) -> CfgNodeFact {
            CfgNodeFact {
                id: CfgNodeId(id),
                cfg_function: CfgFunctionId(1),
                body: MirBodyId(0),
                operation,
                block: BasicBlockId(block),
                kind: if operation.is_some() {
                    CfgNodeKind::Operation
                } else {
                    CfgNodeKind::Synthetic
                },
                span: Some(span()),
                generated: operation.is_none(),
                operation_ordinal: ordinal,
                stable_key: interner.intern(stable_key),
                status: CfgStatus::Resolved,
                precision: CfgPrecision::ExactLowered,
            }
        }

        fn edge(
            interner: &crate::internal_core::StableKeyInterner,
            id: u64,
            from_block: u64,
            to_block: u64,
            kind: CfgEdgeKind,
            stable_key: &str,
        ) -> CfgEdgeFact {
            CfgEdgeFact {
                id: CfgEdgeId(id),
                cfg_function: CfgFunctionId(1),
                view: CfgView::NormalControl,
                from: CfgNodeId(from_block),
                to: CfgNodeId(to_block),
                from_block: BasicBlockId(from_block),
                to_block: BasicBlockId(to_block),
                kind,
                label: None,
                stable_key: interner.intern(stable_key),
                status: CfgStatus::Resolved,
                precision: CfgPrecision::ExactLowered,
            }
        }
    }
}
