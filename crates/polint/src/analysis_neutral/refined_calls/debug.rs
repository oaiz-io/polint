use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::json;

use crate::analysis_api::{FactFamily, FactRef};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::calls::facts::{
    CallAlgorithm, CallPrecision, CallProvenance, CallTargetStatus, UnresolvedCallReason,
};
use crate::analysis_neutral::refined_calls::facts::RefinedCallTier;

pub fn refined_calls_debug_json_for_test(db: &impl AnalysisHost) -> serde_json::Value {
    let edges = db
        .refined_call_edges()
        .iter()
        .map(|edge| RefinedCallDebugRow {
            family: FactFamily::RefinedCallEdge.label(),
            stable_key_text: db.resolve_stable_key(edge.stable_key).to_string(),
            producer_id: "polint.refined_calls",
            layer_id: "polint.refined_calls",
            language: format!("{:?}", edge.language),
            tier: tier_label(edge.tier),
            edge_kind: format!("{:?}", edge.edge_kind),
            algorithm: algorithm_label(edge.algorithm),
            status: status_label(edge.status),
            precision: precision_label(edge.precision),
            provenance: provenance_label(edge.provenance),
            reason: edge.reason.map(reason_label),
            validation: format!("{:?}", edge.validation),
            confidence: format!("{:?}", edge.confidence),
            site_stable_key: db
                .metadata_for(FactRef::new(FactFamily::CallSite, edge.site.0))
                .map(|metadata| db.resolve_stable_key(metadata.stable_key).to_string()),
            base_target_stable_key: edge.base_target.and_then(|target| {
                db.metadata_for(FactRef::new(FactFamily::CallTarget, target.0))
                    .map(|metadata| db.resolve_stable_key(metadata.stable_key).to_string())
            }),
            caller_stable_key: db
                .metadata_for(FactRef::new(FactFamily::Function, edge.caller.0))
                .map(|metadata| db.resolve_stable_key(metadata.stable_key).to_string()),
            target_function_stable_key: edge.target_function.and_then(|function| {
                db.metadata_for(FactRef::new(FactFamily::Function, function.0))
                    .map(|metadata| db.resolve_stable_key(metadata.stable_key).to_string())
            }),
            target_symbol_stable_key: edge.target_symbol.and_then(|symbol| {
                db.metadata_for(FactRef::new(FactFamily::Symbol, symbol.0))
                    .map(|metadata| db.resolve_stable_key(metadata.stable_key).to_string())
            }),
            synthetic_target: edge.synthetic_target.clone(),
            evidence_count: edge.evidence.len(),
            input_count: edge.input_stable_keys.len(),
        })
        .collect::<Vec<_>>();

    json!({
        "edges": edges,
        "counts": counts(db),
        "deltas": deltas(db),
    })
}

#[derive(Serialize)]
struct RefinedCallDebugRow {
    family: &'static str,
    #[serde(rename = "stable_key")]
    stable_key_text: String,
    producer_id: &'static str,
    layer_id: &'static str,
    language: String,
    tier: &'static str,
    edge_kind: String,
    algorithm: &'static str,
    status: &'static str,
    precision: &'static str,
    provenance: &'static str,
    reason: Option<&'static str>,
    validation: String,
    confidence: String,
    site_stable_key: Option<String>,
    base_target_stable_key: Option<String>,
    caller_stable_key: Option<String>,
    target_function_stable_key: Option<String>,
    target_symbol_stable_key: Option<String>,
    synthetic_target: Option<String>,
    evidence_count: usize,
    input_count: usize,
}

fn counts(db: &impl AnalysisHost) -> serde_json::Value {
    let mut by_language = BTreeMap::new();
    let mut by_algorithm = BTreeMap::new();
    let mut by_tier = BTreeMap::new();
    let mut by_status = BTreeMap::new();
    let mut by_precision = BTreeMap::new();
    let mut by_provenance = BTreeMap::new();
    let mut by_reason = BTreeMap::new();

    for edge in db.refined_call_edges() {
        increment(&mut by_language, format!("{:?}", edge.language));
        increment(
            &mut by_algorithm,
            algorithm_label(edge.algorithm).to_string(),
        );
        increment(&mut by_tier, tier_label(edge.tier).to_string());
        increment(&mut by_status, status_label(edge.status).to_string());
        increment(
            &mut by_precision,
            precision_label(edge.precision).to_string(),
        );
        increment(
            &mut by_provenance,
            provenance_label(edge.provenance).to_string(),
        );
        if let Some(reason) = edge.reason {
            increment(&mut by_reason, reason_label(reason).to_string());
        }
    }

    json!({
        "total_edges": db.refined_call_edges().len(),
        "direct_edges": db.call_targets().len(),
        "refined_non_direct_edges": db
            .refined_call_edges()
            .iter()
            .filter(|edge| edge.tier != RefinedCallTier::DirectOnly)
            .count(),
        "by_language": by_language,
        "by_algorithm": by_algorithm,
        "by_tier": by_tier,
        "by_status": by_status,
        "by_precision": by_precision,
        "by_provenance": by_provenance,
        "by_reason": by_reason,
    })
}

fn deltas(db: &impl AnalysisHost) -> serde_json::Value {
    let direct_edges = db.call_targets().len();
    let refined_edges = db.refined_call_edges().len();
    let extension_edges = db
        .refined_call_edges()
        .iter()
        .filter(|edge| edge.tier == RefinedCallTier::ExtensionModel)
        .count();
    let unresolved = db
        .refined_call_edges()
        .iter()
        .filter(|edge| edge.status == CallTargetStatus::Unresolved)
        .count();
    let budget_exceeded = db
        .refined_call_edges()
        .iter()
        .filter(|edge| edge.status == CallTargetStatus::BudgetExceeded)
        .count();

    json!({
        "direct_edges": direct_edges,
        "refined_edges": refined_edges,
        "changed_edges": refined_edges.saturating_sub(direct_edges),
        "extension_model_edges": extension_edges,
        "unresolved_refined_edges": unresolved,
        "budget_exceeded_refined_edges": budget_exceeded,
    })
}

fn increment(map: &mut BTreeMap<String, usize>, key: String) {
    *map.entry(key).or_default() += 1;
}

fn tier_label(tier: RefinedCallTier) -> &'static str {
    match tier {
        RefinedCallTier::DirectOnly => "direct_only",
        RefinedCallTier::DirectPlusFramework => "direct_plus_framework",
        RefinedCallTier::TypeDirected => "type_directed",
        RefinedCallTier::TypeValueFunctionToken => "type_value_function_token",
        RefinedCallTier::SummaryAssisted => "summary_assisted",
        RefinedCallTier::PointsToAssisted => "points_to_assisted",
        RefinedCallTier::ExtensionModel => "extension_model",
        RefinedCallTier::AllAccepted => "all_accepted",
    }
}

fn algorithm_label(algorithm: CallAlgorithm) -> &'static str {
    match algorithm {
        CallAlgorithm::SyntaxOnly => "syntax_only",
        CallAlgorithm::DirectReference => "direct_reference",
        CallAlgorithm::ImportBinding => "import_binding",
        CallAlgorithm::ConstructorBinding => "constructor_binding",
        CallAlgorithm::StaticMember => "static_member",
        CallAlgorithm::DirectMember => "direct_member",
        CallAlgorithm::GoStatic => "go_static",
        CallAlgorithm::GoCha => "go_cha",
        CallAlgorithm::GoRta => "go_rta",
        CallAlgorithm::GoVta => "go_vta",
        CallAlgorithm::TypeHierarchy => "type_hierarchy",
        CallAlgorithm::PointsTo => "points_to",
        CallAlgorithm::SummaryAssisted => "summary_assisted",
        CallAlgorithm::FrameworkModel => "framework_model",
        CallAlgorithm::RepoModel => "repo_model",
        CallAlgorithm::Unsupported => "unsupported",
    }
}

fn status_label(status: CallTargetStatus) -> &'static str {
    match status {
        CallTargetStatus::Resolved => "resolved",
        CallTargetStatus::Ambiguous => "ambiguous",
        CallTargetStatus::Unresolved => "unresolved",
        CallTargetStatus::Unsupported => "unsupported",
        CallTargetStatus::SetupMissing => "setup_missing",
        CallTargetStatus::BudgetExceeded => "budget_exceeded",
        CallTargetStatus::Rejected => "rejected",
    }
}

fn precision_label(precision: CallPrecision) -> &'static str {
    match precision {
        CallPrecision::Exact => "exact",
        CallPrecision::SetupAware => "setup_aware",
        CallPrecision::Conservative => "conservative",
        CallPrecision::Heuristic => "heuristic",
        CallPrecision::Ambiguous => "ambiguous",
        CallPrecision::Unknown => "unknown",
        CallPrecision::Unsupported => "unsupported",
    }
}

fn provenance_label(provenance: CallProvenance) -> &'static str {
    match provenance {
        CallProvenance::NativeDirect => "native_direct",
        CallProvenance::Native => "native",
        CallProvenance::SemanticReference => "semantic_reference",
        CallProvenance::ImportBinding => "import_binding",
        CallProvenance::MirShape => "mir_shape",
        CallProvenance::Topology => "topology",
        CallProvenance::Extension => "extension",
        CallProvenance::Model => "model",
    }
}

fn reason_label(reason: UnresolvedCallReason) -> &'static str {
    match reason {
        UnresolvedCallReason::FunctionValue => "function_value",
        UnresolvedCallReason::DynamicProperty => "dynamic_property",
        UnresolvedCallReason::InterfaceDispatch => "interface_dispatch",
        UnresolvedCallReason::Eval => "eval",
        UnresolvedCallReason::CallApplyBind => "call_apply_bind",
        UnresolvedCallReason::FrameworkDispatch => "framework_dispatch",
        UnresolvedCallReason::Reflection => "reflection",
        UnresolvedCallReason::GoroutineBoundary => "goroutine_boundary",
        UnresolvedCallReason::DynamicImport => "dynamic_import",
        UnresolvedCallReason::ProxyOrAccessor => "proxy_or_accessor",
        UnresolvedCallReason::MissingSemanticReference => "missing_semantic_reference",
        UnresolvedCallReason::MissingImportResolution => "missing_import_resolution",
        UnresolvedCallReason::SetupMissing => "setup_missing",
        UnresolvedCallReason::UnsupportedSyntax => "unsupported_syntax",
        UnresolvedCallReason::BudgetExceeded => "budget_exceeded",
        UnresolvedCallReason::UnknownCallee => "unknown_callee",
        UnresolvedCallReason::Unknown => "unknown",
    }
}
