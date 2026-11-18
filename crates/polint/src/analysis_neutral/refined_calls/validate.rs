use crate::analysis_neutral::AnalysisHost;
use std::collections::BTreeSet;

use crate::analysis_api::FactFamily;
use crate::analysis_neutral::calls::facts::{
    CallAlgorithm, CallEdgeKind, CallPrecision, CallProvenance, CallTargetStatus,
};
use crate::analysis_neutral::refined_calls::facts::{RefinedCallEdgeFact, RefinedCallValidation};
use crate::internal_core::{Diagnostic, DiagnosticRange as TextRange, fingerprint};

pub fn validate_refined_calls(db: &impl AnalysisHost, diagnostics: &mut Vec<Diagnostic>) {
    validate_refined_call_edges(db, db.refined_call_edges(), diagnostics);
}

fn validate_refined_call_edges(
    db: &impl AnalysisHost,
    edges: &[RefinedCallEdgeFact],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let call_sites = db
        .call_sites()
        .iter()
        .map(|site| site.id)
        .collect::<BTreeSet<_>>();
    let call_targets = db
        .call_targets()
        .iter()
        .map(|target| target.id)
        .collect::<BTreeSet<_>>();
    let functions = db
        .functions()
        .iter()
        .map(|function| function.id)
        .collect::<BTreeSet<_>>();
    let symbols = db
        .symbols()
        .iter()
        .map(|symbol| symbol.id)
        .collect::<BTreeSet<_>>();
    let mut seen_stable_keys = BTreeSet::new();

    for edge in edges {
        let stable_key = db.resolve_stable_key(edge.stable_key);
        if !seen_stable_keys.insert(edge.stable_key) {
            diagnostics.push(invalid_refined_call_diagnostic(
                stable_key.as_ref(),
                "duplicate stable key".to_string(),
            ));
        }
        if !call_sites.contains(&edge.site) {
            diagnostics.push(invalid_refined_call_diagnostic(
                stable_key.as_ref(),
                format!("dangling call site {:?}", edge.site),
            ));
        }
        if let Some(base_target) = edge.base_target
            && !call_targets.contains(&base_target)
        {
            diagnostics.push(invalid_refined_call_diagnostic(
                stable_key.as_ref(),
                format!("dangling base call target {base_target:?}"),
            ));
        }
        if !functions.contains(&edge.caller) {
            diagnostics.push(invalid_refined_call_diagnostic(
                stable_key.as_ref(),
                format!("dangling caller {:?}", edge.caller),
            ));
        }
        if let Some(target_function) = edge.target_function
            && !functions.contains(&target_function)
        {
            diagnostics.push(invalid_refined_call_diagnostic(
                stable_key.as_ref(),
                format!("dangling target function {target_function:?}"),
            ));
        }
        if let Some(target_symbol) = edge.target_symbol
            && !symbols.contains(&target_symbol)
        {
            diagnostics.push(invalid_refined_call_diagnostic(
                stable_key.as_ref(),
                format!("dangling target symbol {target_symbol:?}"),
            ));
        }
        if !stable_key.contains(FactFamily::RefinedCallEdge.label()) {
            diagnostics.push(invalid_refined_call_diagnostic(
                stable_key.as_ref(),
                "stable key does not include refined call fact family".to_string(),
            ));
        }
        if edge.evidence.is_empty() {
            diagnostics.push(invalid_refined_call_diagnostic(
                stable_key.as_ref(),
                "missing refined call evidence".to_string(),
            ));
        }
        if edge.input_stable_keys.is_empty() {
            diagnostics.push(invalid_refined_call_diagnostic(
                stable_key.as_ref(),
                "missing refined call input stable keys".to_string(),
            ));
        }
        let points_to_exact =
            edge.algorithm == CallAlgorithm::PointsTo && edge.precision == CallPrecision::Exact;
        if points_to_exact && edge.assumptions.is_empty() {
            diagnostics.push(invalid_refined_call_diagnostic(
                stable_key.as_ref(),
                "points-to refined call edge cannot claim exact precision without recorded assumptions"
                    .to_string(),
            ));
        }
        if !edge.assumptions.is_empty() && !points_to_exact {
            diagnostics.push(invalid_refined_call_diagnostic(
                stable_key.as_ref(),
                "only an exact points-to refined call edge records assumptions".to_string(),
            ));
        }
        if edge.provenance == CallProvenance::Model
            && edge.precision == CallPrecision::Exact
            && !points_to_exact
        {
            diagnostics.push(invalid_refined_call_diagnostic(
                stable_key.as_ref(),
                "model refined call edge cannot claim exact precision".to_string(),
            ));
        }
        if edge.validation == RefinedCallValidation::ExtensionValidated
            && matches!(edge.precision, CallPrecision::Exact)
        {
            diagnostics.push(invalid_refined_call_diagnostic(
                stable_key.as_ref(),
                "extension refined call edge cannot claim exact precision".to_string(),
            ));
        }
        if matches!(
            edge.algorithm,
            CallAlgorithm::FrameworkModel | CallAlgorithm::RepoModel
        ) && edge.precision == CallPrecision::Exact
        {
            diagnostics.push(invalid_refined_call_diagnostic(
                stable_key.as_ref(),
                "dynamic refined call algorithm cannot claim exact precision".to_string(),
            ));
        }
        if let Some(synthetic_target) = &edge.synthetic_target
            && !valid_synthetic_target(synthetic_target)
        {
            diagnostics.push(invalid_refined_call_diagnostic(
                stable_key.as_ref(),
                format!("malformed synthetic target `{synthetic_target}`"),
            ));
        }
        if matches!(
            edge.status,
            CallTargetStatus::Resolved | CallTargetStatus::Ambiguous
        ) && edge.target_function.is_none()
            && edge.target_symbol.is_none()
            && edge.synthetic_target.is_none()
        {
            diagnostics.push(invalid_refined_call_diagnostic(
                stable_key.as_ref(),
                "resolved or ambiguous refined edge has no target".to_string(),
            ));
        }
    }
}

fn valid_synthetic_target(target: &str) -> bool {
    !target.trim().is_empty()
        && !target.starts_with('/')
        && !target.contains("..")
        && target.contains(':')
}

fn invalid_refined_call_diagnostic(stable_key: &str, reason: String) -> Diagnostic {
    Diagnostic::error(
        "polint/internal",
        "<workspace>",
        TextRange::point(1, 1),
        format!("invalid refined call fact `{stable_key}`: {reason}"),
    )
    .with_fingerprint(fingerprint(&[
        "polint.refined_calls.validate",
        stable_key,
        &reason,
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_api::FunctionFact;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::calls::facts::{
        CallAlgorithm, CallCallee, CallPrecision, CallProvenance, CallSiteFact, CallSyntaxKind,
        CallTargetStatus,
    };
    use crate::analysis_neutral::calls::store::CallOutput;
    use crate::analysis_neutral::ids::{CallSiteId, MirBodyId, MirOpId, RefinedCallEdgeId};
    use crate::analysis_neutral::points_to::tiers::Assumption;
    use crate::analysis_neutral::refined_calls::facts::RefinedCallConfidence;
    use crate::analysis_neutral::refined_calls::facts::RefinedCallTier;
    use crate::internal_core::{FileId, FunctionId, Language, Span};

    #[test]
    fn validation_catches_dangling_call_site() {
        let db = db_with_call_site();
        let dangling = edge("family=RefinedCallEdge/test", CallSiteId(99));
        let mut diagnostics = Vec::new();

        validate_refined_call_edges(&db, &[dangling], &mut diagnostics);

        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("dangling call site"))
        );
    }

    #[test]
    fn validation_catches_exact_model_refinement() {
        let db = db_with_call_site();
        let mut refined = edge("family=RefinedCallEdge/model", CallSiteId(0));
        refined.precision = CallPrecision::Exact;
        refined.provenance = CallProvenance::Model;
        refined.algorithm = CallAlgorithm::FrameworkModel;
        refined.tier = RefinedCallTier::DirectPlusFramework;
        let mut diagnostics = Vec::new();

        validate_refined_call_edges(&db, &[refined], &mut diagnostics);

        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("cannot claim exact precision"))
        );
    }

    #[test]
    fn an_exact_points_to_edge_needs_recorded_assumptions() {
        let db = db_with_call_site();
        let mut refined = edge("family=RefinedCallEdge/points-to", CallSiteId(0));
        refined.precision = CallPrecision::Exact;
        refined.provenance = CallProvenance::Model;
        refined.algorithm = CallAlgorithm::PointsTo;
        refined.tier = RefinedCallTier::PointsToAssisted;
        refined.validation = RefinedCallValidation::ReferentiallyValidated;
        let mut diagnostics = Vec::new();

        validate_refined_call_edges(&db, std::slice::from_ref(&refined), &mut diagnostics);

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("cannot claim exact precision without recorded assumptions")
        }));

        refined.assumptions = vec![Assumption::SealedBindings, Assumption::WithinBudget];
        let mut diagnostics = Vec::new();
        validate_refined_call_edges(&db, &[refined.clone()], &mut diagnostics);
        assert!(
            diagnostics.is_empty(),
            "an exact points-to edge with assumptions is valid: {diagnostics:?}"
        );

        refined.precision = CallPrecision::Conservative;
        let mut diagnostics = Vec::new();
        validate_refined_call_edges(&db, &[refined], &mut diagnostics);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("records assumptions"))
        );
    }

    #[test]
    fn validation_catches_duplicate_stable_keys_deterministically() {
        let db = db_with_call_site();
        let edges = vec![
            edge("family=RefinedCallEdge/duplicate", CallSiteId(0)),
            edge("family=RefinedCallEdge/duplicate", CallSiteId(0)),
        ];
        let mut diagnostics = Vec::new();

        validate_refined_call_edges(&db, &edges, &mut diagnostics);

        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("duplicate stable key"))
        );
    }

    #[test]
    fn validation_catches_missing_evidence_and_inputs() {
        let db = db_with_call_site();
        let mut refined = edge("family=RefinedCallEdge/no-evidence", CallSiteId(0));
        refined.evidence.clear();
        refined.input_stable_keys.clear();
        let mut diagnostics = Vec::new();

        validate_refined_call_edges(&db, &[refined], &mut diagnostics);

        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("missing refined call evidence"))
        );
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("missing refined call input stable keys")
        }));
    }

    fn db_with_call_site() -> LocalAnalysisDb {
        let mut db = LocalAnalysisDb::new();
        let file = db.add_file(
            "app.ts".into(),
            "app.ts".to_string(),
            "callee();\n".to_string(),
        );
        db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "caller".to_string(),
            span(),
            Language::TypeScript,
            false,
            true,
            1,
            vec!["callee".to_string()],
        ));
        db.push_function(FunctionFact::new(
            FunctionId::from_raw(1),
            file,
            "callee".to_string(),
            span(),
            Language::TypeScript,
            false,
            true,
            1,
            Vec::new(),
        ));
        db.replace_call_facts(CallOutput {
            sites: vec![CallSiteFact {
                in_throw: false,
                id: CallSiteId(0),
                language: Language::TypeScript,
                file,
                caller: FunctionId::from_raw(0),
                owner_symbol: None,
                body: MirBodyId(0),
                operation: MirOpId(0),
                span: span(),
                kind: CallSyntaxKind::Function,
                callee: CallCallee::Identifier {
                    reference: None,
                    name: "callee".to_string(),
                },
                receiver: None,
                arguments: Vec::new(),
                result: None,
                status: CallTargetStatus::Resolved,
                precision: CallPrecision::SetupAware,
                stable_key: crate::internal_core::StableKeyId(0),
            }],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("valid calls");
        db
    }

    fn edge(stable_key: &str, site: CallSiteId) -> RefinedCallEdgeFact {
        RefinedCallEdgeFact {
            id: RefinedCallEdgeId(0),
            site,
            base_target: None,
            caller: FunctionId::from_raw(0),
            target_function: Some(FunctionId::from_raw(1)),
            target_symbol: None,
            synthetic_target: None,
            language: Language::TypeScript,
            edge_kind: CallEdgeKind::Synthetic,
            algorithm: CallAlgorithm::RepoModel,
            tier: RefinedCallTier::ExtensionModel,
            status: CallTargetStatus::Resolved,
            reason: None,
            provenance: CallProvenance::Extension,
            precision: CallPrecision::Heuristic,
            validation: RefinedCallValidation::ExtensionValidated,
            confidence: RefinedCallConfidence::Medium,
            evidence: vec!["test".to_string()],
            input_stable_keys: vec!["input".to_string()],
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
            assumptions: Vec::new(),
        }
    }

    fn span() -> Span {
        Span::point(FileId::from_raw(0), 1, 1)
    }
}
