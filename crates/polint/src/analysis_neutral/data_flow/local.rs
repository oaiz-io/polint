use std::collections::{BTreeMap, BTreeSet};

use super::facts::{
    DataFlowAlgorithm, DataFlowBudgetFact, DataFlowBudgetReason, DataFlowConfidence,
    DataFlowEdgeFact, DataFlowEdgeKind, DataFlowNodeFact, DataFlowNodeKind, DataFlowPrecision,
    DataFlowProvenance, DataFlowStatus, DataFlowValidation,
};
use super::store::{
    DataFlowOutput, next_data_flow_budget_id, next_data_flow_edge_id, next_data_flow_node_id,
};
use std::sync::Arc;

use crate::analysis_api::{FactFamily, stable_key_from_parts};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::ids::{DataFlowBudgetId, DataFlowNodeId, MirBodyId, PlaceId};
use crate::analysis_neutral::mir_op::{
    AssignMode, ConservativeAction, MirOperation, MirOperationKind, MirValue,
};
use crate::analysis_neutral::places::{PlaceFact, PlaceProjection};
use crate::internal_core::{FunctionId, Language};

pub fn derive_local_value_flow(db: &impl AnalysisHost, output: &mut DataFlowOutput) {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let mut builder = LocalFlowBuilder::new(db, output);
    builder.derive(interner);
}

struct LocalFlowBuilder<'a, 'b, H: AnalysisHost + ?Sized> {
    db: &'a H,
    output: &'b mut DataFlowOutput,
    place_nodes: BTreeMap<PlaceId, DataFlowNodeId>,
    /// node stable key -> node id. `operation_node` is called once per MIR
    /// operation and used to scan `output.nodes` linearly, which is quadratic in
    /// a corpus where `derive_local_place_nodes` has already pushed one node per
    /// MIR place.
    nodes_by_key: BTreeMap<crate::internal_core::StableKeyId, DataFlowNodeId>,
    keys_by_node: std::collections::HashMap<DataFlowNodeId, crate::internal_core::StableKeyId>,
    body_index: BTreeMap<MirBodyId, usize>,
    place_key_index: BTreeMap<PlaceId, usize>,
    /// `PlaceId` -> index into `db.mir_places()`, so `projection_edge_kind` does
    /// not scan the whole place table per projection edge.
    place_index: BTreeMap<PlaceId, usize>,
    emitted_edges: BTreeSet<crate::internal_core::StableKeyId>,
    branchy_bodies: BTreeSet<MirBodyId>,
}

impl<'a, 'b, H: AnalysisHost + ?Sized> LocalFlowBuilder<'a, 'b, H> {
    fn new(db: &'a H, output: &'b mut DataFlowOutput) -> Self {
        let place_nodes = output
            .nodes
            .iter()
            .filter_map(|node| node.place.map(|place| (place, node.id)))
            .collect();
        let branchy_bodies = db
            .mir_operations()
            .iter()
            .filter_map(|operation| {
                matches!(operation.kind, MirOperationKind::Branch { .. }).then_some(operation.body)
            })
            .collect();
        let nodes_by_key = output
            .nodes
            .iter()
            .map(|node| (node.stable_key, node.id))
            .collect();
        let place_index = db
            .mir_places()
            .iter()
            .enumerate()
            .map(|(index, place)| (place.id, index))
            .collect();
        let mut keys_by_node = std::collections::HashMap::new();
        for node in &output.nodes {
            keys_by_node.entry(node.id).or_insert(node.stable_key);
        }
        let mut body_index = BTreeMap::new();
        for (index, body) in db.mir_bodies().iter().enumerate() {
            body_index.entry(body.id).or_insert(index);
        }
        let mut place_key_index = BTreeMap::new();
        for (index, place) in db.mir_places().iter().enumerate() {
            place_key_index.entry(place.id).or_insert(index);
        }
        Self {
            db,
            output,
            place_nodes,
            nodes_by_key,
            keys_by_node,
            body_index,
            place_key_index,
            place_index,
            emitted_edges: BTreeSet::new(),
            branchy_bodies,
        }
    }

    fn derive(&mut self, interner: &crate::internal_core::StableKeyInterner) {
        for operation in self.db.mir_operations() {
            match &operation.kind {
                MirOperationKind::Bind { place, value } => {
                    self.edge_from_value(
                        interner,
                        operation,
                        value,
                        *place,
                        DataFlowEdgeKind::LocalBinding,
                        "bind",
                    );
                }
                MirOperationKind::Assign { place, value, mode } => {
                    let kind = match mode {
                        AssignMode::ProjectionMutation => self.projection_edge_kind(*place),
                        AssignMode::UnknownWrite => DataFlowEdgeKind::HavocFlow,
                        AssignMode::DeclarationBinding => DataFlowEdgeKind::LocalBinding,
                        AssignMode::PartialWrite => DataFlowEdgeKind::LocalWrite,
                        AssignMode::Overwrite | AssignMode::Simultaneous => {
                            DataFlowEdgeKind::LocalAssignment
                        }
                    };
                    self.edge_from_value(
                        interner,
                        operation,
                        value,
                        *place,
                        kind,
                        &format!("assign_mode={mode:?}"),
                    );
                }
                MirOperationKind::Write { place, value } => {
                    self.edge_from_value(
                        interner,
                        operation,
                        value,
                        *place,
                        DataFlowEdgeKind::LocalWrite,
                        "write",
                    );
                }
                MirOperationKind::Read { place } => {
                    if let Some(from) = self.node_for_place(*place) {
                        let to = self.operation_node(
                            interner,
                            operation,
                            DataFlowNodeKind::Value,
                            format!("read:{}", self.operation_key(operation)),
                        );
                        self.push_edge(
                            interner,
                            EdgeDraft {
                                operation,
                                from,
                                to,
                                kind: DataFlowEdgeKind::LocalRead,
                                status: DataFlowStatus::Present,
                                precision: DataFlowPrecision::Syntax,
                                validation: DataFlowValidation::Native,
                                provenance: DataFlowProvenance::Native,
                                budget: None,
                                evidence: vec!["read".to_string()],
                                input_stable_keys: vec![
                                    self.place_key(*place),
                                    self.operation_key(operation),
                                ],
                            },
                        );
                    }
                }
                MirOperationKind::Return { value: Some(value) } => {
                    if let Some(from) = self.node_for_value(interner, operation, value) {
                        let to = self.operation_node(
                            interner,
                            operation,
                            DataFlowNodeKind::SummaryOutput,
                            format!("return:{}", self.operation_key(operation)),
                        );
                        self.push_edge(
                            interner,
                            EdgeDraft {
                                operation,
                                from,
                                to,
                                kind: DataFlowEdgeKind::ReturnValue,
                                status: DataFlowStatus::Present,
                                precision: DataFlowPrecision::Syntax,
                                validation: DataFlowValidation::Native,
                                provenance: DataFlowProvenance::Native,
                                budget: None,
                                evidence: vec!["return_value".to_string()],
                                input_stable_keys: vec![self.operation_key(operation)],
                            },
                        );
                    }
                }
                MirOperationKind::Call {
                    arguments,
                    return_place,
                    ..
                } => {
                    let return_node = self.node_for_place(*return_place);
                    for (index, argument) in arguments.iter().enumerate() {
                        if let (Some(from), Some(to)) =
                            (self.node_for_place(*argument), return_node)
                        {
                            self.push_edge(
                                interner,
                                EdgeDraft {
                                    operation,
                                    from,
                                    to,
                                    kind: DataFlowEdgeKind::CallArgumentToReturn,
                                    status: DataFlowStatus::Present,
                                    precision: DataFlowPrecision::Conservative,
                                    validation: DataFlowValidation::ReferentiallyValidated,
                                    provenance: DataFlowProvenance::Native,
                                    budget: None,
                                    evidence: vec![format!("local_call_argument_index={index}")],
                                    input_stable_keys: vec![
                                        self.place_key(*argument),
                                        self.place_key(*return_place),
                                        self.operation_key(operation),
                                    ],
                                },
                            );
                        }
                    }
                }
                MirOperationKind::Unsupported { unsupported } => {
                    self.emit_unsupported_operation(interner, operation, *unsupported);
                }
                MirOperationKind::StorageLive { .. }
                | MirOperationKind::Branch { .. }
                | MirOperationKind::Return { value: None } => {}
            }
        }
    }

    fn edge_from_value(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        operation: &MirOperation,
        value: &MirValue,
        target: PlaceId,
        kind: DataFlowEdgeKind,
        evidence: &str,
    ) {
        let Some(to) = self.node_for_place(target) else {
            return;
        };
        if matches!(value, MirValue::Literal { .. }) {
            if !self.branchy_bodies.contains(&operation.body) {
                self.kill_incoming_overwrite_flows(to, kind);
            }
            return;
        }
        let status =
            if matches!(value, MirValue::Unknown { .. }) || kind == DataFlowEdgeKind::HavocFlow {
                DataFlowStatus::Unknown
            } else {
                DataFlowStatus::Present
            };
        let from = self
            .node_for_value(interner, operation, value)
            .unwrap_or_else(|| {
                self.operation_node(
                    interner,
                    operation,
                    DataFlowNodeKind::Synthetic,
                    format!("unknown-source:{}", self.operation_key(operation)),
                )
            });
        self.push_edge(
            interner,
            EdgeDraft {
                operation,
                from,
                to,
                kind,
                status,
                precision: if status == DataFlowStatus::Present {
                    DataFlowPrecision::Syntax
                } else {
                    DataFlowPrecision::Unknown
                },
                validation: DataFlowValidation::Native,
                provenance: DataFlowProvenance::Native,
                budget: None,
                evidence: vec![evidence.to_string(), value_evidence(value)],
                input_stable_keys: vec![self.place_key(target), self.operation_key(operation)],
            },
        );
    }

    fn emit_unsupported_operation(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        operation: &MirOperation,
        unsupported: crate::analysis_neutral::ids::UnsupportedId,
    ) {
        let unsupported_fact = self
            .db
            .unsupported_semantics()
            .iter()
            .find(|fact| fact.id == unsupported);
        let affected_places = unsupported_fact
            .map(|fact| fact.affected_places.as_slice())
            .unwrap_or(&[]);
        let action = unsupported_fact
            .map(|fact| fact.conservative_action)
            .unwrap_or(ConservativeAction::PreserveWithUnknownValue);

        for place in affected_places {
            let Some(node) = self.node_for_place(*place) else {
                continue;
            };
            self.push_edge(
                interner,
                EdgeDraft {
                    operation,
                    from: node,
                    to: node,
                    kind: match action {
                        ConservativeAction::HavocAffectedPlaces
                        | ConservativeAction::StopLowering => DataFlowEdgeKind::HavocFlow,
                        ConservativeAction::SkipOperation
                        | ConservativeAction::PreserveWithUnknownValue => {
                            DataFlowEdgeKind::UnknownFlow
                        }
                    },
                    status: DataFlowStatus::Unsupported,
                    precision: DataFlowPrecision::Unknown,
                    validation: DataFlowValidation::Native,
                    provenance: DataFlowProvenance::Native,
                    budget: None,
                    evidence: vec![
                        "unsupported_semantic".to_string(),
                        unsupported_fact
                            .map(|fact| fact.construct.clone())
                            .unwrap_or_else(|| "missing_unsupported_fact".to_string()),
                    ],
                    input_stable_keys: vec![self.place_key(*place), self.operation_key(operation)],
                },
            );
        }
    }

    fn kill_incoming_overwrite_flows(
        &mut self,
        target_node: DataFlowNodeId,
        kind: DataFlowEdgeKind,
    ) {
        if !matches!(
            kind,
            DataFlowEdgeKind::LocalAssignment | DataFlowEdgeKind::LocalBinding
        ) {
            return;
        }

        let removed = self
            .output
            .edges
            .iter()
            .filter(|edge| {
                edge.algorithm == DataFlowAlgorithm::LocalMir
                    && edge.to == target_node
                    && edge.status == DataFlowStatus::Present
                    && matches!(
                        edge.kind,
                        DataFlowEdgeKind::LocalAssignment | DataFlowEdgeKind::LocalBinding
                    )
            })
            .map(|edge| edge.stable_key)
            .collect::<BTreeSet<_>>();
        if removed.is_empty() {
            return;
        }

        self.output
            .edges
            .retain(|edge| !removed.contains(&edge.stable_key));
        for stable_key in &removed {
            self.emitted_edges.remove(stable_key);
        }
    }

    fn projection_edge_kind(&self, place: PlaceId) -> DataFlowEdgeKind {
        let Some(place) = self
            .place_index
            .get(&place)
            .and_then(|index| self.db.mir_places().get(*index))
        else {
            return DataFlowEdgeKind::FieldProjection;
        };
        if place.projections.iter().any(|projection| {
            matches!(
                projection,
                PlaceProjection::IndexKnown(_) | PlaceProjection::IndexUnknown { .. }
            )
        }) {
            DataFlowEdgeKind::IndexProjection
        } else {
            DataFlowEdgeKind::FieldProjection
        }
    }

    fn node_for_value(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        operation: &MirOperation,
        value: &MirValue,
    ) -> Option<DataFlowNodeId> {
        match value {
            MirValue::Place(place) => self.node_for_place(*place),
            MirValue::CallReturn(site) => Some(self.operation_node(
                interner,
                operation,
                DataFlowNodeKind::CallReturn,
                format!("call-return:{}", site.0),
            )),
            MirValue::Temporary(value_id) => Some(self.operation_node(
                interner,
                operation,
                DataFlowNodeKind::Value,
                format!("temporary:{}", value_id.0),
            )),
            MirValue::Unknown { evidence } => Some(self.operation_node(
                interner,
                operation,
                DataFlowNodeKind::Synthetic,
                format!("unknown:{evidence}:{}", self.operation_key(operation)),
            )),
            MirValue::BinOp { op, .. } => Some(self.operation_node(
                interner,
                operation,
                DataFlowNodeKind::Value,
                format!("binop:{op}:{}", self.operation_key(operation)),
            )),
            MirValue::Aggregate { kind, .. } => Some(self.operation_node(
                interner,
                operation,
                DataFlowNodeKind::Value,
                format!("aggregate:{kind:?}:{}", self.operation_key(operation)),
            )),
            MirValue::Closure { body, .. } => Some(self.operation_node(
                interner,
                operation,
                DataFlowNodeKind::Value,
                format!("closure:{}:{}", body.0, self.operation_key(operation)),
            )),
            MirValue::Literal { .. } => None,
        }
    }

    fn node_for_place(&self, place: PlaceId) -> Option<DataFlowNodeId> {
        self.place_nodes.get(&place).copied()
    }

    fn operation_node(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        operation: &MirOperation,
        kind: DataFlowNodeKind,
        suffix: String,
    ) -> DataFlowNodeId {
        let stable_key = stable_key_from_parts(
            interner,
            FactFamily::DataFlowNode,
            &[
                ("operation", self.operation_key(operation).to_string()),
                ("node", suffix),
            ],
        );
        if let Some(node) = self.nodes_by_key.get(&stable_key) {
            return *node;
        }

        let (language, file, function) = self.body_context(operation.body);
        let id = next_data_flow_node_id(&self.output.nodes);
        self.output.nodes.push(DataFlowNodeFact {
            id,
            kind,
            language,
            file,
            function,
            body: Some(operation.body),
            operation: Some(operation.id),
            cfg_node: None,
            place: None,
            symbol: None,
            reference: None,
            call_site: None,
            model: None,
            span: Some(operation.span.clone()),
            stable_key,
        });
        self.nodes_by_key.insert(stable_key, id);
        self.keys_by_node.entry(id).or_insert(stable_key);
        id
    }

    fn push_edge(
        &mut self,
        interner: &crate::internal_core::StableKeyInterner,
        draft: EdgeDraft<'_>,
    ) {
        let call_site = match &draft.operation.kind {
            MirOperationKind::Call { site, .. }
                if draft.kind == DataFlowEdgeKind::CallArgumentToReturn =>
            {
                Some(*site)
            }
            _ => None,
        };
        let stable_key = stable_key_from_parts(
            interner,
            FactFamily::DataFlowEdge,
            &[
                ("kind", format!("{:?}", draft.kind)),
                ("operation", self.operation_key(draft.operation).to_string()),
                ("from", self.node_key(draft.from)),
                ("to", self.node_key(draft.to)),
                ("status", format!("{:?}", draft.status)),
            ],
        );
        if !self.emitted_edges.insert(stable_key) {
            return;
        }
        self.output.edges.push(DataFlowEdgeFact {
            id: next_data_flow_edge_id(&self.output.edges),
            from: draft.from,
            to: draft.to,
            kind: draft.kind,
            algorithm: DataFlowAlgorithm::LocalMir,
            status: draft.status,
            precision: draft.precision,
            validation: draft.validation,
            confidence: if draft.status == DataFlowStatus::Present {
                DataFlowConfidence::High
            } else {
                DataFlowConfidence::Low
            },
            provenance: draft.provenance,
            call_site,
            call_target: None,
            refined_call: None,
            model: None,
            budget: draft.budget,
            evidence: draft.evidence,
            input_stable_keys: draft.input_stable_keys,
            stable_key,
        });
    }

    fn body_context(
        &self,
        body: MirBodyId,
    ) -> (
        Language,
        Option<crate::internal_core::FileId>,
        Option<FunctionId>,
    ) {
        self.body_index
            .get(&body)
            .map(|index| &self.db.mir_bodies()[*index])
            .map(|fact| (fact.language, Some(fact.file), Some(fact.function)))
            .unwrap_or((Language::Unknown, None, None))
    }

    fn place_key(&self, place: PlaceId) -> Arc<str> {
        self.place_key_index
            .get(&place)
            .map(|index| &self.db.mir_places()[*index])
            .map(|place| self.db.resolve_stable_key(place.stable_key))
            .unwrap_or_else(|| Arc::from(format!("place:{}", place.0)))
    }

    fn operation_key(&self, operation: &MirOperation) -> Arc<str> {
        self.db.resolve_stable_key(operation.stable_key)
    }

    fn node_key(&self, node: DataFlowNodeId) -> String {
        self.keys_by_node
            .get(&node)
            .map(|key| self.db.resolve_stable_key(*key).to_string())
            .unwrap_or_else(|| format!("node:{}", node.0))
    }
}

struct EdgeDraft<'a> {
    operation: &'a MirOperation,
    from: DataFlowNodeId,
    to: DataFlowNodeId,
    kind: DataFlowEdgeKind,
    status: DataFlowStatus,
    precision: DataFlowPrecision,
    validation: DataFlowValidation,
    provenance: DataFlowProvenance,
    budget: Option<DataFlowBudgetId>,
    evidence: Vec<String>,
    input_stable_keys: Vec<Arc<str>>,
}

pub fn budget_fact(
    interner: &crate::internal_core::StableKeyInterner,
    reason: DataFlowBudgetReason,
    limit: u64,
    observed: u64,
    context: &str,
    output: &mut DataFlowOutput,
) -> DataFlowBudgetId {
    let stable_key = stable_key_from_parts(
        interner,
        FactFamily::DataFlowBudget,
        &[
            ("reason", format!("{reason:?}")),
            ("limit", limit.to_string()),
            ("observed", observed.to_string()),
            ("context", context.to_string()),
        ],
    );
    if let Some(existing) = output
        .budgets
        .iter()
        .find(|budget| budget.stable_key == stable_key)
        .map(|budget| budget.id)
    {
        return existing;
    }
    let id = next_data_flow_budget_id(&output.budgets);
    output.budgets.push(DataFlowBudgetFact {
        id,
        reason,
        limit,
        observed,
        status: DataFlowStatus::BudgetExceeded,
        stable_key,
    });
    id
}

fn value_evidence(value: &MirValue) -> String {
    match value {
        MirValue::Literal { .. } => "literal".to_string(),
        MirValue::Place(place) => format!("source_place={}", place.0),
        MirValue::Temporary(value) => format!("temporary={}", value.0),
        MirValue::CallReturn(site) => format!("call_return={}", site.0),
        MirValue::BinOp { op, .. } => format!("binop={op}"),
        MirValue::Aggregate { kind, .. } => format!("aggregate={kind:?}"),
        MirValue::Closure { body, captures } => {
            format!("closure={} captures={}", body.0, captures.len())
        }
        MirValue::Unknown { evidence } => format!("unknown={evidence}"),
    }
}

pub fn node_from_place(
    output: &DataFlowOutput,
    place: &PlaceFact,
    db: &impl AnalysisHost,
) -> DataFlowNodeFact {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    DataFlowNodeFact {
        id: next_data_flow_node_id(&output.nodes),
        kind: DataFlowNodeKind::Place,
        language: place.language,
        file: place.file,
        function: place.function,
        body: None,
        operation: None,
        cfg_node: None,
        place: Some(place.id),
        symbol: None,
        reference: None,
        call_site: None,
        model: None,
        span: None,
        stable_key: db
            .metadata_for(crate::analysis_api::FactRef::new(
                FactFamily::Place,
                place.id.0,
            ))
            .map(|metadata| {
                stable_key_from_parts(
                    interner,
                    FactFamily::DataFlowNode,
                    &[(
                        "place",
                        db.resolve_stable_key(metadata.stable_key).to_string(),
                    )],
                )
            })
            .unwrap_or_else(|| {
                stable_key_from_parts(
                    interner,
                    FactFamily::DataFlowNode,
                    &[("place_id", place.id.0.to_string())],
                )
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::AnalysisHost;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::ids::{CallSiteId, MirBodyId, MirOpId, MirPredicateId, PlaceId};
    use crate::analysis_neutral::mir_body::{MirBody, MirOutput, MirStatus};
    use crate::analysis_neutral::mir_op::{AssignMode, MirOperation, MirOperationKind, MirValue};
    use crate::analysis_neutral::places::{PlaceFact, PlaceProjection, PlaceRoot, PlaceStatus};
    use crate::internal_core::{FileId, FunctionId, Language, Span};

    #[test]
    fn local_builder_derives_parameter_local_return_flow() {
        let mut db = LocalAnalysisDb::default();
        let interner = db.stable_key_interner();
        db.replace_semantic_mir(MirOutput {
            bodies: vec![body(&interner)],
            places: vec![
                place(
                    &interner,
                    0,
                    PlaceRoot::Parameter {
                        function: FunctionId::from_raw(1),
                        index: 0,
                        name: Some("input".to_string()),
                    },
                    Vec::new(),
                ),
                place(
                    &interner,
                    1,
                    PlaceRoot::Local {
                        function: FunctionId::from_raw(1),
                        name: "local".to_string(),
                    },
                    Vec::new(),
                ),
            ],
            operations: vec![
                op(
                    &interner,
                    0,
                    MirOperationKind::Bind {
                        place: PlaceId(1),
                        value: MirValue::Place(PlaceId(0)),
                    },
                ),
                op(
                    &interner,
                    1,
                    MirOperationKind::Return {
                        value: Some(MirValue::Place(PlaceId(1))),
                    },
                ),
            ],
            unsupported: Vec::new(),
            ..MirOutput::default()
        })
        .expect("valid MIR");
        let mut output = DataFlowOutput::empty();
        push_place_nodes(&db, &mut output);

        derive_local_value_flow(&db, &mut output);

        assert!(
            output
                .edges
                .iter()
                .any(|edge| edge.kind == DataFlowEdgeKind::LocalBinding)
        );
        assert!(
            output
                .edges
                .iter()
                .any(|edge| edge.kind == DataFlowEdgeKind::ReturnValue)
        );
    }

    #[test]
    fn call_argument_to_return_edge_carries_typed_call_site() {
        let mut db = LocalAnalysisDb::default();
        let interner = db.stable_key_interner();
        db.replace_semantic_mir(MirOutput {
            bodies: vec![body(&interner)],
            places: vec![
                place(
                    &interner,
                    0,
                    PlaceRoot::Parameter {
                        function: FunctionId::from_raw(1),
                        index: 0,
                        name: Some("input".to_string()),
                    },
                    Vec::new(),
                ),
                place(
                    &interner,
                    1,
                    PlaceRoot::Local {
                        function: FunctionId::from_raw(1),
                        name: "result".to_string(),
                    },
                    Vec::new(),
                ),
            ],
            operations: vec![op(
                &interner,
                0,
                MirOperationKind::Call {
                    site: CallSiteId(7),
                    callee: MirValue::Unknown {
                        evidence: "test call".to_string(),
                    },
                    arguments: vec![PlaceId(0)],
                    return_place: PlaceId(1),
                },
            )],
            unsupported: Vec::new(),
            ..MirOutput::default()
        })
        .expect("valid MIR");
        let mut output = DataFlowOutput::empty();
        push_place_nodes(&db, &mut output);

        derive_local_value_flow(&db, &mut output);

        let edge = output
            .edges
            .iter()
            .find(|edge| edge.kind == DataFlowEdgeKind::CallArgumentToReturn)
            .expect("call argument-to-return edge");
        assert_eq!(edge.call_site, Some(CallSiteId(7)));
    }

    #[test]
    fn projection_mutation_emits_projection_edge_with_place_evidence() {
        let mut db = LocalAnalysisDb::default();
        let interner = db.stable_key_interner();
        db.replace_semantic_mir(MirOutput {
            bodies: vec![body(&interner)],
            places: vec![
                place(
                    &interner,
                    0,
                    PlaceRoot::Local {
                        function: FunctionId::from_raw(1),
                        name: "source".to_string(),
                    },
                    Vec::new(),
                ),
                place(
                    &interner,
                    1,
                    PlaceRoot::Local {
                        function: FunctionId::from_raw(1),
                        name: "target".to_string(),
                    },
                    vec![PlaceProjection::Property("field".to_string())],
                ),
            ],
            operations: vec![op(
                &interner,
                0,
                MirOperationKind::Assign {
                    place: PlaceId(1),
                    value: MirValue::Place(PlaceId(0)),
                    mode: AssignMode::ProjectionMutation,
                },
            )],
            unsupported: Vec::new(),
            ..MirOutput::default()
        })
        .expect("valid MIR");
        let mut output = DataFlowOutput::empty();
        push_place_nodes(&db, &mut output);

        derive_local_value_flow(&db, &mut output);

        let edge = output
            .edges
            .iter()
            .find(|edge| edge.kind == DataFlowEdgeKind::FieldProjection)
            .expect("field projection edge");
        assert!(
            edge.input_stable_keys
                .iter()
                .any(|key| key.contains("target"))
        );
    }

    #[test]
    fn unknown_write_emits_havoc_or_unknown_edge() {
        let mut db = LocalAnalysisDb::default();
        let interner = db.stable_key_interner();
        db.replace_semantic_mir(MirOutput {
            bodies: vec![body(&interner)],
            places: vec![place(
                &interner,
                0,
                PlaceRoot::Local {
                    function: FunctionId::from_raw(1),
                    name: "target".to_string(),
                },
                Vec::new(),
            )],
            operations: vec![op(
                &interner,
                0,
                MirOperationKind::Assign {
                    place: PlaceId(0),
                    value: MirValue::Unknown {
                        evidence: "dynamic".to_string(),
                    },
                    mode: AssignMode::UnknownWrite,
                },
            )],
            unsupported: Vec::new(),
            ..MirOutput::default()
        })
        .expect("valid MIR");
        let mut output = DataFlowOutput::empty();
        push_place_nodes(&db, &mut output);

        derive_local_value_flow(&db, &mut output);

        let edge = output
            .edges
            .iter()
            .find(|edge| edge.kind == DataFlowEdgeKind::HavocFlow)
            .expect("havoc edge");
        assert_eq!(edge.status, DataFlowStatus::Unknown);
        assert!(!edge.evidence.is_empty());
    }

    #[test]
    fn literal_assignment_does_not_emit_unknown_source_present_flow() {
        let mut db = LocalAnalysisDb::default();
        let interner = db.stable_key_interner();
        db.replace_semantic_mir(MirOutput {
            bodies: vec![body(&interner)],
            places: vec![place(
                &interner,
                0,
                PlaceRoot::Local {
                    function: FunctionId::from_raw(1),
                    name: "target".to_string(),
                },
                Vec::new(),
            )],
            operations: vec![op(
                &interner,
                0,
                MirOperationKind::Assign {
                    place: PlaceId(0),
                    value: MirValue::Literal {
                        value: "\"safe\"".to_string(),
                    },
                    mode: AssignMode::Overwrite,
                },
            )],
            unsupported: Vec::new(),
            ..MirOutput::default()
        })
        .expect("valid MIR");
        let mut output = DataFlowOutput::empty();
        push_place_nodes(&db, &mut output);

        derive_local_value_flow(&db, &mut output);

        assert!(
            output.edges.is_empty(),
            "literal assignments should not create synthetic source-to-place flows"
        );
    }

    #[test]
    fn literal_overwrite_kills_stale_incoming_local_flow() {
        let mut db = LocalAnalysisDb::default();
        let interner = db.stable_key_interner();
        db.replace_semantic_mir(MirOutput {
            bodies: vec![body(&interner)],
            places: vec![
                place(
                    &interner,
                    0,
                    PlaceRoot::Parameter {
                        function: FunctionId::from_raw(1),
                        index: 0,
                        name: Some("input".to_string()),
                    },
                    Vec::new(),
                ),
                place(
                    &interner,
                    1,
                    PlaceRoot::Local {
                        function: FunctionId::from_raw(1),
                        name: "local".to_string(),
                    },
                    Vec::new(),
                ),
            ],
            operations: vec![
                op(
                    &interner,
                    0,
                    MirOperationKind::Bind {
                        place: PlaceId(1),
                        value: MirValue::Place(PlaceId(0)),
                    },
                ),
                op(
                    &interner,
                    1,
                    MirOperationKind::Assign {
                        place: PlaceId(1),
                        value: MirValue::Literal {
                            value: "\"safe\"".to_string(),
                        },
                        mode: AssignMode::Overwrite,
                    },
                ),
                op(
                    &interner,
                    2,
                    MirOperationKind::Return {
                        value: Some(MirValue::Place(PlaceId(1))),
                    },
                ),
            ],
            unsupported: Vec::new(),
            ..MirOutput::default()
        })
        .expect("valid MIR");
        let mut output = DataFlowOutput::empty();
        push_place_nodes(&db, &mut output);

        derive_local_value_flow(&db, &mut output);

        assert!(
            !output.edges.iter().any(|edge| {
                edge.kind == DataFlowEdgeKind::LocalBinding
                    && edge.from == DataFlowNodeId(0)
                    && edge.to == DataFlowNodeId(1)
            }),
            "literal overwrite should clear stale parameter-to-local flow"
        );
        assert!(
            output
                .edges
                .iter()
                .any(|edge| edge.kind == DataFlowEdgeKind::ReturnValue),
            "the later return edge should still be represented"
        );
    }

    #[test]
    fn literal_overwrite_in_branchy_body_preserves_possible_incoming_local_flow() {
        let mut db = LocalAnalysisDb::default();
        let interner = db.stable_key_interner();
        db.replace_semantic_mir(MirOutput {
            bodies: vec![body(&interner)],
            places: vec![
                place(
                    &interner,
                    0,
                    PlaceRoot::Parameter {
                        function: FunctionId::from_raw(1),
                        index: 0,
                        name: Some("input".to_string()),
                    },
                    Vec::new(),
                ),
                place(
                    &interner,
                    1,
                    PlaceRoot::Local {
                        function: FunctionId::from_raw(1),
                        name: "local".to_string(),
                    },
                    Vec::new(),
                ),
            ],
            operations: vec![
                op(
                    &interner,
                    0,
                    MirOperationKind::Bind {
                        place: PlaceId(1),
                        value: MirValue::Place(PlaceId(0)),
                    },
                ),
                op(
                    &interner,
                    1,
                    MirOperationKind::Branch {
                        predicate: MirPredicateId(1),
                        predicate_place: Some(PlaceId(0)),
                        nil_test: None,
                    },
                ),
                op(
                    &interner,
                    2,
                    MirOperationKind::Assign {
                        place: PlaceId(1),
                        value: MirValue::Literal {
                            value: "\"safe\"".to_string(),
                        },
                        mode: AssignMode::Overwrite,
                    },
                ),
                op(
                    &interner,
                    3,
                    MirOperationKind::Return {
                        value: Some(MirValue::Place(PlaceId(1))),
                    },
                ),
            ],
            unsupported: Vec::new(),
            ..MirOutput::default()
        })
        .expect("valid MIR");
        let mut output = DataFlowOutput::empty();
        push_place_nodes(&db, &mut output);

        derive_local_value_flow(&db, &mut output);

        assert!(
            output.edges.iter().any(|edge| {
                edge.kind == DataFlowEdgeKind::LocalBinding
                    && edge.from == DataFlowNodeId(0)
                    && edge.to == DataFlowNodeId(1)
            }),
            "branchy bodies are not path-sensitive, so a literal overwrite must not erase a possible parameter flow"
        );
    }

    fn body(interner: &crate::internal_core::StableKeyInterner) -> MirBody {
        MirBody {
            id: MirBodyId(1),
            language: Language::TypeScript,
            file: FileId::from_raw(1),
            function: FunctionId::from_raw(1),
            package: None,
            module: None,
            owner_stable_key: interner.intern("function:one".to_string()),
            span: span(),
            stable_key: interner.intern("body:one".to_string()),
            status: MirStatus::Resolved,
        }
    }

    fn push_place_nodes(db: &impl AnalysisHost, output: &mut DataFlowOutput) {
        for place in db.mir_places() {
            let node = node_from_place(output, place, db);
            output.nodes.push(node);
        }
    }

    fn op(
        interner: &crate::internal_core::StableKeyInterner,
        ordinal: u32,
        kind: MirOperationKind,
    ) -> MirOperation {
        MirOperation {
            id: MirOpId(u64::from(ordinal)),
            body: MirBodyId(1),
            ordinal,
            span: span(),
            kind,
            stable_key: interner.intern(format!("op:{ordinal}")),
            status: MirStatus::Resolved,
        }
    }

    fn place(
        interner: &crate::internal_core::StableKeyInterner,
        id: u64,
        root: PlaceRoot,
        projections: Vec<PlaceProjection>,
    ) -> PlaceFact {
        PlaceFact {
            id: PlaceId(id),
            language: Language::TypeScript,
            file: Some(FileId::from_raw(1)),
            function: Some(FunctionId::from_raw(1)),
            root,
            projections,
            stable_key: interner.intern(format!("place:{id}:target")),
            status: PlaceStatus::Resolved,
        }
    }

    fn span() -> Span {
        Span::point(FileId::from_raw(1), 1, 1)
    }
}
