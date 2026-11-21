use crate::analysis_neutral::AnalysisHost;
use std::collections::BTreeMap;

use super::facts::{
    RefinedCallConfidence, RefinedCallEdgeFact, RefinedCallTier, RefinedCallValidation,
};
use super::store::RefinedCallOutput;
use crate::analysis_api::{FactFamily, FactRef, stable_key_from_parts};
use crate::analysis_neutral::calls::facts::{
    CallAlgorithm, CallEdgeKind, CallPrecision, CallProvenance, CallTargetStatus,
    UnresolvedCallReason,
};
use crate::analysis_neutral::entrypoints::facts::{
    EntrypointFact, EntrypointPrecision, FrameworkDispatchEdgeFact, UnresolvedFrameworkFact,
    UnresolvedFrameworkReason,
};
use crate::analysis_neutral::ids::{CallSiteId, RefinedCallEdgeId};
use crate::internal_core::{FunctionId, SymbolId};

pub fn derive_framework_refinements(db: &impl AnalysisHost) -> RefinedCallOutput {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let entrypoints_by_key = db
        .entrypoint_facts()
        .iter()
        .map(|entrypoint| (db.resolve_stable_key(entrypoint.stable_key), entrypoint))
        .collect::<BTreeMap<_, _>>();
    let mut edges = Vec::new();

    for dispatch in db.dispatch_edge_facts() {
        if let Some(entrypoint) = entrypoints_by_key.get(dispatch.from_source.as_str()) {
            edges.push(edge_from_dispatch(db, dispatch, entrypoint, edges.len()));
        }
    }

    for unresolved in db.unresolved_framework_facts() {
        if let Some(caller) = fallback_caller(db, unresolved.framework_id.as_str()) {
            edges.push(edge_from_unresolved(db, unresolved, caller, edges.len()));
        }
    }

    RefinedCallOutput { edges }.normalized(interner)
}

fn edge_from_dispatch(
    db: &impl AnalysisHost,
    dispatch: &FrameworkDispatchEdgeFact,
    entrypoint: &EntrypointFact,
    index: usize,
) -> RefinedCallEdgeFact {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let site = call_site_for_function(db, entrypoint.target_function)
        .or_else(|| call_site_for_function(db, dispatch.to_target))
        .unwrap_or(CallSiteId(dispatch.id.0));
    let dispatch_key = metadata_key(
        db,
        FactFamily::DispatchEdge,
        dispatch.id.0,
        db.resolve_stable_key(dispatch.stable_key).as_ref(),
    );
    RefinedCallEdgeFact {
        id: RefinedCallEdgeId(index as u64),
        site,
        base_target: None,
        caller: entrypoint.target_function,
        target_function: Some(dispatch.to_target),
        target_symbol: dispatch.to_symbol,
        synthetic_target: None,
        language: dispatch.language,
        edge_kind: CallEdgeKind::Synthetic,
        algorithm: CallAlgorithm::FrameworkModel,
        tier: RefinedCallTier::DirectPlusFramework,
        status: CallTargetStatus::Resolved,
        reason: None,
        provenance: CallProvenance::Model,
        precision: framework_precision(dispatch.precision),
        validation: RefinedCallValidation::ReferentiallyValidated,
        confidence: framework_confidence(dispatch.precision),
        evidence: vec![
            "framework_dispatch".to_string(),
            format!("dispatch_edge={dispatch_key}"),
        ],
        input_stable_keys: vec![
            metadata_key(
                db,
                FactFamily::Entrypoint,
                entrypoint.id.0,
                db.resolve_stable_key(entrypoint.stable_key).as_ref(),
            ),
            dispatch_key.clone(),
        ],
        stable_key: stable_key_from_parts(
            interner,
            FactFamily::RefinedCallEdge,
            &[
                ("tier", "direct_plus_framework".to_string()),
                ("dispatch_edge", dispatch_key),
                (
                    "target",
                    function_or_symbol_key(db, Some(dispatch.to_target), dispatch.to_symbol),
                ),
            ],
        ),
        assumptions: Vec::new(),
    }
}

fn edge_from_unresolved(
    db: &impl AnalysisHost,
    unresolved: &UnresolvedFrameworkFact,
    caller: FunctionId,
    index: usize,
) -> RefinedCallEdgeFact {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let unresolved_key = metadata_key(
        db,
        FactFamily::UnresolvedFramework,
        unresolved.id.0,
        db.resolve_stable_key(unresolved.stable_key).as_ref(),
    );
    RefinedCallEdgeFact {
        id: RefinedCallEdgeId(index as u64),
        site: call_site_for_function(db, caller).unwrap_or(CallSiteId(unresolved.id.0)),
        base_target: None,
        caller,
        target_function: None,
        target_symbol: None,
        synthetic_target: Some(format!("framework:{}", unresolved.framework_id)),
        language: unresolved.language,
        edge_kind: CallEdgeKind::Synthetic,
        algorithm: CallAlgorithm::FrameworkModel,
        tier: RefinedCallTier::DirectPlusFramework,
        status: status_for_unresolved(unresolved.reason),
        reason: Some(reason_for_unresolved(unresolved.reason)),
        provenance: CallProvenance::Model,
        precision: framework_precision(unresolved.precision),
        validation: RefinedCallValidation::ReferentiallyValidated,
        confidence: RefinedCallConfidence::Low,
        evidence: vec![
            "unresolved_framework".to_string(),
            format!("reason={:?}", unresolved.reason),
        ],
        input_stable_keys: vec![unresolved_key.clone()],
        stable_key: stable_key_from_parts(
            interner,
            FactFamily::RefinedCallEdge,
            &[
                ("tier", "direct_plus_framework".to_string()),
                ("unresolved_framework", unresolved_key),
                ("framework", unresolved.framework_id.clone()),
            ],
        ),
        assumptions: Vec::new(),
    }
}

fn call_site_for_function(db: &impl AnalysisHost, function: FunctionId) -> Option<CallSiteId> {
    db.call_sites()
        .iter()
        .find(|site| site.caller == function)
        .map(|site| site.id)
}

fn fallback_caller(db: &impl AnalysisHost, framework_id: &str) -> Option<FunctionId> {
    db.entrypoint_facts()
        .iter()
        .find(|entrypoint| entrypoint.framework_id == framework_id)
        .map(|entrypoint| entrypoint.target_function)
        .or_else(|| db.functions().first().map(|function| function.id))
}

fn framework_precision(precision: EntrypointPrecision) -> CallPrecision {
    match precision {
        EntrypointPrecision::ResolvedStatic | EntrypointPrecision::SetupAware => {
            CallPrecision::SetupAware
        }
        EntrypointPrecision::Heuristic => CallPrecision::Heuristic,
        EntrypointPrecision::Conservative => CallPrecision::Conservative,
        EntrypointPrecision::Unknown => CallPrecision::Unknown,
    }
}

fn framework_confidence(precision: EntrypointPrecision) -> RefinedCallConfidence {
    match precision {
        EntrypointPrecision::ResolvedStatic | EntrypointPrecision::SetupAware => {
            RefinedCallConfidence::High
        }
        EntrypointPrecision::Heuristic | EntrypointPrecision::Conservative => {
            RefinedCallConfidence::Medium
        }
        EntrypointPrecision::Unknown => RefinedCallConfidence::Low,
    }
}

fn status_for_unresolved(reason: UnresolvedFrameworkReason) -> CallTargetStatus {
    match reason {
        UnresolvedFrameworkReason::MissingSetup => CallTargetStatus::SetupMissing,
        UnresolvedFrameworkReason::UnsupportedFrameworkVersion => CallTargetStatus::Unsupported,
        UnresolvedFrameworkReason::BudgetExceeded => CallTargetStatus::BudgetExceeded,
        UnresolvedFrameworkReason::DynamicRoute
        | UnresolvedFrameworkReason::UnknownWrapper
        | UnresolvedFrameworkReason::UnresolvedHandler
        | UnresolvedFrameworkReason::DynamicRegistration
        | UnresolvedFrameworkReason::UnrecognizedPattern => CallTargetStatus::Unresolved,
    }
}

fn reason_for_unresolved(reason: UnresolvedFrameworkReason) -> UnresolvedCallReason {
    match reason {
        UnresolvedFrameworkReason::MissingSetup => UnresolvedCallReason::SetupMissing,
        UnresolvedFrameworkReason::UnsupportedFrameworkVersion => {
            UnresolvedCallReason::UnsupportedSyntax
        }
        UnresolvedFrameworkReason::BudgetExceeded => UnresolvedCallReason::BudgetExceeded,
        UnresolvedFrameworkReason::DynamicRoute
        | UnresolvedFrameworkReason::UnknownWrapper
        | UnresolvedFrameworkReason::UnresolvedHandler
        | UnresolvedFrameworkReason::DynamicRegistration
        | UnresolvedFrameworkReason::UnrecognizedPattern => UnresolvedCallReason::FrameworkDispatch,
    }
}

fn function_or_symbol_key(
    db: &impl AnalysisHost,
    function: Option<FunctionId>,
    symbol: Option<SymbolId>,
) -> String {
    function
        .map(|function| metadata_key(db, FactFamily::Function, function.0, ""))
        .or_else(|| symbol.map(|symbol| metadata_key(db, FactFamily::Symbol, symbol.0, "")))
        .unwrap_or_else(|| "none".to_string())
}

fn metadata_key(db: &impl AnalysisHost, family: FactFamily, run_id: u64, fallback: &str) -> String {
    db.metadata_for(FactRef::new(family, run_id))
        .map(|metadata| db.resolve_stable_key(metadata.stable_key).to_string())
        .unwrap_or_else(|| fallback.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_api::FunctionFact;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::calls::facts::{
        CallCallee, CallSiteFact, CallSyntaxKind, CallTargetStatus,
    };
    use crate::analysis_neutral::calls::store::CallOutput;
    use crate::analysis_neutral::entrypoints::facts::{
        DispatchEdgeKind, EntrypointConfidence, EntrypointKind, EntrypointProvenance,
        EntrypointStatus, TriggerMetadata, UnresolvedFrameworkFact,
    };
    use crate::analysis_neutral::entrypoints::store::EntrypointOutput;
    use crate::analysis_neutral::ids::{
        DispatchEdgeId, EntrypointId, MirBodyId, MirOpId, UnresolvedFrameworkId,
    };
    use crate::internal_core::{FileId, Language, Span};

    #[test]
    fn framework_dispatch_fact_produces_refined_edge() {
        let mut db = db_with_function_and_call_site();
        db.replace_entrypoint_facts(EntrypointOutput {
            entrypoints: vec![entrypoint("entrypoint:handler")],
            dispatch_edges: vec![FrameworkDispatchEdgeFact {
                id: DispatchEdgeId(0),
                from_source: "entrypoint:handler".to_string(),
                to_target: FunctionId::from_raw(0),
                to_symbol: None,
                edge_kind: DispatchEdgeKind::RouteDispatch,
                guard_metadata: None,
                ordering: None,
                language: Language::TypeScript,
                file: FileId::from_raw(0),
                span: span(),
                precision: EntrypointPrecision::Heuristic,
                provider_id: "polint.entrypoints".to_string(),
                stable_key: crate::internal_core::stable_key_for_test("dispatch:handler"),
            }],
            ..EntrypointOutput::empty()
        })
        .expect("entrypoints are valid");

        let output = derive_framework_refinements(&db);

        assert_eq!(output.edges.len(), 1);
        assert_eq!(output.edges[0].tier, RefinedCallTier::DirectPlusFramework);
        assert_eq!(output.edges[0].algorithm, CallAlgorithm::FrameworkModel);
        assert_eq!(output.edges[0].precision, CallPrecision::Heuristic);
    }

    #[test]
    fn unresolved_framework_fact_produces_unresolved_refined_row() {
        let mut db = db_with_function_and_call_site();
        db.replace_entrypoint_facts(EntrypointOutput {
            entrypoints: vec![entrypoint("entrypoint:handler")],
            unresolved: vec![UnresolvedFrameworkFact {
                id: UnresolvedFrameworkId(0),
                language: Language::TypeScript,
                file: FileId::from_raw(0),
                span: span(),
                framework_id: "express".to_string(),
                reason: UnresolvedFrameworkReason::DynamicRegistration,
                evidence: "app[method](path, handler)".to_string(),
                scope_description: "router".to_string(),
                precision: EntrypointPrecision::Unknown,
                provider_id: "polint.entrypoints".to_string(),
                stable_key: crate::internal_core::stable_key_for_test("unresolved:framework"),
            }],
            ..EntrypointOutput::empty()
        })
        .expect("entrypoints are valid");

        let output = derive_framework_refinements(&db);

        assert_eq!(output.edges.len(), 1);
        assert_eq!(output.edges[0].status, CallTargetStatus::Unresolved);
        assert_eq!(
            output.edges[0].reason,
            Some(UnresolvedCallReason::FrameworkDispatch)
        );
    }

    fn db_with_function_and_call_site() -> LocalAnalysisDb {
        let mut db = LocalAnalysisDb::new();
        let file = db.add_file(
            "src/app.ts".into(),
            "src/app.ts".to_string(),
            "export function handler() { framework(); }\n".to_string(),
        );
        let function = db.push_function(FunctionFact::new(
            FunctionId::from_raw(99),
            file,
            "handler".to_string(),
            span(),
            Language::TypeScript,
            false,
            true,
            1,
            vec!["framework".to_string()],
        ));
        db.replace_call_facts(CallOutput {
            sites: vec![CallSiteFact {
                in_throw: false,
                id: CallSiteId(0),
                language: Language::TypeScript,
                file,
                caller: function,
                owner_symbol: None,
                body: MirBodyId(0),
                operation: MirOpId(0),
                span: span(),
                kind: CallSyntaxKind::Function,
                callee: CallCallee::Identifier {
                    reference: None,
                    name: "framework".to_string(),
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
        .expect("valid call facts");
        db
    }

    fn entrypoint(stable_key: &str) -> EntrypointFact {
        EntrypointFact {
            id: EntrypointId(0),
            language: Language::TypeScript,
            framework_id: "express".to_string(),
            kind: EntrypointKind::HttpRoute,
            target_function: FunctionId::from_raw(0),
            target_symbol: None,
            registration_span: span(),
            registration_file: FileId::from_raw(0),
            trigger_metadata: TriggerMetadata::empty(),
            trust_boundary_link: None,
            precision: EntrypointPrecision::SetupAware,
            provenance: EntrypointProvenance::NativeRecognizer,
            confidence: EntrypointConfidence::High,
            status: EntrypointStatus::Resolved,
            provider_id: "polint.entrypoints".to_string(),
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
        }
    }

    fn span() -> Span {
        Span::point(FileId::from_raw(0), 1, 1)
    }
}
