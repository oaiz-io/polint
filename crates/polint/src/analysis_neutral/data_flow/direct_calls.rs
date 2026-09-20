use super::facts::{
    DataFlowAlgorithm, DataFlowConfidence, DataFlowEdgeFact, DataFlowEdgeKind, DataFlowNodeFact,
    DataFlowNodeKind, DataFlowPrecision, DataFlowProvenance, DataFlowStatus, DataFlowValidation,
};
use super::store::{DataFlowOutput, next_data_flow_edge_id, next_data_flow_node_id};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use crate::analysis_api::{FactFamily, stable_key_from_key_parts, stable_key_from_parts};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::calls::facts::{CallSiteFact, CallSyntaxKind, CallTargetStatus};
use crate::analysis_neutral::ids::{CallSiteId, DataFlowBudgetId, DataFlowNodeId, PlaceId};
use crate::analysis_neutral::refined_calls::facts::{RefinedCallConfidence, RefinedCallEdgeFact};
use crate::analysis_neutral::summaries::facts::{
    FlowRoot, SummaryDomainKind, SummaryFact, SummaryPrecision, SummaryStatus,
};
use crate::internal_core::{KeyPart, Language};

struct CallInputs<'a> {
    sites: BTreeMap<CallSiteId, &'a CallSiteFact>,
    summaries: BTreeMap<crate::internal_core::FunctionId, Vec<&'a SummaryFact>>,
}

impl<'a> CallInputs<'a> {
    fn new(db: &'a impl AnalysisHost) -> Self {
        let mut sites = BTreeMap::new();
        for site in db.call_sites() {
            sites.entry(site.id).or_insert(site);
        }
        let mut summaries: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for summary in db.summary_facts() {
            if summary.domain == SummaryDomainKind::DataFlowTito
                && summary.status == SummaryStatus::Present
            {
                summaries.entry(summary.function).or_default().push(summary);
            }
        }
        Self { sites, summaries }
    }
}

/// Append-aware indexes for the direct-call projection. The fact vectors keep
/// their original insertion order; hash tables are used only for lookup. Index
/// entries retain the first match, including for sparse or repeated input IDs.
struct CallProjection<'a> {
    facts: &'a mut DataFlowOutput,
    nodes_indexed: usize,
    edges_indexed: usize,
    nodes_by_place: HashMap<PlaceId, DataFlowNodeId>,
    nodes_by_key: HashMap<crate::internal_core::StableKeyId, DataFlowNodeId>,
    keys_by_node: HashMap<DataFlowNodeId, crate::internal_core::StableKeyId>,
    edge_keys: HashSet<crate::internal_core::StableKeyId>,
}

impl<'a> CallProjection<'a> {
    fn new(facts: &'a mut DataFlowOutput) -> Self {
        Self {
            facts,
            nodes_indexed: 0,
            edges_indexed: 0,
            nodes_by_place: HashMap::new(),
            nodes_by_key: HashMap::new(),
            keys_by_node: HashMap::new(),
            edge_keys: HashSet::new(),
        }
    }

    fn refresh_nodes(&mut self) {
        for node in &self.facts.nodes[self.nodes_indexed..] {
            if let Some(place) = node.place {
                self.nodes_by_place.entry(place).or_insert(node.id);
            }
            self.nodes_by_key.entry(node.stable_key).or_insert(node.id);
            self.keys_by_node.entry(node.id).or_insert(node.stable_key);
        }
        self.nodes_indexed = self.facts.nodes.len();
    }

    fn existing_node(&mut self, key: crate::internal_core::StableKeyId) -> Option<DataFlowNodeId> {
        self.refresh_nodes();
        self.nodes_by_key.get(&key).copied()
    }

    fn has_edge(&mut self, key: crate::internal_core::StableKeyId) -> bool {
        self.edge_keys.extend(
            self.facts.edges[self.edges_indexed..]
                .iter()
                .map(|edge| edge.stable_key),
        );
        self.edges_indexed = self.facts.edges.len();
        self.edge_keys.contains(&key)
    }
}

pub fn derive_direct_call_edges(db: &impl AnalysisHost, output: &mut DataFlowOutput) {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let inputs = CallInputs::new(db);
    let mut output = CallProjection::new(output);
    for edge in db.refined_call_edges() {
        if edge.status == CallTargetStatus::Resolved {
            derive_resolved_call_edge(interner, &inputs, &mut output, edge);
        } else {
            derive_unresolved_call_edge(interner, &mut output, edge);
        }
    }
}

fn derive_resolved_call_edge(
    interner: &crate::internal_core::StableKeyInterner,
    inputs: &CallInputs<'_>,
    output: &mut CallProjection<'_>,
    edge: &RefinedCallEdgeFact,
) {
    let site = inputs.sites.get(&edge.site).copied();
    let site_id = site.map(|site| site.id);
    let callee_input = call_node(
        interner,
        output,
        DataFlowNodeKind::SummaryInput,
        edge,
        "callee-input".to_string(),
        site_id,
    );
    let callee_output = call_node(
        interner,
        output,
        DataFlowNodeKind::SummaryOutput,
        edge,
        "callee-output".to_string(),
        site_id,
    );

    let argument_nodes = argument_nodes(output, edge, site);
    for (index, argument) in argument_nodes.iter().copied() {
        push_edge(
            interner,
            output,
            CallEdgeDraft {
                from: argument,
                to: callee_input,
                kind: DataFlowEdgeKind::CallArgumentToParameter,
                status: DataFlowStatus::Present,
                precision: precision(edge),
                validation: DataFlowValidation::ReferentiallyValidated,
                budget: None,
                evidence: vec![
                    "direct_call_argument_boundary".to_string(),
                    format!("argument_index={index}"),
                ],
                extra_input_stable_keys: Vec::new(),
                call_site: site_id,
                edge,
            },
        );
    }
    if let Some(receiver) = receiver_node(output, site) {
        push_edge(
            interner,
            output,
            CallEdgeDraft {
                from: receiver,
                to: callee_input,
                kind: DataFlowEdgeKind::ReceiverToMethod,
                status: DataFlowStatus::Present,
                precision: precision(edge),
                validation: DataFlowValidation::ReferentiallyValidated,
                budget: None,
                evidence: vec!["direct_call_receiver_boundary".to_string()],
                extra_input_stable_keys: Vec::new(),
                call_site: site_id,
                edge,
            },
        );
    }
    bridge_target_summaries(interner, inputs, output, edge, site, site_id);
    if let Some(returned) = return_node(output, site) {
        push_edge(
            interner,
            output,
            CallEdgeDraft {
                from: callee_output,
                to: returned,
                kind: DataFlowEdgeKind::CallReturnToUse,
                status: DataFlowStatus::Present,
                precision: DataFlowPrecision::Conservative,
                validation: DataFlowValidation::ReferentiallyValidated,
                budget: None,
                evidence: vec!["direct_call_return_boundary".to_string()],
                extra_input_stable_keys: Vec::new(),
                call_site: site_id,
                edge,
            },
        );
    }
}

fn bridge_target_summaries(
    interner: &crate::internal_core::StableKeyInterner,
    inputs: &CallInputs<'_>,
    output: &mut CallProjection<'_>,
    edge: &RefinedCallEdgeFact,
    site: Option<&CallSiteFact>,
    call_site: Option<CallSiteId>,
) {
    let Some(target_function) = edge.target_function else {
        return;
    };

    for summary in inputs.summaries.get(&target_function).into_iter().flatten() {
        for flow in &summary.tito_flows {
            if !super::summary_edges::flow_kind_projects_as_tito(flow.kind) {
                continue;
            }
            let Some(from) = call_root_node(output, edge, site, flow.from) else {
                continue;
            };
            let Some(to) = call_root_node(output, edge, site, flow.to) else {
                continue;
            };
            let summary_input = summary_node(
                interner,
                output,
                summary,
                DataFlowNodeKind::SummaryInput,
                &super::summary_edges::root_role(flow.from),
                call_site,
            );
            let summary_output = summary_node(
                interner,
                output,
                summary,
                DataFlowNodeKind::SummaryOutput,
                &super::summary_edges::root_role(flow.to),
                call_site,
            );
            let summary_inputs = vec![
                interner.resolve(summary.stable_key),
                interner.resolve(summary.callable_stable_key),
            ];
            push_edge(
                interner,
                output,
                CallEdgeDraft {
                    from,
                    to: summary_input,
                    kind: DataFlowEdgeKind::SummaryProjected,
                    status: DataFlowStatus::Present,
                    precision: summary_precision(summary.precision),
                    validation: DataFlowValidation::ReferentiallyValidated,
                    budget: None,
                    evidence: vec![
                        "direct_call_summary_input_bridge".to_string(),
                        format!("summary={}", interner.resolve(summary.stable_key)),
                        format!("flow_from={}", super::summary_edges::root_role(flow.from)),
                    ],
                    extra_input_stable_keys: summary_inputs.clone(),
                    call_site,
                    edge,
                },
            );
            push_call_summary_tito_edge(
                interner,
                output,
                edge,
                summary,
                flow,
                summary_input,
                (summary_output, call_site),
            );
            push_edge(
                interner,
                output,
                CallEdgeDraft {
                    from: summary_output,
                    to,
                    kind: DataFlowEdgeKind::SummaryProjected,
                    status: DataFlowStatus::Present,
                    precision: summary_precision(summary.precision),
                    validation: DataFlowValidation::ReferentiallyValidated,
                    budget: None,
                    evidence: vec![
                        "direct_call_summary_output_bridge".to_string(),
                        format!("summary={}", interner.resolve(summary.stable_key)),
                        format!("flow_to={}", super::summary_edges::root_role(flow.to)),
                    ],
                    extra_input_stable_keys: summary_inputs,
                    call_site,
                    edge,
                },
            );
        }
    }
}

fn argument_nodes(
    output: &mut CallProjection<'_>,
    _edge: &RefinedCallEdgeFact,
    site: Option<&CallSiteFact>,
) -> Vec<(usize, DataFlowNodeId)> {
    site.into_iter()
        .flat_map(|site| site.arguments.iter().copied())
        .enumerate()
        .filter_map(|(index, place)| place_node(output, place).map(|node| (index, node)))
        .collect::<Vec<_>>()
}

fn call_root_node(
    output: &mut CallProjection<'_>,
    edge: &RefinedCallEdgeFact,
    site: Option<&CallSiteFact>,
    root: FlowRoot,
) -> Option<DataFlowNodeId> {
    match root {
        FlowRoot::Param(index) => {
            let site = site?;
            if edge.language == Language::Go
                && site.kind == CallSyntaxKind::Method
                && site.receiver.is_some()
            {
                if index == 0 {
                    return receiver_node(output, Some(site));
                }
                return site
                    .arguments
                    .get((index - 1) as usize)
                    .copied()
                    .and_then(|place| place_node(output, place));
            }
            site.arguments
                .get(index as usize)
                .copied()
                .and_then(|place| place_node(output, place))
        }
        FlowRoot::Receiver => receiver_node(output, site),
        FlowRoot::Return => return_node(output, site),
    }
}

fn receiver_node(
    output: &mut CallProjection<'_>,
    site: Option<&CallSiteFact>,
) -> Option<DataFlowNodeId> {
    let receiver = site.and_then(|site| site.receiver)?;
    place_node(output, receiver)
}

fn return_node(
    output: &mut CallProjection<'_>,
    site: Option<&CallSiteFact>,
) -> Option<DataFlowNodeId> {
    site.and_then(|site| site.result)
        .and_then(|place| place_node(output, place))
}

fn place_node(output: &mut CallProjection<'_>, place: PlaceId) -> Option<DataFlowNodeId> {
    output.refresh_nodes();
    output.nodes_by_place.get(&place).copied()
}

fn derive_unresolved_call_edge(
    interner: &crate::internal_core::StableKeyInterner,
    output: &mut CallProjection<'_>,
    edge: &RefinedCallEdgeFact,
) {
    let source = call_node(
        interner,
        output,
        DataFlowNodeKind::CallArgument,
        edge,
        "unresolved-argument".to_string(),
        None,
    );
    let sink = call_node(
        interner,
        output,
        DataFlowNodeKind::Synthetic,
        edge,
        "unresolved-call".to_string(),
        None,
    );
    let status = unresolved_status(edge.status);
    let budget = (status == DataFlowStatus::BudgetExceeded).then(|| {
        super::local::budget_fact(
            interner,
            super::facts::DataFlowBudgetReason::PathCount,
            1,
            2,
            &interner.resolve(edge.stable_key),
            output.facts,
        )
    });
    push_edge(
        interner,
        output,
        CallEdgeDraft {
            from: source,
            to: sink,
            kind: unresolved_kind(status),
            status,
            precision: DataFlowPrecision::Unknown,
            validation: unresolved_validation(status, budget),
            budget,
            evidence: vec![
                "refined_call_unresolved".to_string(),
                edge.reason
                    .map(|reason| format!("reason={reason:?}"))
                    .unwrap_or_else(|| "reason=none".to_string()),
            ],
            extra_input_stable_keys: Vec::new(),
            call_site: None,
            edge,
        },
    );
}

struct CallEdgeDraft<'a> {
    from: DataFlowNodeId,
    to: DataFlowNodeId,
    kind: DataFlowEdgeKind,
    status: DataFlowStatus,
    precision: DataFlowPrecision,
    validation: DataFlowValidation,
    budget: Option<DataFlowBudgetId>,
    evidence: Vec<String>,
    extra_input_stable_keys: Vec<Arc<str>>,
    call_site: Option<CallSiteId>,
    edge: &'a RefinedCallEdgeFact,
}

fn push_edge(
    interner: &crate::internal_core::StableKeyInterner,
    output: &mut CallProjection<'_>,
    draft: CallEdgeDraft<'_>,
) {
    let stable_key = stable_key_from_key_parts(
        interner,
        FactFamily::DataFlowEdge,
        [
            ("kind", KeyPart::Text(&format!("{:?}", draft.kind))),
            ("refined_call", KeyPart::Key(draft.edge.stable_key)),
            (
                "from",
                KeyPart::Text(&node_key(interner, output, draft.from)),
            ),
            ("to", KeyPart::Text(&node_key(interner, output, draft.to))),
            ("status", KeyPart::Text(&format!("{:?}", draft.status))),
        ],
    );
    if output.has_edge(stable_key) {
        return;
    }
    output.facts.edges.push(DataFlowEdgeFact {
        id: next_data_flow_edge_id(&output.facts.edges),
        from: draft.from,
        to: draft.to,
        kind: draft.kind,
        algorithm: DataFlowAlgorithm::DirectCall,
        status: draft.status,
        precision: draft.precision,
        validation: draft.validation,
        confidence: confidence(draft.edge),
        provenance: DataFlowProvenance::Native,
        call_site: draft.call_site,
        call_target: draft.edge.base_target,
        refined_call: Some(draft.edge.id),
        model: None,
        budget: draft.budget,
        evidence: draft.evidence,
        input_stable_keys: {
            let mut keys: Vec<Arc<str>> = draft
                .edge
                .input_stable_keys
                .iter()
                .map(|key| Arc::from(key.as_str()))
                .collect();
            keys.push(interner.resolve(draft.edge.stable_key));
            keys.extend(draft.extra_input_stable_keys);
            keys
        },
        stable_key,
    });
}

fn summary_node(
    interner: &crate::internal_core::StableKeyInterner,
    output: &mut CallProjection<'_>,
    fact: &SummaryFact,
    kind: DataFlowNodeKind,
    role: &str,
    call_site: Option<CallSiteId>,
) -> DataFlowNodeId {
    let stable_key = stable_key_from_key_parts(
        interner,
        FactFamily::DataFlowNode,
        [
            ("kind", KeyPart::Text(&format!("{kind:?}"))),
            ("summary", KeyPart::Key(fact.stable_key)),
            ("role", KeyPart::Text(role)),
            (
                "call_site",
                KeyPart::Text(
                    &call_site
                        .map(|id| id.0.to_string())
                        .unwrap_or_else(|| "none".to_string()),
                ),
            ),
        ],
    );
    if let Some(existing) = output.existing_node(stable_key) {
        return existing;
    }
    let id = next_data_flow_node_id(&output.facts.nodes);
    output.facts.nodes.push(DataFlowNodeFact {
        id,
        kind,
        language: Language::Unknown,
        file: None,
        function: Some(fact.function),
        body: None,
        operation: None,
        cfg_node: None,
        place: None,
        symbol: None,
        reference: None,
        call_site,
        model: None,
        span: None,
        stable_key,
    });
    id
}

fn push_call_summary_tito_edge(
    interner: &crate::internal_core::StableKeyInterner,
    output: &mut CallProjection<'_>,
    edge: &RefinedCallEdgeFact,
    summary: &SummaryFact,
    flow: &crate::analysis_neutral::summaries::facts::SummaryFlowEdge,
    from: DataFlowNodeId,
    pair: (DataFlowNodeId, Option<CallSiteId>),
) {
    let (to, call_site) = pair;
    let stable_key = stable_key_from_key_parts(
        interner,
        FactFamily::DataFlowEdge,
        [
            (
                "kind",
                KeyPart::Text(&format!("{:?}", DataFlowEdgeKind::SummaryTito)),
            ),
            ("refined_call", KeyPart::Key(edge.stable_key)),
            ("summary", KeyPart::Key(summary.stable_key)),
            ("from", KeyPart::Text(&node_key(interner, output, from))),
            ("to", KeyPart::Text(&node_key(interner, output, to))),
            (
                "flow_from",
                KeyPart::Text(&super::summary_edges::root_role(flow.from)),
            ),
            (
                "flow_to",
                KeyPart::Text(&super::summary_edges::root_role(flow.to)),
            ),
            ("flow_kind", KeyPart::Text(&format!("{:?}", flow.kind))),
        ],
    );
    if output.has_edge(stable_key) {
        return;
    }
    output.facts.edges.push(DataFlowEdgeFact {
        id: next_data_flow_edge_id(&output.facts.edges),
        from,
        to,
        kind: DataFlowEdgeKind::SummaryTito,
        algorithm: DataFlowAlgorithm::SummaryProjection,
        status: DataFlowStatus::Present,
        precision: summary_precision(summary.precision),
        validation: DataFlowValidation::ReferentiallyValidated,
        confidence: DataFlowConfidence::Medium,
        provenance: DataFlowProvenance::Summary,
        call_site,
        call_target: edge.base_target,
        refined_call: Some(edge.id),
        model: None,
        budget: None,
        evidence: vec![
            "direct_call_summary_data_flow_tito".to_string(),
            format!("summary={}", interner.resolve(summary.stable_key)),
            format!(
                "flow={}->{}:{:?}",
                super::summary_edges::root_role(flow.from),
                super::summary_edges::root_role(flow.to),
                flow.kind
            ),
        ],
        input_stable_keys: vec![
            interner.resolve(edge.stable_key),
            interner.resolve(summary.stable_key),
            interner.resolve(summary.callable_stable_key),
        ],
        stable_key,
    });
}

fn unresolved_status(status: CallTargetStatus) -> DataFlowStatus {
    match status {
        CallTargetStatus::Resolved => DataFlowStatus::Present,
        CallTargetStatus::Ambiguous | CallTargetStatus::Unresolved => DataFlowStatus::Unknown,
        CallTargetStatus::Unsupported => DataFlowStatus::Unsupported,
        CallTargetStatus::SetupMissing => DataFlowStatus::SetupMissing,
        CallTargetStatus::BudgetExceeded => DataFlowStatus::BudgetExceeded,
        CallTargetStatus::Rejected => DataFlowStatus::Rejected,
    }
}

fn unresolved_kind(status: DataFlowStatus) -> DataFlowEdgeKind {
    match status {
        DataFlowStatus::BudgetExceeded => DataFlowEdgeKind::BudgetTruncated,
        DataFlowStatus::Unsupported => DataFlowEdgeKind::HavocFlow,
        DataFlowStatus::Present
        | DataFlowStatus::Unknown
        | DataFlowStatus::SetupMissing
        | DataFlowStatus::Rejected => DataFlowEdgeKind::UnknownFlow,
    }
}

fn unresolved_validation(
    status: DataFlowStatus,
    budget: Option<DataFlowBudgetId>,
) -> DataFlowValidation {
    match status {
        DataFlowStatus::BudgetExceeded if budget.is_some() => DataFlowValidation::BudgetValidated,
        DataFlowStatus::Rejected => DataFlowValidation::Rejected,
        _ => DataFlowValidation::Native,
    }
}

fn node_key(
    interner: &crate::internal_core::StableKeyInterner,
    output: &mut CallProjection<'_>,
    node: DataFlowNodeId,
) -> String {
    output.refresh_nodes();
    output
        .keys_by_node
        .get(&node)
        .map(|key| interner.resolve(*key).to_string())
        .unwrap_or_else(|| format!("node:{}", node.0))
}

fn call_node(
    interner: &crate::internal_core::StableKeyInterner,
    output: &mut CallProjection<'_>,
    kind: DataFlowNodeKind,
    edge: &RefinedCallEdgeFact,
    suffix: String,
    call_site: Option<CallSiteId>,
) -> DataFlowNodeId {
    let stable_key = stable_key_from_key_parts(
        interner,
        FactFamily::DataFlowNode,
        [
            ("kind", KeyPart::Text(&format!("{kind:?}"))),
            ("refined_call", KeyPart::Key(edge.stable_key)),
            ("node", KeyPart::Text(&suffix)),
        ],
    );
    if let Some(existing) = output.existing_node(stable_key) {
        return existing;
    }
    let id = next_data_flow_node_id(&output.facts.nodes);
    output.facts.nodes.push(DataFlowNodeFact {
        id,
        kind,
        language: edge.language,
        file: None,
        function: Some(edge.caller),
        body: None,
        operation: None,
        cfg_node: None,
        place: None,
        symbol: edge.target_symbol,
        reference: None,
        call_site,
        model: None,
        span: None,
        stable_key,
    });
    id
}

fn precision(edge: &RefinedCallEdgeFact) -> DataFlowPrecision {
    match edge.precision {
        crate::analysis_neutral::calls::facts::CallPrecision::Exact => DataFlowPrecision::Exact,
        crate::analysis_neutral::calls::facts::CallPrecision::SetupAware => {
            DataFlowPrecision::SetupAware
        }
        crate::analysis_neutral::calls::facts::CallPrecision::Conservative => {
            DataFlowPrecision::Conservative
        }
        crate::analysis_neutral::calls::facts::CallPrecision::Heuristic => {
            DataFlowPrecision::Heuristic
        }
        crate::analysis_neutral::calls::facts::CallPrecision::Ambiguous
        | crate::analysis_neutral::calls::facts::CallPrecision::Unknown => {
            DataFlowPrecision::Unknown
        }
        crate::analysis_neutral::calls::facts::CallPrecision::Unsupported => {
            DataFlowPrecision::Unknown
        }
    }
}

fn confidence(edge: &RefinedCallEdgeFact) -> DataFlowConfidence {
    match edge.confidence {
        RefinedCallConfidence::High => DataFlowConfidence::High,
        RefinedCallConfidence::Medium => DataFlowConfidence::Medium,
        RefinedCallConfidence::Low => DataFlowConfidence::Low,
    }
}

fn summary_precision(precision: SummaryPrecision) -> DataFlowPrecision {
    match precision {
        SummaryPrecision::Local => DataFlowPrecision::Syntax,
        SummaryPrecision::SetupAware => DataFlowPrecision::SetupAware,
        SummaryPrecision::Heuristic => DataFlowPrecision::Heuristic,
        SummaryPrecision::UnknownTop => DataFlowPrecision::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_api::FunctionFact;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::calls::facts::{
        CallAlgorithm, CallCallee, CallEdgeKind, CallPrecision, CallProvenance, CallSiteFact,
        CallSyntaxKind, UnresolvedCallReason,
    };
    use crate::analysis_neutral::calls::store::CallOutput;
    use crate::analysis_neutral::data_flow::store::DataFlowStore;
    use crate::analysis_neutral::ids::{
        CallSiteId, DataFlowNodeId, MirBodyId, MirOpId, PlaceId, RefinedCallEdgeId, SummaryId,
    };
    use crate::analysis_neutral::ifds::{
        DataFlowPathStatus, DataFlowSearchBudget, find_taint_paths,
    };
    use crate::analysis_neutral::refined_calls::facts::{
        RefinedCallEdgeFact, RefinedCallTier, RefinedCallValidation,
    };
    use crate::analysis_neutral::refined_calls::store::RefinedCallOutput;
    use crate::analysis_neutral::summaries::facts::{
        FlowKind, FlowRoot, SummaryDomainKind, SummaryFact, SummaryFlowEdge, SummaryPrecision,
        SummaryProvenance, SummaryStatus,
    };
    use crate::analysis_neutral::summaries::store::SummaryOutput;
    use crate::internal_core::{FileId, FunctionId, Language, Span, stable_key_for_test};

    fn test_db() -> LocalAnalysisDb {
        let mut db = LocalAnalysisDb::new();
        db.add_file("padding.ts".into(), "padding.ts".to_string(), String::new());
        let file = db.add_file("app.ts".into(), "app.ts".to_string(), String::new());
        for index in 0..=5 {
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
        db
    }

    #[test]
    fn projection_indexes_preserve_first_matches_as_facts_are_appended() {
        let interner = crate::internal_core::test_stable_key_interner();
        let first = place_node(50, PlaceId(10));
        let first_key = first.stable_key;
        let mut facts = DataFlowOutput {
            nodes: vec![first],
            ..DataFlowOutput::empty()
        };
        let mut output = CallProjection::new(&mut facts);
        assert_eq!(output.existing_node(first_key), Some(DataFlowNodeId(50)));
        let second = place_node(70, PlaceId(20));
        let second_key = second.stable_key;
        output.facts.nodes.push(second);
        assert_eq!(output.existing_node(second_key), Some(DataFlowNodeId(70)));
        let mut repeated = place_node(50, PlaceId(10));
        repeated.stable_key = interner.intern("later identity for repeated id");
        output.facts.nodes.push(repeated);
        output.refresh_nodes();
        assert_eq!(output.nodes_by_place[&PlaceId(10)], DataFlowNodeId(50));
        assert_eq!(output.keys_by_node[&DataFlowNodeId(50)], first_key);
        assert_eq!(
            node_key(&interner, &mut output, DataFlowNodeId(50)),
            interner.resolve(first_key).as_ref()
        );
    }

    #[test]
    fn resolved_refined_call_creates_role_specific_edges() {
        let mut db = test_db();
        db.replace_call_facts(CallOutput {
            sites: vec![call_site(Some(PlaceId(20)))],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("valid call facts");
        let mut output = DataFlowOutput {
            nodes: vec![
                place_node(10, PlaceId(10)),
                place_node(20, PlaceId(20)),
                place_node(30, PlaceId(30)),
            ],
            edges: Vec::new(),
            models: Vec::new(),
            budgets: Vec::new(),
        };
        derive_resolved_call_edge(
            &db.stable_key_interner(),
            &CallInputs::new(&db),
            &mut CallProjection::new(&mut output),
            &refined_edge(CallTargetStatus::Resolved),
        );

        assert!(output.edges.iter().any(|edge| {
            edge.kind == DataFlowEdgeKind::CallArgumentToParameter
                && edge.refined_call == Some(RefinedCallEdgeId(1))
                && edge.call_site == Some(CallSiteId(2))
        }));
        assert!(
            output
                .edges
                .iter()
                .any(|edge| edge.kind == DataFlowEdgeKind::ReceiverToMethod)
        );
        assert!(
            output
                .edges
                .iter()
                .any(|edge| edge.kind == DataFlowEdgeKind::CallReturnToUse
                    && edge.from != DataFlowNodeId(10)
                    && edge.from != DataFlowNodeId(20)
                    && edge.to == DataFlowNodeId(30))
        );
    }

    #[test]
    fn unresolved_refined_call_creates_unknown_row() {
        let mut output = DataFlowOutput::empty();
        derive_unresolved_call_edge(
            &crate::analysis_neutral::LocalAnalysisDb::new().stable_key_interner(),
            &mut CallProjection::new(&mut output),
            &refined_edge(CallTargetStatus::Unresolved),
        );

        let edge = output
            .edges
            .iter()
            .find(|edge| edge.kind == DataFlowEdgeKind::UnknownFlow)
            .expect("unknown edge");
        assert_eq!(edge.status, DataFlowStatus::Unknown);
        assert!(edge.evidence.iter().any(|value| value.contains("reason=")));
    }

    #[test]
    fn direct_call_edges_use_real_call_site_place_nodes_and_skip_missing_receiver() {
        let mut db = test_db();
        db.replace_call_facts(CallOutput {
            sites: vec![call_site(None)],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("valid call facts");
        db.replace_refined_call_facts(RefinedCallOutput {
            edges: vec![refined_edge(CallTargetStatus::Resolved)],
        })
        .expect("valid refined calls");
        let mut output = DataFlowOutput {
            nodes: vec![
                place_node(10, PlaceId(10)),
                place_node(20, PlaceId(20)),
                place_node(30, PlaceId(30)),
            ],
            edges: Vec::new(),
            models: Vec::new(),
            budgets: Vec::new(),
        };

        derive_direct_call_edges(&db, &mut output);

        assert!(output.edges.iter().any(|edge| {
            edge.kind == DataFlowEdgeKind::CallArgumentToParameter
                && edge.from == DataFlowNodeId(10)
        }));
        assert!(output.edges.iter().any(|edge| {
            edge.kind == DataFlowEdgeKind::CallReturnToUse && edge.to == DataFlowNodeId(30)
        }));
        assert!(
            !output
                .edges
                .iter()
                .any(|edge| edge.kind == DataFlowEdgeKind::ReceiverToMethod)
        );
    }

    #[test]
    fn direct_call_edges_emit_receiver_only_for_real_receiver_place() {
        let mut db = test_db();
        db.replace_call_facts(CallOutput {
            sites: vec![call_site(Some(PlaceId(20)))],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("valid call facts");
        db.replace_refined_call_facts(RefinedCallOutput {
            edges: vec![refined_edge(CallTargetStatus::Resolved)],
        })
        .expect("valid refined calls");
        let mut output = DataFlowOutput {
            nodes: vec![
                place_node(10, PlaceId(10)),
                place_node(20, PlaceId(20)),
                place_node(30, PlaceId(30)),
            ],
            edges: Vec::new(),
            models: Vec::new(),
            budgets: Vec::new(),
        };

        derive_direct_call_edges(&db, &mut output);

        assert!(output.edges.iter().any(|edge| {
            edge.kind == DataFlowEdgeKind::ReceiverToMethod && edge.from == DataFlowNodeId(20)
        }));
    }

    #[test]
    fn resolved_refined_call_bridges_through_target_data_flow_summary() {
        let mut db = test_db();
        db.replace_call_facts(CallOutput {
            sites: vec![call_site(None)],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("valid call facts");
        db.replace_refined_call_facts(RefinedCallOutput {
            edges: vec![refined_edge(CallTargetStatus::Resolved)],
        })
        .expect("valid refined calls");
        db.replace_summary_facts(SummaryOutput {
            summaries: vec![summary_fact()],
            events: Vec::new(),
        });
        let mut output = DataFlowOutput {
            nodes: vec![
                place_node(10, PlaceId(10)),
                place_node(20, PlaceId(20)),
                place_node(30, PlaceId(30)),
            ],
            edges: Vec::new(),
            models: Vec::new(),
            budgets: Vec::new(),
        };

        derive_direct_call_edges(&db, &mut output);
        super::super::summary_edges::derive_summary_projected_edges(&db, &mut output);
        let store =
            DataFlowStore::from_output(output, &crate::internal_core::test_stable_key_interner())
                .expect("valid store");
        let source = store
            .nodes()
            .iter()
            .find(|node| node.place == Some(PlaceId(10)))
            .expect("argument node")
            .id;
        let sink = store
            .nodes()
            .iter()
            .find(|node| node.place == Some(PlaceId(30)))
            .expect("return node")
            .id;
        let paths = find_taint_paths(
            &db,
            &store,
            source,
            sink,
            &std::collections::BTreeSet::new(),
            DataFlowSearchBudget {
                max_depth: 8,
                max_paths: 4,
            },
        );

        assert_eq!(paths[0].status, DataFlowPathStatus::Found);
        let found_edges = paths[0]
            .edges
            .iter()
            .filter_map(|id| store.edges().iter().find(|edge| edge.id == *id))
            .collect::<Vec<_>>();
        assert!(
            found_edges
                .iter()
                .any(|edge| edge.kind == DataFlowEdgeKind::SummaryTito),
            "call path should cross the target TITO summary edge: {found_edges:#?}"
        );
        assert!(
            found_edges
                .iter()
                .filter(|edge| edge.kind == DataFlowEdgeKind::SummaryProjected)
                .count()
                >= 2,
            "call boundary must connect to both summary input and output"
        );
    }

    #[test]
    fn target_data_flow_summary_only_bridges_matching_argument_root() {
        let mut db = test_db();
        db.replace_call_facts(CallOutput {
            sites: vec![call_site(None)],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("valid call facts");
        db.replace_refined_call_facts(RefinedCallOutput {
            edges: vec![refined_edge(CallTargetStatus::Resolved)],
        })
        .expect("valid refined calls");
        db.replace_summary_facts(SummaryOutput {
            summaries: vec![summary_fact()],
            events: Vec::new(),
        });
        let mut output = DataFlowOutput {
            nodes: vec![
                place_node(10, PlaceId(10)),
                place_node(20, PlaceId(20)),
                place_node(30, PlaceId(30)),
            ],
            edges: Vec::new(),
            models: Vec::new(),
            budgets: Vec::new(),
        };

        derive_direct_call_edges(&db, &mut output);
        super::super::summary_edges::derive_summary_projected_edges(&db, &mut output);
        let store =
            DataFlowStore::from_output(output, &crate::internal_core::test_stable_key_interner())
                .expect("valid store");
        let unrelated_argument = store
            .nodes()
            .iter()
            .find(|node| node.place == Some(PlaceId(20)))
            .expect("second argument node")
            .id;
        let sink = store
            .nodes()
            .iter()
            .find(|node| node.place == Some(PlaceId(30)))
            .expect("return node")
            .id;
        let paths = find_taint_paths(
            &db,
            &store,
            unrelated_argument,
            sink,
            &std::collections::BTreeSet::new(),
            DataFlowSearchBudget {
                max_depth: 8,
                max_paths: 4,
            },
        );

        assert_eq!(paths[0].status, DataFlowPathStatus::NotFound);
    }

    #[test]
    fn go_method_param_zero_summary_bridges_receiver_not_first_argument() {
        let mut db = test_db();
        db.replace_call_facts(CallOutput {
            sites: vec![go_method_call_site()],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("valid call facts");
        db.replace_refined_call_facts(RefinedCallOutput {
            edges: vec![RefinedCallEdgeFact {
                language: Language::Go,
                ..refined_edge(CallTargetStatus::Resolved)
            }],
        })
        .expect("valid refined calls");
        db.replace_summary_facts(SummaryOutput {
            summaries: vec![summary_fact()],
            events: Vec::new(),
        });
        let mut output = DataFlowOutput {
            nodes: vec![
                place_node(10, PlaceId(10)),
                place_node(20, PlaceId(20)),
                place_node(30, PlaceId(30)),
            ],
            edges: Vec::new(),
            models: Vec::new(),
            budgets: Vec::new(),
        };

        derive_direct_call_edges(&db, &mut output);
        super::super::summary_edges::derive_summary_projected_edges(&db, &mut output);
        let store =
            DataFlowStore::from_output(output, &crate::internal_core::test_stable_key_interner())
                .expect("valid store");
        let receiver = store
            .nodes()
            .iter()
            .find(|node| node.place == Some(PlaceId(20)))
            .expect("receiver node")
            .id;
        let first_argument = store
            .nodes()
            .iter()
            .find(|node| node.place == Some(PlaceId(10)))
            .expect("argument node")
            .id;
        let sink = store
            .nodes()
            .iter()
            .find(|node| node.place == Some(PlaceId(30)))
            .expect("return node")
            .id;

        let receiver_paths = find_taint_paths(
            &db,
            &store,
            receiver,
            sink,
            &std::collections::BTreeSet::new(),
            DataFlowSearchBudget {
                max_depth: 8,
                max_paths: 4,
            },
        );
        let argument_paths = find_taint_paths(
            &db,
            &store,
            first_argument,
            sink,
            &std::collections::BTreeSet::new(),
            DataFlowSearchBudget {
                max_depth: 8,
                max_paths: 4,
            },
        );

        assert_eq!(receiver_paths[0].status, DataFlowPathStatus::Found);
        assert_eq!(argument_paths[0].status, DataFlowPathStatus::NotFound);
    }

    #[test]
    fn barrier_summary_flow_does_not_bridge_direct_call_path() {
        let mut db = test_db();
        db.replace_call_facts(CallOutput {
            sites: vec![call_site(None)],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("valid call facts");
        db.replace_refined_call_facts(RefinedCallOutput {
            edges: vec![refined_edge(CallTargetStatus::Resolved)],
        })
        .expect("valid refined calls");
        let mut summary = summary_fact();
        summary.tito_flows[0].kind = FlowKind::Barrier;
        db.replace_summary_facts(SummaryOutput {
            summaries: vec![summary],
            events: Vec::new(),
        });
        let mut output = DataFlowOutput {
            nodes: vec![
                place_node(10, PlaceId(10)),
                place_node(20, PlaceId(20)),
                place_node(30, PlaceId(30)),
            ],
            edges: Vec::new(),
            models: Vec::new(),
            budgets: Vec::new(),
        };

        derive_direct_call_edges(&db, &mut output);
        super::super::summary_edges::derive_summary_projected_edges(&db, &mut output);
        let store =
            DataFlowStore::from_output(output, &crate::internal_core::test_stable_key_interner())
                .expect("valid store");
        let source = store
            .nodes()
            .iter()
            .find(|node| node.place == Some(PlaceId(10)))
            .expect("argument node")
            .id;
        let sink = store
            .nodes()
            .iter()
            .find(|node| node.place == Some(PlaceId(30)))
            .expect("return node")
            .id;
        let paths = find_taint_paths(
            &db,
            &store,
            source,
            sink,
            &std::collections::BTreeSet::new(),
            DataFlowSearchBudget {
                max_depth: 8,
                max_paths: 4,
            },
        );

        assert_eq!(paths[0].status, DataFlowPathStatus::NotFound);
    }

    #[test]
    fn repeated_target_summary_paths_do_not_cross_between_call_sites() {
        let mut db = test_db();
        db.replace_call_facts(CallOutput {
            sites: vec![
                call_site_with(CallSiteId(2), vec![PlaceId(10)], Some(PlaceId(30))),
                call_site_with(CallSiteId(3), vec![PlaceId(40)], Some(PlaceId(50))),
            ],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("valid call facts");
        db.replace_refined_call_facts(RefinedCallOutput {
            edges: vec![
                refined_edge_for(RefinedCallEdgeId(1), CallSiteId(2)),
                refined_edge_for(RefinedCallEdgeId(2), CallSiteId(3)),
            ],
        })
        .expect("valid refined calls");
        db.replace_summary_facts(SummaryOutput {
            summaries: vec![summary_fact()],
            events: Vec::new(),
        });
        let mut output = DataFlowOutput {
            nodes: vec![
                place_node(0, PlaceId(10)),
                place_node(1, PlaceId(30)),
                place_node(2, PlaceId(40)),
                place_node(3, PlaceId(50)),
            ],
            edges: Vec::new(),
            models: Vec::new(),
            budgets: Vec::new(),
        };

        derive_direct_call_edges(&db, &mut output);
        super::super::summary_edges::derive_summary_projected_edges(&db, &mut output);
        let store =
            DataFlowStore::from_output(output, &crate::internal_core::test_stable_key_interner())
                .expect("valid store");
        let first_argument = store
            .nodes()
            .iter()
            .find(|node| node.place == Some(PlaceId(10)))
            .expect("first argument node")
            .id;
        let first_result = store
            .nodes()
            .iter()
            .find(|node| node.place == Some(PlaceId(30)))
            .expect("first result node")
            .id;
        let second_result = store
            .nodes()
            .iter()
            .find(|node| node.place == Some(PlaceId(50)))
            .expect("second result node")
            .id;

        let same_call_paths = find_taint_paths(
            &db,
            &store,
            first_argument,
            first_result,
            &std::collections::BTreeSet::new(),
            DataFlowSearchBudget {
                max_depth: 8,
                max_paths: 4,
            },
        );
        let cross_call_paths = find_taint_paths(
            &db,
            &store,
            first_argument,
            second_result,
            &std::collections::BTreeSet::new(),
            DataFlowSearchBudget {
                max_depth: 8,
                max_paths: 4,
            },
        );

        assert_eq!(same_call_paths[0].status, DataFlowPathStatus::Found);
        assert_eq!(cross_call_paths[0].status, DataFlowPathStatus::NotFound);
    }

    #[test]
    fn unresolved_refined_call_preserves_non_unknown_statuses() {
        for (call_status, data_flow_status) in [
            (CallTargetStatus::Unsupported, DataFlowStatus::Unsupported),
            (CallTargetStatus::SetupMissing, DataFlowStatus::SetupMissing),
            (
                CallTargetStatus::BudgetExceeded,
                DataFlowStatus::BudgetExceeded,
            ),
            (CallTargetStatus::Rejected, DataFlowStatus::Rejected),
        ] {
            let mut output = DataFlowOutput::empty();
            derive_unresolved_call_edge(
                &crate::analysis_neutral::LocalAnalysisDb::new().stable_key_interner(),
                &mut CallProjection::new(&mut output),
                &refined_edge(call_status),
            );

            assert_eq!(output.edges[0].status, data_flow_status);
            if data_flow_status == DataFlowStatus::BudgetExceeded {
                assert!(
                    output
                        .budgets
                        .iter()
                        .any(|budget| budget.observed > budget.limit),
                    "budget exceeded rows must record observed > limit"
                );
            }
        }
    }

    fn refined_edge(status: CallTargetStatus) -> RefinedCallEdgeFact {
        RefinedCallEdgeFact {
            id: RefinedCallEdgeId(1),
            site: CallSiteId(2),
            base_target: None,
            caller: FunctionId::from_raw(4),
            target_function: Some(FunctionId::from_raw(5)),
            target_symbol: None,
            synthetic_target: None,
            language: Language::TypeScript,
            edge_kind: CallEdgeKind::Direct,
            algorithm: CallAlgorithm::DirectReference,
            tier: RefinedCallTier::TypeValueFunctionToken,
            status,
            reason: if status == CallTargetStatus::Resolved {
                None
            } else {
                Some(UnresolvedCallReason::Unknown)
            },
            provenance: CallProvenance::NativeDirect,
            precision: CallPrecision::SetupAware,
            validation: RefinedCallValidation::ReferentiallyValidated,
            confidence: RefinedCallConfidence::High,
            evidence: vec!["test".to_string()],
            input_stable_keys: vec!["call-site".into()],
            stable_key: crate::internal_core::stable_key_for_test("refined:edge"),
        }
    }

    fn refined_edge_for(id: RefinedCallEdgeId, site: CallSiteId) -> RefinedCallEdgeFact {
        RefinedCallEdgeFact {
            id,
            site,
            stable_key: crate::internal_core::stable_key_for_test(&format!(
                "refined:edge:{}",
                id.0
            )),
            ..refined_edge(CallTargetStatus::Resolved)
        }
    }

    fn call_site(receiver: Option<PlaceId>) -> CallSiteFact {
        CallSiteFact {
            in_throw: false,
            id: CallSiteId(2),
            language: Language::TypeScript,
            file: FileId::from_raw(1),
            caller: FunctionId::from_raw(4),
            owner_symbol: None,
            body: MirBodyId(5),
            operation: MirOpId(6),
            span: Span::point(FileId::from_raw(1), 1, 2),
            kind: if receiver.is_some() {
                CallSyntaxKind::Method
            } else {
                CallSyntaxKind::Function
            },
            callee: CallCallee::Identifier {
                reference: None,
                name: "target".to_string(),
            },
            receiver,
            arguments: vec![PlaceId(10), PlaceId(20)],
            result: Some(PlaceId(30)),
            status: CallTargetStatus::Resolved,
            precision: CallPrecision::Exact,
            stable_key: stable_key_for_test("call-site:2"),
        }
    }

    fn call_site_with(
        id: CallSiteId,
        arguments: Vec<PlaceId>,
        result: Option<PlaceId>,
    ) -> CallSiteFact {
        CallSiteFact {
            id,
            operation: MirOpId(id.0),
            arguments,
            result,
            stable_key: stable_key_for_test(&format!("call-site:{}", id.0)),
            ..call_site(None)
        }
    }

    fn go_method_call_site() -> CallSiteFact {
        CallSiteFact {
            language: Language::Go,
            kind: CallSyntaxKind::Method,
            receiver: Some(PlaceId(20)),
            arguments: vec![PlaceId(10)],
            ..call_site(Some(PlaceId(20)))
        }
    }

    fn place_node(id: u64, place: PlaceId) -> DataFlowNodeFact {
        DataFlowNodeFact {
            id: DataFlowNodeId(id),
            kind: DataFlowNodeKind::Place,
            language: Language::TypeScript,
            file: Some(FileId::from_raw(1)),
            function: Some(FunctionId::from_raw(4)),
            body: None,
            operation: None,
            cfg_node: None,
            place: Some(place),
            symbol: None,
            reference: None,
            call_site: None,
            model: None,
            span: None,
            stable_key: crate::internal_core::stable_key_for_test(&format!(
                "node:place:{}",
                place.0
            )),
        }
    }

    fn summary_fact() -> SummaryFact {
        SummaryFact {
            id: SummaryId(1),
            callable_stable_key: stable_key_for_test("callable:target"),
            function: FunctionId::from_raw(5),
            domain: SummaryDomainKind::DataFlowTito,
            status: SummaryStatus::Present,
            precision: SummaryPrecision::Local,
            provenance: SummaryProvenance::NativeLocal,
            payload_digest: "1234567890abcdef".to_string(),
            tito_flows: vec![SummaryFlowEdge {
                from: FlowRoot::Param(0),
                to: FlowRoot::Return,
                kind: FlowKind::Value,
            }],
            stable_key: stable_key_for_test("summary:target:tito"),
        }
    }
}
