use std::collections::{BTreeMap, BTreeSet};

use crate::analysis_neutral::calls::facts::{CallAlgorithm, CallProvenance, CallTargetStatus};
use crate::analysis_neutral::error::AnalysisError;
use crate::analysis_neutral::ids::CallSiteId;
use crate::analysis_neutral::refined_calls::facts::{RefinedCallEdgeFact, RefinedCallTier};
use crate::internal_core::{FunctionId, StableKeyInterner, SymbolId};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RefinedCallOutput {
    pub edges: Vec<RefinedCallEdgeFact>,
}

impl RefinedCallOutput {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn normalized(mut self, interner: &StableKeyInterner) -> Self {
        self.edges = self
            .edges
            .into_iter()
            .map(RefinedCallEdgeFact::normalized)
            .collect();
        self.edges.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.site.cmp(&right.site))
                .then_with(|| left.tier.cmp(&right.tier))
                .then_with(|| left.algorithm.cmp(&right.algorithm))
                .then_with(|| left.provenance.cmp(&right.provenance))
                .then_with(|| left.status.cmp(&right.status))
                .then_with(|| left.id.cmp(&right.id))
        });
        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct RefinedCallStore {
    output: RefinedCallOutput,
    by_site: BTreeMap<CallSiteId, Vec<usize>>,
    by_caller: BTreeMap<FunctionId, Vec<usize>>,
    by_target_function: BTreeMap<FunctionId, Vec<usize>>,
    by_target_symbol: BTreeMap<SymbolId, Vec<usize>>,
    by_status: BTreeMap<CallTargetStatus, Vec<usize>>,
    by_algorithm: BTreeMap<CallAlgorithm, Vec<usize>>,
    by_provenance: BTreeMap<CallProvenance, Vec<usize>>,
    by_tier: BTreeMap<RefinedCallTier, Vec<usize>>,
}

impl RefinedCallStore {
    #[allow(clippy::too_many_arguments)]
    pub fn from_output(
        output: RefinedCallOutput,
        interner: &StableKeyInterner,
        valid_call_sites: &BTreeSet<CallSiteId>,
        valid_call_targets: &BTreeSet<crate::analysis_neutral::ids::CallTargetId>,
        valid_functions: &BTreeSet<FunctionId>,
        valid_symbols: &BTreeSet<SymbolId>,
    ) -> Result<Self, AnalysisError> {
        Self::from_normalized_output(
            output.normalized(interner),
            interner,
            valid_call_sites,
            valid_call_targets,
            valid_functions,
            valid_symbols,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_normalized_output(
        output: RefinedCallOutput,
        interner: &StableKeyInterner,
        valid_call_sites: &BTreeSet<CallSiteId>,
        valid_call_targets: &BTreeSet<crate::analysis_neutral::ids::CallTargetId>,
        valid_functions: &BTreeSet<FunctionId>,
        valid_symbols: &BTreeSet<SymbolId>,
    ) -> Result<Self, AnalysisError> {
        let mut seen_keys = BTreeSet::new();
        let mut seen_ids = BTreeSet::new();
        for edge in &output.edges {
            validate_relations(
                edge,
                interner,
                valid_call_sites,
                valid_call_targets,
                valid_functions,
                valid_symbols,
            )?;
            if !seen_keys.insert(edge.stable_key) {
                return Err(AnalysisError::InvalidFact {
                    provider: "polint.refined_calls",
                    reason: format!(
                        "duplicate refined call stable key `{}`",
                        interner.resolve(edge.stable_key)
                    ),
                });
            }
            if !seen_ids.insert(edge.id) {
                return Err(AnalysisError::InvalidFact {
                    provider: "polint.refined_calls",
                    reason: format!("duplicate refined call id {:?}", edge.id),
                });
            }
        }

        let mut store = Self {
            output,
            ..Self::default()
        };
        for (index, edge) in store.output.edges.iter().enumerate() {
            store.by_site.entry(edge.site).or_default().push(index);
            store.by_caller.entry(edge.caller).or_default().push(index);
            if let Some(target_function) = edge.target_function {
                store
                    .by_target_function
                    .entry(target_function)
                    .or_default()
                    .push(index);
            }
            if let Some(target_symbol) = edge.target_symbol {
                store
                    .by_target_symbol
                    .entry(target_symbol)
                    .or_default()
                    .push(index);
            }
            store.by_status.entry(edge.status).or_default().push(index);
            store
                .by_algorithm
                .entry(edge.algorithm)
                .or_default()
                .push(index);
            store
                .by_provenance
                .entry(edge.provenance)
                .or_default()
                .push(index);
            store.by_tier.entry(edge.tier).or_default().push(index);
        }
        Ok(store)
    }

    pub fn edges(&self) -> &[RefinedCallEdgeFact] {
        &self.output.edges
    }

    pub fn by_site(&self, site: CallSiteId) -> Vec<&RefinedCallEdgeFact> {
        self.edge_refs(self.by_site.get(&site))
    }

    pub fn by_tier(&self, tier: RefinedCallTier) -> Vec<&RefinedCallEdgeFact> {
        self.edge_refs(self.by_tier.get(&tier))
    }

    pub fn by_status(&self, status: CallTargetStatus) -> Vec<&RefinedCallEdgeFact> {
        self.edge_refs(self.by_status.get(&status))
    }

    pub fn by_algorithm(&self, algorithm: CallAlgorithm) -> Vec<&RefinedCallEdgeFact> {
        self.edge_refs(self.by_algorithm.get(&algorithm))
    }

    pub fn by_provenance(&self, provenance: CallProvenance) -> Vec<&RefinedCallEdgeFact> {
        self.edge_refs(self.by_provenance.get(&provenance))
    }

    pub fn by_caller(&self, caller: FunctionId) -> Vec<&RefinedCallEdgeFact> {
        self.edge_refs(self.by_caller.get(&caller))
    }

    pub fn by_target_function(&self, target: FunctionId) -> Vec<&RefinedCallEdgeFact> {
        self.edge_refs(self.by_target_function.get(&target))
    }

    pub fn by_target_symbol(&self, target: SymbolId) -> Vec<&RefinedCallEdgeFact> {
        self.edge_refs(self.by_target_symbol.get(&target))
    }

    fn edge_refs(&self, indexes: Option<&Vec<usize>>) -> Vec<&RefinedCallEdgeFact> {
        indexes.map_or_else(Vec::new, |indexes| {
            indexes
                .iter()
                .map(|index| &self.output.edges[*index])
                .collect()
        })
    }
}

fn validate_relations(
    edge: &RefinedCallEdgeFact,
    interner: &StableKeyInterner,
    valid_call_sites: &BTreeSet<CallSiteId>,
    valid_call_targets: &BTreeSet<crate::analysis_neutral::ids::CallTargetId>,
    valid_functions: &BTreeSet<FunctionId>,
    valid_symbols: &BTreeSet<SymbolId>,
) -> Result<(), AnalysisError> {
    let invalid = |reason| AnalysisError::InvalidFact {
        provider: "polint.refined_calls",
        reason: format!(
            "{reason} for refined call `{}`",
            interner.resolve(edge.stable_key)
        ),
    };

    if !valid_call_sites.contains(&edge.site) {
        return Err(invalid(format!("dangling call site {:?}", edge.site)));
    }
    if let Some(target) = edge.base_target
        && !valid_call_targets.contains(&target)
    {
        return Err(invalid(format!("dangling base call target {target:?}")));
    }
    if !valid_functions.contains(&edge.caller) {
        return Err(invalid(format!(
            "dangling caller function {:?}",
            edge.caller
        )));
    }
    if let Some(function) = edge.target_function
        && !valid_functions.contains(&function)
    {
        return Err(invalid(format!("dangling target function {function:?}")));
    }
    if let Some(symbol) = edge.target_symbol
        && !valid_symbols.contains(&symbol)
    {
        return Err(invalid(format!("dangling target symbol {symbol:?}")));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::calls::facts::{CallEdgeKind, CallPrecision};
    use crate::analysis_neutral::ids::RefinedCallEdgeId;
    use crate::analysis_neutral::refined_calls::facts::{
        RefinedCallConfidence, RefinedCallValidation,
    };
    use crate::internal_core::Language;

    #[test]
    fn store_normalizes_edges_and_indexes_by_tier() {
        let interner = crate::internal_core::test_stable_key_interner();
        let store = store_from_output(
            RefinedCallOutput {
                edges: vec![
                    edge(1, "b", RefinedCallTier::PointsToAssisted),
                    edge(0, "a", RefinedCallTier::DirectPlusFramework),
                ],
            },
            &interner,
        )
        .expect("valid refined calls");

        assert_eq!(interner.resolve(store.edges()[0].stable_key).as_ref(), "a");
        assert_eq!(store.by_tier(RefinedCallTier::DirectPlusFramework).len(), 1);
    }

    #[test]
    fn store_rejects_every_dangling_refined_call_relation() {
        let interner = crate::internal_core::test_stable_key_interner();
        let valid_sites = BTreeSet::from([CallSiteId(0)]);
        let valid_targets = BTreeSet::from([crate::analysis_neutral::ids::CallTargetId(1)]);
        let valid_functions = BTreeSet::from([FunctionId::from_raw(0), FunctionId::from_raw(1)]);
        let valid_symbols = BTreeSet::from([SymbolId::from_raw(2)]);
        let mut complete = edge(0, "complete", RefinedCallTier::DirectOnly);
        complete.base_target = Some(crate::analysis_neutral::ids::CallTargetId(1));
        complete.target_function = Some(FunctionId::from_raw(1));
        complete.target_symbol = Some(SymbolId::from_raw(2));

        let mut cases = Vec::new();
        let mut missing_site = complete.clone();
        missing_site.site = CallSiteId(99);
        cases.push((missing_site, "dangling call site"));
        let mut missing_target = complete.clone();
        missing_target.base_target = Some(crate::analysis_neutral::ids::CallTargetId(99));
        cases.push((missing_target, "dangling base call target"));
        let mut missing_caller = complete.clone();
        missing_caller.caller = FunctionId::from_raw(99);
        cases.push((missing_caller, "dangling caller function"));
        let mut missing_function = complete.clone();
        missing_function.target_function = Some(FunctionId::from_raw(99));
        cases.push((missing_function, "dangling target function"));
        let mut missing_symbol = complete;
        missing_symbol.target_symbol = Some(SymbolId::from_raw(99));
        cases.push((missing_symbol, "dangling target symbol"));

        for (edge, expected) in cases {
            let error = RefinedCallStore::from_output(
                RefinedCallOutput { edges: vec![edge] },
                &interner,
                &valid_sites,
                &valid_targets,
                &valid_functions,
                &valid_symbols,
            )
            .expect_err("dangling relation must be rejected");
            assert!(error.to_string().contains(expected), "{error}");
        }
    }

    #[test]
    fn store_rejects_duplicate_stable_keys() {
        let interner = crate::internal_core::test_stable_key_interner();
        let result = store_from_output(
            RefinedCallOutput {
                edges: vec![
                    edge(0, "duplicate", RefinedCallTier::DirectOnly),
                    edge(1, "duplicate", RefinedCallTier::DirectOnly),
                ],
            },
            &interner,
        );

        assert!(result.is_err());
    }

    fn store_from_output(
        output: RefinedCallOutput,
        interner: &StableKeyInterner,
    ) -> Result<RefinedCallStore, AnalysisError> {
        RefinedCallStore::from_output(
            output,
            interner,
            &BTreeSet::from([CallSiteId(0)]),
            &BTreeSet::new(),
            &BTreeSet::from([FunctionId::from_raw(0)]),
            &BTreeSet::new(),
        )
    }

    fn edge(id: u64, stable_key: &str, tier: RefinedCallTier) -> RefinedCallEdgeFact {
        RefinedCallEdgeFact {
            id: RefinedCallEdgeId(id),
            site: CallSiteId(0),
            base_target: None,
            caller: FunctionId::from_raw(0),
            target_function: None,
            target_symbol: None,
            synthetic_target: None,
            language: Language::TypeScript,
            edge_kind: CallEdgeKind::Direct,
            algorithm: CallAlgorithm::DirectReference,
            tier,
            status: CallTargetStatus::Resolved,
            reason: None,
            provenance: CallProvenance::Native,
            precision: CallPrecision::SetupAware,
            validation: RefinedCallValidation::Native,
            confidence: RefinedCallConfidence::High,
            evidence: vec![],
            input_stable_keys: vec![],
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
        }
    }
}
