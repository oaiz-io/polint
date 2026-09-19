//! Facade composition for refined-call provider inputs.

use crate::analysis_api::{Digest, InputSnapshot, ProviderManifest};

use crate::core::AnalysisDb;

pub(crate) use crate::analysis_neutral::refined_calls::provider::RefinedCallsProviderOutput;
use crate::analysis_neutral::refined_calls::provider::{
    TsTypeCalleeInput, TsTypeCallsiteInput, TsTypeDispatchKind, TsTypeFileDensityInput,
    TsTypeSiteStatus,
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn derive_refined_calls_with_cache_stats(
    db: &mut AnalysisDb,
    input_snapshot: &InputSnapshot,
    manifest: &ProviderManifest,
    calls_output_digest: Digest,
    entrypoints_output_digest: Digest,
    direct_summaries_output_digest: Digest,
    type_value_alias_output_digest: Digest,
    extensions_output_digest: Digest,
    solver_output_digest: Digest,
) -> RefinedCallsProviderOutput {
    let go_semantic_functions = db
        .go_semantic_functions()
        .iter()
        .map(
            |function| crate::analysis_neutral::refined_calls::provider::GoSemanticFunctionInput {
                qualified: function.qualified.clone(),
                name: function.name.clone(),
                file: function.file,
                span: function.span.clone(),
            },
        )
        .collect::<Vec<_>>();
    let go_semantic_callsites = db
        .go_semantic_callsites()
        .iter()
        .map(
            |callsite| crate::analysis_neutral::refined_calls::provider::GoSemanticCallsiteInput {
                stable_key: callsite.stable_key,
                caller: callsite.caller.clone(),
                file: callsite.file,
                span: callsite.span.clone(),
            },
        )
        .collect::<Vec<_>>();
    let (ts_type_callsites, ts_type_callees, ts_type_file_densities) = ts_type_inputs(db);
    crate::analysis_neutral::refined_calls::provider::derive_refined_calls_with_cache_stats(
        db,
        input_snapshot,
        manifest,
        calls_output_digest,
        entrypoints_output_digest,
        direct_summaries_output_digest,
        type_value_alias_output_digest,
        extensions_output_digest,
        solver_output_digest,
        &go_semantic_functions,
        &go_semantic_callsites,
        &ts_type_callsites,
        &ts_type_callees,
        &ts_type_file_densities,
    )
}

/// Projects the TS type-sidecar store into the neutral refinement's inputs.
///
/// The neutral refinement must not know about the TypeScript frontend, so the
/// composition root flattens the two rows it needs to join — a callee and the
/// callable it names — into one input carrying the declaration's name and name
/// span.
fn ts_type_inputs(
    db: &AnalysisDb,
) -> (
    Vec<TsTypeCallsiteInput>,
    Vec<TsTypeCalleeInput>,
    Vec<TsTypeFileDensityInput>,
) {
    use crate::ts::types::facts::{TsTypeCallStatus, TsTypeDispatch};

    let printed_by_site = db
        .ts_type_receivers()
        .iter()
        .map(|receiver| (receiver.callsite_stable_key, receiver.printed.as_str()))
        .collect::<std::collections::BTreeMap<_, _>>();

    let callsites = db
        .ts_type_callsites()
        .iter()
        .map(|callsite| TsTypeCallsiteInput {
            stable_key: callsite.stable_key,
            file: callsite.file,
            span: callsite.span.clone(),
            receiver_printed: printed_by_site
                .get(&callsite.stable_key)
                .copied()
                .filter(|printed| !printed.is_empty())
                .map(str::to_string),
            status: match callsite.status {
                TsTypeCallStatus::Resolved => TsTypeSiteStatus::Resolved,
                TsTypeCallStatus::Union => TsTypeSiteStatus::Union,
                TsTypeCallStatus::External => TsTypeSiteStatus::External,
                TsTypeCallStatus::AnyReceiver => TsTypeSiteStatus::AnyReceiver,
                TsTypeCallStatus::Unresolved => TsTypeSiteStatus::Unresolved,
            },
        })
        .collect::<Vec<_>>();

    let callables_by_identity = db
        .ts_type_callables()
        .iter()
        .map(|callable| (callable.callable.as_str(), callable))
        .collect::<std::collections::BTreeMap<_, _>>();

    let callees = db
        .ts_type_callees()
        .iter()
        .map(|callee| {
            let declaration = callee
                .callable
                .as_deref()
                .and_then(|identity| callables_by_identity.get(identity).copied());
            TsTypeCalleeInput {
                stable_key: callee.stable_key,
                callsite_stable_key: callee.callsite_stable_key,
                dispatch: match callee.dispatch {
                    TsTypeDispatch::Declared => TsTypeDispatchKind::Declared,
                    TsTypeDispatch::DeclaredSignature => TsTypeDispatchKind::DeclaredSignature,
                    TsTypeDispatch::Implementation => TsTypeDispatchKind::Implementation,
                    TsTypeDispatch::UnionMember => TsTypeDispatchKind::UnionMember,
                },
                name: declaration
                    .map(|callable| callable.name.clone())
                    .unwrap_or_default(),
                external: callee.external.clone(),
                file: callee.file,
                span: callee.span.clone(),
                name_span: declaration.and_then(|callable| callable.name_span.clone()),
            }
        })
        .collect::<Vec<_>>();

    let densities = db
        .ts_type_file_densities()
        .iter()
        .map(|density| TsTypeFileDensityInput {
            file: density.file,
            any_percent: density.any_percent(),
        })
        .collect::<Vec<_>>();

    (callsites, callees, densities)
}

#[cfg(test)]
pub(crate) use crate::analysis::calls::facts::{
    CallPrecision, CallSiteFact, CallSyntaxKind, CallTargetStatus,
};
#[cfg(test)]
pub(crate) use crate::analysis::points_to::facts::{PointsToPrecision, PointsToStatus};
#[cfg(test)]
pub(crate) use crate::analysis::refined_calls::facts::{
    RefinedCallConfidence, RefinedCallEdgeFact, RefinedCallTier, RefinedCallValidation,
};
#[cfg(test)]
pub(crate) use crate::analysis::refined_calls::store::RefinedCallOutput;
#[cfg(test)]
pub(crate) use crate::analysis::semantic_graph::facts::NodeKind;
#[cfg(test)]
pub(crate) use crate::analysis::solver::facts::DerivedEdgeFact;
#[cfg(test)]
pub(crate) use crate::analysis_kernel::incremental::DigestKind;
pub(crate) const REFINED_CALLS_PROVIDER_ID: &str =
    crate::analysis_neutral::refined_calls::provider::REFINED_CALLS_PROVIDER_ID;
#[cfg(test)]
use crate::internal_core::StableKeyInterner;

#[cfg(test)]
fn derive_solver_refinements(
    db: &AnalysisDb,
) -> crate::analysis::refined_calls::store::RefinedCallOutput {
    let go_semantic_functions = db
        .go_semantic_functions()
        .iter()
        .map(
            |function| crate::analysis_neutral::refined_calls::provider::GoSemanticFunctionInput {
                qualified: function.qualified.clone(),
                name: function.name.clone(),
                file: function.file,
                span: function.span.clone(),
            },
        )
        .collect::<Vec<_>>();
    let go_semantic_callsites = db
        .go_semantic_callsites()
        .iter()
        .map(
            |callsite| crate::analysis_neutral::refined_calls::provider::GoSemanticCallsiteInput {
                stable_key: callsite.stable_key,
                caller: callsite.caller.clone(),
                file: callsite.file,
                span: callsite.span.clone(),
            },
        )
        .collect::<Vec<_>>();
    crate::analysis_neutral::refined_calls::provider::derive_solver_refinements_with_inputs(
        db,
        &go_semantic_functions,
        &go_semantic_callsites,
    )
}

#[cfg(test)]
fn finalized_output(
    interner: &StableKeyInterner,
    output: crate::analysis::refined_calls::store::RefinedCallOutput,
) -> crate::analysis::refined_calls::store::RefinedCallOutput {
    crate::analysis_neutral::refined_calls::provider::finalized_output(interner, output)
}

#[cfg(test)]
fn stable_refined_call_key(
    interner: &StableKeyInterner,
    tier: crate::analysis::refined_calls::facts::RefinedCallTier,
    base_target_key: &str,
    site_key: &str,
) -> crate::internal_core::StableKeyId {
    crate::analysis_neutral::refined_calls::provider::stable_refined_call_key(
        interner,
        tier,
        base_target_key,
        site_key,
    )
}

#[cfg(test)]
mod solver_projection_tests {
    use super::*;
    use crate::analysis::calls::facts::CallCallee;
    use crate::analysis::calls::store::CallOutput;
    use crate::analysis::ids::{
        CallSiteId, DerivedEdgeId, MirBodyId, MirOpId, SemanticConstraintId, SemanticNodeId,
    };
    use crate::analysis::semantic_graph::constraints::{ConstraintFact, ConstraintKind};
    use crate::analysis::semantic_graph::facts::{SemanticNodeFact, SemanticPrecision};
    use crate::analysis::semantic_graph::store::SemanticGraphOutput;
    use crate::analysis::solver::budget::BudgetStatus;
    use crate::analysis::solver::provenance::{ContributingFact, DerivedEdgeProvenance};
    use crate::analysis::solver::store::SolverOutput;
    use crate::analysis_kernel::AnalysisKernel;
    use crate::analysis_kernel::incremental::InputSnapshot;
    use crate::analysis_plan::AnalysisPlan;
    use crate::config::load_config;
    use crate::core::{FileId, FunctionFact, FunctionId, Language, Span};
    use crate::go::semantic::facts::{
        GoSemanticCallStatus, GoSemanticCallsiteFact, GoSemanticFunctionFact, GoSemanticFunctionId,
        GoSemanticFunctionKind,
    };
    use crate::go::semantic::store::GoSemanticFactsOutput;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn solver_derived_call_edges_project_to_refined_calls() {
        let db = db_with_solver_edge();

        let output = derive_solver_refinements(&db);

        assert_eq!(output.edges.len(), 1);
        let edge = &output.edges[0];
        assert_eq!(edge.site, CallSiteId(0));
        assert_eq!(edge.base_target, None);
        assert_eq!(edge.caller, FunctionId::from_raw(0));
        assert_eq!(edge.target_function, Some(FunctionId::from_raw(1)));
        assert_eq!(edge.tier, RefinedCallTier::PointsToAssisted);
        assert_eq!(edge.status, CallTargetStatus::Resolved);
        assert_eq!(edge.precision, CallPrecision::SetupAware);
        assert_eq!(
            edge.validation,
            RefinedCallValidation::ReferentiallyValidated
        );
        assert!(edge.evidence.contains(&"solver_derived_edge".to_string()));
        assert!(
            edge.input_stable_keys
                .iter()
                .any(|key| key == "call-site:callee")
        );
    }

    #[test]
    fn solver_projection_resolves_semantic_call_constraint_keys_to_core_callsites() {
        let db = db_with_solver_edge_referenced_by_semantic_constraint_key();

        let output = derive_solver_refinements(&db);

        assert_eq!(output.edges.len(), 1);
        let edge = &output.edges[0];
        assert_eq!(edge.site, CallSiteId(0));
        assert_eq!(edge.caller, FunctionId::from_raw(0));
        assert_eq!(edge.target_function, Some(FunctionId::from_raw(1)));
        assert_eq!(edge.tier, RefinedCallTier::PointsToAssisted);
        assert!(
            edge.input_stable_keys
                .iter()
                .any(|key| key == "constraint:go-semantic-callsite")
        );
    }

    #[test]
    fn solver_projection_resolves_go_semantic_callsite_keys_to_core_callsites() {
        let db = db_with_solver_edge_referenced_by_go_semantic_callsite_key();

        let output = derive_solver_refinements(&db);

        assert_eq!(output.edges.len(), 1);
        let edge = &output.edges[0];
        assert_eq!(edge.site, CallSiteId(0));
        assert_eq!(edge.caller, FunctionId::from_raw(0));
        assert_eq!(edge.target_function, Some(FunctionId::from_raw(1)));
        assert_eq!(edge.tier, RefinedCallTier::PointsToAssisted);
        assert!(
            edge.input_stable_keys
                .iter()
                .any(|key| key == "go-semantic-callsite:caller-callee")
        );
    }

    #[test]
    fn solver_projection_resolves_zero_width_go_semantic_method_callers() {
        let db = db_with_zero_width_go_method_caller_solver_edge();

        let output = derive_solver_refinements(&db);

        assert_eq!(output.edges.len(), 1);
        let edge = &output.edges[0];
        assert_eq!(edge.site, CallSiteId(0));
        assert_eq!(edge.caller, FunctionId::from_raw(0));
        assert_eq!(edge.target_function, Some(FunctionId::from_raw(1)));
        assert_eq!(edge.tier, RefinedCallTier::PointsToAssisted);
        assert!(
            edge.input_stable_keys
                .iter()
                .any(|key| key == "go-semantic-callsite:method-dispatch")
        );
    }

    #[test]
    fn refined_calls_digest_changes_with_solver_output_digest() {
        let mut db_a = db_with_solver_edge();
        let snapshot_a = snapshot(&db_a);
        let base = derive_refined_calls_with_cache_stats(
            &mut db_a,
            &snapshot_a,
            manifest(),
            absent("polint.calls"),
            absent("polint.entrypoints"),
            absent("polint.direct_summaries"),
            absent("polint.type_value_alias"),
            absent("polint.extensions"),
            Digest::from_parts(DigestKind::ProviderOutput, "polint.solver", &["a"]),
        )
        .output_digest;

        let mut db_b = db_with_solver_edge();
        let snapshot_b = snapshot(&db_b);
        let changed = derive_refined_calls_with_cache_stats(
            &mut db_b,
            &snapshot_b,
            manifest(),
            absent("polint.calls"),
            absent("polint.entrypoints"),
            absent("polint.direct_summaries"),
            absent("polint.type_value_alias"),
            absent("polint.extensions"),
            Digest::from_parts(DigestKind::ProviderOutput, "polint.solver", &["b"]),
        )
        .output_digest;

        assert_ne!(base, changed);
    }

    fn db_with_solver_edge() -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let interner = db.stable_key_interner();
        let file = db.add_file(
            "app.ts".into(),
            "app.ts".to_string(),
            "callee();\n".to_string(),
        );
        db.push_function(ts_function(
            FunctionId::from_raw(0),
            file,
            "caller",
            vec!["callee".to_string()],
        ));
        db.push_function(ts_function(
            FunctionId::from_raw(1),
            file,
            "callee",
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
                stable_key: interner.intern("call-site:callee"),
            }],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("valid calls");
        db.replace_semantic_graph_facts(SemanticGraphOutput {
            nodes: vec![
                semantic_node(
                    &db,
                    SemanticNodeId(0),
                    NodeKind::Function(FunctionId::from_raw(0)),
                    "node:function:caller",
                ),
                semantic_node(
                    &db,
                    SemanticNodeId(1),
                    NodeKind::Function(FunctionId::from_raw(1)),
                    "node:function:callee",
                ),
            ],
            edges: Vec::new(),
            constraints: Vec::new(),
        })
        .expect("valid semantic graph");

        let caller_node = function_node(&db, FunctionId::from_raw(0));
        let callee_node = function_node(&db, FunctionId::from_raw(1));
        let provenance = DerivedEdgeProvenance::new(
            &interner,
            vec![
                ContributingFact {
                    stable_key: interner.intern("call-site:callee"),
                },
                ContributingFact {
                    stable_key: interner.intern("constraint:call"),
                },
            ],
            &ConstraintKind::CallConstraint {
                callsite: SemanticNodeId(99),
            },
            1,
        );
        db.replace_solver_facts(SolverOutput {
            derived_edges: vec![DerivedEdgeFact {
                id: DerivedEdgeId(0),
                source: caller_node,
                target: callee_node,
                status: PointsToStatus::Present,
                precision: PointsToPrecision::FlowInsensitive,
                stable_key: interner.intern("solver-edge:caller-callee"),
                provenance,
            }],
            budget_status: BudgetStatus::WithinBudget,
            ..SolverOutput::default()
        })
        .expect("valid solver facts");
        db
    }

    fn db_with_solver_edge_referenced_by_semantic_constraint_key() -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            "main.go".into(),
            "main.go".to_string(),
            "package main\nfunc caller(){ callee() }\nfunc callee() {}\n".to_string(),
        );
        db.push_function(go_function(
            FunctionId::from_raw(0),
            file,
            "caller",
            vec!["callee".to_string()],
        ));
        db.push_function(go_function(
            FunctionId::from_raw(1),
            file,
            "callee",
            Vec::new(),
        ));
        db.replace_call_facts(CallOutput {
            sites: vec![CallSiteFact {
                in_throw: false,
                id: CallSiteId(0),
                language: Language::Go,
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
                stable_key: db.stable_key_interner().intern("call-site:go-callee"),
            }],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("valid calls");
        db.replace_semantic_graph_facts(SemanticGraphOutput {
            nodes: vec![
                semantic_node(
                    &db,
                    SemanticNodeId(0),
                    NodeKind::Function(FunctionId::from_raw(0)),
                    "node:function:caller",
                ),
                semantic_node(
                    &db,
                    SemanticNodeId(1),
                    NodeKind::Function(FunctionId::from_raw(1)),
                    "node:function:callee",
                ),
                semantic_node(
                    &db,
                    SemanticNodeId(2),
                    NodeKind::Callsite(CallSiteId(0)),
                    "node:callsite:callee",
                ),
            ],
            edges: Vec::new(),
            constraints: vec![ConstraintFact {
                id: SemanticConstraintId(0),
                kind: ConstraintKind::CallConstraint {
                    callsite: SemanticNodeId(2),
                },
                status: PointsToStatus::Present,
                precision: PointsToPrecision::FlowInsensitive,
                stable_key: db
                    .stable_key_interner()
                    .intern("constraint:go-semantic-callsite"),
            }],
        })
        .expect("valid semantic graph");

        let caller_node = function_node(&db, FunctionId::from_raw(0));
        let callee_node = function_node(&db, FunctionId::from_raw(1));
        let callsite_node = callsite_node(&db, CallSiteId(0));
        let interner = db.stable_key_interner();
        let provenance = DerivedEdgeProvenance::new(
            &interner,
            vec![ContributingFact {
                stable_key: interner.intern("constraint:go-semantic-callsite"),
            }],
            &ConstraintKind::CallConstraint {
                callsite: callsite_node,
            },
            1,
        );
        db.replace_solver_facts(SolverOutput {
            derived_edges: vec![DerivedEdgeFact {
                id: DerivedEdgeId(0),
                source: caller_node,
                target: callee_node,
                status: PointsToStatus::Present,
                precision: PointsToPrecision::FlowInsensitive,
                stable_key: interner.intern("solver-edge:go-caller-callee"),
                provenance,
            }],
            budget_status: BudgetStatus::WithinBudget,
            ..SolverOutput::default()
        })
        .expect("valid solver facts");
        db
    }

    fn db_with_solver_edge_referenced_by_go_semantic_callsite_key() -> AnalysisDb {
        let mut db = db_with_solver_edge_referenced_by_semantic_constraint_key();
        let interner = db.stable_key_interner();
        let file = db
            .files()
            .iter()
            .find(|file| file.relative_path == "main.go")
            .map(|file| file.id)
            .expect("main.go file");
        db.replace_go_semantic_facts(GoSemanticFactsOutput {
            functions: vec![
                go_semantic_function(
                    &interner,
                    "go-function:caller",
                    "pkg.caller",
                    file,
                    FunctionId::from_raw(0),
                ),
                go_semantic_function(
                    &interner,
                    "go-function:callee",
                    "pkg.callee",
                    file,
                    FunctionId::from_raw(1),
                ),
            ],
            callsites: vec![GoSemanticCallsiteFact {
                id: crate::go::semantic::facts::GoSemanticCallsiteId(0),
                stable_key: interner.intern("go-semantic-callsite:caller-callee"),
                package_id: "pkg".to_string(),
                package_path: "pkg".to_string(),
                caller: "pkg.caller".to_string(),
                static_callee: None,
                status: GoSemanticCallStatus::UnresolvedDynamic,
                reason: None,
                relative_file: Some("main.go".to_string()),
                file: Some(file),
                span: Some(span()),
            }],
            ..GoSemanticFactsOutput::default()
        })
        .expect("valid go semantic facts");

        let caller_node = function_node(&db, FunctionId::from_raw(0));
        let callee_node = function_node(&db, FunctionId::from_raw(1));
        let callsite_node = callsite_node(&db, CallSiteId(0));
        let provenance = DerivedEdgeProvenance::new(
            &interner,
            vec![ContributingFact {
                stable_key: interner.intern("go-semantic-callsite:caller-callee"),
            }],
            &ConstraintKind::CallConstraint {
                callsite: callsite_node,
            },
            1,
        );
        db.replace_solver_facts(SolverOutput {
            derived_edges: vec![DerivedEdgeFact {
                id: DerivedEdgeId(0),
                source: caller_node,
                target: callee_node,
                status: PointsToStatus::Present,
                precision: PointsToPrecision::FlowInsensitive,
                stable_key: interner.intern("solver-edge:go-semantic-caller-callee"),
                provenance,
            }],
            budget_status: BudgetStatus::WithinBudget,
            ..SolverOutput::default()
        })
        .expect("valid solver facts");
        db
    }

    fn db_with_zero_width_go_method_caller_solver_edge() -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            "main.go".into(),
            "main.go".to_string(),
            "package main\ntype Handler struct{}\nfunc (Handler) Handle(){ speaker.Speak() }\nfunc other(){ speaker.Speak() }\nfunc Speak() {}\n".to_string(),
        );
        let caller_span = span_for_file(file, 20, 80);
        let decoy_span = span_for_file(file, 90, 120);
        let callee_span = span_for_file(file, 130, 145);
        let call_span = span_for_file(file, 60, 75);
        let interner = db.stable_key_interner();
        db.push_function(go_function_with_span(
            FunctionId::from_raw(0),
            file,
            "Handler.Handle",
            caller_span,
            vec!["Speak".to_string()],
        ));
        db.push_function(go_function_with_span(
            FunctionId::from_raw(1),
            file,
            "Speak",
            callee_span.clone(),
            Vec::new(),
        ));
        db.push_function(go_function_with_span(
            FunctionId::from_raw(2),
            file,
            "other",
            decoy_span,
            vec!["Speak".to_string()],
        ));
        db.replace_call_facts(CallOutput {
            sites: vec![
                CallSiteFact {
                    in_throw: false,
                    id: CallSiteId(0),
                    language: Language::Go,
                    file,
                    caller: FunctionId::from_raw(0),
                    owner_symbol: None,
                    body: MirBodyId(0),
                    operation: MirOpId(0),
                    span: call_span.clone(),
                    kind: CallSyntaxKind::Method,
                    callee: CallCallee::Identifier {
                        reference: None,
                        name: "Speak".to_string(),
                    },
                    receiver: None,
                    arguments: Vec::new(),
                    result: None,
                    status: CallTargetStatus::Unresolved,
                    precision: CallPrecision::Conservative,
                    stable_key: interner.intern("call-site:handler-speak"),
                },
                CallSiteFact {
                    in_throw: false,
                    id: CallSiteId(1),
                    language: Language::Go,
                    file,
                    caller: FunctionId::from_raw(2),
                    owner_symbol: None,
                    body: MirBodyId(1),
                    operation: MirOpId(0),
                    span: call_span.clone(),
                    kind: CallSyntaxKind::Method,
                    callee: CallCallee::Identifier {
                        reference: None,
                        name: "Speak".to_string(),
                    },
                    receiver: None,
                    arguments: Vec::new(),
                    result: None,
                    status: CallTargetStatus::Unresolved,
                    precision: CallPrecision::Conservative,
                    stable_key: interner.intern("call-site:other-speak"),
                },
            ],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("valid calls");
        db.replace_semantic_graph_facts(SemanticGraphOutput {
            nodes: vec![
                semantic_node(
                    &db,
                    SemanticNodeId(0),
                    NodeKind::Function(FunctionId::from_raw(0)),
                    "node:function:handler-handle",
                ),
                semantic_node(
                    &db,
                    SemanticNodeId(1),
                    NodeKind::Function(FunctionId::from_raw(1)),
                    "node:function:speak",
                ),
            ],
            edges: Vec::new(),
            constraints: Vec::new(),
        })
        .expect("valid semantic graph");
        db.replace_go_semantic_facts(GoSemanticFactsOutput {
            functions: vec![
                go_semantic_method_function_with_span(
                    &interner,
                    "go-function:handler-handle",
                    "Handler.Handle",
                    "pkg.Handler.Handle",
                    file,
                    FunctionId::from_raw(0),
                    span_for_file(file, 25, 25),
                ),
                go_semantic_function_with_span(
                    &interner,
                    "go-function:speak",
                    "Speak",
                    "pkg.Speak",
                    file,
                    FunctionId::from_raw(1),
                    callee_span,
                ),
            ],
            callsites: vec![GoSemanticCallsiteFact {
                id: crate::go::semantic::facts::GoSemanticCallsiteId(0),
                stable_key: interner.intern("go-semantic-callsite:method-dispatch"),
                package_id: "pkg".to_string(),
                package_path: "pkg".to_string(),
                caller: "pkg.Handler.Handle".to_string(),
                static_callee: None,
                status: GoSemanticCallStatus::UnresolvedDynamic,
                reason: None,
                relative_file: Some("main.go".to_string()),
                file: Some(file),
                span: Some(call_span),
            }],
            ..GoSemanticFactsOutput::default()
        })
        .expect("valid go semantic facts");

        let caller_node = function_node(&db, FunctionId::from_raw(0));
        let callee_node = function_node(&db, FunctionId::from_raw(1));
        let provenance = DerivedEdgeProvenance::new(
            &interner,
            vec![ContributingFact {
                stable_key: interner.intern("go-semantic-callsite:method-dispatch"),
            }],
            &ConstraintKind::CallConstraint {
                callsite: SemanticNodeId(99),
            },
            1,
        );
        db.replace_solver_facts(SolverOutput {
            derived_edges: vec![DerivedEdgeFact {
                id: DerivedEdgeId(0),
                source: caller_node,
                target: callee_node,
                status: PointsToStatus::Present,
                precision: PointsToPrecision::FlowInsensitive,
                stable_key: interner.intern("solver-edge:method-dispatch"),
                provenance,
            }],
            budget_status: BudgetStatus::WithinBudget,
            ..SolverOutput::default()
        })
        .expect("valid solver facts");
        db
    }

    fn ts_function(id: FunctionId, file: FileId, name: &str, calls: Vec<String>) -> FunctionFact {
        function(id, file, name, Language::TypeScript, calls)
    }

    fn go_function(id: FunctionId, file: FileId, name: &str, calls: Vec<String>) -> FunctionFact {
        function(id, file, name, Language::Go, calls)
    }

    fn go_function_with_span(
        id: FunctionId,
        file: FileId,
        name: &str,
        span: Span,
        calls: Vec<String>,
    ) -> FunctionFact {
        function_with_span(id, file, name, Language::Go, span, calls)
    }

    fn function(
        id: FunctionId,
        file: FileId,
        name: &str,
        language: Language,
        calls: Vec<String>,
    ) -> FunctionFact {
        function_with_span(id, file, name, language, span(), calls)
    }

    fn function_with_span(
        id: FunctionId,
        file: FileId,
        name: &str,
        language: Language,
        span: Span,
        calls: Vec<String>,
    ) -> FunctionFact {
        FunctionFact::new(
            id,
            file,
            name.to_string(),
            span,
            language,
            false,
            true,
            1,
            calls,
        )
    }

    fn go_semantic_function(
        interner: &crate::core::StableKeyInterner,
        stable_key: &str,
        qualified: &str,
        file: FileId,
        function: FunctionId,
    ) -> GoSemanticFunctionFact {
        go_semantic_function_with_span(
            interner,
            stable_key,
            qualified.rsplit('.').next().unwrap_or(qualified),
            qualified,
            file,
            function,
            span(),
        )
    }

    fn go_semantic_function_with_span(
        interner: &crate::core::StableKeyInterner,
        stable_key: &str,
        name: &str,
        qualified: &str,
        file: FileId,
        function: FunctionId,
        span: Span,
    ) -> GoSemanticFunctionFact {
        GoSemanticFunctionFact {
            id: GoSemanticFunctionId(function.0),
            stable_key: interner.intern(stable_key),
            package_id: "pkg".to_string(),
            package_path: "pkg".to_string(),
            name: name.to_string(),
            qualified: qualified.to_string(),
            signature: "func()".to_string(),
            kind: GoSemanticFunctionKind::Function,
            receiver: None,
            relative_file: Some("main.go".to_string()),
            file: Some(file),
            span: Some(span),
        }
    }

    fn go_semantic_method_function_with_span(
        interner: &crate::core::StableKeyInterner,
        stable_key: &str,
        name: &str,
        qualified: &str,
        file: FileId,
        function: FunctionId,
        span: Span,
    ) -> GoSemanticFunctionFact {
        GoSemanticFunctionFact {
            kind: GoSemanticFunctionKind::Method,
            receiver: Some("pkg.Handler".to_string()),
            ..go_semantic_function_with_span(
                interner, stable_key, name, qualified, file, function, span,
            )
        }
    }

    fn semantic_node(
        db: &AnalysisDb,
        id: SemanticNodeId,
        kind: NodeKind,
        stable_key: &str,
    ) -> SemanticNodeFact {
        SemanticNodeFact {
            id,
            kind,
            precision: SemanticPrecision::SetupAware,
            stable_key: db.stable_key_interner().intern(stable_key),
        }
    }

    fn function_node(db: &AnalysisDb, function: FunctionId) -> SemanticNodeId {
        db.semantic_nodes()
            .iter()
            .find_map(|node| {
                let NodeKind::Function(candidate) = node.kind else {
                    return None;
                };
                (candidate == function).then_some(node.id)
            })
            .expect("function node")
    }

    fn callsite_node(db: &AnalysisDb, callsite: CallSiteId) -> SemanticNodeId {
        db.semantic_nodes()
            .iter()
            .find_map(|node| {
                let NodeKind::Callsite(candidate) = node.kind else {
                    return None;
                };
                (candidate == callsite).then_some(node.id)
            })
            .expect("callsite node")
    }

    fn manifest() -> &'static ProviderManifest {
        AnalysisKernel::provider_manifests()
            .iter()
            .find(|manifest| manifest.id == REFINED_CALLS_PROVIDER_ID)
            .expect("refined calls manifest")
    }

    fn snapshot(db: &AnalysisDb) -> InputSnapshot {
        let temp = tempdir().expect("tempdir");
        fs::write(temp.path().join(".polint.toml"), "").expect("config");
        let loaded = load_config(temp.path()).expect("config loads");
        crate::analysis_kernel::incremental::input_snapshot_from_run_inputs(
            &loaded,
            db,
            "config-a",
            "rules-a",
            AnalysisPlan::empty().digest(),
            AnalysisKernel::provider_manifests(),
        )
    }

    fn absent(provider: &str) -> Digest {
        Digest::absent(DigestKind::ProviderOutput, provider)
    }

    fn span() -> Span {
        Span::point(FileId::from_raw(0), 1, 1)
    }

    fn span_for_file(file: FileId, start_byte: u32, end_byte: u32) -> Span {
        Span::new(file, start_byte, end_byte, 1, 1, 1, 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::calls::facts::{
        CallAlgorithm, CallEdgeKind, CallPrecision, CallProvenance, CallTargetStatus,
    };
    use crate::analysis::ids::CallSiteId;
    use crate::core::FunctionId;

    #[test]
    fn refined_key_uses_stable_site_key_instead_of_dense_site_id() {
        let db = crate::core::AnalysisDb::new();
        let interner = db.stable_key_interner();
        assert_eq!(
            stable_refined_call_key(
                &interner,
                RefinedCallTier::DirectOnly,
                "call-target:stable",
                "call-site:stable",
            ),
            stable_refined_call_key(
                &interner,
                RefinedCallTier::DirectOnly,
                "call-target:stable",
                "call-site:stable",
            )
        );
    }

    #[test]
    fn finalized_output_reassigns_dense_ids_after_sorting() {
        let interner = crate::core::test_stable_key_interner();
        let output = finalized_output(
            &interner,
            RefinedCallOutput {
                edges: vec![refined_edge("z", 10), refined_edge("a", 10)],
            },
        );

        assert_eq!(
            output
                .edges
                .iter()
                .map(|edge| (edge.id.0, interner.resolve(edge.stable_key).to_string()))
                .collect::<Vec<_>>(),
            vec![(0, "a".to_string()), (1, "z".to_string())]
        );
    }

    fn refined_edge(stable_key: &str, id: u64) -> RefinedCallEdgeFact {
        RefinedCallEdgeFact {
            id: crate::analysis::ids::RefinedCallEdgeId(id),
            site: CallSiteId(0),
            base_target: None,
            caller: FunctionId::from_raw(0),
            target_function: None,
            target_symbol: None,
            synthetic_target: None,
            language: crate::core::Language::TypeScript,
            edge_kind: CallEdgeKind::Synthetic,
            algorithm: CallAlgorithm::FrameworkModel,
            tier: RefinedCallTier::DirectPlusFramework,
            status: CallTargetStatus::Resolved,
            reason: None,
            provenance: CallProvenance::Model,
            precision: CallPrecision::Heuristic,
            validation: RefinedCallValidation::Native,
            confidence: RefinedCallConfidence::Medium,
            evidence: Vec::new(),
            input_stable_keys: Vec::new(),
            stable_key: crate::core::stable_key_for_test(stable_key),
        }
    }
}
