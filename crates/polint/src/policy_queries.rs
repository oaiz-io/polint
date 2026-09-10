use crate::analysis::calls::facts::{CallCallee, CallPrecision, CallSiteFact, CallTargetStatus};
use crate::analysis::cfg::ids::BasicBlockId;
use crate::analysis::data_flow::facts::{
    DataFlowConfidence, DataFlowEdgeFact, DataFlowEdgeKind, DataFlowNodeFact, DataFlowNodeKind,
    DataFlowPrecision,
};
use crate::analysis::data_flow::store::DataFlowStore;
use crate::analysis::ids::{CallSiteId, DataFlowNodeId, MirBodyId, MirOpId, PlaceId};
use crate::analysis::ifds::{
    DataFlowPath, DataFlowPathStatus, DataFlowSearchBudget, find_taint_paths,
};
use crate::analysis::places::{PlaceFact, PlaceProjection, PlaceRoot};
use crate::analysis::reachability::facts::{ReachabilityRootFact, RootKind};
use crate::analysis::refined_calls::facts::{RefinedCallConfidence, RefinedCallEdgeFact};
use crate::core::{AnalysisDb, FileId, FunctionFact, FunctionId};
use crate::sdk::policy::{
    BarrierPattern, BarrierPatternKind, EventPattern, EventPatternKind, FlowQuery, GuardPattern,
    GuardPatternKind, GuardQuery, LifecycleQuery, PolicyConfidence, PolicyEvidenceEdgeKind,
    PolicyOperation, PolicyPrecision, PolicyStatus, PolicyViolation, ReachQuery, SinkPattern,
    SinkPatternKind, SourcePattern, SourcePatternKind,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub(crate) fn matching_events(db: &AnalysisDb, query: EventPattern) -> Vec<PolicyViolation> {
    let query_digest = query.query_digest();
    normalize_policy_results(match query.kind() {
        EventPatternKind::Call => matching_call_events(db, &query, &query_digest),
        EventPatternKind::WriteField => Vec::new(),
    })
}

pub(crate) fn forbidden_reachable(db: &AnalysisDb, query: ReachQuery) -> Vec<PolicyViolation> {
    let query_digest = query.query_digest();
    normalize_policy_results(forbidden_reachable_calls(db, &query, &query_digest))
}

pub(crate) fn missing_guards(db: &AnalysisDb, query: GuardQuery) -> Vec<PolicyViolation> {
    let query_digest = query.query_digest();
    normalize_policy_results(missing_guard_calls(db, &query, &query_digest))
}

pub(crate) fn missing_cleanup(db: &AnalysisDb, query: LifecycleQuery) -> Vec<PolicyViolation> {
    let query_digest = query.query_digest();
    normalize_policy_results(missing_cleanup_calls(db, &query, &query_digest))
}

pub(crate) fn forbidden_flows(db: &AnalysisDb, query: FlowQuery) -> Vec<PolicyViolation> {
    let query_digest = query.query_digest();
    normalize_policy_results(forbidden_data_flows(db, &query, &query_digest))
}

fn normalize_policy_results(mut results: Vec<PolicyViolation>) -> Vec<PolicyViolation> {
    results.sort_by_key(PolicyViolation::stable_key);
    results.dedup_by(|left, right| left.stable_key() == right.stable_key());
    results
}

fn forbidden_data_flows(
    db: &AnalysisDb,
    query: &FlowQuery,
    query_digest: &str,
) -> Vec<PolicyViolation> {
    if query.max_depth == 0 || query.max_paths == 0 {
        return Vec::new();
    }

    let Some(store) = db.data_flow_store() else {
        return Vec::new();
    };

    let site_by_id = call_sites_by_id(db);
    let sanitizer_sites = matching_sanitizer_sites(db, &site_by_id, &query.barriers);
    let sources = matching_flow_sources(db, store, &query.source);
    let sinks = matching_flow_sinks(db, store, &query.sink, &site_by_id);
    if sources.is_empty() || sinks.is_empty() {
        return Vec::new();
    }

    let budget = DataFlowSearchBudget {
        max_depth: query.max_depth,
        max_paths: query.max_paths,
    };
    let mut results: Vec<PolicyViolation> = Vec::new();
    let mut emitted = BTreeSet::new();

    'sources: for source in &sources {
        for sink in &sinks {
            let paths =
                find_taint_paths(db, store, source.node, sink.node, &sanitizer_sites, budget);
            for path in paths {
                match path.status {
                    DataFlowPathStatus::Found => {
                        let key = flow_result_key(source, sink, &path);
                        if emitted.insert(key) {
                            let violation =
                                flow_violation(db, store, source, sink, &path, query, query_digest);
                            if !data_flow_violation_meets_minimum(
                                &violation,
                                query.minimum_precision,
                            ) {
                                continue;
                            }
                            if !push_capped_data_flow_violation(
                                &mut results,
                                violation,
                                query.max_paths,
                            ) {
                                break 'sources;
                            }
                        }
                    }
                    DataFlowPathStatus::Unknown | DataFlowPathStatus::BudgetExceeded => {
                        let key = flow_result_key(source, sink, &path);
                        if emitted.insert(key) {
                            let violation =
                                flow_violation(db, store, source, sink, &path, query, query_digest);
                            if !data_flow_violation_meets_minimum(
                                &violation,
                                query.minimum_precision,
                            ) {
                                continue;
                            }
                            if !push_capped_data_flow_violation(
                                &mut results,
                                violation,
                                query.max_paths,
                            ) {
                                break 'sources;
                            }
                        }
                    }
                    DataFlowPathStatus::NotFound => {}
                }
            }
        }
    }

    results
}

fn matching_call_events(
    db: &AnalysisDb,
    pattern: &EventPattern,
    query_digest: &str,
) -> Vec<PolicyViolation> {
    let site_by_id = call_sites_by_id(db);
    if !db.refined_call_edges().is_empty() {
        return db
            .refined_call_edges()
            .iter()
            .filter_map(|edge| {
                if call_pattern_matches_edge(db, pattern, edge, &site_by_id) {
                    Some(violation_for_edge(
                        db,
                        edge,
                        &site_by_id,
                        PolicyOperation::EventsMatching,
                        query_digest,
                        vec![
                            ("event", "call".to_string()),
                            ("target", best_call_target_label(db, edge, &site_by_id)),
                        ],
                    ))
                } else {
                    None
                }
            })
            .collect();
    }

    db.call_sites()
        .iter()
        .filter_map(|site| {
            if call_pattern_matches_site(pattern, site) {
                Some(PolicyViolation::new(
                    PolicyOperation::EventsMatching,
                    query_digest,
                    db.path_for(site.file),
                    site.span.diagnostic_range(),
                    policy_status_from_call_status_and_precision(site.status, site.precision),
                    policy_precision_from_call_precision(site.precision),
                    vec![
                        ("event".to_string(), "call".to_string()),
                        ("target".to_string(), call_site_label(site)),
                    ],
                ))
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .into_iter()
        .chain(matching_function_call_events(db, pattern, query_digest))
        .collect()
}

fn matching_function_call_events(
    db: &AnalysisDb,
    pattern: &EventPattern,
    query_digest: &str,
) -> Vec<PolicyViolation> {
    if !db.call_sites().is_empty() {
        return Vec::new();
    }

    db.functions()
        .iter()
        .filter_map(|function| {
            let target = function_call_match(function, pattern)?;
            Some(PolicyViolation::new(
                PolicyOperation::EventsMatching,
                query_digest,
                db.path_for(function.file),
                function.span.diagnostic_range(),
                PolicyStatus::Heuristic,
                PolicyPrecision::Syntax,
                vec![
                    ("event".to_string(), "call".to_string()),
                    ("target".to_string(), target.to_string()),
                    ("function".to_string(), function.name.clone()),
                    (
                        "event_source".to_string(),
                        "syntax_function_calls".to_string(),
                    ),
                ],
            ))
        })
        .collect()
}

fn forbidden_reachable_calls(
    db: &AnalysisDb,
    query: &ReachQuery,
    query_digest: &str,
) -> Vec<PolicyViolation> {
    if query.max_depth == 0 || query.max_paths == 0 {
        return Vec::new();
    }

    let site_by_id = call_sites_by_id(db);
    let roots = selected_reachability_roots(db, query);
    let adjacency = refined_adjacency(db, query, &site_by_id);
    let mut results = Vec::new();
    let mut truncated = false;

    'roots: for root in roots {
        let root_label = root_label(db, root);
        let mut queue = VecDeque::new();
        let mut visited = BTreeSet::new();
        queue.push_back(SearchState {
            function: root.target_function,
            depth: 0,
            path: vec![root_label.clone()],
        });
        visited.insert(root.target_function);

        while let Some(state) = queue.pop_front() {
            let Some(edges) = adjacency.get(&state.function) else {
                continue;
            };

            for edge in edges {
                let edge_depth = state.depth + 1;
                if edge_depth > query.max_depth {
                    truncated = true;
                    continue;
                }

                let target_label = best_call_target_label(db, edge, &site_by_id);
                let mut path = state.path.clone();
                path.push(target_label.clone());

                if call_pattern_matches_edge(db, &query.target, edge, &site_by_id) {
                    if results.len() >= query.max_paths {
                        truncated = true;
                        break 'roots;
                    }
                    results.push(
                        violation_for_edge(
                            db,
                            edge,
                            &site_by_id,
                            PolicyOperation::CallsForbiddenReachable,
                            query_digest,
                            vec![
                                ("root", root_label.clone()),
                                ("path", path.join(" -> ")),
                                ("target", target_label),
                                ("depth", edge_depth.to_string()),
                            ],
                        )
                        .with_path_evidence(path.clone(), PolicyEvidenceEdgeKind::Call),
                    );
                }

                if traversable_status(edge.status)
                    && let Some(target_function) = edge.target_function
                    && visited.insert(target_function)
                {
                    queue.push_back(SearchState {
                        function: target_function,
                        depth: edge_depth,
                        path,
                    });
                }
            }
        }
    }

    if truncated && let Some(last) = results.pop() {
        results.push(last.with_budget_exceeded("reach_query_budget"));
    }

    results
}

fn missing_guard_calls(
    db: &AnalysisDb,
    query: &GuardQuery,
    query_digest: &str,
) -> Vec<PolicyViolation> {
    if query.max_depth == 0
        || query.max_paths == 0
        || query.event.kind() != EventPatternKind::Call
        || query.guard.kind() != GuardPatternKind::CallAny
    {
        return Vec::new();
    }

    let events = ordered_control_call_events(db, query.minimum_precision);
    let by_function = control_events_by_function(&events);
    let dominators = BlockRelation::dominators(db);
    let mut results = Vec::new();
    let mut truncated = false;

    'functions: for function_events in by_function.values() {
        for (index, event) in function_events.iter().enumerate() {
            if !event_matches_pattern(event, &query.event) {
                continue;
            }
            let coverage = relation_coverage(
                function_events[..index]
                    .iter()
                    .filter(|candidate| guard_matches_event(candidate, &query.guard)),
                event,
                &dominators,
            );
            match coverage {
                RelationCoverage::Established => continue,
                RelationCoverage::NotEstablished(_) if !query.report_unknown_coverage => continue,
                RelationCoverage::NotEstablished(_) | RelationCoverage::Refuted { .. } => {}
            }
            if results.len() >= query.max_paths {
                truncated = true;
                break 'functions;
            }
            match coverage {
                RelationCoverage::NotEstablished(reason) => results.push(unknown_control_result(
                    db,
                    event,
                    PolicyOperation::ControlFlowMissingGuard,
                    query_digest,
                    vec![
                        ("policy", "missing_guard".to_string()),
                        ("required_guard", query.guard.values().join(",")),
                        ("dominance_evidence", reason.to_string()),
                        ("requested_max_depth", query.max_depth.to_string()),
                    ],
                )),
                _ => results.push(
                    control_violation(
                        db,
                        event,
                        PolicyOperation::ControlFlowMissingGuard,
                        query_digest,
                        vec![
                            ("policy", "missing_guard".to_string()),
                            ("required_guard", query.guard.values().join(",")),
                            ("uncovered_path", format!("entry -> {}", event.target_label)),
                            (
                                "dominance_evidence",
                                coverage
                                    .refuted_evidence(DOMINANCE_NO_CANDIDATE)
                                    .to_string(),
                            ),
                            ("requested_max_depth", query.max_depth.to_string()),
                        ],
                    )
                    .with_path_evidence(
                        ["entry".to_string(), event.target_label.clone()],
                        PolicyEvidenceEdgeKind::Control,
                    ),
                ),
            }
        }
    }

    if truncated && let Some(last) = results.pop() {
        results.push(last.with_budget_exceeded("control_flow_query_budget"));
    }

    results
}

fn missing_cleanup_calls(
    db: &AnalysisDb,
    query: &LifecycleQuery,
    query_digest: &str,
) -> Vec<PolicyViolation> {
    if query.max_depth == 0
        || query.max_paths == 0
        || query.start.kind() != EventPatternKind::Call
        || query.cleanup.kind() != EventPatternKind::Call
    {
        return Vec::new();
    }

    let events = ordered_control_call_events(db, query.minimum_precision);
    let by_function = control_events_by_function(&events);
    let postdominators = BlockRelation::postdominators(db);
    let mut results = Vec::new();
    let mut truncated = false;

    'functions: for function_events in by_function.values() {
        for (index, event) in function_events.iter().enumerate() {
            if !event_matches_pattern(event, &query.start) {
                continue;
            }
            let coverage = relation_coverage(
                function_events[index + 1..]
                    .iter()
                    .filter(|candidate| event_matches_pattern(candidate, &query.cleanup)),
                event,
                &postdominators,
            );
            match coverage {
                RelationCoverage::Established => continue,
                RelationCoverage::NotEstablished(_) if !query.report_unknown_coverage => continue,
                RelationCoverage::NotEstablished(_) | RelationCoverage::Refuted { .. } => {}
            }
            if results.len() >= query.max_paths {
                truncated = true;
                break 'functions;
            }
            match coverage {
                RelationCoverage::NotEstablished(reason) => results.push(unknown_control_result(
                    db,
                    event,
                    PolicyOperation::ControlFlowMissingCleanup,
                    query_digest,
                    vec![
                        ("policy", "missing_cleanup".to_string()),
                        ("required_cleanup", query.cleanup.values().join(",")),
                        ("dominance_evidence", reason.to_string()),
                        (
                            "require_error_cleanup",
                            query.require_error_cleanup.to_string(),
                        ),
                        ("requested_max_depth", query.max_depth.to_string()),
                    ],
                )),
                _ => results.push(
                    control_violation(
                        db,
                        event,
                        PolicyOperation::ControlFlowMissingCleanup,
                        query_digest,
                        vec![
                            ("policy", "missing_cleanup".to_string()),
                            ("required_cleanup", query.cleanup.values().join(",")),
                            ("uncovered_path", format!("{} -> exit", event.target_label)),
                            (
                                "dominance_evidence",
                                coverage
                                    .refuted_evidence(DOMINANCE_NO_CLEANUP_CANDIDATE)
                                    .to_string(),
                            ),
                            (
                                "require_error_cleanup",
                                query.require_error_cleanup.to_string(),
                            ),
                            ("requested_max_depth", query.max_depth.to_string()),
                        ],
                    )
                    .with_path_evidence(
                        [event.target_label.clone(), "exit".to_string()],
                        PolicyEvidenceEdgeKind::Control,
                    ),
                ),
            }
        }
    }

    if truncated && let Some(last) = results.pop() {
        results.push(last.with_budget_exceeded("control_flow_query_budget"));
    }

    results
}

/// Whether one CFG block relation covered an event, across every candidate.
///
/// `Established` is the only answer that proves coverage. `NotEstablished`
/// carries the first reason a lookup could not be evaluated, and `Refuted`
/// means every candidate was evaluated and none covered the event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelationCoverage {
    Established,
    NotEstablished(&'static str),
    Refuted { saw_candidate: bool },
}

impl RelationCoverage {
    /// Evidence value naming what decided a refuted result.
    fn refuted_evidence(self, without_candidate: &'static str) -> &'static str {
        match self {
            Self::Refuted {
                saw_candidate: true,
            } => DOMINANCE_FROM_RELATION,
            _ => without_candidate,
        }
    }
}

/// Folds every candidate's relation answer into one coverage verdict.
///
/// A single `Holds` proves coverage; otherwise the first `Unknown` reason wins
/// over refutation, because an unevaluated lookup is not a refutation.
fn relation_coverage<'a>(
    candidates: impl Iterator<Item = &'a &'a ControlCallEvent>,
    event: &ControlCallEvent,
    relation: &BlockRelation,
) -> RelationCoverage {
    let mut unknown_reason = None;
    let mut saw_candidate = false;
    for candidate in candidates {
        saw_candidate = true;
        match relation.answer(candidate, event) {
            DominanceAnswer::Holds => return RelationCoverage::Established,
            DominanceAnswer::DoesNotHold => {}
            DominanceAnswer::Unknown(reason) => {
                unknown_reason.get_or_insert(reason);
            }
        }
    }
    match unknown_reason {
        Some(reason) => RelationCoverage::NotEstablished(reason),
        None => RelationCoverage::Refuted { saw_candidate },
    }
}

/// Reason a CFG block relation could not answer a lookup.
///
/// One of the two events carried no basic block, so the relation has nothing to
/// look up.
pub(crate) const DOMINANCE_MISSING_BLOCK_IDS: &str = "missing_block_ids";
/// Reason a CFG block relation could not answer a lookup: the relation itself
/// has no rows, so the analysis never established it for this run.
pub(crate) const DOMINANCE_EMPTY_RELATION: &str = "empty_relation";
/// Evidence value naming the relation that decided an emitted control result.
const DOMINANCE_FROM_RELATION: &str = "dominator_relation";
/// Evidence value for a control result decided without any candidate to relate.
const DOMINANCE_NO_CANDIDATE: &str = "no_guard_candidate";
/// Evidence value for a lifecycle result decided without any cleanup candidate.
const DOMINANCE_NO_CLEANUP_CANDIDATE: &str = "no_cleanup_candidate";

/// Three-valued answer from one CFG block relation lookup.
///
/// The relation is only a proof when it answers [`DominanceAnswer::Holds`].
/// Missing inputs answer [`DominanceAnswer::Unknown`] with the reason, so a
/// caller can report "not established" instead of reading absence as coverage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DominanceAnswer {
    Holds,
    DoesNotHold,
    Unknown(&'static str),
}

/// Adjacency over one CFG block relation, built once per query.
struct BlockRelation {
    by_source: BTreeMap<BasicBlockId, Vec<BasicBlockId>>,
}

impl BlockRelation {
    fn new(edges: impl IntoIterator<Item = (BasicBlockId, BasicBlockId)>) -> Self {
        let mut by_source = BTreeMap::<BasicBlockId, Vec<BasicBlockId>>::new();
        for (source, destination) in edges {
            by_source.entry(source).or_default().push(destination);
        }
        Self { by_source }
    }

    fn dominators(db: &AnalysisDb) -> Self {
        Self::new(
            db.cfg_dominators()
                .iter()
                .map(|fact| (fact.dominated, fact.dominator)),
        )
    }

    fn postdominators(db: &AnalysisDb) -> Self {
        Self::new(
            db.cfg_postdominators()
                .iter()
                .map(|fact| (fact.postdominated, fact.postdominator)),
        )
    }

    fn answer(&self, candidate: &ControlCallEvent, event: &ControlCallEvent) -> DominanceAnswer {
        match (candidate.block, event.block) {
            (Some(candidate), Some(event)) => self.answer_for_blocks(candidate, event),
            _ => DominanceAnswer::Unknown(DOMINANCE_MISSING_BLOCK_IDS),
        }
    }

    fn answer_for_blocks(&self, candidate: BasicBlockId, event: BasicBlockId) -> DominanceAnswer {
        if candidate == event {
            return DominanceAnswer::Holds;
        }
        if self.by_source.is_empty() {
            return DominanceAnswer::Unknown(DOMINANCE_EMPTY_RELATION);
        }
        if self.reaches(event, candidate) {
            DominanceAnswer::Holds
        } else {
            DominanceAnswer::DoesNotHold
        }
    }

    fn reaches(&self, start: BasicBlockId, target: BasicBlockId) -> bool {
        let mut seen = BTreeSet::from([start]);
        let mut queue = VecDeque::from([start]);
        while let Some(current) = queue.pop_front() {
            for destination in self.by_source.get(&current).into_iter().flatten().copied() {
                if destination == target {
                    return true;
                }
                if seen.insert(destination) {
                    queue.push_back(destination);
                }
            }
        }
        false
    }
}

#[derive(Debug, Clone)]
struct FlowSource {
    node: DataFlowNodeId,
    label: String,
    precision: PolicyPrecision,
    heuristic: bool,
}

#[derive(Debug, Clone)]
struct FlowSink {
    node: DataFlowNodeId,
    site: CallSiteId,
    file: String,
    range: crate::diagnostics::TextRange,
    target_label: String,
    precision: PolicyPrecision,
    heuristic: bool,
}

fn matching_flow_sources(
    db: &AnalysisDb,
    store: &DataFlowStore,
    pattern: &SourcePattern,
) -> Vec<FlowSource> {
    let mut sources = BTreeMap::<DataFlowNodeId, FlowSource>::new();
    match pattern.kind() {
        SourcePatternKind::HttpRequest => {
            for node in store
                .nodes()
                .iter()
                .filter(|node| node.kind == DataFlowNodeKind::Source)
            {
                let Some(model) = node
                    .model
                    .and_then(|model| store.models().iter().find(|fact| fact.id == model))
                else {
                    continue;
                };
                if model
                    .payload_labels
                    .iter()
                    .any(|label| http_source_label(label))
                {
                    sources.insert(
                        node.id,
                        FlowSource {
                            node: node.id,
                            label: model
                                .model_id
                                .clone()
                                .unwrap_or_else(|| "http_request".to_string()),
                            precision: policy_precision_from_data_flow(model.precision),
                            heuristic: true,
                        },
                    );
                }
            }
        }
        SourcePatternKind::SecretLike => {
            for node in store.nodes() {
                if node.kind == DataFlowNodeKind::Source
                    && source_model_matches_values(store, node, pattern.values())
                {
                    sources.insert(
                        node.id,
                        FlowSource {
                            node: node.id,
                            label: source_node_label(store, node),
                            precision: PolicyPrecision::Heuristic,
                            heuristic: true,
                        },
                    );
                }

                if node.kind == DataFlowNodeKind::Place
                    && let Some(place) = node.place.and_then(|place| place_by_id(db, place))
                    && place_matches_values(place, pattern.values())
                {
                    sources.insert(
                        node.id,
                        FlowSource {
                            node: node.id,
                            label: place_label(place),
                            precision: PolicyPrecision::Heuristic,
                            heuristic: true,
                        },
                    );
                }
            }
        }
    }
    sources.into_values().collect()
}

fn matching_flow_sinks(
    db: &AnalysisDb,
    store: &DataFlowStore,
    pattern: &SinkPattern,
    site_by_id: &BTreeMap<CallSiteId, &CallSiteFact>,
) -> Vec<FlowSink> {
    let mut sinks = BTreeMap::<(CallSiteId, DataFlowNodeId), FlowSink>::new();

    if !db.refined_call_edges().is_empty() {
        for edge in db.refined_call_edges() {
            let Some(site) = site_by_id.get(&edge.site).copied() else {
                continue;
            };
            if !sink_matches_edge(db, pattern, edge, site_by_id) {
                continue;
            }
            push_flow_sinks_for_site(
                db,
                store,
                pattern,
                site,
                best_call_target_label(db, edge, site_by_id),
                policy_precision_from_call_precision(edge.precision),
                &mut sinks,
            );
        }
    } else {
        for site in db.call_sites() {
            if !sink_matches_site(pattern, site) {
                continue;
            }
            push_flow_sinks_for_site(
                db,
                store,
                pattern,
                site,
                call_site_label(site),
                policy_precision_from_call_precision(site.precision),
                &mut sinks,
            );
        }
    }

    sinks.into_values().collect()
}

fn push_flow_sinks_for_site(
    db: &AnalysisDb,
    store: &DataFlowStore,
    pattern: &SinkPattern,
    site: &CallSiteFact,
    target_label: String,
    precision: PolicyPrecision,
    sinks: &mut BTreeMap<(CallSiteId, DataFlowNodeId), FlowSink>,
) {
    let sink_precision = match pattern.kind() {
        SinkPatternKind::Call => precision,
        SinkPatternKind::Logger => PolicyPrecision::Heuristic,
    };
    let sink_heuristic = pattern.kind() == SinkPatternKind::Logger;

    for place in site.arguments.iter().copied().chain(site.receiver) {
        for node in nodes_for_place(store, place) {
            sinks.insert(
                (site.id, node.id),
                FlowSink {
                    node: node.id,
                    site: site.id,
                    file: db.path_for(site.file),
                    range: site.span.diagnostic_range(),
                    target_label: target_label.clone(),
                    precision: sink_precision,
                    heuristic: sink_heuristic,
                },
            );
        }
    }
}

fn nodes_for_place(store: &DataFlowStore, place: PlaceId) -> Vec<&DataFlowNodeFact> {
    store
        .nodes()
        .iter()
        .filter(|node| node.place == Some(place))
        .collect()
}

fn sink_matches_candidates(pattern: &SinkPattern, candidates: &[String]) -> bool {
    match pattern.kind() {
        SinkPatternKind::Call => pattern
            .values()
            .iter()
            .any(|value| candidates.iter().any(|candidate| candidate == value)),
        SinkPatternKind::Logger => candidates
            .iter()
            .any(|candidate| logger_candidate(candidate)),
    }
}

fn sink_matches_edge(
    db: &AnalysisDb,
    pattern: &SinkPattern,
    edge: &RefinedCallEdgeFact,
    site_by_id: &BTreeMap<CallSiteId, &CallSiteFact>,
) -> bool {
    let candidates = call_edge_match_candidates(db, edge, site_by_id);
    match pattern.kind() {
        SinkPatternKind::Call => explicit_values_match_preferred(
            pattern.values(),
            &candidates.semantic,
            &candidates.syntax,
        ),
        SinkPatternKind::Logger => candidates
            .all()
            .any(|candidate| logger_candidate(candidate)),
    }
}

fn sink_matches_site(pattern: &SinkPattern, site: &CallSiteFact) -> bool {
    let candidates = vec![call_site_label(site)];
    sink_matches_candidates(pattern, &candidates)
}

fn function_call_match<'a>(function: &'a FunctionFact, pattern: &EventPattern) -> Option<&'a str> {
    if pattern.kind() != EventPatternKind::Call {
        return None;
    }

    function
        .calls
        .iter()
        .find(|candidate| {
            pattern
                .values()
                .iter()
                .any(|value| call_name_matches(candidate, value))
        })
        .map(String::as_str)
}

fn call_name_matches(candidate: &str, value: &str) -> bool {
    candidate == value
        || candidate
            .rsplit_once('.')
            .is_some_and(|(_, tail)| tail == value)
}

fn matching_sanitizer_sites(
    db: &AnalysisDb,
    site_by_id: &BTreeMap<CallSiteId, &CallSiteFact>,
    barrier: &BarrierPattern,
) -> BTreeSet<CallSiteId> {
    match barrier.kind() {
        BarrierPatternKind::None => BTreeSet::new(),
        BarrierPatternKind::CallAny => db
            .call_sites()
            .iter()
            .map(|site| site.id)
            .filter(|site| barrier_matches_call_site(db, site_by_id, *site, barrier))
            .collect(),
    }
}

fn barrier_matches_call_site(
    db: &AnalysisDb,
    site_by_id: &BTreeMap<CallSiteId, &CallSiteFact>,
    site: CallSiteId,
    barrier: &BarrierPattern,
) -> bool {
    if barrier.kind() != BarrierPatternKind::CallAny {
        return false;
    }
    let mut semantic = Vec::new();
    let mut syntax = Vec::new();
    for edge in db
        .refined_call_edges()
        .iter()
        .filter(|edge| edge.site == site)
    {
        let candidates = call_edge_match_candidates(db, edge, site_by_id);
        semantic.extend(candidates.semantic);
        syntax.extend(candidates.syntax);
    }
    if let Some(site) = site_by_id.get(&site) {
        syntax.push(call_site_label(site));
    }
    semantic.sort();
    semantic.dedup();
    syntax.sort();
    syntax.dedup();
    explicit_values_match_preferred(barrier.values(), &semantic, &syntax)
}

fn flow_violation(
    _db: &AnalysisDb,
    store: &DataFlowStore,
    source: &FlowSource,
    sink: &FlowSink,
    path: &DataFlowPath,
    query: &FlowQuery,
    query_digest: &str,
) -> PolicyViolation {
    let (status, precision) = match path.status {
        DataFlowPathStatus::Found => {
            let path_precision = path_precision(store, path).unwrap_or(source.precision);
            let precision =
                min_policy_precision([source.precision, sink.precision, path_precision]);
            let status = found_data_flow_status(source, sink, precision);
            (status, precision)
        }
        DataFlowPathStatus::Unknown => (PolicyStatus::Unknown, PolicyPrecision::Unknown),
        DataFlowPathStatus::BudgetExceeded => {
            (PolicyStatus::BudgetExceeded, PolicyPrecision::Unknown)
        }
        DataFlowPathStatus::NotFound => (PolicyStatus::Unknown, PolicyPrecision::Unknown),
    };

    let barrier_status = match query.barriers.kind() {
        BarrierPatternKind::None => "not_configured",
        BarrierPatternKind::CallAny => "uncovered",
    };
    let mut evidence = vec![
        ("policy".to_string(), "forbidden_flow".to_string()),
        ("source".to_string(), source.label.clone()),
        ("sink".to_string(), sink.target_label.clone()),
        ("sink_call_site".to_string(), sink.site.0.to_string()),
        (
            "path_status".to_string(),
            data_flow_path_status_label(path.status).to_string(),
        ),
        ("path".to_string(), path_label(store, path)),
        ("path_edge_count".to_string(), path.edges.len().to_string()),
        ("barrier_status".to_string(), barrier_status.to_string()),
        (
            "supported_scope".to_string(),
            "bounded_private_data_flow".to_string(),
        ),
        (
            "requested_max_depth".to_string(),
            query.max_depth.to_string(),
        ),
        (
            "requested_max_paths".to_string(),
            query.max_paths.to_string(),
        ),
        (
            "minimum_precision".to_string(),
            policy_precision_label(query.minimum_precision).to_string(),
        ),
    ];
    if query.barriers.kind() == BarrierPatternKind::CallAny {
        evidence.push((
            "required_barrier".to_string(),
            query.barriers.values().join(","),
        ));
    }
    if let Some(reason) = path.budget_reason {
        evidence.push(("budget_reason".to_string(), format!("{reason:?}")));
    }
    if let Some(confidence) = path_confidence(store, path) {
        evidence.push((
            "data_flow_confidence".to_string(),
            data_flow_confidence_label(confidence).to_string(),
        ));
    }

    let mut hops = vec![source.label.clone()];
    for edge in path_edges(store, path) {
        hops.push(data_flow_edge_step_label(edge.kind).to_string());
    }
    hops.push(sink.target_label.clone());

    PolicyViolation::new(
        PolicyOperation::DataFlowForbidden,
        query_digest,
        sink.file.clone(),
        sink.range,
        status,
        precision,
        evidence,
    )
    .with_path_evidence(hops, PolicyEvidenceEdgeKind::DataTaint)
}

fn found_data_flow_status(
    source: &FlowSource,
    sink: &FlowSink,
    precision: PolicyPrecision,
) -> PolicyStatus {
    if source.heuristic
        || sink.heuristic
        || !matches!(
            precision,
            PolicyPrecision::Exact | PolicyPrecision::SetupAware
        )
    {
        PolicyStatus::Heuristic
    } else {
        PolicyStatus::Exact
    }
}

fn flow_result_key(source: &FlowSource, sink: &FlowSink, path: &DataFlowPath) -> String {
    format!(
        "{}:{}:{}:{}:{:?}:{:?}",
        source.node.0, sink.site.0, sink.node.0, sink.target_label, path.status, path.edges
    )
}

fn push_capped_data_flow_violation(
    results: &mut Vec<PolicyViolation>,
    violation: PolicyViolation,
    max_paths: usize,
) -> bool {
    if results.len() < max_paths {
        results.push(violation);
        true
    } else {
        if let Some(last) = results.pop() {
            results.push(last.with_budget_exceeded("data_flow_query_budget"));
        }
        false
    }
}

fn data_flow_violation_meets_minimum(
    violation: &PolicyViolation,
    minimum: PolicyPrecision,
) -> bool {
    if matches!(
        violation.status(),
        PolicyStatus::Unknown | PolicyStatus::BudgetExceeded | PolicyStatus::Unsupported
    ) {
        return true;
    }
    precision_rank(violation.precision()) >= precision_rank(minimum)
}

fn path_precision(store: &DataFlowStore, path: &DataFlowPath) -> Option<PolicyPrecision> {
    let mut precision = None;
    for edge in path_edges(store, path) {
        precision = Some(match precision {
            Some(current) => {
                min_policy_precision([current, policy_precision_from_data_flow(edge.precision)])
            }
            None => policy_precision_from_data_flow(edge.precision),
        });
    }
    precision
}

fn path_confidence(store: &DataFlowStore, path: &DataFlowPath) -> Option<DataFlowConfidence> {
    let mut confidence = None;
    for edge in path_edges(store, path) {
        confidence = Some(match confidence {
            Some(current) => min_data_flow_confidence(current, edge.confidence),
            None => edge.confidence,
        });
    }
    confidence
}

fn path_edges<'a>(store: &'a DataFlowStore, path: &DataFlowPath) -> Vec<&'a DataFlowEdgeFact> {
    path.edges
        .iter()
        .filter_map(|id| store.edges().iter().find(|edge| edge.id == *id))
        .collect()
}

fn path_label(store: &DataFlowStore, path: &DataFlowPath) -> String {
    if path.edges.is_empty() {
        return data_flow_path_status_label(path.status).to_string();
    }
    path_edges(store, path)
        .into_iter()
        .map(|edge| data_flow_edge_step_label(edge.kind))
        .collect::<Vec<_>>()
        .join(" -> ")
}

fn min_policy_precision<const N: usize>(values: [PolicyPrecision; N]) -> PolicyPrecision {
    values
        .into_iter()
        .min_by_key(|precision| precision_rank(*precision))
        .unwrap_or(PolicyPrecision::Unknown)
}

fn min_data_flow_confidence(
    left: DataFlowConfidence,
    right: DataFlowConfidence,
) -> DataFlowConfidence {
    if data_flow_confidence_rank(left) <= data_flow_confidence_rank(right) {
        left
    } else {
        right
    }
}

fn source_model_matches_values(
    store: &DataFlowStore,
    node: &DataFlowNodeFact,
    values: &[String],
) -> bool {
    node.model
        .and_then(|model| store.models().iter().find(|fact| fact.id == model))
        .is_some_and(|model| {
            model
                .payload_labels
                .iter()
                .chain(model.model_id.iter())
                .any(|label| contains_any_folded(label, values))
        })
}

fn source_node_label(store: &DataFlowStore, node: &DataFlowNodeFact) -> String {
    node.model
        .and_then(|model| store.models().iter().find(|fact| fact.id == model))
        .and_then(|model| model.model_id.clone())
        .unwrap_or_else(|| "source".to_string())
}

fn place_by_id(db: &AnalysisDb, place: PlaceId) -> Option<&PlaceFact> {
    db.mir_places().iter().find(|fact| fact.id == place)
}

fn place_matches_values(place: &PlaceFact, values: &[String]) -> bool {
    place_name_values(place)
        .iter()
        .any(|candidate| contains_any_folded(candidate, values))
}

fn place_label(place: &PlaceFact) -> String {
    place_name_values(place)
        .into_iter()
        .find(|value| !value.is_empty())
        .unwrap_or_else(|| "place".to_string())
}

fn place_name_values(place: &PlaceFact) -> Vec<String> {
    let mut values = Vec::new();
    match &place.root {
        PlaceRoot::Local { name, .. } | PlaceRoot::Global { name, .. } => {
            values.push(name.clone());
        }
        PlaceRoot::Parameter { name, index, .. } => {
            values.push(name.clone().unwrap_or_else(|| format!("param{index}")));
        }
        PlaceRoot::Temporary { ordinal, .. } => values.push(format!("temporary{ordinal}")),
        PlaceRoot::CallReturn { call } => values.push(format!("call_return{}", call.0)),
        PlaceRoot::Unknown { evidence } => values.push(evidence.clone()),
    }
    for projection in &place.projections {
        match projection {
            PlaceProjection::Field(value)
            | PlaceProjection::Property(value)
            | PlaceProjection::IndexKnown(value) => values.push(value.clone()),
            PlaceProjection::IndexUnknown { evidence } | PlaceProjection::Unknown { evidence } => {
                values.push(evidence.clone());
            }
            PlaceProjection::Deref => values.push("deref".to_string()),
            PlaceProjection::AwaitResult => values.push("await".to_string()),
            PlaceProjection::CallReturn(call) => values.push(format!("call_return{}", call.0)),
        }
    }
    values
}

fn contains_any_folded(candidate: &str, values: &[String]) -> bool {
    if values.is_empty() {
        return false;
    }
    let candidate = candidate.to_ascii_lowercase();
    values
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .any(|value| candidate.contains(&value))
}

fn http_source_label(label: &str) -> bool {
    [
        "source_kind=PathParam",
        "source_kind=QueryString",
        "source_kind=RequestBody",
        "source_kind=RequestHeader",
        "source_kind=Cookie",
    ]
    .contains(&label)
}

fn logger_candidate(candidate: &str) -> bool {
    let candidate = candidate.to_ascii_lowercase();
    candidate == "log"
        || candidate == "print"
        || candidate == "printf"
        || candidate == "println"
        || candidate == "log.print"
        || candidate == "log.printf"
        || candidate == "log.println"
        || candidate == "console.log"
        || candidate == "logger.log"
        || candidate == "logger.info"
        || candidate == "logger.warn"
        || candidate == "logger.error"
        || candidate.ends_with(".log")
        || candidate.ends_with(".info")
        || candidate.ends_with(".warn")
        || candidate.ends_with(".error")
}

fn policy_precision_from_data_flow(precision: DataFlowPrecision) -> PolicyPrecision {
    match precision {
        DataFlowPrecision::Exact => PolicyPrecision::Exact,
        DataFlowPrecision::SetupAware => PolicyPrecision::SetupAware,
        DataFlowPrecision::Syntax => PolicyPrecision::Syntax,
        DataFlowPrecision::Conservative => PolicyPrecision::Conservative,
        DataFlowPrecision::Heuristic => PolicyPrecision::Heuristic,
        DataFlowPrecision::Unknown => PolicyPrecision::Unknown,
    }
}

fn data_flow_path_status_label(status: DataFlowPathStatus) -> &'static str {
    match status {
        DataFlowPathStatus::Found => "found",
        DataFlowPathStatus::NotFound => "not_found",
        DataFlowPathStatus::Unknown => "unknown",
        DataFlowPathStatus::BudgetExceeded => "budget_exceeded",
    }
}

fn data_flow_edge_step_label(kind: DataFlowEdgeKind) -> &'static str {
    match kind {
        DataFlowEdgeKind::LocalBinding
        | DataFlowEdgeKind::LocalAssignment
        | DataFlowEdgeKind::LocalUse
        | DataFlowEdgeKind::LocalRead
        | DataFlowEdgeKind::LocalWrite
        | DataFlowEdgeKind::FieldProjection
        | DataFlowEdgeKind::IndexProjection
        | DataFlowEdgeKind::Dereference
        | DataFlowEdgeKind::AddressOf => "local_value",
        DataFlowEdgeKind::ReturnValue => "return_value",
        DataFlowEdgeKind::CallArgumentToParameter
        | DataFlowEdgeKind::CallArgumentToReturn
        | DataFlowEdgeKind::CallReturnToUse
        | DataFlowEdgeKind::ReceiverToMethod => "call_boundary",
        DataFlowEdgeKind::SummaryTito | DataFlowEdgeKind::SummaryProjected => "summary",
        DataFlowEdgeKind::UnknownFlow | DataFlowEdgeKind::HavocFlow => "unknown",
        DataFlowEdgeKind::BudgetTruncated => "budget",
        DataFlowEdgeKind::SourceIntroduction => "source",
        DataFlowEdgeKind::Sanitizer => "sanitizer",
        DataFlowEdgeKind::Barrier => "barrier",
        DataFlowEdgeKind::Model => "model",
    }
}

fn data_flow_confidence_label(confidence: DataFlowConfidence) -> &'static str {
    match confidence {
        DataFlowConfidence::High => "high",
        DataFlowConfidence::Medium => "medium",
        DataFlowConfidence::Low => "low",
    }
}

fn data_flow_confidence_rank(confidence: DataFlowConfidence) -> u8 {
    match confidence {
        DataFlowConfidence::High => 3,
        DataFlowConfidence::Medium => 2,
        DataFlowConfidence::Low => 1,
    }
}

fn policy_precision_label(precision: PolicyPrecision) -> &'static str {
    match precision {
        PolicyPrecision::Exact => "exact",
        PolicyPrecision::SetupAware => "setup_aware",
        PolicyPrecision::Syntax => "syntax",
        PolicyPrecision::Conservative => "conservative",
        PolicyPrecision::Heuristic => "heuristic",
        PolicyPrecision::Unknown => "unknown",
    }
}

#[derive(Debug, Clone)]
struct SearchState {
    function: FunctionId,
    depth: usize,
    path: Vec<String>,
}

#[derive(Debug, Clone)]
struct ControlCallEvent {
    function: FunctionId,
    block: Option<BasicBlockId>,
    file: String,
    range: crate::diagnostics::TextRange,
    target_label: String,
    candidates: Vec<String>,
    order: ControlOrder,
    order_source: &'static str,
    status: CallTargetStatus,
    precision: CallPrecision,
    confidence: Option<RefinedCallConfidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ControlOrder {
    order_source_rank: u8,
    operation_ordinal: u32,
    line: u32,
    col: u32,
    byte: u32,
    /// Resolved callsite identity text used only for deterministic ordering.
    /// Never a StableKeyId — allocation order must not affect control order.
    stable_key_text: String,
}

#[derive(Debug, Clone, Copy)]
struct OperationOrder {
    operation_ordinal: u32,
    source_rank: u8,
    source: &'static str,
}

fn call_sites_by_id(db: &AnalysisDb) -> BTreeMap<CallSiteId, &CallSiteFact> {
    db.call_sites().iter().map(|site| (site.id, site)).collect()
}

fn ordered_control_call_events(
    db: &AnalysisDb,
    minimum_precision: PolicyPrecision,
) -> Vec<ControlCallEvent> {
    let site_by_id = call_sites_by_id(db);
    let operation_orders = control_orders_by_operation(db);
    let blocks = cfg_blocks_by_operation(db);
    let mut events = Vec::new();

    if !db.refined_call_edges().is_empty() {
        for edge in db.refined_call_edges() {
            if !precision_meets(edge.precision, minimum_precision) {
                continue;
            }
            if let Some(event) =
                control_event_for_edge(db, edge, &site_by_id, &operation_orders, &blocks)
            {
                events.push(event);
            }
        }
    } else {
        for site in db.call_sites() {
            if precision_rank(policy_precision_from_call_precision(site.precision))
                < precision_rank(minimum_precision)
            {
                continue;
            }
            events.push(control_event_for_site(db, site, &operation_orders, &blocks));
        }
    }

    events.sort_by(|left, right| {
        (
            left.function,
            &left.order,
            left.target_label.as_str(),
            left.file.as_str(),
        )
            .cmp(&(
                right.function,
                &right.order,
                right.target_label.as_str(),
                right.file.as_str(),
            ))
    });
    events
}

fn control_orders_by_operation(db: &AnalysisDb) -> BTreeMap<(MirBodyId, MirOpId), OperationOrder> {
    let mut orders = BTreeMap::new();
    for operation in db.mir_operations() {
        insert_operation_order(
            &mut orders,
            (operation.body, operation.id),
            OperationOrder {
                operation_ordinal: operation.ordinal,
                source_rank: 1,
                source: "mir_operation_order",
            },
        );
    }
    for node in db.cfg_nodes() {
        let Some(operation) = node.operation else {
            continue;
        };
        insert_operation_order(
            &mut orders,
            (node.body, operation),
            OperationOrder {
                operation_ordinal: node.operation_ordinal,
                source_rank: 0,
                source: "cfg_operation_order",
            },
        );
    }
    orders
}

fn insert_operation_order(
    orders: &mut BTreeMap<(MirBodyId, MirOpId), OperationOrder>,
    key: (MirBodyId, MirOpId),
    next: OperationOrder,
) {
    orders
        .entry(key)
        .and_modify(|existing| {
            if (next.source_rank, next.operation_ordinal)
                < (existing.source_rank, existing.operation_ordinal)
            {
                *existing = next;
            }
        })
        .or_insert(next);
}

fn control_event_for_edge(
    db: &AnalysisDb,
    edge: &RefinedCallEdgeFact,
    site_by_id: &BTreeMap<CallSiteId, &CallSiteFact>,
    operation_orders: &BTreeMap<(MirBodyId, MirOpId), OperationOrder>,
    blocks: &BTreeMap<(MirBodyId, MirOpId), BasicBlockId>,
) -> Option<ControlCallEvent> {
    let site = site_by_id.get(&edge.site)?;
    let match_candidates = call_edge_match_candidates(db, edge, site_by_id);
    let candidates = if match_candidates.semantic.is_empty() {
        match_candidates.syntax
    } else {
        match_candidates.semantic
    };
    let target_label = best_call_target_label(db, edge, site_by_id);
    Some(ControlCallEvent {
        function: edge.caller,
        block: blocks.get(&(site.body, site.operation)).copied(),
        file: db.path_for(site.file),
        range: site.span.diagnostic_range(),
        target_label,
        candidates,
        order: control_order(db, site, operation_orders),
        order_source: control_order_source(site, operation_orders),
        status: edge.status,
        precision: edge.precision,
        confidence: Some(edge.confidence),
    })
}

fn control_event_for_site(
    db: &AnalysisDb,
    site: &CallSiteFact,
    operation_orders: &BTreeMap<(MirBodyId, MirOpId), OperationOrder>,
    blocks: &BTreeMap<(MirBodyId, MirOpId), BasicBlockId>,
) -> ControlCallEvent {
    let label = call_site_label(site);
    ControlCallEvent {
        function: site.caller,
        block: blocks.get(&(site.body, site.operation)).copied(),
        file: db.path_for(site.file),
        range: site.span.diagnostic_range(),
        target_label: label.clone(),
        candidates: vec![label],
        order: control_order(db, site, operation_orders),
        order_source: control_order_source(site, operation_orders),
        status: site.status,
        precision: site.precision,
        confidence: None,
    }
}

/// Block of every CFG-placed MIR operation, built once per control-event pass.
fn cfg_blocks_by_operation(db: &AnalysisDb) -> BTreeMap<(MirBodyId, MirOpId), BasicBlockId> {
    db.cfg_nodes()
        .iter()
        .filter_map(|node| {
            node.operation
                .map(|operation| ((node.body, operation), node.block))
        })
        .collect()
}

fn control_order(
    db: &AnalysisDb,
    site: &CallSiteFact,
    operation_orders: &BTreeMap<(MirBodyId, MirOpId), OperationOrder>,
) -> ControlOrder {
    let operation_order = operation_orders.get(&(site.body, site.operation));
    ControlOrder {
        order_source_rank: operation_order.map(|order| order.source_rank).unwrap_or(2),
        operation_ordinal: operation_order
            .map(|order| order.operation_ordinal)
            .unwrap_or(u32::MAX),
        line: site.span.start_line,
        col: site.span.start_col,
        byte: site.span.start_byte,
        stable_key_text: db.resolve_stable_key(site.stable_key).to_string(),
    }
}

fn control_order_source(
    site: &CallSiteFact,
    operation_orders: &BTreeMap<(MirBodyId, MirOpId), OperationOrder>,
) -> &'static str {
    operation_orders
        .get(&(site.body, site.operation))
        .map(|order| order.source)
        .unwrap_or("source_span")
}

fn control_events_by_function(
    events: &[ControlCallEvent],
) -> BTreeMap<FunctionId, Vec<&ControlCallEvent>> {
    let mut by_function = BTreeMap::<FunctionId, Vec<&ControlCallEvent>>::new();
    for event in events {
        by_function.entry(event.function).or_default().push(event);
    }
    by_function
}

fn event_matches_pattern(event: &ControlCallEvent, pattern: &EventPattern) -> bool {
    if pattern.kind() != EventPatternKind::Call {
        return false;
    }
    explicit_values_match_preferred(pattern.values(), &event.candidates, &[])
}

fn guard_matches_event(event: &ControlCallEvent, guard: &GuardPattern) -> bool {
    match guard.kind() {
        GuardPatternKind::CallAny => guard
            .values()
            .iter()
            .any(|value| event.candidates.iter().any(|candidate| candidate == value)),
    }
}

fn control_violation(
    db: &AnalysisDb,
    event: &ControlCallEvent,
    query: PolicyOperation,
    query_digest: &str,
    mut evidence: Vec<(&'static str, String)>,
) -> PolicyViolation {
    evidence.extend([
        ("target", event.target_label.clone()),
        (
            "function",
            function_by_id(db, event.function)
                .map(|function| function.name.clone())
                .unwrap_or_else(|| "<unknown-function>".to_string()),
        ),
        ("control_scope", "same_function".to_string()),
        ("supported_depth", "1".to_string()),
        ("order_source", event.order_source.to_string()),
        ("call_status", call_status_label(event.status).to_string()),
        (
            "call_precision",
            call_precision_label(event.precision).to_string(),
        ),
    ]);
    if let Some(confidence) = event.confidence {
        evidence.push(("confidence", confidence_label(confidence).to_string()));
    }

    PolicyViolation::new(
        query,
        query_digest,
        event.file.clone(),
        event.range,
        control_policy_status(event.status, event.precision),
        control_policy_precision(event.precision),
        evidence
            .into_iter()
            .map(|(label, value)| (label.to_string(), value))
            .collect(),
    )
}

/// A control result that reports "not established" rather than a violation.
///
/// It carries no `uncovered_path` and no `evidence_v1`: there is no path to
/// claim when the relation that would have proved coverage was unavailable.
fn unknown_control_result(
    db: &AnalysisDb,
    event: &ControlCallEvent,
    query: PolicyOperation,
    query_digest: &str,
    evidence: Vec<(&'static str, String)>,
) -> PolicyViolation {
    let mut result = control_violation(db, event, query, query_digest, evidence);
    result.set_status(PolicyStatus::Unknown);
    result.set_precision(PolicyPrecision::Unknown);
    result
}

fn control_policy_status(status: CallTargetStatus, precision: CallPrecision) -> PolicyStatus {
    match status {
        CallTargetStatus::BudgetExceeded => PolicyStatus::BudgetExceeded,
        CallTargetStatus::Unsupported => PolicyStatus::Unsupported,
        CallTargetStatus::SetupMissing => PolicyStatus::Unknown,
        CallTargetStatus::Unresolved => match precision {
            CallPrecision::Unknown | CallPrecision::Unsupported => PolicyStatus::Unknown,
            CallPrecision::Exact
            | CallPrecision::SetupAware
            | CallPrecision::Conservative
            | CallPrecision::Heuristic
            | CallPrecision::Ambiguous => PolicyStatus::Heuristic,
        },
        CallTargetStatus::Resolved | CallTargetStatus::Ambiguous | CallTargetStatus::Rejected => {
            PolicyStatus::Heuristic
        }
    }
}

fn control_policy_precision(precision: CallPrecision) -> PolicyPrecision {
    min_policy_precision([
        PolicyPrecision::Conservative,
        policy_precision_from_call_precision(precision),
    ])
}

fn refined_adjacency<'a>(
    db: &'a AnalysisDb,
    query: &ReachQuery,
    site_by_id: &BTreeMap<CallSiteId, &CallSiteFact>,
) -> BTreeMap<FunctionId, Vec<&'a RefinedCallEdgeFact>> {
    let mut adjacency: BTreeMap<FunctionId, Vec<&RefinedCallEdgeFact>> = BTreeMap::new();
    for edge in db.refined_call_edges() {
        if !query.include_tests
            && site_by_id
                .get(&edge.site)
                .is_some_and(|site| file_or_function_is_test(db, site.file, site.caller))
        {
            continue;
        }
        if edge_is_query_candidate(edge, query) {
            adjacency.entry(edge.caller).or_default().push(edge);
        }
    }
    for edges in adjacency.values_mut() {
        edges.sort_by(|left, right| {
            (
                best_call_target_label(db, left, site_by_id),
                db.resolve_stable_key(left.stable_key).as_ref(),
            )
                .cmp(&(
                    best_call_target_label(db, right, site_by_id),
                    db.resolve_stable_key(right.stable_key).as_ref(),
                ))
        });
    }
    adjacency
}

fn selected_reachability_roots<'a>(
    db: &'a AnalysisDb,
    query: &ReachQuery,
) -> Vec<&'a ReachabilityRootFact> {
    let mut roots = db
        .reachability_roots()
        .iter()
        .filter(|root| {
            query.include_tests
                || (root.kind != RootKind::Test
                    && !function_by_id(db, root.target_function).is_some_and(|f| f.is_test))
        })
        .filter(|root| {
            query.roots.is_empty()
                || query
                    .roots
                    .iter()
                    .any(|pattern| call_pattern_matches_root(db, pattern, root))
        })
        .collect::<Vec<_>>();
    roots.sort_by(|left, right| {
        (
            root_label(db, left),
            db.resolve_stable_key(left.stable_key).as_ref(),
        )
            .cmp(&(
                root_label(db, right),
                db.resolve_stable_key(right.stable_key).as_ref(),
            ))
    });
    roots
}

fn call_pattern_matches_edge(
    db: &AnalysisDb,
    pattern: &EventPattern,
    edge: &RefinedCallEdgeFact,
    site_by_id: &BTreeMap<CallSiteId, &CallSiteFact>,
) -> bool {
    if pattern.kind() != EventPatternKind::Call {
        return false;
    }
    let candidates = call_edge_match_candidates(db, edge, site_by_id);
    explicit_values_match_preferred(pattern.values(), &candidates.semantic, &candidates.syntax)
}

fn call_pattern_matches_site(pattern: &EventPattern, site: &CallSiteFact) -> bool {
    if pattern.kind() != EventPatternKind::Call {
        return false;
    }
    let label = call_site_label(site);
    pattern.values().iter().any(|value| value == &label)
}

fn call_pattern_matches_root(
    db: &AnalysisDb,
    pattern: &EventPattern,
    root: &ReachabilityRootFact,
) -> bool {
    if pattern.kind() != EventPatternKind::Call {
        return false;
    }
    let mut candidates = vec![
        root.kind.as_str().to_string(),
        format!("root.{}", root.kind.as_str()),
        root_label(db, root),
    ];
    if let Some(function) = function_by_id(db, root.target_function) {
        candidates.push(function.name.clone());
    }
    if let Some(symbol) = root
        .target_symbol
        .and_then(|symbol| db.symbol_by_id(symbol))
    {
        candidates.push(symbol.name.clone());
        candidates.push(symbol.qualified_name.clone());
    }
    pattern
        .values()
        .iter()
        .any(|value| candidates.iter().any(|candidate| candidate == value))
}

#[derive(Debug, Clone, Default)]
struct CallMatchCandidates {
    semantic: Vec<String>,
    syntax: Vec<String>,
}

impl CallMatchCandidates {
    fn all(&self) -> impl Iterator<Item = &String> {
        self.semantic.iter().chain(self.syntax.iter())
    }
}

fn call_edge_match_candidates(
    db: &AnalysisDb,
    edge: &RefinedCallEdgeFact,
    site_by_id: &BTreeMap<CallSiteId, &CallSiteFact>,
) -> CallMatchCandidates {
    let mut semantic = Vec::new();
    if let Some(synthetic_target) = &edge.synthetic_target
        && synthetic_target_is_user_matchable(synthetic_target)
    {
        semantic.push(synthetic_target.clone());
    }
    if let Some(symbol) = edge
        .target_symbol
        .and_then(|symbol| db.symbol_by_id(symbol))
    {
        semantic.push(symbol.name.clone());
        semantic.push(symbol.qualified_name.clone());
    }
    if let Some(function) = edge
        .target_function
        .and_then(|function| function_by_id(db, function))
    {
        semantic.push(function.name.clone());
    }
    let mut syntax = Vec::new();
    if let Some(site) = site_by_id.get(&edge.site) {
        syntax.push(call_site_label(site));
    }
    semantic.sort();
    semantic.dedup();
    syntax.sort();
    syntax.dedup();
    CallMatchCandidates { semantic, syntax }
}

fn synthetic_target_is_user_matchable(target: &str) -> bool {
    !target.starts_with("ts-js:")
        && !target.starts_with("go:")
        && !target.starts_with("framework:")
        && !target.starts_with("synthetic:")
}

fn explicit_values_match_preferred(
    values: &[String],
    semantic: &[String],
    syntax: &[String],
) -> bool {
    let candidates = if semantic.is_empty() {
        syntax
    } else {
        semantic
    };
    values
        .iter()
        .any(|value| candidates.iter().any(|candidate| candidate == value))
}

fn best_call_target_label(
    db: &AnalysisDb,
    edge: &RefinedCallEdgeFact,
    site_by_id: &BTreeMap<CallSiteId, &CallSiteFact>,
) -> String {
    if let Some(symbol) = edge
        .target_symbol
        .and_then(|symbol| db.symbol_by_id(symbol))
    {
        if !symbol.qualified_name.is_empty() {
            return symbol.qualified_name.clone();
        }
        return symbol.name.clone();
    }
    if let Some(function) = edge
        .target_function
        .and_then(|function| function_by_id(db, function))
    {
        return function.name.clone();
    }
    if let Some(synthetic_target) = &edge.synthetic_target
        && synthetic_target_is_user_matchable(synthetic_target)
    {
        return synthetic_target.clone();
    }
    site_by_id
        .get(&edge.site)
        .map(|site| call_site_label(site))
        .unwrap_or_else(|| "<unknown-call>".to_string())
}

fn call_site_label(site: &CallSiteFact) -> String {
    match &site.callee {
        CallCallee::Identifier { name, .. } => name.clone(),
        CallCallee::Member { property, .. } => property.clone(),
        CallCallee::Constructor { name, .. } => name
            .clone()
            .unwrap_or_else(|| "<anonymous-constructor>".to_string()),
        CallCallee::Index { .. } => "<index-call>".to_string(),
        CallCallee::Super => "super".to_string(),
        CallCallee::Import => "import".to_string(),
        CallCallee::FunctionValue { .. } => "<function-value>".to_string(),
        CallCallee::Unknown { reason } => format!("unknown:{reason:?}"),
    }
}

fn violation_for_edge(
    db: &AnalysisDb,
    edge: &RefinedCallEdgeFact,
    site_by_id: &BTreeMap<CallSiteId, &CallSiteFact>,
    query: PolicyOperation,
    query_digest: &str,
    mut evidence: Vec<(&'static str, String)>,
) -> PolicyViolation {
    let (file, range) = site_by_id.get(&edge.site).map_or_else(
        || {
            (
                "<unknown>".to_string(),
                crate::diagnostics::TextRange::point(1, 1),
            )
        },
        |site| (db.path_for(site.file), site.span.diagnostic_range()),
    );
    evidence.extend([
        ("call_status", call_status_label(edge.status).to_string()),
        (
            "call_precision",
            call_precision_label(edge.precision).to_string(),
        ),
        ("confidence", confidence_label(edge.confidence).to_string()),
    ]);
    if let Some(reason) = edge.reason {
        evidence.push(("unknown_reason", format!("{reason:?}")));
    }
    PolicyViolation::new(
        query,
        query_digest,
        file,
        range,
        policy_status_from_call_status_and_precision(edge.status, edge.precision),
        policy_precision_from_call_precision(edge.precision),
        evidence
            .into_iter()
            .map(|(label, value)| (label.to_string(), value))
            .collect(),
    )
}

trait PolicyViolationExt {
    fn with_budget_exceeded(self, budget: &'static str) -> Self;
}

impl PolicyViolationExt for PolicyViolation {
    fn with_budget_exceeded(self, budget: &'static str) -> Self {
        let mut diagnostic = self;
        diagnostic.push_evidence("budget", budget);
        diagnostic.set_status(PolicyStatus::BudgetExceeded);
        diagnostic
    }
}

fn traversable_status(status: CallTargetStatus) -> bool {
    matches!(status, CallTargetStatus::Resolved)
}

fn edge_is_query_candidate(edge: &RefinedCallEdgeFact, query: &ReachQuery) -> bool {
    confidence_meets(edge.confidence, query.minimum_confidence)
        && (precision_meets(edge.precision, query.minimum_precision)
            || !traversable_status(edge.status))
}

fn policy_status_from_call_status_and_precision(
    status: CallTargetStatus,
    precision: CallPrecision,
) -> PolicyStatus {
    match status {
        CallTargetStatus::Resolved => {
            let precision = policy_precision_from_call_precision(precision);
            if matches!(
                precision,
                PolicyPrecision::Exact | PolicyPrecision::SetupAware
            ) {
                PolicyStatus::Exact
            } else {
                PolicyStatus::Heuristic
            }
        }
        CallTargetStatus::Ambiguous | CallTargetStatus::Rejected => PolicyStatus::Heuristic,
        CallTargetStatus::BudgetExceeded => PolicyStatus::BudgetExceeded,
        CallTargetStatus::Unsupported => PolicyStatus::Unsupported,
        CallTargetStatus::Unresolved | CallTargetStatus::SetupMissing => PolicyStatus::Unknown,
    }
}

fn policy_precision_from_call_precision(precision: CallPrecision) -> PolicyPrecision {
    match precision {
        CallPrecision::Exact => PolicyPrecision::Exact,
        CallPrecision::SetupAware => PolicyPrecision::SetupAware,
        CallPrecision::Conservative | CallPrecision::Ambiguous => PolicyPrecision::Conservative,
        CallPrecision::Heuristic => PolicyPrecision::Heuristic,
        CallPrecision::Unknown | CallPrecision::Unsupported => PolicyPrecision::Unknown,
    }
}

fn precision_meets(actual: CallPrecision, minimum: PolicyPrecision) -> bool {
    precision_rank(policy_precision_from_call_precision(actual)) >= precision_rank(minimum)
}

fn precision_rank(precision: PolicyPrecision) -> u8 {
    match precision {
        PolicyPrecision::Exact => 5,
        PolicyPrecision::SetupAware => 4,
        PolicyPrecision::Syntax => 3,
        PolicyPrecision::Conservative => 2,
        PolicyPrecision::Heuristic => 1,
        PolicyPrecision::Unknown => 0,
    }
}

fn confidence_meets(actual: RefinedCallConfidence, minimum: PolicyConfidence) -> bool {
    confidence_rank(policy_confidence_from_refined(actual)) >= confidence_rank(minimum)
}

fn policy_confidence_from_refined(confidence: RefinedCallConfidence) -> PolicyConfidence {
    match confidence {
        RefinedCallConfidence::High => PolicyConfidence::High,
        RefinedCallConfidence::Medium => PolicyConfidence::Medium,
        RefinedCallConfidence::Low => PolicyConfidence::Low,
    }
}

fn confidence_rank(confidence: PolicyConfidence) -> u8 {
    match confidence {
        PolicyConfidence::High => 3,
        PolicyConfidence::Medium => 2,
        PolicyConfidence::Low => 1,
    }
}

fn function_by_id(db: &AnalysisDb, id: FunctionId) -> Option<&FunctionFact> {
    db.functions().iter().find(|function| function.id == id)
}

fn file_or_function_is_test(db: &AnalysisDb, file: FileId, function: FunctionId) -> bool {
    function_by_id(db, function).is_some_and(|function| function.is_test)
        || db.tests().iter().any(|test| test.file == file)
}

fn root_label(db: &AnalysisDb, root: &ReachabilityRootFact) -> String {
    if let Some(symbol) = root
        .target_symbol
        .and_then(|symbol| db.symbol_by_id(symbol))
    {
        if !symbol.qualified_name.is_empty() {
            return symbol.qualified_name.clone();
        }
        return symbol.name.clone();
    }
    function_by_id(db, root.target_function)
        .map(|function| function.name.clone())
        .unwrap_or_else(|| root.kind.as_str().to_string())
}

fn call_status_label(status: CallTargetStatus) -> &'static str {
    match status {
        CallTargetStatus::Resolved => "resolved",
        CallTargetStatus::Ambiguous => "ambiguous",
        CallTargetStatus::Unresolved => "unresolved",
        CallTargetStatus::Unsupported => "unsupported",
        CallTargetStatus::SetupMissing => "setup_missing",
        CallTargetStatus::BudgetExceeded => "budget_exceeded",
        CallTargetStatus::Rejected => "rejected",
    }
}

fn call_precision_label(precision: CallPrecision) -> &'static str {
    match precision {
        CallPrecision::Exact => "exact",
        CallPrecision::SetupAware => "setup_aware",
        CallPrecision::Conservative => "conservative",
        CallPrecision::Heuristic => "heuristic",
        CallPrecision::Ambiguous => "ambiguous",
        CallPrecision::Unknown => "unknown",
        CallPrecision::Unsupported => "unsupported",
    }
}

fn confidence_label(confidence: RefinedCallConfidence) -> &'static str {
    match confidence {
        RefinedCallConfidence::High => "high",
        RefinedCallConfidence::Medium => "medium",
        RefinedCallConfidence::Low => "low",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::calls::facts::{
        CallAlgorithm, CallEdgeKind, CallProvenance, CallSyntaxKind, CallTargetFact,
        UnresolvedCallReason,
    };
    use crate::analysis::calls::store::CallOutput;
    use crate::analysis::cfg::facts::{
        BasicBlockFact, BasicBlockKind, CfgFunctionFact, CfgNodeFact, CfgNodeKind, CfgPrecision,
        CfgStatus, CfgView, DominatorFact, PostDominatorFact,
    };
    use crate::analysis::cfg::ids::{
        BasicBlockId, CfgFunctionId, CfgNodeId, DominatorId, PostDominatorId,
    };
    use crate::analysis::cfg::store::CfgOutput;
    use crate::analysis::data_flow::facts::{
        DataFlowAlgorithm, DataFlowConfidence, DataFlowEdgeFact, DataFlowEdgeKind,
        DataFlowModelFact, DataFlowModelKind, DataFlowNodeFact, DataFlowNodeKind,
        DataFlowPrecision, DataFlowProvenance, DataFlowStatus, DataFlowValidation,
    };
    use crate::analysis::data_flow::store::DataFlowOutput;
    use crate::analysis::ids::{
        CallTargetId, DataFlowEdgeId, DataFlowModelId, DataFlowNodeId, MirBodyId, MirOpId,
        MirPredicateId, PlaceId, ReachabilityRootId, RefinedCallEdgeId,
    };
    use crate::analysis::mir::body::{MirBody, MirOutput, MirStatus};
    use crate::analysis::mir::op::{MirOperation, MirOperationKind};
    use crate::analysis::reachability::facts::{
        RootPrecision, RootProvenance, RootStatus, compute_reachability_root_stable_key,
    };
    use crate::analysis::reachability::store::{
        REACHABILITY_PROVIDER_ID, ReachabilityProviderOutput,
    };
    use crate::analysis::refined_calls::facts::{RefinedCallTier, RefinedCallValidation};
    use crate::analysis::refined_calls::store::RefinedCallOutput;
    use crate::core::{FileId, FunctionFact, Language, Span};
    use std::path::PathBuf;

    #[test]
    fn events_matching_call_returns_provider_backed_violation() {
        let db = policy_query_db(false);

        let violations = matching_events(&db, EventPattern::call("dangerous"));

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].status(), PolicyStatus::Exact);
        assert_eq!(violations[0].precision(), PolicyPrecision::SetupAware);
    }

    #[test]
    fn events_matching_fallback_uses_call_site_precision_for_status() {
        let db = policy_query_db_with_conservative_fallback_call_site();

        let violations = matching_events(&db, EventPattern::call("dangerous"));

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].status(), PolicyStatus::Heuristic);
        assert_eq!(violations[0].precision(), PolicyPrecision::Conservative);
    }

    #[test]
    fn events_matching_uses_syntax_function_calls_without_call_site_facts() {
        let db = policy_query_db_with_syntax_function_call("client.dangerous");

        let violations = matching_events(&db, EventPattern::call("dangerous"));

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].status(), PolicyStatus::Heuristic);
        assert_eq!(violations[0].precision(), PolicyPrecision::Syntax);
        let diagnostic = violations[0].diagnostic("local/test", "event");
        assert!(diagnostic.evidence.iter().any(|evidence| {
            evidence.label == "event_source" && evidence.value == "syntax_function_calls"
        }));
        assert!(diagnostic.evidence.iter().any(|evidence| {
            evidence.label == "target" && evidence.value == "client.dangerous"
        }));
    }

    #[test]
    fn events_matching_call_prefers_semantic_target_over_syntax_label() {
        let db = policy_query_db_with_syntax_label_resolved_to_different_target();

        assert!(matching_events(&db, EventPattern::call("dangerous")).is_empty());
        assert_eq!(
            matching_events(&db, EventPattern::call("safeTarget")).len(),
            1
        );
    }

    #[test]
    fn events_matching_call_uses_syntax_label_for_internal_synthetic_target() {
        let db = policy_query_db_with_internal_synthetic_target();

        assert_eq!(
            matching_events(&db, EventPattern::call("dangerous")).len(),
            1
        );
    }

    #[test]
    fn calls_forbidden_reachable_finds_target_from_root() {
        let db = policy_query_db(false);
        let mut query = ReachQuery::new(EventPattern::call("dangerous"));
        query.roots = vec![EventPattern::call("main")];

        let violations = forbidden_reachable(&db, query);

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].status(), PolicyStatus::Exact);
        assert_eq!(violations[0].precision(), PolicyPrecision::SetupAware);
    }

    #[test]
    fn policy_query_findings_carry_replayable_evidence() {
        let db = policy_query_db(false);
        let mut query = ReachQuery::new(EventPattern::call("dangerous"));
        query.roots = vec![EventPattern::call("main")];

        let violations = forbidden_reachable(&db, query);
        assert!(!violations.is_empty());

        let with_path = violations
            .iter()
            .map(|violation| violation.diagnostic("local/test", "reachable"))
            .filter(|diagnostic| {
                diagnostic
                    .evidence_v1
                    .as_ref()
                    .and_then(|evidence| evidence.as_value().get("paths"))
                    .and_then(|paths| paths.as_array())
                    .is_some_and(|paths| !paths.is_empty())
            })
            .collect::<Vec<_>>();
        let coverage = (with_path.len() as f64) / (violations.len() as f64);
        assert!(
            coverage > 0.90,
            "expected >90% policy-query findings with evidence paths, got {:.1}% ({}/{})",
            coverage * 100.0,
            with_path.len(),
            violations.len()
        );

        let evidence = with_path[0]
            .evidence_v1
            .as_ref()
            .expect("structured evidence")
            .as_value();
        assert!(
            evidence["bundle"]["replay_key"]
                .as_str()
                .is_some_and(|key| !key.is_empty())
        );
        assert!(evidence["unknowns"].as_array().is_some());
        assert!(evidence["limits"].as_object().is_some());
    }

    #[test]
    fn calls_forbidden_reachable_excludes_test_roots_by_default() {
        let db = policy_query_db(true);
        let query = ReachQuery::new(EventPattern::call("dangerous"));

        assert!(forbidden_reachable(&db, query.clone()).is_empty());

        let mut include_tests = query;
        include_tests.include_tests = true;
        assert_eq!(forbidden_reachable(&db, include_tests).len(), 1);
    }

    #[test]
    fn calls_forbidden_reachable_surfaces_unresolved_matching_target() {
        let db = policy_query_db_with_unresolved_target();
        let mut query = ReachQuery::new(EventPattern::call("dangerous"));
        query.minimum_precision = PolicyPrecision::Unknown;
        let violations = forbidden_reachable(&db, query);

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].status(), PolicyStatus::Unknown);
        assert_eq!(violations[0].precision(), PolicyPrecision::Unknown);
    }

    #[test]
    fn calls_forbidden_reachable_surfaces_unresolved_matching_target_at_default_precision() {
        let db = policy_query_db_with_unresolved_target();
        let violations = forbidden_reachable(&db, ReachQuery::new(EventPattern::call("dangerous")));

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].status(), PolicyStatus::Unknown);
        assert_eq!(violations[0].precision(), PolicyPrecision::Unknown);
    }

    #[test]
    fn calls_forbidden_reachable_marks_budget_when_path_cap_truncates() {
        let db = policy_query_db_with_two_matching_targets();
        let mut query = ReachQuery::new(EventPattern::call("dangerous"));
        query.max_paths = 1;

        let violations = forbidden_reachable(&db, query);

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].status(), PolicyStatus::BudgetExceeded);
        let diagnostic = violations[0].diagnostic("local/test", "dangerous call is reachable");
        assert!(
            diagnostic.evidence.iter().any(
                |evidence| evidence.label == "budget" && evidence.value == "reach_query_budget"
            )
        );
    }

    #[test]
    fn control_flow_missing_guard_reports_call_without_prior_guard() {
        let db = policy_query_db_with_call_sequence(&["dangerous", "authorize"]);
        let query = GuardQuery::new(
            EventPattern::call("dangerous"),
            GuardPattern::call_any(["authorize"]),
        );

        let violations = missing_guards(&db, query);

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].status(), PolicyStatus::Heuristic);
        assert_eq!(violations[0].precision(), PolicyPrecision::Conservative);
        let diagnostic = violations[0].diagnostic("local/test", "missing guard");
        assert!(
            diagnostic
                .evidence
                .iter()
                .any(|evidence| evidence.label == "policy" && evidence.value == "missing_guard")
        );
        assert!(diagnostic.evidence.iter().any(|evidence| {
            evidence.label == "required_guard" && evidence.value == "authorize"
        }));
    }

    #[test]
    fn control_flow_missing_guard_treats_local_callsite_match_as_heuristic() {
        let db = policy_query_db_with_unresolved_conservative_call_site();
        let query = GuardQuery::new(
            EventPattern::call("dangerous"),
            GuardPattern::call_any(["authorize"]),
        );

        let violations = missing_guards(&db, query);

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].status(), PolicyStatus::Heuristic);
        assert_eq!(violations[0].precision(), PolicyPrecision::Conservative);
    }

    #[test]
    fn control_flow_missing_guard_ignores_prior_guard() {
        let db = policy_query_db_with_call_sequence(&["authorize", "dangerous"]);
        let query = GuardQuery::new(
            EventPattern::call("dangerous"),
            GuardPattern::call_any(["authorize"]),
        );

        assert!(missing_guards(&db, query).is_empty());
    }

    #[test]
    fn control_flow_missing_guard_prefers_cfg_order_over_mir_order() {
        let db = policy_query_db_with_cfg_ordered_call_sequence(&[
            ("dangerous", 1, 2),
            ("authorize", 2, 1),
        ]);
        let query = GuardQuery::new(
            EventPattern::call("dangerous"),
            GuardPattern::call_any(["authorize"]),
        );

        assert!(missing_guards(&db, query).is_empty());
    }

    #[test]
    fn control_flow_missing_guard_reports_unknown_when_dominator_relation_is_empty() {
        let db = policy_query_db_with_cfg_ordered_call_sequence(&[
            ("dangerous", 1, 2),
            ("authorize", 2, 1),
        ]);
        let mut query = GuardQuery::new(
            EventPattern::call("dangerous"),
            GuardPattern::call_any(["authorize"]),
        );
        query.report_unknown_coverage = true;

        let results = missing_guards(&db, query);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status(), PolicyStatus::Unknown);
        assert_eq!(results[0].precision(), PolicyPrecision::Unknown);
        assert!(has_evidence(
            &results[0],
            "dominance_evidence",
            DOMINANCE_EMPTY_RELATION
        ));
    }

    #[test]
    fn control_flow_missing_guard_reports_unknown_when_operation_has_no_cfg_block() {
        let db = policy_query_db_with_cfg_calls(
            &[
                CfgCall::new("authorize", 1, 1),
                CfgCall::new("dangerous", 2, 2).unplaced(),
            ],
            &[(2, 1)],
        );
        let mut query = GuardQuery::new(
            EventPattern::call("dangerous"),
            GuardPattern::call_any(["authorize"]),
        );
        query.report_unknown_coverage = true;

        let results = missing_guards(&db, query);

        assert_eq!(results.len(), 1);
        assert!(has_evidence(
            &results[0],
            "dominance_evidence",
            DOMINANCE_MISSING_BLOCK_IDS
        ));
    }

    #[test]
    fn control_flow_missing_guard_unknown_result_carries_no_uncovered_path() {
        let db = policy_query_db_with_cfg_ordered_call_sequence(&[
            ("dangerous", 1, 2),
            ("authorize", 2, 1),
        ]);
        let mut query = GuardQuery::new(
            EventPattern::call("dangerous"),
            GuardPattern::call_any(["authorize"]),
        );
        query.report_unknown_coverage = true;

        let results = missing_guards(&db, query);
        let diagnostic = results[0].diagnostic("local/test", "coverage unknown");

        assert!(diagnostic.evidence_v1.is_none());
        assert!(
            !diagnostic
                .evidence
                .iter()
                .any(|evidence| evidence.label == "uncovered_path")
        );
    }

    #[test]
    fn control_flow_missing_guard_holds_with_immediate_only_dominators() {
        let db = policy_query_db_with_cfg_calls(
            &[
                CfgCall::new("authorize", 1, 1),
                CfgCall::new("intermediate", 2, 2),
                CfgCall::new("dangerous", 3, 3),
            ],
            &[(3, 2), (2, 1)],
        );
        let mut query = GuardQuery::new(
            EventPattern::call("dangerous"),
            GuardPattern::call_any(["authorize"]),
        );
        query.report_unknown_coverage = true;

        assert!(missing_guards(&db, query).is_empty());
    }

    #[test]
    fn control_flow_missing_guard_reports_violation_when_dominance_is_refuted() {
        let db = policy_query_db_with_cfg_calls(
            &[
                CfgCall::new("authorize", 1, 1),
                CfgCall::new("dangerous", 2, 2),
            ],
            &[(1, 1)],
        );
        let mut query = GuardQuery::new(
            EventPattern::call("dangerous"),
            GuardPattern::call_any(["authorize"]),
        );
        query.report_unknown_coverage = true;

        let results = missing_guards(&db, query);

        assert_eq!(results.len(), 1);
        assert!(has_evidence(
            &results[0],
            "dominance_evidence",
            "dominator_relation"
        ));
    }

    #[test]
    fn control_flow_missing_guard_keeps_fail_open_behavior_by_default() {
        let db = policy_query_db_with_cfg_ordered_call_sequence(&[
            ("dangerous", 1, 2),
            ("authorize", 2, 1),
        ]);
        let query = GuardQuery::new(
            EventPattern::call("dangerous"),
            GuardPattern::call_any(["authorize"]),
        );

        assert!(missing_guards(&db, query).is_empty());
    }

    #[test]
    fn control_flow_missing_cleanup_reports_unknown_when_postdominator_relation_is_empty() {
        let db =
            policy_query_db_with_cfg_ordered_call_sequence(&[("Begin", 1, 1), ("Close", 2, 2)]);
        let mut query =
            LifecycleQuery::new(EventPattern::call("Begin"), EventPattern::call("Close"));
        query.report_unknown_coverage = true;

        let results = missing_cleanup(&db, query);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status(), PolicyStatus::Unknown);
        assert!(has_evidence(
            &results[0],
            "dominance_evidence",
            DOMINANCE_EMPTY_RELATION
        ));
    }

    #[test]
    fn control_flow_missing_cleanup_reports_unknown_when_operation_has_no_cfg_block() {
        let db = policy_query_db_with_cfg_calls(
            &[
                CfgCall::new("Begin", 1, 1),
                CfgCall::new("Close", 2, 2).unplaced(),
            ],
            &[(1, 2)],
        );
        let mut query =
            LifecycleQuery::new(EventPattern::call("Begin"), EventPattern::call("Close"));
        query.report_unknown_coverage = true;

        let results = missing_cleanup(&db, query);

        assert_eq!(results.len(), 1);
        assert!(has_evidence(
            &results[0],
            "dominance_evidence",
            DOMINANCE_MISSING_BLOCK_IDS
        ));
    }

    #[test]
    fn control_flow_missing_cleanup_keeps_fail_open_behavior_by_default() {
        let db =
            policy_query_db_with_cfg_ordered_call_sequence(&[("Begin", 1, 1), ("Close", 2, 2)]);
        let query = LifecycleQuery::new(EventPattern::call("Begin"), EventPattern::call("Close"));

        assert!(missing_cleanup(&db, query).is_empty());
    }

    #[test]
    fn control_flow_missing_guard_falls_back_to_mir_order_when_cfg_is_absent() {
        let db =
            policy_query_db_with_mir_ordered_call_sequence(&[("dangerous", 2), ("authorize", 1)]);
        let query = GuardQuery::new(
            EventPattern::call("dangerous"),
            GuardPattern::call_any(["authorize"]),
        );

        assert!(missing_guards(&db, query).is_empty());
    }

    #[test]
    fn control_flow_missing_guard_ignores_write_field_until_backed() {
        let db = policy_query_db_with_call_sequence(&["dangerous"]);
        let query = GuardQuery::new(
            EventPattern::write_field("account.balance"),
            GuardPattern::call_any(["authorize"]),
        );

        assert!(missing_guards(&db, query).is_empty());
    }

    #[test]
    fn control_flow_missing_guard_marks_budget_when_path_cap_truncates() {
        let db = policy_query_db_with_two_matching_targets();
        let mut query = GuardQuery::new(
            EventPattern::call("dangerous"),
            GuardPattern::call_any(["authorize"]),
        );
        query.max_paths = 1;

        let violations = missing_guards(&db, query);

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].status(), PolicyStatus::BudgetExceeded);
        let diagnostic = violations[0].diagnostic("local/test", "missing guard");
        assert!(diagnostic.evidence.iter().any(|evidence| {
            evidence.label == "budget" && evidence.value == "control_flow_query_budget"
        }));
    }

    #[test]
    fn control_flow_missing_cleanup_reports_start_without_later_cleanup() {
        let db = policy_query_db_with_call_sequence(&["Begin", "work"]);
        let query =
            LifecycleQuery::new(EventPattern::call("Begin"), EventPattern::call("Rollback"));

        let violations = missing_cleanup(&db, query);

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].status(), PolicyStatus::Heuristic);
        assert_eq!(violations[0].precision(), PolicyPrecision::Conservative);
        let diagnostic = violations[0].diagnostic("local/test", "missing cleanup");
        assert!(
            diagnostic
                .evidence
                .iter()
                .any(|evidence| evidence.label == "policy" && evidence.value == "missing_cleanup")
        );
        assert!(diagnostic.evidence.iter().any(|evidence| {
            evidence.label == "required_cleanup" && evidence.value == "Rollback"
        }));
    }

    #[test]
    fn control_flow_missing_cleanup_ignores_later_cleanup() {
        let db = policy_query_db_with_call_sequence(&["Begin", "work", "Rollback"]);
        let query =
            LifecycleQuery::new(EventPattern::call("Begin"), EventPattern::call("Rollback"));

        assert!(missing_cleanup(&db, query).is_empty());
    }

    #[test]
    fn data_flow_forbidden_reports_http_source_to_call_sink() {
        let db = data_flow_policy_db(vec![data_flow_edge(
            0,
            0,
            1,
            DataFlowEdgeKind::SourceIntroduction,
            DataFlowStatus::Present,
            Vec::new(),
        )]);

        let violations = forbidden_flows(
            &db,
            FlowQuery::new(
                SourcePattern::http_request(),
                SinkPattern::call("dangerous"),
            ),
        );

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].status(), PolicyStatus::Heuristic);
        assert_eq!(violations[0].precision(), PolicyPrecision::SetupAware);
        let diagnostic = violations[0].diagnostic("local/test", "request reaches sink");
        assert!(
            diagnostic
                .evidence
                .iter()
                .any(|evidence| evidence.label == "policy" && evidence.value == "forbidden_flow")
        );
        assert!(diagnostic.evidence.iter().any(|evidence| {
            evidence.label == "barrier_status" && evidence.value == "not_configured"
        }));
    }

    #[test]
    fn data_flow_forbidden_reports_distinct_sink_call_sites_for_same_place() {
        let db = data_flow_policy_db_with_two_sink_sites_same_place();

        let violations = forbidden_flows(
            &db,
            FlowQuery::new(
                SourcePattern::http_request(),
                SinkPattern::call("dangerous"),
            ),
        );

        assert_eq!(violations.len(), 2);
        assert_ne!(violations[0].stable_key(), violations[1].stable_key());
    }

    #[test]
    fn data_flow_forbidden_filters_found_paths_below_minimum_precision() {
        let db = data_flow_policy_db(vec![data_flow_edge(
            0,
            0,
            1,
            DataFlowEdgeKind::SourceIntroduction,
            DataFlowStatus::Present,
            Vec::new(),
        )]);
        let mut query = FlowQuery::new(
            SourcePattern::http_request(),
            SinkPattern::call("dangerous"),
        );
        query.minimum_precision = PolicyPrecision::Exact;

        assert!(forbidden_flows(&db, query).is_empty());
    }

    #[test]
    fn data_flow_forbidden_logger_sink_uses_heuristic_precision_for_syntax_match() {
        let db = data_flow_policy_db_with_unresolved_logger_sink(vec![data_flow_edge(
            0,
            0,
            1,
            DataFlowEdgeKind::SourceIntroduction,
            DataFlowStatus::Present,
            Vec::new(),
        )]);
        let mut query = FlowQuery::new(SourcePattern::http_request(), SinkPattern::logger());
        query.minimum_precision = PolicyPrecision::Heuristic;

        let violations = forbidden_flows(&db, query);

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].status(), PolicyStatus::Heuristic);
        assert_eq!(violations[0].precision(), PolicyPrecision::Heuristic);
    }

    #[test]
    fn data_flow_forbidden_call_sink_uses_syntax_label_for_internal_synthetic_target() {
        let db = data_flow_policy_db_with_internal_synthetic_target(vec![data_flow_edge(
            0,
            0,
            1,
            DataFlowEdgeKind::SourceIntroduction,
            DataFlowStatus::Present,
            Vec::new(),
        )]);
        let mut query = FlowQuery::new(
            SourcePattern::http_request(),
            SinkPattern::call("dangerous"),
        );
        query.minimum_precision = PolicyPrecision::Unknown;

        let violations = forbidden_flows(&db, query);

        assert_eq!(violations.len(), 1);
    }

    #[test]
    fn data_flow_forbidden_respects_barrier_call() {
        let db = data_flow_policy_db_with_barrier();
        let mut query = FlowQuery::new(
            SourcePattern::http_request(),
            SinkPattern::call("dangerous"),
        );
        query.barriers = BarrierPattern::call_any(["validate"]);

        assert!(forbidden_flows(&db, query).is_empty());
    }

    #[test]
    fn data_flow_forbidden_marks_budget_when_depth_cap_truncates() {
        let db = data_flow_policy_db_with_sink_place(
            vec![
                data_flow_edge(
                    0,
                    0,
                    1,
                    DataFlowEdgeKind::SourceIntroduction,
                    DataFlowStatus::Present,
                    Vec::new(),
                ),
                data_flow_edge(
                    1,
                    1,
                    2,
                    DataFlowEdgeKind::LocalAssignment,
                    DataFlowStatus::Present,
                    Vec::new(),
                ),
            ],
            PlaceId(2),
        );
        let mut query = FlowQuery::new(
            SourcePattern::http_request(),
            SinkPattern::call("dangerous"),
        );
        query.max_depth = 1;
        query.minimum_precision = PolicyPrecision::Unknown;

        let violations = forbidden_flows(&db, query);

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].status(), PolicyStatus::BudgetExceeded);
        let diagnostic = violations[0].diagnostic("local/test", "request reaches sink");
        assert!(diagnostic.evidence.iter().any(|evidence| {
            evidence.label == "budget_reason" && evidence.value == "PathDepth"
        }));
    }

    #[test]
    fn data_flow_forbidden_caps_found_plus_budget_status_to_max_paths() {
        let db = data_flow_policy_db_with_sink_place(
            vec![
                data_flow_edge(
                    0,
                    0,
                    1,
                    DataFlowEdgeKind::SourceIntroduction,
                    DataFlowStatus::Present,
                    Vec::new(),
                ),
                data_flow_edge(
                    1,
                    0,
                    2,
                    DataFlowEdgeKind::SourceIntroduction,
                    DataFlowStatus::Present,
                    Vec::new(),
                ),
                data_flow_edge(
                    2,
                    2,
                    1,
                    DataFlowEdgeKind::LocalAssignment,
                    DataFlowStatus::Present,
                    Vec::new(),
                ),
            ],
            PlaceId(1),
        );
        let mut query = FlowQuery::new(
            SourcePattern::http_request(),
            SinkPattern::call("dangerous"),
        );
        query.max_paths = 1;
        query.minimum_precision = PolicyPrecision::Unknown;

        let violations = forbidden_flows(&db, query);

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].status(), PolicyStatus::BudgetExceeded);
    }

    #[test]
    fn data_flow_forbidden_returns_no_results_for_zero_path_budget() {
        let db = data_flow_policy_db(vec![data_flow_edge(
            0,
            0,
            1,
            DataFlowEdgeKind::SourceIntroduction,
            DataFlowStatus::Present,
            Vec::new(),
        )]);
        let mut query = FlowQuery::new(
            SourcePattern::http_request(),
            SinkPattern::call("dangerous"),
        );
        query.max_paths = 0;

        assert!(forbidden_flows(&db, query).is_empty());
    }

    #[test]
    fn data_flow_forbidden_surfaces_unknown_path() {
        let db = data_flow_policy_db(vec![data_flow_edge(
            0,
            0,
            1,
            DataFlowEdgeKind::UnknownFlow,
            DataFlowStatus::Unknown,
            Vec::new(),
        )]);

        let mut query = FlowQuery::new(
            SourcePattern::http_request(),
            SinkPattern::call("dangerous"),
        );
        query.minimum_precision = PolicyPrecision::Unknown;

        let violations = forbidden_flows(&db, query);

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].status(), PolicyStatus::Unknown);
        assert_eq!(violations[0].precision(), PolicyPrecision::Unknown);
    }

    #[test]
    fn data_flow_forbidden_surfaces_unknown_path_at_default_precision() {
        let db = data_flow_policy_db(vec![data_flow_edge(
            0,
            0,
            1,
            DataFlowEdgeKind::UnknownFlow,
            DataFlowStatus::Unknown,
            Vec::new(),
        )]);

        let violations = forbidden_flows(
            &db,
            FlowQuery::new(
                SourcePattern::http_request(),
                SinkPattern::call("dangerous"),
            ),
        );

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].status(), PolicyStatus::Unknown);
    }

    #[test]
    fn data_flow_forbidden_returns_no_results_without_store() {
        let db = AnalysisDb::new();

        assert!(
            forbidden_flows(
                &db,
                FlowQuery::new(
                    SourcePattern::http_request(),
                    SinkPattern::call("dangerous")
                )
            )
            .is_empty()
        );
    }

    #[test]
    fn policy_query_diagnostics_include_normalized_header() {
        let db = policy_query_db(false);

        let event_query = EventPattern::call("dangerous");
        let event_digest = event_query.query_digest();
        let event = matching_events(&db, event_query)
            .pop()
            .expect("event violation")
            .diagnostic("local/test", "event");
        assert_policy_header(&event, "events.matching", &event_digest);

        let mut reach_query = ReachQuery::new(EventPattern::call("dangerous"));
        reach_query.roots = vec![EventPattern::call("main")];
        let reach_digest = reach_query.query_digest();
        let reach = forbidden_reachable(&db, reach_query)
            .pop()
            .expect("reach violation")
            .diagnostic("local/test", "reach");
        assert_policy_header(&reach, "calls.forbidden_reachable", &reach_digest);

        let control_db = policy_query_db_with_call_sequence(&["dangerous"]);
        let guard_query = GuardQuery::new(
            EventPattern::call("dangerous"),
            GuardPattern::call_any(["authorize"]),
        );
        let guard_digest = guard_query.query_digest();
        let guard = missing_guards(&control_db, guard_query)
            .pop()
            .expect("guard violation")
            .diagnostic("local/test", "guard");
        assert_policy_header(&guard, "control_flow.missing_guard", &guard_digest);

        let lifecycle_db = policy_query_db_with_call_sequence(&["Begin"]);
        let lifecycle_query =
            LifecycleQuery::new(EventPattern::call("Begin"), EventPattern::call("Rollback"));
        let lifecycle_digest = lifecycle_query.query_digest();
        let lifecycle = missing_cleanup(&lifecycle_db, lifecycle_query)
            .pop()
            .expect("lifecycle violation")
            .diagnostic("local/test", "lifecycle");
        assert_policy_header(
            &lifecycle,
            "control_flow.missing_cleanup",
            &lifecycle_digest,
        );

        let flow_db = data_flow_policy_db(vec![data_flow_edge(
            0,
            0,
            1,
            DataFlowEdgeKind::SourceIntroduction,
            DataFlowStatus::Present,
            Vec::new(),
        )]);
        let flow_query = FlowQuery::new(
            SourcePattern::http_request(),
            SinkPattern::call("dangerous"),
        );
        let flow_digest = flow_query.query_digest();
        let flow = forbidden_flows(&flow_db, flow_query)
            .pop()
            .expect("flow violation")
            .diagnostic("local/test", "flow");
        assert_policy_header(&flow, "data_flow.forbidden", &flow_digest);
    }

    #[test]
    fn policy_query_results_are_sorted_and_repeatable() {
        let db = policy_query_db_with_two_matching_targets();
        let query = ReachQuery::new(EventPattern::call("dangerous"));

        let first = forbidden_reachable(&db, query.clone())
            .into_iter()
            .map(|violation| violation.stable_key())
            .collect::<Vec<_>>();
        let second = forbidden_reachable(&db, query)
            .into_iter()
            .map(|violation| violation.stable_key())
            .collect::<Vec<_>>();
        let mut sorted = first.clone();
        sorted.sort();

        assert_eq!(first, second);
        assert_eq!(first, sorted);
    }

    fn has_evidence(violation: &PolicyViolation, label: &str, value: &str) -> bool {
        violation
            .diagnostic("local/test", "message")
            .evidence
            .iter()
            .any(|evidence| evidence.label == label && evidence.value == value)
    }

    fn assert_policy_header(
        diagnostic: &crate::diagnostics::Diagnostic,
        query: &str,
        digest: &str,
    ) {
        assert!(
            diagnostic
                .evidence
                .iter()
                .any(|evidence| { evidence.label == "policy_query" && evidence.value == query })
        );
        assert!(diagnostic.evidence.iter().any(|evidence| {
            evidence.label == "policy_query_version"
                && evidence.value == crate::sdk::policy::POLICY_QUERY_VERSION
        }));
        assert!(
            diagnostic
                .evidence
                .iter()
                .any(|evidence| { evidence.label == "query_digest" && evidence.value == digest })
        );
        assert!(
            diagnostic.evidence.iter().any(|evidence| {
                evidence.label == "policy_status" && !evidence.value.is_empty()
            })
        );
        assert!(diagnostic.evidence.iter().any(|evidence| {
            evidence.label == "policy_precision" && !evidence.value.is_empty()
        }));
    }

    fn policy_query_db(test_root: bool) -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/main.go"),
            "src/main.go".to_string(),
            "package main\nfunc main() { handler() }\nfunc handler() { dangerous() }\nfunc dangerous() {}\n".to_string(),
        );
        let main = push_test_function(&mut db, file, "main", 1, test_root);
        let handler = push_test_function(&mut db, file, "handler", 2, false);
        let dangerous = push_test_function(&mut db, file, "dangerous", 3, false);

        let main_to_handler = call_site(0, file, main, "handler");
        let handler_to_dangerous = call_site(1, file, handler, "dangerous");
        db.replace_call_facts(CallOutput {
            sites: vec![main_to_handler.clone(), handler_to_dangerous.clone()],
            targets: vec![
                call_target(0, main_to_handler.id, main, handler),
                call_target(1, handler_to_dangerous.id, handler, dangerous),
            ],
            unresolved: Vec::new(),
        })
        .expect("valid call facts");
        db.replace_refined_call_facts(RefinedCallOutput {
            edges: vec![
                refined_edge(0, main_to_handler.id, main, handler, "handler"),
                refined_edge(1, handler_to_dangerous.id, handler, dangerous, "dangerous"),
            ],
        })
        .expect("valid refined call facts");

        let root_kind = if test_root {
            RootKind::Test
        } else {
            RootKind::Main
        };
        let root_span = Span::point(file, 1, 1);
        db.replace_reachability_facts(ReachabilityProviderOutput {
            roots: vec![ReachabilityRootFact {
                id: ReachabilityRootId(0),
                kind: root_kind,
                language: Language::Go,
                target_function: main,
                target_symbol: None,
                originating_entrypoint: None,
                file,
                span: root_span.clone(),
                precision: RootPrecision::ResolvedStatic,
                provenance: RootProvenance::NativeDiscovery,
                status: RootStatus::Resolved,
                provider_id: REACHABILITY_PROVIDER_ID.to_string(),
                stable_key: crate::core::stable_key_for_test(
                    &compute_reachability_root_stable_key(
                        root_kind,
                        Language::Go,
                        "main.main",
                        "src/main.go",
                        &root_span,
                    ),
                ),
            }],
        })
        .expect("valid reachability facts");
        db
    }

    fn policy_query_db_with_conservative_fallback_call_site() -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/main.go"),
            "src/main.go".to_string(),
            "package main\nfunc main() { dangerous() }\n".to_string(),
        );
        let main = push_test_function(&mut db, file, "main", 1, false);
        let mut site = call_site(0, file, main, "dangerous");
        site.precision = CallPrecision::Conservative;

        db.replace_call_facts(CallOutput {
            sites: vec![site],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("valid call facts");
        db.replace_refined_call_facts(RefinedCallOutput { edges: Vec::new() })
            .expect("valid refined call facts");
        db
    }

    fn policy_query_db_with_unresolved_conservative_call_site() -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/main.go"),
            "src/main.go".to_string(),
            "package main\nfunc main() { dangerous() }\n".to_string(),
        );
        let main = push_test_function(&mut db, file, "main", 1, false);
        let mut site = call_site(0, file, main, "dangerous");
        site.status = CallTargetStatus::Unresolved;
        site.precision = CallPrecision::Conservative;

        db.replace_call_facts(CallOutput {
            sites: vec![site],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("valid call facts");
        db
    }

    fn policy_query_db_with_syntax_function_call(call: &str) -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/main.ts"),
            "src/main.ts".to_string(),
            "export function handler() { client.dangerous(); }\n".to_string(),
        );
        db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "handler".to_string(),
            Span::point(file, 1, 1),
            Language::TypeScript,
            false,
            true,
            1,
            vec![call.to_string()],
        ));
        db
    }

    fn policy_query_db_with_syntax_label_resolved_to_different_target() -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/main.go"),
            "src/main.go".to_string(),
            "package main\nfunc main() { dangerous() }\nfunc safeTarget() {}\n".to_string(),
        );
        let main = push_test_function(&mut db, file, "main", 1, false);
        let safe = push_test_function(&mut db, file, "safeTarget", 2, false);
        let site = call_site(0, file, main, "dangerous");

        db.replace_call_facts(CallOutput {
            sites: vec![site.clone()],
            targets: vec![call_target(0, site.id, main, safe)],
            unresolved: Vec::new(),
        })
        .expect("valid call facts");
        db.replace_refined_call_facts(RefinedCallOutput {
            edges: vec![refined_edge(0, site.id, main, safe, "safeTarget")],
        })
        .expect("valid refined call facts");
        db
    }

    fn policy_query_db_with_internal_synthetic_target() -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/main.go"),
            "src/main.go".to_string(),
            "package main\nfunc main() { dangerous() }\n".to_string(),
        );
        let main = push_test_function(&mut db, file, "main", 1, false);
        let mut site = call_site(0, file, main, "dangerous");
        site.status = CallTargetStatus::BudgetExceeded;
        site.precision = CallPrecision::Unknown;
        let mut edge = unresolved_refined_edge(0, site.id, main, "dangerous");
        edge.status = CallTargetStatus::BudgetExceeded;
        edge.synthetic_target = Some("ts-js:points-to-budget:1".to_string());

        db.replace_call_facts(CallOutput {
            sites: vec![site],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("valid call facts");
        db.replace_refined_call_facts(RefinedCallOutput { edges: vec![edge] })
            .expect("valid refined call facts");
        db
    }

    fn policy_query_db_with_unresolved_target() -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/main.go"),
            "src/main.go".to_string(),
            "package main\nfunc main() { handler() }\nfunc handler() { dangerous() }\n".to_string(),
        );
        let main = push_test_function(&mut db, file, "main", 1, false);
        let handler = push_test_function(&mut db, file, "handler", 2, false);

        let main_to_handler = call_site(0, file, main, "handler");
        let mut unresolved = call_site(1, file, handler, "dangerous");
        unresolved.status = CallTargetStatus::Unresolved;
        unresolved.precision = CallPrecision::Unknown;
        db.replace_call_facts(CallOutput {
            sites: vec![main_to_handler.clone(), unresolved.clone()],
            targets: vec![call_target(0, main_to_handler.id, main, handler)],
            unresolved: Vec::new(),
        })
        .expect("valid call facts");
        db.replace_refined_call_facts(RefinedCallOutput {
            edges: vec![
                refined_edge(0, main_to_handler.id, main, handler, "handler"),
                unresolved_refined_edge(1, unresolved.id, handler, "dangerous"),
            ],
        })
        .expect("valid refined call facts");

        let root_span = Span::point(file, 1, 1);
        db.replace_reachability_facts(ReachabilityProviderOutput {
            roots: vec![ReachabilityRootFact {
                id: ReachabilityRootId(0),
                kind: RootKind::Main,
                language: Language::Go,
                target_function: main,
                target_symbol: None,
                originating_entrypoint: None,
                file,
                span: root_span.clone(),
                precision: RootPrecision::ResolvedStatic,
                provenance: RootProvenance::NativeDiscovery,
                status: RootStatus::Resolved,
                provider_id: REACHABILITY_PROVIDER_ID.to_string(),
                stable_key: crate::core::stable_key_for_test(
                    &compute_reachability_root_stable_key(
                        RootKind::Main,
                        Language::Go,
                        "main.main",
                        "src/main.go",
                        &root_span,
                    ),
                ),
            }],
        })
        .expect("valid reachability facts");
        db
    }

    fn policy_query_db_with_two_matching_targets() -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/main.go"),
            "src/main.go".to_string(),
            "package main\nfunc main() { dangerous(); dangerous() }\nfunc dangerousA() {}\nfunc dangerousB() {}\n".to_string(),
        );
        let main = push_test_function(&mut db, file, "main", 1, false);
        let dangerous_a = push_test_function(&mut db, file, "dangerous", 2, false);
        let dangerous_b = push_test_function(&mut db, file, "dangerous", 3, false);

        let first = call_site(0, file, main, "dangerous");
        let second = call_site(1, file, main, "dangerous");
        db.replace_call_facts(CallOutput {
            sites: vec![first.clone(), second.clone()],
            targets: vec![
                call_target(0, first.id, main, dangerous_a),
                call_target(1, second.id, main, dangerous_b),
            ],
            unresolved: Vec::new(),
        })
        .expect("valid call facts");
        db.replace_refined_call_facts(RefinedCallOutput {
            edges: vec![
                refined_edge(0, first.id, main, dangerous_a, "dangerousA"),
                refined_edge(1, second.id, main, dangerous_b, "dangerousB"),
            ],
        })
        .expect("valid refined call facts");

        let root_span = Span::point(file, 1, 1);
        db.replace_reachability_facts(ReachabilityProviderOutput {
            roots: vec![ReachabilityRootFact {
                id: ReachabilityRootId(0),
                kind: RootKind::Main,
                language: Language::Go,
                target_function: main,
                target_symbol: None,
                originating_entrypoint: None,
                file,
                span: root_span.clone(),
                precision: RootPrecision::ResolvedStatic,
                provenance: RootProvenance::NativeDiscovery,
                status: RootStatus::Resolved,
                provider_id: REACHABILITY_PROVIDER_ID.to_string(),
                stable_key: crate::core::stable_key_for_test(
                    &compute_reachability_root_stable_key(
                        RootKind::Main,
                        Language::Go,
                        "main.main",
                        "src/main.go",
                        &root_span,
                    ),
                ),
            }],
        })
        .expect("valid reachability facts");
        db
    }

    fn policy_query_db_with_call_sequence(callees: &[&str]) -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/main.go"),
            "src/main.go".to_string(),
            "package main\nfunc handler() {}\n".to_string(),
        );
        let handler = push_test_function(&mut db, file, "handler", 1, false);
        let mut sites = Vec::new();
        let mut targets = Vec::new();
        let mut edges = Vec::new();

        for (index, callee) in callees.iter().enumerate() {
            let id = index as u64;
            let callee_function =
                push_test_function(&mut db, file, callee, index as u32 + 10, false);
            let site = call_site(id, file, handler, callee);
            targets.push(call_target(id, site.id, handler, callee_function));
            edges.push(refined_edge(id, site.id, handler, callee_function, callee));
            sites.push(site);
        }

        db.replace_call_facts(CallOutput {
            sites,
            targets,
            unresolved: Vec::new(),
        })
        .expect("valid call facts");
        db.replace_refined_call_facts(RefinedCallOutput { edges })
            .expect("valid refined call facts");
        db
    }

    fn policy_query_db_with_mir_ordered_call_sequence(callees: &[(&str, u32)]) -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let interner = db.stable_key_interner();
        let file = db.add_file(
            PathBuf::from("src/main.go"),
            "src/main.go".to_string(),
            "package main\nfunc handler() {}\n".to_string(),
        );
        let handler = push_test_function(&mut db, file, "handler", 1, false);
        let mut sites = Vec::new();
        let mut targets = Vec::new();
        let mut edges = Vec::new();
        let mut operations = Vec::new();

        for (index, (callee, operation_ordinal)) in callees.iter().enumerate() {
            let id = index as u64;
            let operation = MirOpId(id);
            let callee_function =
                push_test_function(&mut db, file, callee, index as u32 + 10, false);
            let site = call_site(id, file, handler, callee);
            targets.push(call_target(id, site.id, handler, callee_function));
            edges.push(refined_edge(id, site.id, handler, callee_function, callee));
            operations.push(mir_operation(
                &interner,
                operation,
                MirBodyId(handler.0),
                *operation_ordinal,
                file,
            ));
            sites.push(site);
        }

        db.replace_semantic_mir(MirOutput {
            bodies: vec![mir_body(&interner, file, handler)],
            places: Vec::new(),
            operations,
            unsupported: Vec::new(),
            ..MirOutput::default()
        })
        .expect("valid semantic MIR facts");
        db.replace_call_facts(CallOutput {
            sites,
            targets,
            unresolved: Vec::new(),
        })
        .expect("valid call facts");
        db.replace_refined_call_facts(RefinedCallOutput { edges })
            .expect("valid refined call facts");
        db
    }

    /// One call in a synthetic CFG-placed sequence.
    #[derive(Debug, Clone, Copy)]
    struct CfgCall<'a> {
        callee: &'a str,
        mir_ordinal: u32,
        cfg_ordinal: u32,
        /// When false the call keeps its MIR operation but gets no CFG node, so
        /// the query cannot map it to a basic block.
        placed: bool,
    }

    impl<'a> CfgCall<'a> {
        fn new(callee: &'a str, mir_ordinal: u32, cfg_ordinal: u32) -> Self {
            Self {
                callee,
                mir_ordinal,
                cfg_ordinal,
                placed: true,
            }
        }

        fn unplaced(mut self) -> Self {
            self.placed = false;
            self
        }
    }

    fn policy_query_db_with_cfg_ordered_call_sequence(callees: &[(&str, u32, u32)]) -> AnalysisDb {
        let calls = callees
            .iter()
            .map(|(callee, mir_ordinal, cfg_ordinal)| {
                CfgCall::new(callee, *mir_ordinal, *cfg_ordinal)
            })
            .collect::<Vec<_>>();
        policy_query_db_with_cfg_calls(&calls, &[])
    }

    /// Builds a single-function DB whose calls sit in distinct basic blocks.
    ///
    /// `dominators` are `(dominated, dominator)` block pairs; an empty list
    /// leaves the relation unestablished, which is the shape the fail-open
    /// fallback used to read as coverage.
    fn policy_query_db_with_cfg_calls(
        calls: &[CfgCall<'_>],
        dominators: &[(u64, u64)],
    ) -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let interner = db.stable_key_interner();
        let file = db.add_file(
            PathBuf::from("src/main.go"),
            "src/main.go".to_string(),
            "package main\nfunc handler() {}\n".to_string(),
        );
        let handler = push_test_function(&mut db, file, "handler", 1, false);
        let body = MirBodyId(handler.0);
        let cfg_function = CfgFunctionId(handler.0);
        let mut sites = Vec::new();
        let mut targets = Vec::new();
        let mut edges = Vec::new();
        let mut operations = Vec::new();
        let mut cfg_nodes = Vec::new();
        let mut cfg_blocks = Vec::new();

        for (index, call) in calls.iter().enumerate() {
            let CfgCall {
                callee,
                mir_ordinal,
                cfg_ordinal,
                placed,
            } = *call;
            let id = index as u64;
            let operation = MirOpId(id);
            let block = BasicBlockId(id + 1);
            let cfg_node = CfgNodeId(id + 1);
            let callee_function =
                push_test_function(&mut db, file, callee, index as u32 + 10, false);
            let site = call_site(id, file, handler, callee);
            targets.push(call_target(id, site.id, handler, callee_function));
            edges.push(refined_edge(id, site.id, handler, callee_function, callee));
            operations.push(mir_operation(&interner, operation, body, mir_ordinal, file));
            if placed {
                cfg_nodes.push(CfgNodeFact {
                    id: cfg_node,
                    cfg_function,
                    body,
                    operation: Some(operation),
                    block,
                    kind: CfgNodeKind::CallSite,
                    span: Some(site.span.clone()),
                    generated: false,
                    operation_ordinal: cfg_ordinal,
                    stable_key: interner.intern(format!("cfg:node:{id}:{callee}")),
                    status: CfgStatus::Resolved,
                    precision: CfgPrecision::ExactLowered,
                });
                cfg_blocks.push(BasicBlockFact {
                    id: block,
                    cfg_function,
                    kind: BasicBlockKind::StraightLine,
                    first_node: Some(cfg_node),
                    last_node: Some(cfg_node),
                    reachable: true,
                    reverse_postorder: cfg_ordinal,
                    stable_key: interner.intern(format!("cfg:block:{id}:{callee}")),
                    status: CfgStatus::Resolved,
                    precision: CfgPrecision::ExactLowered,
                });
            }
            sites.push(site);
        }

        let dominator_facts = dominators
            .iter()
            .enumerate()
            .map(|(index, (dominated, dominator))| DominatorFact {
                id: DominatorId(index as u64),
                cfg_function,
                view: CfgView::NormalControl,
                dominator: BasicBlockId(*dominator),
                dominated: BasicBlockId(*dominated),
                immediate: true,
                stable_key: interner.intern(format!("cfg:dom:{dominated}:{dominator}")),
                status: CfgStatus::Resolved,
                precision: CfgPrecision::ExactLowered,
            })
            .collect::<Vec<_>>();
        let postdominator_facts = dominators
            .iter()
            .enumerate()
            .map(|(index, (dominated, dominator))| PostDominatorFact {
                id: PostDominatorId(index as u64),
                cfg_function,
                view: CfgView::NormalControl,
                postdominator: BasicBlockId(*dominator),
                postdominated: BasicBlockId(*dominated),
                immediate: true,
                stable_key: interner.intern(format!("cfg:pdom:{dominated}:{dominator}")),
                status: CfgStatus::Resolved,
                precision: CfgPrecision::ExactLowered,
            })
            .collect::<Vec<_>>();

        db.replace_semantic_mir(MirOutput {
            bodies: vec![mir_body(&interner, file, handler)],
            places: Vec::new(),
            operations,
            unsupported: Vec::new(),
            ..MirOutput::default()
        })
        .expect("valid semantic MIR facts");
        db.replace_cfg_facts(CfgOutput {
            functions: vec![CfgFunctionFact {
                id: cfg_function,
                body,
                function: handler,
                language: Language::Go,
                file,
                span: Span::point(file, 1, 1),
                entry_node: CfgNodeId(10_000),
                normal_exit_node: CfgNodeId(10_001),
                exceptional_exit_node: None,
                stable_key: interner.intern("cfg:function:handler"),
                status: CfgStatus::Resolved,
                precision: CfgPrecision::ExactLowered,
            }],
            nodes: cfg_nodes,
            blocks: cfg_blocks,
            dominators: dominator_facts,
            postdominators: postdominator_facts,
            ..CfgOutput::empty()
        })
        .expect("valid CFG facts");
        db.replace_call_facts(CallOutput {
            sites,
            targets,
            unresolved: Vec::new(),
        })
        .expect("valid call facts");
        db.replace_refined_call_facts(RefinedCallOutput { edges })
            .expect("valid refined call facts");
        db
    }

    fn data_flow_policy_db(edges: Vec<DataFlowEdgeFact>) -> AnalysisDb {
        data_flow_policy_db_with_sink_place(edges, PlaceId(1))
    }

    fn data_flow_policy_db_with_unresolved_logger_sink(edges: Vec<DataFlowEdgeFact>) -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/main.go"),
            "src/main.go".to_string(),
            "package main\nfunc handler() { log(input) }\n".to_string(),
        );
        let handler = push_test_function(&mut db, file, "handler", 1, false);
        let site = call_site_with_args(1, file, handler, "log", vec![PlaceId(1)]);
        let site_id = site.id;

        db.replace_call_facts(CallOutput {
            sites: vec![site],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("valid call facts");
        db.replace_refined_call_facts(RefinedCallOutput {
            edges: vec![unresolved_refined_edge(1, site_id, handler, "log")],
        })
        .expect("valid refined call facts");
        db.replace_data_flow_facts(DataFlowOutput {
            nodes: data_flow_nodes(file, handler),
            edges,
            models: vec![source_model()],
            budgets: Vec::new(),
        })
        .expect("valid data-flow facts");
        db
    }

    fn data_flow_policy_db_with_internal_synthetic_target(
        edges: Vec<DataFlowEdgeFact>,
    ) -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/main.go"),
            "src/main.go".to_string(),
            "package main\nfunc handler() { dangerous(input) }\n".to_string(),
        );
        let handler = push_test_function(&mut db, file, "handler", 1, false);
        let site = call_site_with_args(1, file, handler, "dangerous", vec![PlaceId(1)]);
        let mut edge = unresolved_refined_edge(1, site.id, handler, "dangerous");
        edge.synthetic_target = Some("ts-js:callable-place:1".to_string());

        db.replace_call_facts(CallOutput {
            sites: vec![site],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("valid call facts");
        db.replace_refined_call_facts(RefinedCallOutput { edges: vec![edge] })
            .expect("valid refined call facts");
        db.replace_data_flow_facts(DataFlowOutput {
            nodes: data_flow_nodes(file, handler),
            edges,
            models: vec![source_model()],
            budgets: Vec::new(),
        })
        .expect("valid data-flow facts");
        db
    }

    fn data_flow_policy_db_with_sink_place(
        edges: Vec<DataFlowEdgeFact>,
        sink_place: PlaceId,
    ) -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/main.go"),
            "src/main.go".to_string(),
            "package main\nfunc handler() { dangerous(input) }\n".to_string(),
        );
        let handler = push_test_function(&mut db, file, "handler", 1, false);
        let dangerous = push_test_function(&mut db, file, "dangerous", 2, false);
        let site = call_site_with_args(1, file, handler, "dangerous", vec![sink_place]);

        db.replace_call_facts(CallOutput {
            sites: vec![site.clone()],
            targets: vec![call_target(1, site.id, handler, dangerous)],
            unresolved: Vec::new(),
        })
        .expect("valid call facts");
        db.replace_refined_call_facts(RefinedCallOutput {
            edges: vec![refined_edge(1, site.id, handler, dangerous, "dangerous")],
        })
        .expect("valid refined call facts");
        db.replace_data_flow_facts(DataFlowOutput {
            nodes: data_flow_nodes(file, handler),
            edges,
            models: vec![source_model()],
            budgets: Vec::new(),
        })
        .expect("valid data-flow facts");
        db
    }

    fn data_flow_policy_db_with_two_sink_sites_same_place() -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/main.go"),
            "src/main.go".to_string(),
            "package main\nfunc handler() { dangerous(input); dangerous(input) }\n".to_string(),
        );
        let handler = push_test_function(&mut db, file, "handler", 1, false);
        let dangerous = push_test_function(&mut db, file, "dangerous", 2, false);
        let first_site = call_site_with_args(1, file, handler, "dangerous", vec![PlaceId(1)]);
        let second_site = call_site_with_args(2, file, handler, "dangerous", vec![PlaceId(1)]);

        db.replace_call_facts(CallOutput {
            sites: vec![first_site.clone(), second_site.clone()],
            targets: vec![
                call_target(1, first_site.id, handler, dangerous),
                call_target(2, second_site.id, handler, dangerous),
            ],
            unresolved: Vec::new(),
        })
        .expect("valid call facts");
        db.replace_refined_call_facts(RefinedCallOutput {
            edges: vec![
                refined_edge(1, first_site.id, handler, dangerous, "dangerous"),
                refined_edge(2, second_site.id, handler, dangerous, "dangerous"),
            ],
        })
        .expect("valid refined call facts");
        db.replace_data_flow_facts(DataFlowOutput {
            nodes: data_flow_nodes(file, handler),
            edges: vec![data_flow_edge(
                0,
                0,
                1,
                DataFlowEdgeKind::SourceIntroduction,
                DataFlowStatus::Present,
                Vec::new(),
            )],
            models: vec![source_model()],
            budgets: Vec::new(),
        })
        .expect("valid data-flow facts");
        db
    }

    fn data_flow_policy_db_with_barrier() -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/main.go"),
            "src/main.go".to_string(),
            "package main\nfunc handler() { dangerous(validate(input)) }\n".to_string(),
        );
        let handler = push_test_function(&mut db, file, "handler", 1, false);
        let dangerous = push_test_function(&mut db, file, "dangerous", 2, false);
        let validate = push_test_function(&mut db, file, "validate", 3, false);
        let dangerous_site = call_site_with_args(1, file, handler, "dangerous", vec![PlaceId(2)]);
        let validate_site = call_site_with_args(2, file, handler, "validate", vec![PlaceId(1)]);

        db.replace_call_facts(CallOutput {
            sites: vec![dangerous_site.clone(), validate_site.clone()],
            targets: vec![
                call_target(1, dangerous_site.id, handler, dangerous),
                call_target(2, validate_site.id, handler, validate),
            ],
            unresolved: Vec::new(),
        })
        .expect("valid call facts");
        db.replace_refined_call_facts(RefinedCallOutput {
            edges: vec![
                refined_edge(1, dangerous_site.id, handler, dangerous, "dangerous"),
                refined_edge(2, validate_site.id, handler, validate, "validate"),
            ],
        })
        .expect("valid refined call facts");
        let mut sanitizer_edge = data_flow_edge(
            1,
            1,
            2,
            DataFlowEdgeKind::CallArgumentToReturn,
            DataFlowStatus::Present,
            Vec::new(),
        );
        sanitizer_edge.call_site = Some(validate_site.id);
        db.replace_data_flow_facts(DataFlowOutput {
            nodes: data_flow_nodes(file, handler),
            edges: vec![
                data_flow_edge(
                    0,
                    0,
                    1,
                    DataFlowEdgeKind::SourceIntroduction,
                    DataFlowStatus::Present,
                    Vec::new(),
                ),
                sanitizer_edge,
            ],
            models: vec![source_model()],
            budgets: Vec::new(),
        })
        .expect("valid data-flow facts");
        db
    }

    fn data_flow_nodes(file: FileId, function: FunctionId) -> Vec<DataFlowNodeFact> {
        vec![
            DataFlowNodeFact {
                id: DataFlowNodeId(0),
                kind: DataFlowNodeKind::Source,
                language: Language::Go,
                file: Some(file),
                function: Some(function),
                body: None,
                operation: None,
                cfg_node: None,
                place: None,
                symbol: None,
                reference: None,
                call_site: None,
                model: Some(DataFlowModelId(0)),
                span: Some(Span::point(file, 1, 1)),
                stable_key: crate::core::stable_key_for_test("node:source"),
            },
            data_flow_place_node(1, file, function, PlaceId(1)),
            data_flow_place_node(2, file, function, PlaceId(2)),
        ]
    }

    fn data_flow_place_node(
        id: u64,
        file: FileId,
        function: FunctionId,
        place: PlaceId,
    ) -> DataFlowNodeFact {
        DataFlowNodeFact {
            id: DataFlowNodeId(id),
            kind: DataFlowNodeKind::Place,
            language: Language::Go,
            file: Some(file),
            function: Some(function),
            body: None,
            operation: None,
            cfg_node: None,
            place: Some(place),
            symbol: None,
            reference: None,
            call_site: None,
            model: None,
            span: Some(Span::point(file, id as u32 + 1, 1)),
            stable_key: crate::core::stable_key_for_test(&format!("node:place:{id}")),
        }
    }

    fn source_model() -> DataFlowModelFact {
        DataFlowModelFact {
            id: DataFlowModelId(0),
            kind: DataFlowModelKind::Source,
            language: Language::Go,
            provider_id: "test".to_string(),
            model_id: Some("QueryString".to_string()),
            source_stable_key: Some("trust-boundary:test".to_string()),
            status: DataFlowStatus::Present,
            precision: DataFlowPrecision::SetupAware,
            validation: DataFlowValidation::ReferentiallyValidated,
            confidence: DataFlowConfidence::High,
            provenance: DataFlowProvenance::Native,
            evidence: vec!["trust_boundary".to_string()],
            payload_labels: vec!["source_kind=QueryString".to_string()],
            stable_key: crate::core::stable_key_for_test("model:source"),
        }
    }

    fn data_flow_edge(
        id: u64,
        from: u64,
        to: u64,
        kind: DataFlowEdgeKind,
        status: DataFlowStatus,
        evidence: Vec<String>,
    ) -> DataFlowEdgeFact {
        DataFlowEdgeFact {
            id: DataFlowEdgeId(id),
            from: DataFlowNodeId(from),
            to: DataFlowNodeId(to),
            kind,
            algorithm: DataFlowAlgorithm::LocalMir,
            status,
            precision: if status == DataFlowStatus::Present {
                match kind {
                    DataFlowEdgeKind::LocalBinding
                    | DataFlowEdgeKind::LocalAssignment
                    | DataFlowEdgeKind::LocalUse
                    | DataFlowEdgeKind::LocalRead
                    | DataFlowEdgeKind::LocalWrite => DataFlowPrecision::Syntax,
                    _ => DataFlowPrecision::SetupAware,
                }
            } else {
                DataFlowPrecision::Unknown
            },
            validation: DataFlowValidation::Native,
            confidence: if status == DataFlowStatus::Present {
                DataFlowConfidence::High
            } else {
                DataFlowConfidence::Low
            },
            provenance: DataFlowProvenance::Native,
            call_site: None,
            call_target: None,
            refined_call: None,
            model: None,
            budget: None,
            evidence,
            input_stable_keys: Vec::new(),
            stable_key: crate::core::stable_key_for_test(&format!(
                "edge:{id}:{from}:{to}:{status:?}"
            )),
        }
    }

    fn mir_body(
        interner: &crate::core::StableKeyInterner,
        file: FileId,
        function: FunctionId,
    ) -> MirBody {
        MirBody {
            id: MirBodyId(function.0),
            language: Language::Go,
            file,
            function,
            package: None,
            module: None,
            owner_stable_key: interner.intern("function:handler".to_string()),
            span: Span::point(file, 1, 1),
            stable_key: interner.intern("mir:body:handler".to_string()),
            status: MirStatus::Resolved,
        }
    }

    fn mir_operation(
        interner: &crate::core::StableKeyInterner,
        operation: MirOpId,
        body: MirBodyId,
        ordinal: u32,
        file: FileId,
    ) -> MirOperation {
        MirOperation {
            id: operation,
            body,
            ordinal,
            span: Span::point(file, ordinal + 1, 1),
            kind: MirOperationKind::Branch {
                predicate: MirPredicateId(operation.0),
                predicate_place: None,
                nil_test: None,
            },
            stable_key: interner.intern(format!("mir:op:{:05}", operation.0)),
            status: MirStatus::Resolved,
        }
    }

    fn push_test_function(
        db: &mut AnalysisDb,
        file: FileId,
        name: &str,
        line: u32,
        is_test: bool,
    ) -> FunctionId {
        db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            name.to_string(),
            Span::point(file, line, 1),
            Language::Go,
            is_test,
            false,
            1,
            Vec::new(),
        ))
    }

    fn call_site(id: u64, file: FileId, caller: FunctionId, callee: &str) -> CallSiteFact {
        CallSiteFact {
            id: CallSiteId(id),
            language: Language::Go,
            file,
            caller,
            owner_symbol: None,
            body: MirBodyId(caller.0),
            operation: MirOpId(id),
            span: Span::point(file, id as u32 + 1, 1),
            kind: CallSyntaxKind::Function,
            callee: CallCallee::Identifier {
                reference: None,
                name: callee.to_string(),
            },
            receiver: None,
            arguments: Vec::new(),
            result: None,
            status: CallTargetStatus::Resolved,
            precision: CallPrecision::Exact,
            in_throw: false,
            stable_key: crate::core::StableKeyId(id as u32),
        }
    }

    fn call_site_with_args(
        id: u64,
        file: FileId,
        caller: FunctionId,
        callee: &str,
        arguments: Vec<PlaceId>,
    ) -> CallSiteFact {
        let mut site = call_site(id, file, caller, callee);
        site.arguments = arguments;
        site
    }

    fn call_target(
        id: u64,
        site: CallSiteId,
        caller: FunctionId,
        callee: FunctionId,
    ) -> CallTargetFact {
        CallTargetFact {
            id: CallTargetId(id),
            site,
            caller,
            target_function: Some(callee),
            target_symbol: None,
            edge_kind: CallEdgeKind::Direct,
            algorithm: CallAlgorithm::DirectReference,
            status: CallTargetStatus::Resolved,
            reason: None,
            provenance: CallProvenance::NativeDirect,
            precision: CallPrecision::SetupAware,
            stable_key: crate::core::StableKeyId(id as u32),
        }
    }

    fn refined_edge(
        id: u64,
        site: CallSiteId,
        caller: FunctionId,
        callee: FunctionId,
        label: &str,
    ) -> RefinedCallEdgeFact {
        RefinedCallEdgeFact {
            id: RefinedCallEdgeId(id),
            site,
            base_target: Some(CallTargetId(id)),
            caller,
            target_function: Some(callee),
            target_symbol: None,
            synthetic_target: None,
            language: Language::Go,
            edge_kind: CallEdgeKind::Direct,
            algorithm: CallAlgorithm::DirectReference,
            tier: RefinedCallTier::DirectOnly,
            status: CallTargetStatus::Resolved,
            reason: None::<UnresolvedCallReason>,
            provenance: CallProvenance::NativeDirect,
            precision: CallPrecision::SetupAware,
            validation: RefinedCallValidation::Native,
            confidence: RefinedCallConfidence::High,
            evidence: Vec::new(),
            input_stable_keys: Vec::new(),
            stable_key: crate::core::stable_key_for_test(&format!("refined:{id}:{label}")),
        }
    }

    fn unresolved_refined_edge(
        id: u64,
        site: CallSiteId,
        caller: FunctionId,
        label: &str,
    ) -> RefinedCallEdgeFact {
        RefinedCallEdgeFact {
            id: RefinedCallEdgeId(id),
            site,
            base_target: None,
            caller,
            target_function: None,
            target_symbol: None,
            synthetic_target: None,
            language: Language::Go,
            edge_kind: CallEdgeKind::Unknown,
            algorithm: CallAlgorithm::Unsupported,
            tier: RefinedCallTier::DirectOnly,
            status: CallTargetStatus::Unresolved,
            reason: Some(UnresolvedCallReason::UnknownCallee),
            provenance: CallProvenance::NativeDirect,
            precision: CallPrecision::Unknown,
            validation: RefinedCallValidation::Native,
            confidence: RefinedCallConfidence::High,
            evidence: Vec::new(),
            input_stable_keys: Vec::new(),
            stable_key: crate::core::stable_key_for_test(&format!(
                "refined:{id}:{label}:unresolved"
            )),
        }
    }
}
