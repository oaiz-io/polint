use super::facts::{
    DataFlowAlgorithm, DataFlowBudgetReason, DataFlowConfidence, DataFlowEdgeFact,
    DataFlowEdgeKind, DataFlowNodeFact, DataFlowNodeKind, DataFlowPrecision, DataFlowProvenance,
    DataFlowStatus, DataFlowValidation,
};
use super::local::budget_fact;
use super::store::{DataFlowOutput, next_data_flow_edge_id, next_data_flow_node_id};
use std::sync::Arc;

use crate::analysis_api::{FactFamily, stable_key_from_key_parts, stable_key_from_parts};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::ids::DataFlowNodeId;
use crate::analysis_neutral::summaries::facts::{
    FlowKind, FlowRoot, SummaryDomainKind, SummaryEventFact, SummaryFact, SummaryFlowEdge,
    SummaryPrecision, SummaryStatus,
};
use crate::internal_core::{KeyPart, Language};

/// Key -> id indexes for the summary projection.
///
/// `summary_node` and `push_edge` used to scan the whole node/edge vectors on
/// every insert. `output.nodes` already holds one node per MIR place by the time
/// this runs, so that is quadratic in corpus size.
#[derive(Default)]
struct ProjectionIndex {
    nodes: std::collections::BTreeMap<crate::internal_core::StableKeyId, DataFlowNodeId>,
    edges: std::collections::BTreeSet<crate::internal_core::StableKeyId>,
}

pub fn derive_summary_projected_edges(db: &impl AnalysisHost, output: &mut DataFlowOutput) {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let mut index = ProjectionIndex {
        nodes: output
            .nodes
            .iter()
            .map(|node| (node.stable_key, node.id))
            .collect(),
        edges: output.edges.iter().map(|edge| edge.stable_key).collect(),
    };
    for fact in db.summary_facts() {
        if fact.domain != SummaryDomainKind::DataFlowTito {
            continue;
        }
        if fact.status == SummaryStatus::Present {
            project_present_tito(interner, output, &mut index, fact);
        } else {
            project_summary_status(interner, output, &mut index, fact);
        }
    }
    for event in db.summary_events() {
        if event.domain == SummaryDomainKind::DataFlowTito && event.status != SummaryStatus::Present
        {
            project_summary_event(interner, output, &mut index, event);
        }
    }
}

fn project_present_tito(
    interner: &crate::internal_core::StableKeyInterner,
    output: &mut DataFlowOutput,
    index: &mut ProjectionIndex,
    fact: &SummaryFact,
) {
    for flow in &fact.tito_flows {
        if !flow_kind_projects_as_tito(flow.kind) {
            continue;
        }
        let input = summary_node(
            interner,
            output,
            index,
            fact,
            DataFlowNodeKind::SummaryInput,
            &root_role(flow.from),
        );
        let output_node = summary_node(
            interner,
            output,
            index,
            fact,
            DataFlowNodeKind::SummaryOutput,
            &root_role(flow.to),
        );
        push_edge(
            interner,
            output,
            index,
            SummaryEdgeDraft {
                from: input,
                to: output_node,
                kind: DataFlowEdgeKind::SummaryTito,
                status: DataFlowStatus::Present,
                precision: precision(fact.precision),
                validation: DataFlowValidation::ReferentiallyValidated,
                confidence: DataFlowConfidence::Medium,
                budget: None,
                evidence: vec![
                    "summary_data_flow_tito".to_string(),
                    format!("payload_digest={}", fact.payload_digest),
                    flow_evidence(flow),
                ],
                input_stable_keys: vec![
                    interner.resolve(fact.stable_key),
                    interner.resolve(fact.callable_stable_key),
                ],
                stable_anchor: format!(
                    "{}:{}->{}:{:?}",
                    interner.resolve(fact.stable_key),
                    root_role(flow.from),
                    root_role(flow.to),
                    flow.kind
                ),
            },
        );
    }
}

fn project_summary_status(
    interner: &crate::internal_core::StableKeyInterner,
    output: &mut DataFlowOutput,
    index: &mut ProjectionIndex,
    fact: &SummaryFact,
) {
    let source = summary_node(
        interner,
        output,
        index,
        fact,
        DataFlowNodeKind::SummaryInput,
        "uncertain-input",
    );
    let sink = summary_node(
        interner,
        output,
        index,
        fact,
        DataFlowNodeKind::Synthetic,
        "uncertain-output",
    );
    let status = status(fact.status);
    let budget = (status == DataFlowStatus::BudgetExceeded).then(|| {
        budget_fact(
            interner,
            DataFlowBudgetReason::PathCount,
            1,
            2,
            &interner.resolve(fact.stable_key),
            output,
        )
    });
    push_edge(
        interner,
        output,
        index,
        SummaryEdgeDraft {
            from: source,
            to: sink,
            kind: if status == DataFlowStatus::BudgetExceeded {
                DataFlowEdgeKind::BudgetTruncated
            } else {
                DataFlowEdgeKind::UnknownFlow
            },
            status,
            precision: DataFlowPrecision::Unknown,
            validation: if budget.is_some() {
                DataFlowValidation::BudgetValidated
            } else {
                DataFlowValidation::Native
            },
            confidence: DataFlowConfidence::Low,
            budget,
            evidence: vec![
                "summary_uncertainty".to_string(),
                format!("domain={}", fact.domain.as_str()),
                format!("status={}", fact.status.as_str()),
            ],
            input_stable_keys: vec![interner.resolve(fact.stable_key)],
            stable_anchor: interner.resolve(fact.stable_key).to_string(),
        },
    );
}

fn project_summary_event(
    interner: &crate::internal_core::StableKeyInterner,
    output: &mut DataFlowOutput,
    index: &mut ProjectionIndex,
    event: &SummaryEventFact,
) {
    let source = event_node(
        interner,
        output,
        index,
        event,
        DataFlowNodeKind::SummaryInput,
        "event-input",
    );
    let sink = event_node(
        interner,
        output,
        index,
        event,
        DataFlowNodeKind::Synthetic,
        "event-output",
    );
    let status = status(event.status);
    let budget = (status == DataFlowStatus::BudgetExceeded).then(|| {
        budget_fact(
            interner,
            DataFlowBudgetReason::PathCount,
            1,
            2,
            &interner.resolve(event.stable_key),
            output,
        )
    });
    push_edge(
        interner,
        output,
        index,
        SummaryEdgeDraft {
            from: source,
            to: sink,
            kind: if status == DataFlowStatus::BudgetExceeded {
                DataFlowEdgeKind::BudgetTruncated
            } else if status == DataFlowStatus::Unsupported {
                DataFlowEdgeKind::HavocFlow
            } else {
                DataFlowEdgeKind::UnknownFlow
            },
            status,
            precision: DataFlowPrecision::Unknown,
            validation: if budget.is_some() {
                DataFlowValidation::BudgetValidated
            } else {
                DataFlowValidation::Native
            },
            confidence: DataFlowConfidence::Low,
            budget,
            evidence: vec![
                "summary_event_uncertainty".to_string(),
                format!("domain={}", event.domain.as_str()),
                format!("event_kind={}", event.event_kind),
                format!("reason={}", event.reason),
            ],
            input_stable_keys: vec![interner.resolve(event.stable_key)],
            stable_anchor: interner.resolve(event.stable_key).to_string(),
        },
    );
}

struct SummaryEdgeDraft {
    from: DataFlowNodeId,
    to: DataFlowNodeId,
    kind: DataFlowEdgeKind,
    status: DataFlowStatus,
    precision: DataFlowPrecision,
    validation: DataFlowValidation,
    confidence: DataFlowConfidence,
    budget: Option<crate::analysis_neutral::ids::DataFlowBudgetId>,
    evidence: Vec<String>,
    input_stable_keys: Vec<Arc<str>>,
    stable_anchor: String,
}

fn push_edge(
    interner: &crate::internal_core::StableKeyInterner,
    output: &mut DataFlowOutput,
    index: &mut ProjectionIndex,
    draft: SummaryEdgeDraft,
) {
    let stable_key = stable_key_from_parts(
        interner,
        FactFamily::DataFlowEdge,
        &[
            ("kind", format!("{:?}", draft.kind)),
            ("summary", draft.stable_anchor.clone()),
            ("status", format!("{:?}", draft.status)),
        ],
    );
    if !index.edges.insert(stable_key) {
        return;
    }
    output.edges.push(DataFlowEdgeFact {
        id: next_data_flow_edge_id(&output.edges),
        from: draft.from,
        to: draft.to,
        kind: draft.kind,
        algorithm: DataFlowAlgorithm::SummaryProjection,
        status: draft.status,
        precision: draft.precision,
        validation: draft.validation,
        confidence: draft.confidence,
        provenance: DataFlowProvenance::Summary,
        call_site: None,
        call_target: None,
        refined_call: None,
        model: None,
        budget: draft.budget,
        evidence: draft.evidence,
        input_stable_keys: draft.input_stable_keys,
        stable_key,
    });
}

fn summary_node(
    interner: &crate::internal_core::StableKeyInterner,
    output: &mut DataFlowOutput,
    index: &mut ProjectionIndex,
    fact: &SummaryFact,
    kind: DataFlowNodeKind,
    role: &str,
) -> DataFlowNodeId {
    let stable_key = stable_key_from_key_parts(
        interner,
        FactFamily::DataFlowNode,
        [
            ("kind", KeyPart::Text(&format!("{kind:?}"))),
            ("summary", KeyPart::Key(fact.stable_key)),
            ("role", KeyPart::Text(role)),
        ],
    );
    if let Some(existing) = index.nodes.get(&stable_key) {
        return *existing;
    }
    let id = next_data_flow_node_id(&output.nodes);
    output.nodes.push(DataFlowNodeFact {
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
        call_site: None,
        model: None,
        span: None,
        stable_key,
    });
    index.nodes.insert(stable_key, id);
    id
}

fn event_node(
    interner: &crate::internal_core::StableKeyInterner,
    output: &mut DataFlowOutput,
    index: &mut ProjectionIndex,
    event: &SummaryEventFact,
    kind: DataFlowNodeKind,
    role: &str,
) -> DataFlowNodeId {
    let stable_key = stable_key_from_key_parts(
        interner,
        FactFamily::DataFlowNode,
        [
            ("kind", KeyPart::Text(&format!("{kind:?}"))),
            ("summary_event", KeyPart::Key(event.stable_key)),
            ("role", KeyPart::Text(role)),
        ],
    );
    if let Some(existing) = index.nodes.get(&stable_key) {
        return *existing;
    }
    let id = next_data_flow_node_id(&output.nodes);
    output.nodes.push(DataFlowNodeFact {
        id,
        kind,
        language: Language::Unknown,
        file: None,
        function: Some(event.function),
        body: None,
        operation: None,
        cfg_node: None,
        place: None,
        symbol: None,
        reference: None,
        call_site: None,
        model: None,
        span: None,
        stable_key,
    });
    index.nodes.insert(stable_key, id);
    id
}

fn status(status: SummaryStatus) -> DataFlowStatus {
    match status {
        SummaryStatus::Present => DataFlowStatus::Present,
        SummaryStatus::Unknown => DataFlowStatus::Unknown,
        SummaryStatus::Unsupported => DataFlowStatus::Unsupported,
        SummaryStatus::SetupMissing => DataFlowStatus::SetupMissing,
        SummaryStatus::BudgetExceeded => DataFlowStatus::BudgetExceeded,
    }
}

fn precision(precision: SummaryPrecision) -> DataFlowPrecision {
    match precision {
        SummaryPrecision::Local => DataFlowPrecision::Syntax,
        SummaryPrecision::SetupAware => DataFlowPrecision::SetupAware,
        SummaryPrecision::Heuristic => DataFlowPrecision::Heuristic,
        SummaryPrecision::UnknownTop => DataFlowPrecision::Unknown,
    }
}

pub fn root_role(root: FlowRoot) -> String {
    match root {
        FlowRoot::Param(index) => format!("param:{index}"),
        FlowRoot::Receiver => "receiver".to_string(),
        FlowRoot::Return => "return".to_string(),
    }
}

pub fn flow_kind_projects_as_tito(kind: FlowKind) -> bool {
    matches!(kind, FlowKind::Value | FlowKind::BySideEffect)
}

fn flow_evidence(flow: &SummaryFlowEdge) -> String {
    format!(
        "flow={}->{}:{:?}",
        root_role(flow.from),
        root_role(flow.to),
        flow.kind
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::ids::{SummaryEventId, SummaryId};
    use crate::analysis_neutral::summaries::facts::SummaryProvenance;
    use crate::internal_core::FunctionId;
    use crate::internal_core::stable_key_for_test;

    #[test]
    fn data_flow_tito_summary_produces_projected_edge() {
        let mut output = DataFlowOutput::empty();
        project_present_tito(
            &crate::analysis_neutral::LocalAnalysisDb::new().stable_key_interner(),
            &mut output,
            &mut ProjectionIndex::default(),
            &summary(SummaryStatus::Present),
        );

        assert!(
            output
                .edges
                .iter()
                .any(|edge| edge.kind == DataFlowEdgeKind::SummaryTito)
        );
        assert!(
            output.edges[0]
                .input_stable_keys
                .iter()
                .any(|key| key.as_ref() == "summary:tito")
        );
    }

    #[test]
    fn present_data_flow_tito_summary_without_structured_flows_produces_no_edge() {
        let mut output = DataFlowOutput::empty();
        let mut fact = summary(SummaryStatus::Present);
        fact.tito_flows.clear();

        project_present_tito(
            &crate::analysis_neutral::LocalAnalysisDb::new().stable_key_interner(),
            &mut output,
            &mut ProjectionIndex::default(),
            &fact,
        );

        assert!(output.edges.is_empty());
        assert!(output.nodes.is_empty());
    }

    #[test]
    fn barrier_and_sanitizer_summary_flows_do_not_project_as_tito_edges() {
        for kind in [FlowKind::Barrier, FlowKind::Sanitizer] {
            let mut output = DataFlowOutput::empty();
            let mut fact = summary(SummaryStatus::Present);
            fact.tito_flows = vec![SummaryFlowEdge {
                from: FlowRoot::Param(0),
                to: FlowRoot::Return,
                kind,
            }];

            project_present_tito(
                &crate::analysis_neutral::LocalAnalysisDb::new().stable_key_interner(),
                &mut output,
                &mut ProjectionIndex::default(),
                &fact,
            );

            assert!(
                output.edges.is_empty(),
                "{kind:?} flow should not become a present TITO edge"
            );
            assert!(
                output.nodes.is_empty(),
                "{kind:?} flow should not create projected summary nodes"
            );
        }
    }

    #[test]
    fn summary_uncertainty_statuses_map_to_visible_rows() {
        for summary_status in [
            SummaryStatus::Unknown,
            SummaryStatus::Unsupported,
            SummaryStatus::SetupMissing,
            SummaryStatus::BudgetExceeded,
        ] {
            let mut output = DataFlowOutput::empty();
            project_summary_status(
                &crate::analysis_neutral::LocalAnalysisDb::new().stable_key_interner(),
                &mut output,
                &mut ProjectionIndex::default(),
                &summary(summary_status),
            );
            assert_eq!(output.edges.len(), 1);
            assert_eq!(output.edges[0].status, status(summary_status));
            if summary_status == SummaryStatus::BudgetExceeded {
                assert_eq!(output.budgets.len(), 1);
                assert!(output.edges[0].budget.is_some());
            }
        }
    }

    #[test]
    fn summary_projection_ignores_present_events_and_non_data_flow_events() {
        let mut db = LocalAnalysisDb::default();
        db.replace_summary_facts(crate::analysis_neutral::summaries::store::SummaryOutput {
            summaries: Vec::new(),
            events: vec![
                event_with_domain(
                    SummaryDomainKind::DataFlowTito,
                    SummaryStatus::Present,
                    "summary-event:tito:no-flow",
                ),
                event_with_domain(
                    SummaryDomainKind::ControlEffects,
                    SummaryStatus::Unsupported,
                    "summary-event:control:unsupported",
                ),
            ],
        });
        let mut output = DataFlowOutput::empty();

        derive_summary_projected_edges(&db, &mut output);

        assert!(output.edges.is_empty());
        assert!(output.nodes.is_empty());
    }

    fn summary(status: SummaryStatus) -> SummaryFact {
        SummaryFact {
            id: SummaryId(1),
            callable_stable_key: stable_key_for_test("callable:identity"),
            function: FunctionId::from_raw(1),
            domain: SummaryDomainKind::DataFlowTito,
            status,
            precision: SummaryPrecision::Local,
            provenance: SummaryProvenance::NativeLocal,
            payload_digest: "1234567890abcdef".to_string(),
            tito_flows: if status == SummaryStatus::Present {
                vec![SummaryFlowEdge {
                    from: FlowRoot::Param(0),
                    to: FlowRoot::Return,
                    kind: FlowKind::Value,
                }]
            } else {
                Vec::new()
            },
            stable_key: stable_key_for_test("summary:tito"),
        }
    }

    fn event_with_domain(
        domain: SummaryDomainKind,
        status: SummaryStatus,
        stable_key: &str,
    ) -> SummaryEventFact {
        SummaryEventFact {
            id: SummaryEventId(1),
            callable_stable_key: stable_key_for_test("callable:identity"),
            function: FunctionId::from_raw(1),
            domain,
            event_kind: "missing_summary".to_string(),
            reason: "test".to_string(),
            status,
            precision: SummaryPrecision::UnknownTop,
            stable_key: stable_key_for_test(stable_key),
        }
    }
}
