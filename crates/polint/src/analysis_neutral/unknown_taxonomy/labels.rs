//! Neutral unknown-taxonomy labels and capability documentation mapping.

use crate::analysis_api::{FactFamily, FactRef};
use crate::analysis_neutral::AnalysisHost;

use crate::analysis_api::{
    ResolutionPrecision, ResolutionStatus, SymbolPrecision, SymbolResolutionStatus,
    UnresolvedReason,
};
use crate::analysis_neutral::calls::facts::{CallTargetStatus, UnresolvedCallReason};
use crate::analysis_neutral::data_flow::facts::{
    DataFlowBudgetReason, DataFlowEdgeFact, DataFlowEdgeKind, DataFlowPrecision, DataFlowStatus,
};
use crate::analysis_neutral::evidence::facts::EvidenceUnknownReason;
use crate::analysis_neutral::points_to::facts::{PointsToPrecision, PointsToStatus};
use crate::analysis_neutral::unknown_taxonomy::facts::{
    UnknownCategory, UnknownRow, UnknownRowInput,
};

pub fn policy_capability_docs_path(capability: &str) -> &'static str {
    match capability {
        "events" => "docs/facts/events.md",
        "calls" => "docs/facts/calls.md",
        "control_flow" => "docs/facts/control-flow.md",
        "dataflow" => "docs/facts/data-flow.md",
        _ => "docs/facts/capability-plans.md",
    }
}

pub fn resolution_category(
    status: ResolutionStatus,
    reason: Option<UnresolvedReason>,
) -> UnknownCategory {
    match (status, reason) {
        (ResolutionStatus::SetupMissing, _) | (_, Some(UnresolvedReason::SetupMissing)) => {
            UnknownCategory::SetupMissing
        }
        (ResolutionStatus::Unsupported, _)
        | (_, Some(UnresolvedReason::UnsupportedLanguage))
        | (_, Some(UnresolvedReason::UnsupportedImport)) => UnknownCategory::UnsupportedSemantic,
        (ResolutionStatus::Dynamic, _) => UnknownCategory::OutOfScope,
        (_, Some(UnresolvedReason::OutsideWorkspace)) => UnknownCategory::OutOfScope,
        _ => UnknownCategory::MissingFact,
    }
}

pub fn symbol_precision_category(precision: SymbolPrecision) -> UnknownCategory {
    match precision {
        SymbolPrecision::SetupMissing => UnknownCategory::SetupMissing,
        SymbolPrecision::Unsupported => UnknownCategory::UnsupportedSemantic,
        SymbolPrecision::Unresolved | SymbolPrecision::Ambiguous => UnknownCategory::MissingFact,
        _ => UnknownCategory::MissingFact,
    }
}

pub fn symbol_status_category(status: SymbolResolutionStatus) -> UnknownCategory {
    match status {
        SymbolResolutionStatus::SetupMissing => UnknownCategory::SetupMissing,
        SymbolResolutionStatus::Unsupported => UnknownCategory::UnsupportedSemantic,
        SymbolResolutionStatus::Unresolved | SymbolResolutionStatus::Ambiguous => {
            UnknownCategory::MissingFact
        }
        SymbolResolutionStatus::Resolved => UnknownCategory::MissingFact,
        _ => UnknownCategory::MissingFact,
    }
}

pub fn resolution_status_label(status: ResolutionStatus) -> &'static str {
    match status {
        ResolutionStatus::Resolved => "resolved",
        ResolutionStatus::External => "external",
        ResolutionStatus::Unresolved => "unresolved",
        ResolutionStatus::SetupMissing => "setup_missing",
        ResolutionStatus::Dynamic => "dynamic",
        ResolutionStatus::Unsupported => "unsupported",
        _ => "unknown",
    }
}

pub fn resolution_precision_label(precision: ResolutionPrecision) -> &'static str {
    match precision {
        ResolutionPrecision::ExactFile => "exact_file",
        ResolutionPrecision::Package => "package",
        ResolutionPrecision::ExternalPackage => "external_package",
        ResolutionPrecision::Heuristic => "heuristic",
        ResolutionPrecision::None => "none",
        _ => "unknown",
    }
}

pub fn unresolved_reason_label(reason: UnresolvedReason) -> &'static str {
    match reason {
        UnresolvedReason::NotFound => "not_found",
        UnresolvedReason::SetupMissing => "setup_missing",
        UnresolvedReason::DynamicExpression => "dynamic_expression",
        UnresolvedReason::UnsupportedLanguage => "unsupported_language",
        UnresolvedReason::UnsupportedImport => "unsupported_import",
        UnresolvedReason::ResolverError => "resolver_error",
        UnresolvedReason::OutsideWorkspace => "outside_workspace",
        _ => "unknown",
    }
}

pub fn symbol_status_label(status: SymbolResolutionStatus) -> &'static str {
    match status {
        SymbolResolutionStatus::Resolved => "resolved",
        SymbolResolutionStatus::Unresolved => "unresolved",
        SymbolResolutionStatus::Ambiguous => "ambiguous",
        SymbolResolutionStatus::SetupMissing => "setup_missing",
        SymbolResolutionStatus::Unsupported => "unsupported",
        _ => "unknown",
    }
}

pub fn symbol_precision_label(precision: SymbolPrecision) -> &'static str {
    match precision {
        SymbolPrecision::ExactSemantic => "exact_semantic",
        SymbolPrecision::ExactLocal => "exact_local",
        SymbolPrecision::ModuleLinked => "module_linked",
        SymbolPrecision::Heuristic => "heuristic",
        SymbolPrecision::Unresolved => "unresolved",
        SymbolPrecision::Ambiguous => "ambiguous",
        SymbolPrecision::SetupMissing => "setup_missing",
        SymbolPrecision::Unsupported => "unsupported",
        _ => "unknown",
    }
}

pub fn artifact_for_resolution_status(status: ResolutionStatus) -> &'static str {
    match status {
        ResolutionStatus::SetupMissing => "fixture",
        ResolutionStatus::Dynamic | ResolutionStatus::Unsupported => "provider",
        ResolutionStatus::Unresolved => "model",
        ResolutionStatus::External | ResolutionStatus::Resolved => "rule",
        _ => "unknown",
    }
}

pub fn points_to_status_label(status: PointsToStatus) -> &'static str {
    match status {
        PointsToStatus::Present => "present",
        PointsToStatus::Unknown => "unknown",
        PointsToStatus::Unsupported => "unsupported",
        PointsToStatus::SetupMissing => "setup_missing",
        PointsToStatus::BudgetExceeded => "budget_exceeded",
    }
}

pub fn points_to_precision_label(precision: PointsToPrecision) -> &'static str {
    match precision {
        PointsToPrecision::FlowInsensitive => "flow_insensitive",
        PointsToPrecision::LocalFlowSensitive => "local_flow_sensitive",
        PointsToPrecision::SummaryProjected => "summary_projected",
        PointsToPrecision::Heuristic => "heuristic",
        PointsToPrecision::Unknown => "unknown",
        PointsToPrecision::Unsupported => "unsupported",
    }
}

pub fn call_status_label(status: CallTargetStatus) -> &'static str {
    match status {
        CallTargetStatus::Resolved => "resolved",
        CallTargetStatus::Ambiguous => "ambiguous",
        CallTargetStatus::Unresolved => "unresolved",
        CallTargetStatus::Unsupported => "unsupported",
        CallTargetStatus::SetupMissing => "setup_missing",
        CallTargetStatus::BudgetExceeded => "budget_exceeded",
        CallTargetStatus::Rejected => "rejected",
        CallTargetStatus::Unreachable => "unreachable",
    }
}

pub fn call_precision_label(
    precision: crate::analysis_neutral::calls::facts::CallPrecision,
) -> &'static str {
    match precision {
        crate::analysis_neutral::calls::facts::CallPrecision::Exact => "exact",
        crate::analysis_neutral::calls::facts::CallPrecision::SetupAware => "setup_aware",
        crate::analysis_neutral::calls::facts::CallPrecision::Conservative => "conservative",
        crate::analysis_neutral::calls::facts::CallPrecision::Heuristic => "heuristic",
        crate::analysis_neutral::calls::facts::CallPrecision::Ambiguous => "ambiguous",
        crate::analysis_neutral::calls::facts::CallPrecision::Unknown => "unknown",
        crate::analysis_neutral::calls::facts::CallPrecision::Unsupported => "unsupported",
    }
}

pub fn data_flow_status_label(status: DataFlowStatus) -> &'static str {
    match status {
        DataFlowStatus::Present => "present",
        DataFlowStatus::Unknown => "unknown",
        DataFlowStatus::Unsupported => "unsupported",
        DataFlowStatus::SetupMissing => "setup_missing",
        DataFlowStatus::BudgetExceeded => "budget_exceeded",
        DataFlowStatus::Rejected => "rejected",
    }
}

pub fn data_flow_precision_label(precision: DataFlowPrecision) -> &'static str {
    match precision {
        DataFlowPrecision::Exact => "exact",
        DataFlowPrecision::SetupAware => "setup_aware",
        DataFlowPrecision::Syntax => "syntax",
        DataFlowPrecision::Conservative => "conservative",
        DataFlowPrecision::Heuristic => "heuristic",
        DataFlowPrecision::Unknown => "unknown",
    }
}

pub fn data_flow_edge_kind_label(kind: DataFlowEdgeKind) -> &'static str {
    match kind {
        DataFlowEdgeKind::LocalBinding => "local_binding",
        DataFlowEdgeKind::LocalAssignment => "local_assignment",
        DataFlowEdgeKind::LocalUse => "local_use",
        DataFlowEdgeKind::LocalRead => "local_read",
        DataFlowEdgeKind::LocalWrite => "local_write",
        DataFlowEdgeKind::ReturnValue => "return_value",
        DataFlowEdgeKind::FieldProjection => "field_projection",
        DataFlowEdgeKind::IndexProjection => "index_projection",
        DataFlowEdgeKind::Dereference => "dereference",
        DataFlowEdgeKind::AddressOf => "address_of",
        DataFlowEdgeKind::CallArgumentToParameter => "call_argument_to_parameter",
        DataFlowEdgeKind::CallArgumentToReturn => "call_argument_to_return",
        DataFlowEdgeKind::CallReturnToUse => "call_return_to_use",
        DataFlowEdgeKind::ReceiverToMethod => "receiver_to_method",
        DataFlowEdgeKind::SummaryTito => "summary_tito",
        DataFlowEdgeKind::SummaryProjected => "summary_projected",
        DataFlowEdgeKind::UnknownFlow => "unknown_flow",
        DataFlowEdgeKind::HavocFlow => "havoc_flow",
        DataFlowEdgeKind::BudgetTruncated => "budget_truncated",
        DataFlowEdgeKind::SourceIntroduction => "source_introduction",
        DataFlowEdgeKind::Sanitizer => "sanitizer",
        DataFlowEdgeKind::Barrier => "barrier",
        DataFlowEdgeKind::Model => "model",
    }
}

pub fn data_flow_edge_reason(edge: &DataFlowEdgeFact) -> String {
    if edge.evidence.is_empty() {
        data_flow_edge_kind_label(edge.kind).to_string()
    } else {
        edge.evidence.join(",")
    }
}

pub fn data_flow_budget_reason_label(reason: DataFlowBudgetReason) -> &'static str {
    match reason {
        DataFlowBudgetReason::NodeLimit => "node_limit",
        DataFlowBudgetReason::EdgeLimit => "edge_limit",
        DataFlowBudgetReason::PathDepth => "path_depth",
        DataFlowBudgetReason::PathCount => "path_count",
    }
}

pub fn evidence_unknown_reason_label(reason: EvidenceUnknownReason) -> &'static str {
    match reason {
        EvidenceUnknownReason::DynamicCall => "dynamic_call",
        EvidenceUnknownReason::UnsupportedEdge => "unsupported_edge",
        EvidenceUnknownReason::SetupMissing => "setup_missing",
        EvidenceUnknownReason::BudgetExceeded => "budget_exceeded",
        EvidenceUnknownReason::OpaqueSummary => "opaque_summary",
    }
}

pub fn unresolved_call_reason_label(reason: UnresolvedCallReason) -> &'static str {
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

pub fn unsupported_capability_row(capability: &str, docs_path: Option<&str>) -> UnknownRow {
    UnknownRow::new(UnknownRowInput {
        category: UnknownCategory::UnsupportedSemantic,
        capability: Some(capability.to_string()),
        family: None,
        provider: "polint.capabilities".to_string(),
        file: "<workspace>".to_string(),
        span: None,
        status: "unsupported".to_string(),
        reason: Some("Capability does not support public unknown inspection.".to_string()),
        precision: None,
        docs_path: docs_path
            .or(Some("docs/facts/capability-plans.md"))
            .map(str::to_string),
        suggested_artifact: Some("provider".to_string()),
        source_stable_key: Some(format!("capability:{capability}")),
    })
}

pub fn stable_key_for(db: &impl AnalysisHost, family: FactFamily, run_id: u64) -> Option<String> {
    db.metadata_for(FactRef::new(family, run_id))
        .map(|metadata| db.resolve_stable_key(metadata.stable_key).to_string())
}
