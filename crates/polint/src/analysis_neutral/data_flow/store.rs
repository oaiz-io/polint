use std::collections::{BTreeMap, BTreeSet};

use super::facts::{
    DataFlowBudgetFact, DataFlowBudgetReason, DataFlowEdgeFact, DataFlowEdgeKind,
    DataFlowModelFact, DataFlowNodeFact, DataFlowNodeKind, DataFlowProvenance, DataFlowStatus,
};
use crate::analysis_neutral::error::AnalysisError;
use crate::analysis_neutral::ids::{
    CallSiteId, DataFlowBudgetId, DataFlowEdgeId, DataFlowModelId, DataFlowNodeId, MirOpId, PlaceId,
};
use crate::internal_core::{FunctionId, StableKeyId, StableKeyInterner};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataFlowOutput {
    pub nodes: Vec<DataFlowNodeFact>,
    pub edges: Vec<DataFlowEdgeFact>,
    pub models: Vec<DataFlowModelFact>,
    pub budgets: Vec<DataFlowBudgetFact>,
}

impl DataFlowOutput {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn normalized(mut self, interner: &StableKeyInterner) -> Self {
        self.nodes.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| left.id.cmp(&right.id))
        });
        self.models = self
            .models
            .into_iter()
            .map(DataFlowModelFact::normalized)
            .collect();
        self.models.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| left.id.cmp(&right.id))
        });
        self.budgets.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.reason.cmp(&right.reason))
                .then_with(|| left.id.cmp(&right.id))
        });

        let node_remap = self
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id, DataFlowNodeId(index as u64)))
            .collect::<BTreeMap<_, _>>();
        let model_remap = self
            .models
            .iter()
            .enumerate()
            .map(|(index, model)| (model.id, DataFlowModelId(index as u64)))
            .collect::<BTreeMap<_, _>>();
        let budget_remap = self
            .budgets
            .iter()
            .enumerate()
            .map(|(index, budget)| (budget.id, DataFlowBudgetId(index as u64)))
            .collect::<BTreeMap<_, _>>();

        for (index, node) in self.nodes.iter_mut().enumerate() {
            node.id = DataFlowNodeId(index as u64);
            if let Some(model) = node.model.and_then(|id| model_remap.get(&id).copied()) {
                node.model = Some(model);
            }
        }
        for (index, model) in self.models.iter_mut().enumerate() {
            model.id = DataFlowModelId(index as u64);
        }
        for (index, budget) in self.budgets.iter_mut().enumerate() {
            budget.id = DataFlowBudgetId(index as u64);
        }

        self.edges = self
            .edges
            .into_iter()
            .map(DataFlowEdgeFact::normalized)
            .map(|mut edge| {
                if let Some(remapped) = node_remap.get(&edge.from) {
                    edge.from = *remapped;
                }
                if let Some(remapped) = node_remap.get(&edge.to) {
                    edge.to = *remapped;
                }
                if let Some(model) = edge.model.and_then(|id| model_remap.get(&id).copied()) {
                    edge.model = Some(model);
                }
                if let Some(budget) = edge.budget.and_then(|id| budget_remap.get(&id).copied()) {
                    edge.budget = Some(budget);
                }
                edge
            })
            .collect();
        self.edges.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.from.cmp(&right.from))
                .then_with(|| left.to.cmp(&right.to))
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| left.algorithm.cmp(&right.algorithm))
                .then_with(|| left.id.cmp(&right.id))
        });
        for (index, edge) in self.edges.iter_mut().enumerate() {
            edge.id = DataFlowEdgeId(index as u64);
        }
        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct DataFlowStore {
    output: DataFlowOutput,
    by_node_kind: BTreeMap<DataFlowNodeKind, Vec<usize>>,
    by_edge_kind: BTreeMap<DataFlowEdgeKind, Vec<usize>>,
    by_status: BTreeMap<DataFlowStatus, Vec<usize>>,
    by_budget_reason: BTreeMap<DataFlowBudgetReason, Vec<usize>>,
    by_provenance: BTreeMap<DataFlowProvenance, Vec<usize>>,
    by_place: BTreeMap<PlaceId, Vec<usize>>,
    by_operation: BTreeMap<MirOpId, Vec<usize>>,
    by_call_site: BTreeMap<CallSiteId, Vec<usize>>,
    by_function: BTreeMap<FunctionId, Vec<usize>>,
    outgoing: BTreeMap<DataFlowNodeId, Vec<usize>>,
    incoming: BTreeMap<DataFlowNodeId, Vec<usize>>,
}

impl DataFlowStore {
    pub fn from_output(
        output: DataFlowOutput,
        interner: &StableKeyInterner,
    ) -> Result<Self, AnalysisError> {
        validate_references(&output, interner)?;
        let output = output.normalized(interner);
        validate_references(&output, interner)?;
        reject_duplicates(
            interner,
            output.nodes.iter().map(|node| node.stable_key),
            "data-flow node stable key",
        )?;
        reject_duplicates(
            interner,
            output.edges.iter().map(|edge| edge.stable_key),
            "data-flow edge stable key",
        )?;
        reject_duplicates(
            interner,
            output.models.iter().map(|model| model.stable_key),
            "data-flow model stable key",
        )?;
        reject_duplicates(
            interner,
            output.budgets.iter().map(|budget| budget.stable_key),
            "data-flow budget stable key",
        )?;

        let mut store = Self {
            output,
            ..Self::default()
        };
        for (index, node) in store.output.nodes.iter().enumerate() {
            store.by_node_kind.entry(node.kind).or_default().push(index);
            if let Some(place) = node.place {
                store.by_place.entry(place).or_default().push(index);
            }
            if let Some(operation) = node.operation {
                store.by_operation.entry(operation).or_default().push(index);
            }
            if let Some(call_site) = node.call_site {
                store.by_call_site.entry(call_site).or_default().push(index);
            }
            if let Some(function) = node.function {
                store.by_function.entry(function).or_default().push(index);
            }
        }
        for (index, edge) in store.output.edges.iter().enumerate() {
            store.by_edge_kind.entry(edge.kind).or_default().push(index);
            store.by_status.entry(edge.status).or_default().push(index);
            store
                .by_provenance
                .entry(edge.provenance)
                .or_default()
                .push(index);
            store.outgoing.entry(edge.from).or_default().push(index);
            store.incoming.entry(edge.to).or_default().push(index);
        }
        for (index, budget) in store.output.budgets.iter().enumerate() {
            store
                .by_budget_reason
                .entry(budget.reason)
                .or_default()
                .push(index);
        }
        Ok(store)
    }

    pub fn nodes(&self) -> &[DataFlowNodeFact] {
        &self.output.nodes
    }

    pub fn edges(&self) -> &[DataFlowEdgeFact] {
        &self.output.edges
    }

    pub fn models(&self) -> &[DataFlowModelFact] {
        &self.output.models
    }

    pub fn budgets(&self) -> &[DataFlowBudgetFact] {
        &self.output.budgets
    }

    pub fn outgoing(&self, node: DataFlowNodeId) -> Vec<&DataFlowEdgeFact> {
        self.edge_refs(self.outgoing.get(&node))
    }

    pub fn by_place(&self, place: PlaceId) -> Vec<&DataFlowNodeFact> {
        self.node_refs(self.by_place.get(&place))
    }

    pub fn by_operation(&self, operation: MirOpId) -> Vec<&DataFlowNodeFact> {
        self.node_refs(self.by_operation.get(&operation))
    }

    pub fn by_edge_kind(&self, kind: DataFlowEdgeKind) -> Vec<&DataFlowEdgeFact> {
        self.edge_refs(self.by_edge_kind.get(&kind))
    }

    pub fn by_status(&self, status: DataFlowStatus) -> Vec<&DataFlowEdgeFact> {
        self.edge_refs(self.by_status.get(&status))
    }

    pub fn budgets_by_reason(&self, reason: DataFlowBudgetReason) -> Vec<&DataFlowBudgetFact> {
        self.budget_refs(self.by_budget_reason.get(&reason))
    }

    fn node_refs(&self, indexes: Option<&Vec<usize>>) -> Vec<&DataFlowNodeFact> {
        indexes.map_or_else(Vec::new, |indexes| {
            indexes
                .iter()
                .map(|index| &self.output.nodes[*index])
                .collect()
        })
    }

    fn edge_refs(&self, indexes: Option<&Vec<usize>>) -> Vec<&DataFlowEdgeFact> {
        indexes.map_or_else(Vec::new, |indexes| {
            indexes
                .iter()
                .map(|index| &self.output.edges[*index])
                .collect()
        })
    }

    fn budget_refs(&self, indexes: Option<&Vec<usize>>) -> Vec<&DataFlowBudgetFact> {
        indexes.map_or_else(Vec::new, |indexes| {
            indexes
                .iter()
                .map(|index| &self.output.budgets[*index])
                .collect()
        })
    }
}

fn validate_references(
    output: &DataFlowOutput,
    interner: &StableKeyInterner,
) -> Result<(), AnalysisError> {
    reject_duplicate_ids(output.nodes.iter().map(|node| node.id), "data-flow node id")?;
    reject_duplicate_ids(
        output.models.iter().map(|model| model.id),
        "data-flow model id",
    )?;
    reject_duplicate_ids(
        output.budgets.iter().map(|budget| budget.id),
        "data-flow budget id",
    )?;

    let nodes = output
        .nodes
        .iter()
        .map(|node| node.id)
        .collect::<BTreeSet<_>>();
    let models = output
        .models
        .iter()
        .map(|model| model.id)
        .collect::<BTreeSet<_>>();
    let budgets = output
        .budgets
        .iter()
        .map(|budget| budget.id)
        .collect::<BTreeSet<_>>();

    for node in &output.nodes {
        if let Some(model) = node.model
            && !models.contains(&model)
        {
            return Err(AnalysisError::InvalidFact {
                provider: "polint.data_flow",
                reason: format!(
                    "data-flow node `{}` references missing model",
                    interner.resolve(node.stable_key)
                ),
            });
        }
    }

    for edge in &output.edges {
        if !nodes.contains(&edge.from) || !nodes.contains(&edge.to) {
            return Err(AnalysisError::InvalidFact {
                provider: "polint.data_flow",
                reason: format!(
                    "data-flow edge `{}` references missing endpoint",
                    interner.resolve(edge.stable_key)
                ),
            });
        }
        if let Some(model) = edge.model
            && !models.contains(&model)
        {
            return Err(AnalysisError::InvalidFact {
                provider: "polint.data_flow",
                reason: format!(
                    "data-flow edge `{}` references missing model",
                    interner.resolve(edge.stable_key)
                ),
            });
        }
        if let Some(budget) = edge.budget
            && !budgets.contains(&budget)
        {
            return Err(AnalysisError::InvalidFact {
                provider: "polint.data_flow",
                reason: format!(
                    "data-flow edge `{}` references missing budget",
                    interner.resolve(edge.stable_key)
                ),
            });
        }
        if matches!(
            edge.status,
            DataFlowStatus::BudgetExceeded | DataFlowStatus::Unknown | DataFlowStatus::Unsupported
        ) && edge.kind == DataFlowEdgeKind::BudgetTruncated
            && edge.budget.is_none()
        {
            return Err(AnalysisError::InvalidFact {
                provider: "polint.data_flow",
                reason: format!(
                    "data-flow edge `{}` is budget-linked but has no budget row",
                    interner.resolve(edge.stable_key)
                ),
            });
        }
    }

    Ok(())
}

fn reject_duplicate_ids<T>(
    ids: impl Iterator<Item = T>,
    label: &'static str,
) -> Result<(), AnalysisError>
where
    T: Copy + Ord + std::fmt::Debug,
{
    let mut seen = BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            return Err(AnalysisError::InvalidFact {
                provider: "polint.data_flow",
                reason: format!("duplicate {label} `{id:?}`"),
            });
        }
    }
    Ok(())
}

fn reject_duplicates(
    interner: &StableKeyInterner,
    keys: impl Iterator<Item = StableKeyId>,
    label: &'static str,
) -> Result<(), AnalysisError> {
    let mut seen = BTreeSet::new();
    for key in keys {
        if !seen.insert(key) {
            return Err(AnalysisError::InvalidFact {
                provider: "polint.data_flow",
                reason: format!("duplicate {label} `{}`", interner.resolve(key)),
            });
        }
    }
    Ok(())
}

pub fn next_data_flow_node_id(nodes: &[DataFlowNodeFact]) -> DataFlowNodeId {
    DataFlowNodeId(nodes.len() as u64)
}

pub fn next_data_flow_edge_id(edges: &[DataFlowEdgeFact]) -> DataFlowEdgeId {
    DataFlowEdgeId(edges.len() as u64)
}

pub fn next_data_flow_model_id(models: &[DataFlowModelFact]) -> DataFlowModelId {
    DataFlowModelId(models.len() as u64)
}

pub fn next_data_flow_budget_id(budgets: &[DataFlowBudgetFact]) -> DataFlowBudgetId {
    DataFlowBudgetId(budgets.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::data_flow::facts::{
        DataFlowAlgorithm, DataFlowConfidence, DataFlowModelKind, DataFlowPrecision,
        DataFlowProvenance, DataFlowValidation,
    };
    use crate::internal_core::Language;

    #[test]
    fn output_sorts_and_reassigns_ids_with_edge_endpoint_remap() {
        let interner = crate::internal_core::test_stable_key_interner();
        let output = DataFlowOutput {
            nodes: vec![node(9, "node:z"), node(3, "node:a")],
            edges: vec![edge(7, 9, 3, "edge:z_to_a")],
            models: Vec::new(),
            budgets: Vec::new(),
        }
        .normalized(&interner);

        assert_eq!(
            interner.resolve(output.nodes[0].stable_key).as_ref(),
            "node:a"
        );
        assert_eq!(output.nodes[0].id, DataFlowNodeId(0));
        assert_eq!(output.edges[0].from, DataFlowNodeId(1));
        assert_eq!(output.edges[0].to, DataFlowNodeId(0));
        assert_eq!(output.edges[0].id, DataFlowEdgeId(0));
    }

    #[test]
    fn store_rejects_dangling_edge_endpoints() {
        let result = DataFlowStore::from_output(
            DataFlowOutput {
                nodes: vec![node(0, "node:a")],
                edges: vec![edge(0, 0, 4, "edge:dangling")],
                models: Vec::new(),
                budgets: Vec::new(),
            },
            &crate::internal_core::test_stable_key_interner(),
        );

        assert!(result.is_err());
    }

    #[test]
    fn store_rejects_dangling_edge_endpoint_that_collides_after_normalization() {
        let result = DataFlowStore::from_output(
            DataFlowOutput {
                nodes: vec![node(10, "node:a"), node(20, "node:b")],
                edges: vec![edge(0, 10, 0, "edge:dangling-colliding")],
                models: Vec::new(),
                budgets: Vec::new(),
            },
            &crate::internal_core::test_stable_key_interner(),
        );

        assert!(result.is_err());
    }

    #[test]
    fn store_rejects_dangling_model_reference_that_collides_after_normalization() {
        let mut source = node(10, "node:source");
        source.model = Some(DataFlowModelId(0));

        let result = DataFlowStore::from_output(
            DataFlowOutput {
                nodes: vec![source],
                edges: Vec::new(),
                models: vec![model(10, "model:source")],
                budgets: Vec::new(),
            },
            &crate::internal_core::test_stable_key_interner(),
        );

        assert!(result.is_err());
    }

    #[test]
    fn store_rejects_duplicate_node_ids_before_endpoint_remap() {
        let result = DataFlowStore::from_output(
            DataFlowOutput {
                nodes: vec![node(10, "node:a"), node(10, "node:b")],
                edges: vec![edge(0, 10, 10, "edge:ambiguous")],
                models: Vec::new(),
                budgets: Vec::new(),
            },
            &crate::internal_core::test_stable_key_interner(),
        );

        assert!(result.is_err());
    }

    #[test]
    fn store_rejects_duplicate_model_ids_before_model_remap() {
        let mut source = node(10, "node:source");
        source.model = Some(DataFlowModelId(20));

        let result = DataFlowStore::from_output(
            DataFlowOutput {
                nodes: vec![source],
                edges: Vec::new(),
                models: vec![model(20, "model:a"), model(20, "model:b")],
                budgets: Vec::new(),
            },
            &crate::internal_core::test_stable_key_interner(),
        );

        assert!(result.is_err());
    }

    #[test]
    fn store_indexes_budget_rows_and_unknown_edges() {
        let mut budget_edge = edge(0, 0, 1, "edge:budget");
        budget_edge.kind = DataFlowEdgeKind::BudgetTruncated;
        budget_edge.status = DataFlowStatus::BudgetExceeded;
        budget_edge.budget = Some(DataFlowBudgetId(5));

        let store = DataFlowStore::from_output(
            DataFlowOutput {
                nodes: vec![node(0, "node:a"), node(1, "node:b")],
                edges: vec![budget_edge],
                models: Vec::new(),
                budgets: vec![budget(5, "budget:path-depth")],
            },
            &crate::internal_core::test_stable_key_interner(),
        )
        .expect("budget-linked store is valid");

        assert_eq!(store.by_status(DataFlowStatus::BudgetExceeded).len(), 1);
        assert_eq!(
            store
                .budgets_by_reason(DataFlowBudgetReason::PathDepth)
                .len(),
            1
        );
    }

    #[test]
    fn store_rejects_budget_truncated_edge_without_budget_row() {
        let mut budget_edge = edge(0, 0, 1, "edge:budget-missing");
        budget_edge.kind = DataFlowEdgeKind::BudgetTruncated;
        budget_edge.status = DataFlowStatus::BudgetExceeded;

        let result = DataFlowStore::from_output(
            DataFlowOutput {
                nodes: vec![node(0, "node:a"), node(1, "node:b")],
                edges: vec![budget_edge],
                models: Vec::new(),
                budgets: Vec::new(),
            },
            &crate::internal_core::test_stable_key_interner(),
        );

        assert!(result.is_err());
    }

    fn node(id: u64, stable_key: &str) -> DataFlowNodeFact {
        DataFlowNodeFact {
            id: DataFlowNodeId(id),
            kind: DataFlowNodeKind::Place,
            language: Language::TypeScript,
            file: None,
            function: None,
            body: None,
            operation: None,
            cfg_node: None,
            place: Some(PlaceId(id)),
            symbol: None,
            reference: None,
            call_site: None,
            model: None,
            span: None,
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
        }
    }

    fn model(id: u64, stable_key: &str) -> DataFlowModelFact {
        DataFlowModelFact {
            id: DataFlowModelId(id),
            kind: DataFlowModelKind::Source,
            language: Language::TypeScript,
            provider_id: "test".to_string(),
            model_id: None,
            source_stable_key: None,
            status: DataFlowStatus::Present,
            precision: DataFlowPrecision::SetupAware,
            validation: DataFlowValidation::ReferentiallyValidated,
            confidence: DataFlowConfidence::High,
            provenance: DataFlowProvenance::Native,
            evidence: Vec::new(),
            payload_labels: Vec::new(),
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
        }
    }

    fn edge(id: u64, from: u64, to: u64, stable_key: &str) -> DataFlowEdgeFact {
        DataFlowEdgeFact {
            id: DataFlowEdgeId(id),
            from: DataFlowNodeId(from),
            to: DataFlowNodeId(to),
            kind: DataFlowEdgeKind::LocalAssignment,
            algorithm: DataFlowAlgorithm::LocalMir,
            status: DataFlowStatus::Present,
            precision: DataFlowPrecision::Syntax,
            validation: DataFlowValidation::Native,
            confidence: DataFlowConfidence::High,
            provenance: DataFlowProvenance::Native,
            call_site: None,
            call_target: None,
            refined_call: None,
            model: None,
            budget: None,
            evidence: Vec::new(),
            input_stable_keys: Vec::new(),
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
        }
    }

    fn budget(id: u64, stable_key: &str) -> DataFlowBudgetFact {
        DataFlowBudgetFact {
            id: DataFlowBudgetId(id),
            reason: DataFlowBudgetReason::PathDepth,
            limit: 1,
            observed: 2,
            status: DataFlowStatus::BudgetExceeded,
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
        }
    }
}
