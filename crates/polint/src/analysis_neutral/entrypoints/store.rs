use std::collections::{BTreeMap, BTreeSet};

use crate::analysis_neutral::entrypoints::facts::{
    EntrypointFact, EntrypointKind, FrameworkDispatchEdgeFact, TrustBoundaryFact,
    UnresolvedFrameworkFact, UnresolvedFrameworkReason,
};
use crate::analysis_neutral::error::AnalysisError;
use crate::analysis_neutral::ids::{
    DispatchEdgeId, EntrypointId, TrustBoundaryId, UnresolvedFrameworkId,
};
use crate::internal_core::{FileId, StableKeyId, StableKeyInterner};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EntrypointOutput {
    pub entrypoints: Vec<EntrypointFact>,
    pub trust_boundaries: Vec<TrustBoundaryFact>,
    pub dispatch_edges: Vec<FrameworkDispatchEdgeFact>,
    pub unresolved: Vec<UnresolvedFrameworkFact>,
}

impl EntrypointOutput {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn normalized(mut self, interner: &StableKeyInterner) -> Self {
        // Sort entrypoints by (stable_key, id)
        self.entrypoints.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.id.cmp(&right.id))
        });
        // Reassign sequential IDs after sorting
        for (index, entrypoint) in self.entrypoints.iter_mut().enumerate() {
            entrypoint.id = EntrypointId(index as u64);
        }

        // Sort trust_boundaries by (entrypoint_stable_key, stable_key, id)
        self.trust_boundaries.sort_by(|left, right| {
            interner
                .compare_canonical(left.entrypoint_stable_key, right.entrypoint_stable_key)
                .then_with(|| interner.compare_canonical(left.stable_key, right.stable_key))
                .then_with(|| left.id.cmp(&right.id))
        });
        for (index, boundary) in self.trust_boundaries.iter_mut().enumerate() {
            boundary.id = TrustBoundaryId(index as u64);
        }

        // Sort dispatch_edges by (from_source, stable_key, id)
        self.dispatch_edges.sort_by(|left, right| {
            left.from_source
                .as_str()
                .cmp(right.from_source.as_str())
                .then_with(|| interner.compare_canonical(left.stable_key, right.stable_key))
                .then_with(|| left.id.cmp(&right.id))
        });
        for (index, edge) in self.dispatch_edges.iter_mut().enumerate() {
            edge.id = DispatchEdgeId(index as u64);
        }

        // Sort unresolved by (stable_key, id)
        self.unresolved.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.id.cmp(&right.id))
        });
        for (index, fact) in self.unresolved.iter_mut().enumerate() {
            fact.id = UnresolvedFrameworkId(index as u64);
        }

        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct EntrypointStore {
    output: EntrypointOutput,
    entrypoints_by_kind: BTreeMap<EntrypointKind, Vec<usize>>,
    entrypoints_by_file: BTreeMap<FileId, Vec<usize>>,
    entrypoints_by_framework: BTreeMap<String, Vec<usize>>,
    trust_boundaries_by_entrypoint_key: BTreeMap<StableKeyId, Vec<usize>>,
    dispatch_edges_by_entrypoint_key: BTreeMap<String, Vec<usize>>,
    unresolved_by_reason: BTreeMap<UnresolvedFrameworkReason, Vec<usize>>,
}

impl EntrypointStore {
    pub fn from_output(
        output: EntrypointOutput,
        interner: &StableKeyInterner,
    ) -> Result<Self, AnalysisError> {
        let output = output.normalized(interner);

        // Collect valid entrypoint stable keys for referential integrity checks
        let entrypoint_keys: BTreeSet<StableKeyId> =
            output.entrypoints.iter().map(|ep| ep.stable_key).collect();
        let entrypoint_key_texts = entrypoint_keys
            .iter()
            .map(|key| interner.resolve(*key))
            .collect::<BTreeSet<_>>();

        // Validate: every trust boundary must reference an existing entrypoint
        for boundary in &output.trust_boundaries {
            if !entrypoint_keys.contains(&boundary.entrypoint_stable_key) {
                return Err(AnalysisError::InvalidFact {
                    provider: "polint.entrypoints",
                    reason: format!(
                        "dangling entrypoint stable key {:?} for trust boundary `{}`",
                        interner.resolve(boundary.entrypoint_stable_key),
                        interner.resolve(boundary.stable_key)
                    ),
                });
            }
        }

        // Validate: every dispatch edge must reference an existing entrypoint
        for edge in &output.dispatch_edges {
            if !entrypoint_key_texts.contains(edge.from_source.as_str()) {
                return Err(AnalysisError::InvalidFact {
                    provider: "polint.entrypoints",
                    reason: format!(
                        "dangling entrypoint stable key {:?} for dispatch edge `{}`",
                        edge.from_source,
                        interner.resolve(edge.stable_key)
                    ),
                });
            }
        }

        // Build indexes
        let mut store = Self {
            output,
            ..Self::default()
        };

        for (index, entrypoint) in store.output.entrypoints.iter().enumerate() {
            store
                .entrypoints_by_kind
                .entry(entrypoint.kind)
                .or_default()
                .push(index);
            store
                .entrypoints_by_file
                .entry(entrypoint.registration_file)
                .or_default()
                .push(index);
            store
                .entrypoints_by_framework
                .entry(entrypoint.framework_id.clone())
                .or_default()
                .push(index);
        }

        for (index, boundary) in store.output.trust_boundaries.iter().enumerate() {
            store
                .trust_boundaries_by_entrypoint_key
                .entry(boundary.entrypoint_stable_key)
                .or_default()
                .push(index);
        }

        for (index, edge) in store.output.dispatch_edges.iter().enumerate() {
            store
                .dispatch_edges_by_entrypoint_key
                .entry(edge.from_source.clone())
                .or_default()
                .push(index);
        }

        for (index, fact) in store.output.unresolved.iter().enumerate() {
            store
                .unresolved_by_reason
                .entry(fact.reason)
                .or_default()
                .push(index);
        }

        Ok(store)
    }

    pub fn entrypoints(&self) -> &[EntrypointFact] {
        &self.output.entrypoints
    }

    pub fn trust_boundaries(&self) -> &[TrustBoundaryFact] {
        &self.output.trust_boundaries
    }

    pub fn dispatch_edges(&self) -> &[FrameworkDispatchEdgeFact] {
        &self.output.dispatch_edges
    }

    pub fn unresolved(&self) -> &[UnresolvedFrameworkFact] {
        &self.output.unresolved
    }

    pub fn output(&self) -> &EntrypointOutput {
        &self.output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::entrypoints::facts::{
        DispatchEdgeKind, EntrypointConfidence, EntrypointPrecision, EntrypointProvenance,
        EntrypointStatus, TriggerMetadata, TrustBoundarySourceKind,
    };
    use crate::analysis_neutral::ids::{
        DispatchEdgeId, EntrypointId, TrustBoundaryId, UnresolvedFrameworkId,
    };
    use crate::internal_core::{FileId, FunctionId, Language, Span, SymbolId};

    fn span() -> Span {
        Span::point(FileId::from_raw(1), 1, 1)
    }

    fn entrypoint(id: u64, stable_key: &str) -> EntrypointFact {
        EntrypointFact {
            id: EntrypointId(id),
            language: Language::TypeScript,
            framework_id: "express".to_string(),
            kind: EntrypointKind::HttpRoute,
            target_function: FunctionId::from_raw(10),
            target_symbol: Some(SymbolId::from_raw(20)),
            registration_span: span(),
            registration_file: FileId::from_raw(1),
            trigger_metadata: TriggerMetadata::empty(),
            trust_boundary_link: None,
            precision: EntrypointPrecision::ResolvedStatic,
            provenance: EntrypointProvenance::NativeRecognizer,
            confidence: EntrypointConfidence::High,
            status: EntrypointStatus::Resolved,
            provider_id: "polint.entrypoints".to_string(),
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
        }
    }

    fn trust_boundary(id: u64, entrypoint_key: &str, stable_key: &str) -> TrustBoundaryFact {
        TrustBoundaryFact {
            id: TrustBoundaryId(id),
            entrypoint_stable_key: crate::internal_core::stable_key_for_test(entrypoint_key),
            source_kind: TrustBoundarySourceKind::PathParam,
            target_parameter: Some(FunctionId::from_raw(10)),
            target_parameter_index: Some(0),
            access_path: None,
            protocol: None,
            language: Language::TypeScript,
            file: FileId::from_raw(1),
            span: span(),
            precision: EntrypointPrecision::ResolvedStatic,
            provider_id: "polint.entrypoints".to_string(),
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
        }
    }

    fn dispatch_edge(id: u64, from_source: &str, stable_key: &str) -> FrameworkDispatchEdgeFact {
        FrameworkDispatchEdgeFact {
            id: DispatchEdgeId(id),
            from_source: from_source.to_string(),
            to_target: FunctionId::from_raw(10),
            to_symbol: Some(SymbolId::from_raw(20)),
            edge_kind: DispatchEdgeKind::RouteDispatch,
            guard_metadata: None,
            ordering: None,
            language: Language::TypeScript,
            file: FileId::from_raw(1),
            span: span(),
            precision: EntrypointPrecision::ResolvedStatic,
            provider_id: "polint.entrypoints".to_string(),
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
        }
    }

    fn unresolved(
        id: u64,
        reason: UnresolvedFrameworkReason,
        stable_key: &str,
    ) -> UnresolvedFrameworkFact {
        UnresolvedFrameworkFact {
            id: UnresolvedFrameworkId(id),
            language: Language::TypeScript,
            file: FileId::from_raw(2),
            span: span(),
            framework_id: "fastify".to_string(),
            reason,
            evidence: "import detected".to_string(),
            scope_description: "server registration".to_string(),
            precision: EntrypointPrecision::Conservative,
            provider_id: "polint.entrypoints".to_string(),
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
        }
    }

    #[test]
    fn normalized_sorts_entrypoints_by_stable_key_and_reassigns_ids() {
        let interner = crate::internal_core::test_stable_key_interner();
        let output = EntrypointOutput {
            entrypoints: vec![
                entrypoint(5, "ep-z"),
                entrypoint(3, "ep-a"),
                entrypoint(1, "ep-m"),
            ],
            trust_boundaries: Vec::new(),
            dispatch_edges: Vec::new(),
            unresolved: Vec::new(),
        }
        .normalized(&interner);

        assert_eq!(
            output
                .entrypoints
                .iter()
                .map(|ep| interner.resolve(ep.stable_key))
                .collect::<Vec<_>>(),
            vec![
                std::sync::Arc::<str>::from("ep-a"),
                std::sync::Arc::<str>::from("ep-m"),
                std::sync::Arc::<str>::from("ep-z"),
            ]
        );
        // IDs are reassigned sequentially after sorting
        assert_eq!(output.entrypoints[0].id, EntrypointId(0));
        assert_eq!(output.entrypoints[1].id, EntrypointId(1));
        assert_eq!(output.entrypoints[2].id, EntrypointId(2));
    }

    #[test]
    fn normalized_sorts_trust_boundaries_by_entrypoint_key_then_stable_key() {
        let interner = crate::internal_core::test_stable_key_interner();
        let output = EntrypointOutput {
            entrypoints: Vec::new(),
            trust_boundaries: vec![
                trust_boundary(2, "ep-b", "tb-z"),
                trust_boundary(1, "ep-a", "tb-a"),
                trust_boundary(3, "ep-b", "tb-a"),
            ],
            dispatch_edges: Vec::new(),
            unresolved: Vec::new(),
        }
        .normalized(&interner);

        assert_eq!(
            output
                .trust_boundaries
                .iter()
                .map(|tb| {
                    (
                        interner.resolve(tb.entrypoint_stable_key),
                        interner.resolve(tb.stable_key),
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                (
                    std::sync::Arc::<str>::from("ep-a"),
                    std::sync::Arc::<str>::from("tb-a"),
                ),
                (
                    std::sync::Arc::<str>::from("ep-b"),
                    std::sync::Arc::<str>::from("tb-a"),
                ),
                (
                    std::sync::Arc::<str>::from("ep-b"),
                    std::sync::Arc::<str>::from("tb-z"),
                ),
            ]
        );
        assert_eq!(output.trust_boundaries[0].id, TrustBoundaryId(0));
        assert_eq!(output.trust_boundaries[1].id, TrustBoundaryId(1));
        assert_eq!(output.trust_boundaries[2].id, TrustBoundaryId(2));
    }

    #[test]
    fn from_output_builds_deterministic_entrypoint_indexes() {
        let store = EntrypointStore::from_output(
            EntrypointOutput {
                entrypoints: vec![entrypoint(2, "ep-b"), {
                    let mut ep = entrypoint(1, "ep-a");
                    ep.kind = EntrypointKind::McpTool;
                    ep.registration_file = FileId::from_raw(2);
                    ep.framework_id = "mcp-sdk".to_string();
                    ep
                }],
                trust_boundaries: vec![
                    trust_boundary(1, "ep-a", "tb-a"),
                    trust_boundary(2, "ep-b", "tb-b"),
                ],
                dispatch_edges: vec![dispatch_edge(1, "ep-a", "de-a")],
                unresolved: vec![unresolved(
                    1,
                    UnresolvedFrameworkReason::UnrecognizedPattern,
                    "ur-a",
                )],
            },
            &crate::internal_core::test_stable_key_interner(),
        )
        .expect("entrypoint output should be valid");

        // Entrypoints
        assert_eq!(store.entrypoints().len(), 2);

        // Indexes by kind
        let http = store.entrypoints_by_kind.get(&EntrypointKind::HttpRoute);
        assert_eq!(http.map(|v| v.len()), Some(1));
        let mcp = store.entrypoints_by_kind.get(&EntrypointKind::McpTool);
        assert_eq!(mcp.map(|v| v.len()), Some(1));

        // Indexes by file
        let file1 = store.entrypoints_by_file.get(&FileId::from_raw(1));
        assert_eq!(file1.map(|v| v.len()), Some(1));
        let file2 = store.entrypoints_by_file.get(&FileId::from_raw(2));
        assert_eq!(file2.map(|v| v.len()), Some(1));

        // Indexes by framework
        let express = store.entrypoints_by_framework.get("express");
        assert_eq!(express.map(|v| v.len()), Some(1));
        let mcp_sdk = store.entrypoints_by_framework.get("mcp-sdk");
        assert_eq!(mcp_sdk.map(|v| v.len()), Some(1));

        // Trust boundaries
        assert_eq!(store.trust_boundaries().len(), 2);
        let tb_for_a = store
            .trust_boundaries_by_entrypoint_key
            .get(&crate::internal_core::stable_key_for_test("ep-a"));
        assert_eq!(tb_for_a.map(|v| v.len()), Some(1));
        let tb_for_b = store
            .trust_boundaries_by_entrypoint_key
            .get(&crate::internal_core::stable_key_for_test("ep-b"));
        assert_eq!(tb_for_b.map(|v| v.len()), Some(1));

        // Dispatch edges
        assert_eq!(store.dispatch_edges().len(), 1);
        let de_for_a = store.dispatch_edges_by_entrypoint_key.get("ep-a");
        assert_eq!(de_for_a.map(|v| v.len()), Some(1));

        // Unresolved
        assert_eq!(store.unresolved().len(), 1);
        let ur_by_reason = store
            .unresolved_by_reason
            .get(&UnresolvedFrameworkReason::UnrecognizedPattern);
        assert_eq!(ur_by_reason.map(|v| v.len()), Some(1));
    }

    #[test]
    fn from_output_rejects_dangling_trust_boundary_reference() {
        let error = EntrypointStore::from_output(
            EntrypointOutput {
                entrypoints: vec![entrypoint(1, "ep-a")],
                trust_boundaries: vec![trust_boundary(1, "ep-nonexistent", "tb-a")],
                dispatch_edges: Vec::new(),
                unresolved: Vec::new(),
            },
            &crate::internal_core::test_stable_key_interner(),
        )
        .unwrap_err();

        assert!(error.to_string().contains("dangling entrypoint stable key"));
        assert!(error.to_string().contains("ep-nonexistent"));
    }

    #[test]
    fn from_output_rejects_dangling_dispatch_edge_reference() {
        let error = EntrypointStore::from_output(
            EntrypointOutput {
                entrypoints: vec![entrypoint(1, "ep-a")],
                trust_boundaries: Vec::new(),
                dispatch_edges: vec![dispatch_edge(1, "ep-nonexistent", "de-a")],
                unresolved: Vec::new(),
            },
            &crate::internal_core::test_stable_key_interner(),
        )
        .unwrap_err();

        assert!(error.to_string().contains("dangling entrypoint stable key"));
        assert!(error.to_string().contains("ep-nonexistent"));
    }

    #[test]
    fn empty_output_builds_empty_store() {
        let store = EntrypointStore::from_output(
            EntrypointOutput::empty(),
            &crate::internal_core::test_stable_key_interner(),
        )
        .expect("empty output is valid");

        assert!(store.entrypoints().is_empty());
        assert!(store.trust_boundaries().is_empty());
        assert!(store.dispatch_edges().is_empty());
        assert!(store.unresolved().is_empty());
    }

    #[test]
    fn output_accessor_returns_normalized_output() {
        let store = EntrypointStore::from_output(
            EntrypointOutput {
                entrypoints: vec![entrypoint(5, "ep-z"), entrypoint(3, "ep-a")],
                trust_boundaries: Vec::new(),
                dispatch_edges: Vec::new(),
                unresolved: Vec::new(),
            },
            &crate::internal_core::test_stable_key_interner(),
        )
        .expect("valid output");

        let output = store.output();
        let interner = crate::internal_core::test_stable_key_interner();
        assert_eq!(
            interner.resolve(output.entrypoints[0].stable_key).as_ref(),
            "ep-a"
        );
        assert_eq!(
            interner.resolve(output.entrypoints[1].stable_key).as_ref(),
            "ep-z"
        );
    }
}
