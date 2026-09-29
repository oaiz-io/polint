use crate::analysis_neutral::evidence::facts::{
    EvidenceExpansion, EvidencePrecision, EvidenceProvenance, EvidenceStatus, EvidenceValidation,
    ExtensionEvidenceCandidateFact, ExtensionEvidenceMergeFact, ExtensionEvidenceMergeReason,
    ExtensionEvidenceMergeVerdict,
};
use crate::analysis_neutral::evidence::store::EvidenceStore;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtensionEvidenceMergeReport {
    pub rows: Vec<ExtensionEvidenceMergeFact>,
}

pub fn validate_extension_evidence(
    store: &EvidenceStore,
    interner: &crate::internal_core::StableKeyInterner,
    candidates: Vec<ExtensionEvidenceCandidateFact>,
) -> ExtensionEvidenceMergeReport {
    let mut rows = candidates
        .into_iter()
        .map(ExtensionEvidenceCandidateFact::normalized)
        .map(|candidate| validate_candidate(store, interner, candidate))
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        left.extension_id
            .as_str()
            .cmp(right.extension_id.as_str())
            .then_with(|| left.provider_id.as_str().cmp(right.provider_id.as_str()))
            .then_with(|| interner.compare_canonical(left.stable_key, right.stable_key))
            .then_with(|| left.verdict.cmp(&right.verdict))
            .then_with(|| left.reason.cmp(&right.reason))
    });
    ExtensionEvidenceMergeReport { rows }
}

fn validate_candidate(
    store: &EvidenceStore,
    interner: &crate::internal_core::StableKeyInterner,
    candidate: ExtensionEvidenceCandidateFact,
) -> ExtensionEvidenceMergeFact {
    let reason = rejection_or_downgrade_reason(store, interner, &candidate);
    let (verdict, effective_status, effective_precision) = match reason {
        Some(ExtensionEvidenceMergeReason::InvalidEndpoint)
        | Some(ExtensionEvidenceMergeReason::InvalidSpan)
        | Some(ExtensionEvidenceMergeReason::UnboundedExpansion) => (
            ExtensionEvidenceMergeVerdict::Rejected,
            EvidenceStatus::Rejected,
            EvidencePrecision::Unknown,
        ),
        Some(ExtensionEvidenceMergeReason::CandidateCannotStrengthenDiagnostic) => (
            ExtensionEvidenceMergeVerdict::CandidateOnly,
            candidate.claimed_status,
            downgrade_precision(candidate.claimed_precision),
        ),
        Some(ExtensionEvidenceMergeReason::ExactClaimRequiresNativeAnchor) => (
            ExtensionEvidenceMergeVerdict::AcceptedWithPrecisionDowngrade,
            candidate.claimed_status,
            downgrade_precision(candidate.claimed_precision),
        ),
        None => (
            ExtensionEvidenceMergeVerdict::Accepted,
            candidate.claimed_status,
            candidate.claimed_precision,
        ),
    };
    ExtensionEvidenceMergeFact {
        extension_id: candidate.extension_id,
        provider_id: candidate.provider_id,
        stable_key: candidate.stable_key,
        verdict,
        reason,
        effective_status,
        effective_precision,
    }
}

fn rejection_or_downgrade_reason(
    store: &EvidenceStore,
    interner: &crate::internal_core::StableKeyInterner,
    candidate: &ExtensionEvidenceCandidateFact,
) -> Option<ExtensionEvidenceMergeReason> {
    if store.node(candidate.from).is_none() || store.node(candidate.to).is_none() {
        return Some(ExtensionEvidenceMergeReason::InvalidEndpoint);
    }
    if candidate
        .source_path
        .as_deref()
        .is_some_and(is_invalid_extension_source_path)
    {
        return Some(ExtensionEvidenceMergeReason::InvalidSpan);
    }
    if candidate
        .source_span
        .as_ref()
        .is_some_and(|span| span.start_byte > span.end_byte)
    {
        return Some(ExtensionEvidenceMergeReason::InvalidSpan);
    }
    if matches!(candidate.expansion, EvidenceExpansion::Expandable { ref key } if key.is_empty())
        || candidate.replay_key.as_deref() == Some("")
    {
        return Some(ExtensionEvidenceMergeReason::UnboundedExpansion);
    }
    if candidate.claimed_status != EvidenceStatus::Present {
        return Some(ExtensionEvidenceMergeReason::CandidateCannotStrengthenDiagnostic);
    }
    if candidate.claimed_precision == EvidencePrecision::Exact
        && !has_native_anchor(store, interner, candidate)
    {
        return Some(ExtensionEvidenceMergeReason::ExactClaimRequiresNativeAnchor);
    }
    None
}

fn is_invalid_extension_source_path(path: &str) -> bool {
    path.starts_with('/')
        || path.starts_with('\\')
        || path.contains("..")
        || path.contains('\\')
        || path.as_bytes().get(1).is_some_and(|byte| *byte == b':')
}

fn has_native_anchor(
    store: &EvidenceStore,
    interner: &crate::internal_core::StableKeyInterner,
    candidate: &ExtensionEvidenceCandidateFact,
) -> bool {
    !candidate.native_anchor_stable_keys.is_empty()
        && store.edges().iter().any(|edge| {
            edge.provenance == EvidenceProvenance::Native
                && matches!(
                    edge.validation,
                    EvidenceValidation::Native | EvidenceValidation::ReferentiallyValidated
                )
                && edge.from == candidate.from
                && edge.to == candidate.to
                && candidate.native_anchor_stable_keys.iter().any(|key| {
                    interner.resolve(edge.stable_key).as_ref() == key
                        || edge
                            .source_fact_stable_keys
                            .iter()
                            .any(|source| source.as_ref() == key.as_str())
                })
        })
}

fn downgrade_precision(precision: EvidencePrecision) -> EvidencePrecision {
    match precision {
        EvidencePrecision::Exact => EvidencePrecision::Conservative,
        other => other,
    }
}

pub fn merge_validated_extension_edges(
    native_edges: usize,
    rows: &[ExtensionEvidenceMergeFact],
) -> ExtensionEvidenceDelta {
    ExtensionEvidenceDelta {
        native_edge_count: native_edges,
        accepted: count(rows, ExtensionEvidenceMergeVerdict::Accepted),
        downgraded: count(
            rows,
            ExtensionEvidenceMergeVerdict::AcceptedWithPrecisionDowngrade,
        ),
        candidate_only: count(rows, ExtensionEvidenceMergeVerdict::CandidateOnly),
        rejected: count(rows, ExtensionEvidenceMergeVerdict::Rejected),
        representative_reasons: rows
            .iter()
            .filter_map(|row| row.reason.map(|reason| format!("{reason:?}")))
            .take(8)
            .collect(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionEvidenceDelta {
    pub native_edge_count: usize,
    pub accepted: usize,
    pub downgraded: usize,
    pub candidate_only: usize,
    pub rejected: usize,
    pub representative_reasons: Vec<String>,
}

fn count(rows: &[ExtensionEvidenceMergeFact], verdict: ExtensionEvidenceMergeVerdict) -> usize {
    rows.iter().filter(|row| row.verdict == verdict).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::evidence::facts::{
        EvidenceConfidence, EvidenceEdgeFact, EvidenceEdgeKind, EvidenceNodeFact, EvidenceNodeKind,
        EvidenceQueryMode,
    };
    use crate::analysis_neutral::evidence::store::EvidenceOutput;
    use crate::analysis_neutral::ids::{EvidenceEdgeId, EvidenceNodeId};
    use crate::internal_core::{FileId, Language, Span};

    #[test]
    fn merge_verdicts_are_sorted_and_resolvable() {
        let store = store();
        let interner = crate::internal_core::test_stable_key_interner();
        let rows = validate_extension_evidence(
            &store,
            &interner,
            vec![
                candidate("z", EvidenceNodeId(99)),
                candidate("a", EvidenceNodeId(0)),
            ],
        )
        .rows;

        assert_eq!(interner.resolve(rows[0].stable_key).as_ref(), "a");
        assert_eq!(rows[1].verdict, ExtensionEvidenceMergeVerdict::Rejected);
    }

    #[test]
    fn exact_claims_are_downgraded_without_native_anchor() {
        let store = store();
        let mut candidate = candidate("edge:extension", EvidenceNodeId(0));
        candidate.claimed_precision = EvidencePrecision::Exact;

        let row = validate_extension_evidence(
            &store,
            &crate::internal_core::test_stable_key_interner(),
            vec![candidate],
        )
        .rows
        .remove(0);

        assert_eq!(
            row.verdict,
            ExtensionEvidenceMergeVerdict::AcceptedWithPrecisionDowngrade
        );
        assert_eq!(row.effective_precision, EvidencePrecision::Conservative);
    }

    #[test]
    fn native_anchor_allows_exact_but_does_not_remove_native_may_edges() {
        let store = store();
        let mut candidate = candidate("edge:extension", EvidenceNodeId(0));
        candidate.claimed_precision = EvidencePrecision::Exact;
        candidate.native_anchor_stable_keys = vec!["edge:native".to_string()];

        let report = validate_extension_evidence(
            &store,
            &crate::internal_core::test_stable_key_interner(),
            vec![candidate],
        );
        let delta = merge_validated_extension_edges(store.edges().len(), &report.rows);

        assert_eq!(
            report.rows[0].verdict,
            ExtensionEvidenceMergeVerdict::Accepted
        );
        assert_eq!(delta.native_edge_count, 1);
        assert_eq!(
            crate::internal_core::test_stable_key_interner()
                .resolve(store.edges()[0].stable_key)
                .as_ref(),
            "edge:native"
        );
    }

    #[test]
    fn exact_native_anchor_must_match_candidate_endpoints() {
        let store = store();
        let mut candidate = candidate("edge:extension", EvidenceNodeId(1));
        candidate.to = EvidenceNodeId(0);
        candidate.claimed_precision = EvidencePrecision::Exact;
        candidate.native_anchor_stable_keys = vec!["edge:native".to_string()];

        let row = validate_extension_evidence(
            &store,
            &crate::internal_core::test_stable_key_interner(),
            vec![candidate],
        )
        .rows
        .remove(0);

        assert_eq!(
            row.verdict,
            ExtensionEvidenceMergeVerdict::AcceptedWithPrecisionDowngrade
        );
        assert_eq!(
            row.reason,
            Some(ExtensionEvidenceMergeReason::ExactClaimRequiresNativeAnchor)
        );
    }

    #[test]
    fn invalid_spans_and_unbounded_expansion_are_rejected() {
        let store = store();
        let mut bad_span = candidate("edge:bad-span", EvidenceNodeId(0));
        bad_span.source_path = Some("/Users/me/repo/src.rs".to_string());
        let mut bad_windows_span = candidate("edge:bad-windows-span", EvidenceNodeId(0));
        bad_windows_span.source_path = Some("C:\\Users\\me\\repo\\src.rs".to_string());
        let mut bad_expansion = candidate("edge:bad-expansion", EvidenceNodeId(0));
        bad_expansion.expansion = EvidenceExpansion::Expandable { key: String::new() };

        let rows = validate_extension_evidence(
            &store,
            &crate::internal_core::test_stable_key_interner(),
            vec![bad_span, bad_windows_span, bad_expansion],
        )
        .rows;

        assert!(
            rows.iter()
                .all(|row| row.verdict == ExtensionEvidenceMergeVerdict::Rejected)
        );
        assert!(
            rows.iter()
                .any(|row| row.reason == Some(ExtensionEvidenceMergeReason::InvalidSpan))
        );
        assert!(
            rows.iter()
                .any(|row| row.reason == Some(ExtensionEvidenceMergeReason::UnboundedExpansion))
        );
    }

    #[test]
    fn non_present_extension_evidence_is_candidate_only() {
        let store = store();
        let mut candidate = candidate("edge:uncertain", EvidenceNodeId(0));
        candidate.claimed_status = EvidenceStatus::Unknown;

        let row = validate_extension_evidence(
            &store,
            &crate::internal_core::test_stable_key_interner(),
            vec![candidate],
        )
        .rows
        .remove(0);

        assert_eq!(row.verdict, ExtensionEvidenceMergeVerdict::CandidateOnly);
    }

    fn store() -> EvidenceStore {
        EvidenceStore::from_output(
            EvidenceOutput {
                nodes: vec![node(0), node(1)],
                edges: vec![EvidenceEdgeFact {
                    id: EvidenceEdgeId(0),
                    from: EvidenceNodeId(0),
                    to: EvidenceNodeId(1),
                    kind: EvidenceEdgeKind::DataValue,
                    query_mode: EvidenceQueryMode::Path,
                    status: EvidenceStatus::Present,
                    precision: EvidencePrecision::Conservative,
                    provenance: EvidenceProvenance::Native,
                    validation: EvidenceValidation::ReferentiallyValidated,
                    confidence: EvidenceConfidence::Medium,
                    call_site: None,
                    summary_stable_key: None,
                    expansion: EvidenceExpansion::None,
                    compact_label: None,
                    source_fact_stable_keys: vec!["df:native".into()],
                    stable_key: crate::internal_core::stable_key_for_test("edge:native"),
                }],
                bundles: Vec::new(),
                paths: Vec::new(),
                slices: Vec::new(),
                unknowns: Vec::new(),
                omitted_regions: Vec::new(),
                replay_keys: Vec::new(),
            },
            &crate::internal_core::test_stable_key_interner(),
        )
        .expect("valid evidence")
    }

    fn candidate(stable_key: &str, from: EvidenceNodeId) -> ExtensionEvidenceCandidateFact {
        ExtensionEvidenceCandidateFact {
            extension_id: "demo".to_string(),
            provider_id: "model".to_string(),
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
            from,
            to: EvidenceNodeId(1),
            kind: EvidenceEdgeKind::Model,
            claimed_status: EvidenceStatus::Present,
            claimed_precision: EvidencePrecision::Heuristic,
            confidence: EvidenceConfidence::Medium,
            source_path: Some("src/lib.rs".to_string()),
            source_span: Some(Span::new(FileId::from_raw(1), 0, 1, 1, 1, 1, 2)),
            summary_stable_key: None,
            expansion: EvidenceExpansion::None,
            replay_key: Some("replay:extension".to_string()),
            native_anchor_stable_keys: Vec::new(),
            evidence: vec!["extension-model".to_string()],
        }
    }

    fn node(id: u64) -> EvidenceNodeFact {
        EvidenceNodeFact {
            id: EvidenceNodeId(id),
            kind: EvidenceNodeKind::Operation,
            language: Language::Go,
            file: None,
            function: None,
            body: None,
            operation: None,
            cfg_node: None,
            place: None,
            symbol: None,
            reference: None,
            call_site: None,
            span: None,
            status: EvidenceStatus::Present,
            precision: EvidencePrecision::Syntax,
            provenance: EvidenceProvenance::Native,
            validation: EvidenceValidation::Native,
            confidence: EvidenceConfidence::High,
            compact_label: None,
            source_fact_stable_keys: Vec::new(),
            stable_key: crate::internal_core::stable_key_for_test(&format!("node:{id}")),
        }
    }
}
