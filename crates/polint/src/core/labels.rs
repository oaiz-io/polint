//! Status/precision label and mapper helpers for analysis fact metadata.
//!
//! Extracted from the core monolith without behaviour changes.

use super::facts::SourceFile;
use super::ids::FunctionId;
use super::lang::Language;
use super::{GO_SYNTAX_PROVIDER_ID, SOURCE_PROVIDER_ID, TS_SYNTAX_PROVIDER_ID};
use crate::analysis::aliases::facts::{AliasPrecision, AliasStatus};
use crate::analysis::calls::facts::{
    CallAlgorithm, CallEdgeKind, CallPrecision, CallSyntaxKind, CallTargetStatus,
    UnresolvedCallReason,
};
use crate::analysis::cfg::facts::{CfgPrecision, CfgStatus};
use crate::analysis::data_flow::facts::{
    DataFlowConfidence, DataFlowPrecision, DataFlowStatus, DataFlowValidation,
};
use crate::analysis::evidence::facts::{
    EvidenceConfidence, EvidencePrecision, EvidenceProvenance, EvidenceStatus, EvidenceValidation,
};
use crate::analysis::mir::body::MirStatus;
use crate::analysis::places::PlaceStatus;
use crate::analysis::points_to::facts::{PointsToPrecision, PointsToStatus};
use crate::analysis::refined_calls::facts::{
    RefinedCallConfidence, RefinedCallTier, RefinedCallValidation,
};
use crate::analysis::summaries::facts::{SummaryDomainKind, SummaryPrecision, SummaryStatus};
use crate::analysis::types::facts::{TypeConfidence, TypePrecision, TypeStatus};
use crate::analysis::values::facts::{ValuePrecision, ValueStatus};
use crate::analysis_kernel::{FactConfidence, FactFamily, FactPrecision, ValidationStatus};
use crate::module_graph::topology::TopologyPrecision;
use crate::symbol_graph::semantic::SemanticStatus;

pub(super) fn type_metadata_precision(
    status: TypeStatus,
    precision: TypePrecision,
    confidence: Option<TypeConfidence>,
) -> (FactPrecision, FactConfidence) {
    let fact_precision = match status {
        TypeStatus::SetupMissing => FactPrecision::SetupMissing,
        TypeStatus::Unsupported => FactPrecision::Unsupported,
        TypeStatus::Unknown => FactPrecision::Unresolved,
        TypeStatus::BudgetExceeded => FactPrecision::Heuristic,
        TypeStatus::Present => match precision {
            TypePrecision::ExactLocal => FactPrecision::Exact,
            TypePrecision::SetupAware => FactPrecision::SetupAware,
            TypePrecision::Conservative | TypePrecision::Heuristic => FactPrecision::Heuristic,
            TypePrecision::Unknown => FactPrecision::Unresolved,
            TypePrecision::Unsupported => FactPrecision::Unsupported,
        },
    };
    let confidence = match confidence.unwrap_or(TypeConfidence::Medium) {
        TypeConfidence::High => FactConfidence::High,
        TypeConfidence::Medium => FactConfidence::Medium,
        TypeConfidence::Low => FactConfidence::Low,
    };
    (fact_precision, confidence)
}

pub(super) fn value_metadata_precision(
    status: ValueStatus,
    precision: ValuePrecision,
) -> (FactPrecision, FactConfidence) {
    let fact_precision = match status {
        ValueStatus::SetupMissing => FactPrecision::SetupMissing,
        ValueStatus::Unsupported => FactPrecision::Unsupported,
        ValueStatus::Unknown => FactPrecision::Unresolved,
        ValueStatus::BudgetExceeded => FactPrecision::Heuristic,
        ValueStatus::Present => match precision {
            ValuePrecision::ExactLocal => FactPrecision::SetupAware,
            ValuePrecision::SetupAware => FactPrecision::SetupAware,
            ValuePrecision::Conservative | ValuePrecision::Heuristic => FactPrecision::Heuristic,
            ValuePrecision::Unknown => FactPrecision::Unresolved,
            ValuePrecision::Unsupported => FactPrecision::Unsupported,
        },
    };
    (fact_precision, FactConfidence::Medium)
}

pub(super) fn points_to_metadata_precision(
    status: PointsToStatus,
    precision: PointsToPrecision,
) -> (FactPrecision, FactConfidence) {
    let fact_precision = match status {
        PointsToStatus::SetupMissing => FactPrecision::SetupMissing,
        PointsToStatus::Unsupported => FactPrecision::Unsupported,
        PointsToStatus::Unknown => FactPrecision::Unresolved,
        PointsToStatus::BudgetExceeded => FactPrecision::Heuristic,
        PointsToStatus::Present => match precision {
            PointsToPrecision::LocalFlowSensitive => FactPrecision::SetupAware,
            PointsToPrecision::FlowInsensitive
            | PointsToPrecision::SummaryProjected
            | PointsToPrecision::Heuristic => FactPrecision::Heuristic,
            PointsToPrecision::Unknown => FactPrecision::Unresolved,
            PointsToPrecision::Unsupported => FactPrecision::Unsupported,
        },
    };
    (fact_precision, FactConfidence::Medium)
}

pub(super) fn alias_metadata_precision(
    status: AliasStatus,
    precision: AliasPrecision,
) -> (FactPrecision, FactConfidence) {
    let fact_precision = match status {
        AliasStatus::NoAlias | AliasStatus::MustAlias => match precision {
            AliasPrecision::ExactLocal
            | AliasPrecision::FlowInsensitive
            | AliasPrecision::SetupAware
            | AliasPrecision::Conservative => FactPrecision::SetupAware,
            AliasPrecision::Heuristic => FactPrecision::Heuristic,
            AliasPrecision::Unknown => FactPrecision::Unresolved,
            AliasPrecision::Unsupported => FactPrecision::Unsupported,
        },
        AliasStatus::MayAlias | AliasStatus::PartialAlias => FactPrecision::Ambiguous,
        AliasStatus::Unknown => FactPrecision::Unresolved,
    };
    let confidence = match status {
        AliasStatus::NoAlias | AliasStatus::MustAlias => FactConfidence::High,
        AliasStatus::MayAlias | AliasStatus::PartialAlias => FactConfidence::Medium,
        AliasStatus::Unknown => FactConfidence::Low,
    };
    (fact_precision, confidence)
}

pub(super) fn summary_domain_to_fact_family(domain: SummaryDomainKind) -> FactFamily {
    match domain {
        SummaryDomainKind::ControlEffects => FactFamily::SummaryControl,
        SummaryDomainKind::CallEffects => FactFamily::SummaryCall,
        SummaryDomainKind::MemoryEffects => FactFamily::SummaryMemory,
        SummaryDomainKind::DataFlowTito => FactFamily::SummaryTito,
    }
}

pub(super) fn summary_precision_metadata(
    status: SummaryStatus,
    precision: SummaryPrecision,
) -> (FactPrecision, FactConfidence) {
    let fact_precision = match status {
        SummaryStatus::Present => match precision {
            SummaryPrecision::Local | SummaryPrecision::SetupAware => FactPrecision::SetupAware,
            SummaryPrecision::Heuristic => FactPrecision::Heuristic,
            SummaryPrecision::UnknownTop => FactPrecision::Unresolved,
        },
        SummaryStatus::Unknown | SummaryStatus::BudgetExceeded => FactPrecision::Unresolved,
        SummaryStatus::Unsupported => FactPrecision::Unsupported,
        SummaryStatus::SetupMissing => FactPrecision::SetupMissing,
    };
    let confidence = match status {
        SummaryStatus::Present => match precision {
            SummaryPrecision::Local | SummaryPrecision::SetupAware => FactConfidence::High,
            SummaryPrecision::Heuristic => FactConfidence::Medium,
            SummaryPrecision::UnknownTop => FactConfidence::Low,
        },
        SummaryStatus::Unknown
        | SummaryStatus::Unsupported
        | SummaryStatus::SetupMissing
        | SummaryStatus::BudgetExceeded => FactConfidence::Low,
    };
    (fact_precision, confidence)
}

pub(super) fn call_status_label(status: CallTargetStatus) -> &'static str {
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

pub(super) fn call_precision_label(precision: CallPrecision) -> &'static str {
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

#[allow(
    dead_code,
    reason = "Retained for AnalysisDb call metadata until dual accessors are removed."
)]
pub(super) fn call_syntax_kind_label(kind: CallSyntaxKind) -> &'static str {
    match kind {
        CallSyntaxKind::Function => "function",
        CallSyntaxKind::Method => "method",
        CallSyntaxKind::Constructor => "constructor",
        CallSyntaxKind::StaticMember => "static_member",
        CallSyntaxKind::Member => "member",
        CallSyntaxKind::Index => "index",
        CallSyntaxKind::Super => "super",
        CallSyntaxKind::Import => "import",
        CallSyntaxKind::New => "new",
        CallSyntaxKind::TaggedTemplate => "tagged_template",
        CallSyntaxKind::GoRoutine => "go_routine",
        CallSyntaxKind::Deferred => "deferred",
        CallSyntaxKind::DynamicImport => "dynamic_import",
        CallSyntaxKind::Require => "require",
        CallSyntaxKind::FunctionValue => "function_value",
        CallSyntaxKind::Unknown => "unknown",
    }
}

pub(super) fn call_edge_kind_label(kind: CallEdgeKind) -> &'static str {
    match kind {
        CallEdgeKind::Direct => "direct",
        CallEdgeKind::Constructor => "constructor",
        CallEdgeKind::StaticMember => "static_member",
        CallEdgeKind::MethodDirect => "method_direct",
        CallEdgeKind::Method => "method",
        CallEdgeKind::FunctionValue => "function_value",
        CallEdgeKind::Synthetic => "synthetic",
        CallEdgeKind::Spawn => "spawn",
        CallEdgeKind::Deferred => "deferred",
        CallEdgeKind::Unknown => "unknown",
    }
}

pub(super) fn call_algorithm_label(algorithm: CallAlgorithm) -> &'static str {
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

pub(super) fn call_unresolved_reason_label(reason: UnresolvedCallReason) -> &'static str {
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

pub(super) fn refined_call_tier_label(tier: RefinedCallTier) -> &'static str {
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

pub(super) fn refined_call_validation_label(validation: RefinedCallValidation) -> &'static str {
    match validation {
        RefinedCallValidation::Native => "native",
        RefinedCallValidation::ReferentiallyValidated => "referentially_validated",
        RefinedCallValidation::ExtensionValidated => "extension_validated",
        RefinedCallValidation::Rejected => "rejected",
    }
}

pub(super) fn refined_call_validation_metadata(
    validation: RefinedCallValidation,
) -> ValidationStatus {
    match validation {
        RefinedCallValidation::Native => ValidationStatus::NativeTrusted,
        RefinedCallValidation::ReferentiallyValidated => ValidationStatus::ReferentiallyValidated,
        RefinedCallValidation::ExtensionValidated => ValidationStatus::SchemaValidated,
        RefinedCallValidation::Rejected => ValidationStatus::ConflictRejected,
    }
}

pub(super) fn refined_call_confidence_metadata(
    confidence: RefinedCallConfidence,
    fallback: FactConfidence,
) -> FactConfidence {
    let requested = match confidence {
        RefinedCallConfidence::High => FactConfidence::High,
        RefinedCallConfidence::Medium => FactConfidence::Medium,
        RefinedCallConfidence::Low => FactConfidence::Low,
    };
    match (requested, fallback) {
        (FactConfidence::Low, _) | (_, FactConfidence::Low) => FactConfidence::Low,
        (FactConfidence::Medium, _) | (_, FactConfidence::Medium) => FactConfidence::Medium,
        (FactConfidence::High, FactConfidence::High) => FactConfidence::High,
    }
}

pub(super) fn data_flow_status_metadata(
    status: DataFlowStatus,
    precision: DataFlowPrecision,
) -> (FactPrecision, FactConfidence) {
    let fact_precision = match status {
        DataFlowStatus::Present => match precision {
            DataFlowPrecision::Exact => FactPrecision::Exact,
            DataFlowPrecision::SetupAware => FactPrecision::SetupAware,
            DataFlowPrecision::Syntax => FactPrecision::Syntax,
            DataFlowPrecision::Conservative | DataFlowPrecision::Heuristic => {
                FactPrecision::Heuristic
            }
            DataFlowPrecision::Unknown => FactPrecision::Unresolved,
        },
        DataFlowStatus::Unknown | DataFlowStatus::BudgetExceeded => FactPrecision::Unresolved,
        DataFlowStatus::Unsupported | DataFlowStatus::Rejected => FactPrecision::Unsupported,
        DataFlowStatus::SetupMissing => FactPrecision::SetupMissing,
    };
    let confidence = match status {
        DataFlowStatus::Present => match precision {
            DataFlowPrecision::Exact
            | DataFlowPrecision::SetupAware
            | DataFlowPrecision::Syntax => FactConfidence::High,
            DataFlowPrecision::Conservative | DataFlowPrecision::Heuristic => {
                FactConfidence::Medium
            }
            DataFlowPrecision::Unknown => FactConfidence::Low,
        },
        DataFlowStatus::Unknown
        | DataFlowStatus::Unsupported
        | DataFlowStatus::SetupMissing
        | DataFlowStatus::BudgetExceeded
        | DataFlowStatus::Rejected => FactConfidence::Low,
    };
    (fact_precision, confidence)
}

pub(super) fn data_flow_confidence_metadata(
    confidence: DataFlowConfidence,
    fallback: FactConfidence,
) -> FactConfidence {
    let requested = match confidence {
        DataFlowConfidence::High => FactConfidence::High,
        DataFlowConfidence::Medium => FactConfidence::Medium,
        DataFlowConfidence::Low => FactConfidence::Low,
    };
    match (requested, fallback) {
        (FactConfidence::Low, _) | (_, FactConfidence::Low) => FactConfidence::Low,
        (FactConfidence::Medium, _) | (_, FactConfidence::Medium) => FactConfidence::Medium,
        (FactConfidence::High, FactConfidence::High) => FactConfidence::High,
    }
}

pub(super) fn data_flow_validation_metadata(validation: DataFlowValidation) -> ValidationStatus {
    match validation {
        DataFlowValidation::Native => ValidationStatus::NativeTrusted,
        DataFlowValidation::ReferentiallyValidated => ValidationStatus::ReferentiallyValidated,
        DataFlowValidation::ExtensionValidated => ValidationStatus::SchemaValidated,
        DataFlowValidation::BudgetValidated => ValidationStatus::StableKeyValidated,
        DataFlowValidation::Rejected => ValidationStatus::ConflictRejected,
    }
}

#[allow(
    dead_code,
    reason = "Retained for AnalysisDb metadata until dual accessors are removed."
)]
pub(super) fn data_flow_status_label(status: DataFlowStatus) -> &'static str {
    match status {
        DataFlowStatus::Present => "present",
        DataFlowStatus::Unknown => "unknown",
        DataFlowStatus::Unsupported => "unsupported",
        DataFlowStatus::SetupMissing => "setup_missing",
        DataFlowStatus::BudgetExceeded => "budget_exceeded",
        DataFlowStatus::Rejected => "rejected",
    }
}

#[allow(
    dead_code,
    reason = "Retained for AnalysisDb metadata until dual accessors are removed."
)]
pub(super) fn data_flow_precision_label(precision: DataFlowPrecision) -> &'static str {
    match precision {
        DataFlowPrecision::Exact => "exact",
        DataFlowPrecision::SetupAware => "setup_aware",
        DataFlowPrecision::Syntax => "syntax",
        DataFlowPrecision::Conservative => "conservative",
        DataFlowPrecision::Heuristic => "heuristic",
        DataFlowPrecision::Unknown => "unknown",
    }
}

#[allow(
    dead_code,
    reason = "Retained for AnalysisDb metadata until dual accessors are removed."
)]
pub(super) fn data_flow_validation_label(validation: DataFlowValidation) -> &'static str {
    match validation {
        DataFlowValidation::Native => "native",
        DataFlowValidation::ReferentiallyValidated => "referentially_validated",
        DataFlowValidation::ExtensionValidated => "extension_validated",
        DataFlowValidation::BudgetValidated => "budget_validated",
        DataFlowValidation::Rejected => "rejected",
    }
}

pub(super) fn evidence_status_metadata(
    status: EvidenceStatus,
    precision: EvidencePrecision,
) -> (FactPrecision, FactConfidence) {
    let fact_precision = match status {
        EvidenceStatus::Present => match precision {
            EvidencePrecision::Exact => FactPrecision::SetupAware,
            EvidencePrecision::SetupAware => FactPrecision::SetupAware,
            EvidencePrecision::Syntax => FactPrecision::Syntax,
            EvidencePrecision::Conservative | EvidencePrecision::Heuristic => {
                FactPrecision::Heuristic
            }
            EvidencePrecision::Unknown => FactPrecision::Unresolved,
        },
        EvidenceStatus::Partial | EvidenceStatus::Unknown | EvidenceStatus::BudgetExceeded => {
            FactPrecision::Unresolved
        }
        EvidenceStatus::Unsupported | EvidenceStatus::Rejected => FactPrecision::Unsupported,
        EvidenceStatus::SetupMissing => FactPrecision::SetupMissing,
    };
    let confidence = match status {
        EvidenceStatus::Present => match precision {
            EvidencePrecision::Exact
            | EvidencePrecision::SetupAware
            | EvidencePrecision::Syntax => FactConfidence::High,
            EvidencePrecision::Conservative | EvidencePrecision::Heuristic => {
                FactConfidence::Medium
            }
            EvidencePrecision::Unknown => FactConfidence::Low,
        },
        EvidenceStatus::Partial => FactConfidence::Medium,
        EvidenceStatus::Unknown
        | EvidenceStatus::Unsupported
        | EvidenceStatus::SetupMissing
        | EvidenceStatus::BudgetExceeded
        | EvidenceStatus::Rejected => FactConfidence::Low,
    };
    (fact_precision, confidence)
}

pub(super) fn evidence_confidence_metadata(
    confidence: EvidenceConfidence,
    fallback: FactConfidence,
) -> FactConfidence {
    let requested = match confidence {
        EvidenceConfidence::High => FactConfidence::High,
        EvidenceConfidence::Medium => FactConfidence::Medium,
        EvidenceConfidence::Low => FactConfidence::Low,
    };
    match (requested, fallback) {
        (FactConfidence::Low, _) | (_, FactConfidence::Low) => FactConfidence::Low,
        (FactConfidence::Medium, _) | (_, FactConfidence::Medium) => FactConfidence::Medium,
        (FactConfidence::High, FactConfidence::High) => FactConfidence::High,
    }
}

pub(super) fn evidence_validation_metadata(validation: EvidenceValidation) -> ValidationStatus {
    match validation {
        EvidenceValidation::Native => ValidationStatus::NativeTrusted,
        EvidenceValidation::ReferentiallyValidated => ValidationStatus::ReferentiallyValidated,
        EvidenceValidation::ExtensionValidated => ValidationStatus::SchemaValidated,
        EvidenceValidation::BudgetValidated | EvidenceValidation::RendererValidated => {
            ValidationStatus::StableKeyValidated
        }
        EvidenceValidation::Rejected => ValidationStatus::ConflictRejected,
    }
}

#[allow(
    dead_code,
    reason = "Retained for AnalysisDb metadata until dual accessors are removed."
)]
pub(super) fn evidence_status_label(status: EvidenceStatus) -> &'static str {
    match status {
        EvidenceStatus::Present => "present",
        EvidenceStatus::Partial => "partial",
        EvidenceStatus::Unknown => "unknown",
        EvidenceStatus::Unsupported => "unsupported",
        EvidenceStatus::SetupMissing => "setup_missing",
        EvidenceStatus::BudgetExceeded => "budget_exceeded",
        EvidenceStatus::Rejected => "rejected",
    }
}

#[allow(
    dead_code,
    reason = "Retained for AnalysisDb metadata until dual accessors are removed."
)]
pub(super) fn evidence_precision_label(precision: EvidencePrecision) -> &'static str {
    match precision {
        EvidencePrecision::Exact => "exact",
        EvidencePrecision::SetupAware => "setup_aware",
        EvidencePrecision::Syntax => "syntax",
        EvidencePrecision::Conservative => "conservative",
        EvidencePrecision::Heuristic => "heuristic",
        EvidencePrecision::Unknown => "unknown",
    }
}

#[allow(
    dead_code,
    reason = "Retained for AnalysisDb metadata until dual accessors are removed."
)]
pub(super) fn evidence_provenance_label(provenance: EvidenceProvenance) -> &'static str {
    match provenance {
        EvidenceProvenance::Native => "native",
        EvidenceProvenance::Summary => "summary",
        EvidenceProvenance::Extension => "extension",
        EvidenceProvenance::Model => "model",
        EvidenceProvenance::Query => "query",
        EvidenceProvenance::Synthetic => "synthetic",
    }
}

#[allow(
    dead_code,
    reason = "Retained for AnalysisDb metadata until dual accessors are removed."
)]
pub(super) fn evidence_validation_label(validation: EvidenceValidation) -> &'static str {
    match validation {
        EvidenceValidation::Native => "native",
        EvidenceValidation::ReferentiallyValidated => "referentially_validated",
        EvidenceValidation::ExtensionValidated => "extension_validated",
        EvidenceValidation::BudgetValidated => "budget_validated",
        EvidenceValidation::RendererValidated => "renderer_validated",
        EvidenceValidation::Rejected => "rejected",
    }
}

#[allow(
    dead_code,
    reason = "Retained for AnalysisDb CFG metadata until dual accessors are removed."
)]
pub(super) fn cfg_status_label(status: CfgStatus) -> &'static str {
    match status {
        CfgStatus::Resolved => "resolved",
        CfgStatus::Partial => "partial",
        CfgStatus::Unknown => "unknown",
        CfgStatus::Unsupported => "unsupported",
    }
}

#[allow(
    dead_code,
    reason = "Retained for AnalysisDb CFG metadata until dual accessors are removed."
)]
pub(super) fn cfg_precision_label(precision: CfgPrecision) -> &'static str {
    match precision {
        CfgPrecision::ExactSyntax => "exact_syntax",
        CfgPrecision::ExactLowered => "exact_lowered",
        CfgPrecision::SetupAware => "setup_aware",
        CfgPrecision::Conservative => "conservative",
        CfgPrecision::Heuristic => "heuristic",
        CfgPrecision::Unknown => "unknown",
        CfgPrecision::Unsupported => "unsupported",
    }
}

#[allow(
    dead_code,
    reason = "Retained for AnalysisDb CFG metadata until dual accessors are removed."
)]
pub(super) fn cfg_view_label(view: crate::analysis::cfg::facts::CfgView) -> &'static str {
    match view {
        crate::analysis::cfg::facts::CfgView::NormalControl => "normal_control",
        crate::analysis::cfg::facts::CfgView::AbruptAware => "abrupt_aware",
        crate::analysis::cfg::facts::CfgView::ExceptionConservative => "exception_conservative",
    }
}

#[allow(
    dead_code,
    reason = "Retained for AnalysisDb CFG metadata until dual accessors are removed."
)]
pub(super) fn cfg_node_kind_label(kind: crate::analysis::cfg::facts::CfgNodeKind) -> &'static str {
    use crate::analysis::cfg::facts::CfgNodeKind;

    match kind {
        CfgNodeKind::Entry => "entry",
        CfgNodeKind::ExitNormal => "exit_normal",
        CfgNodeKind::ExitExceptional => "exit_exceptional",
        CfgNodeKind::Operation => "operation",
        CfgNodeKind::Condition => "condition",
        CfgNodeKind::CallSite => "call_site",
        CfgNodeKind::Return => "return",
        CfgNodeKind::Throw => "throw",
        CfgNodeKind::Panic => "panic",
        CfgNodeKind::Break => "break",
        CfgNodeKind::Continue => "continue",
        CfgNodeKind::Goto => "goto",
        CfgNodeKind::Yield => "yield",
        CfgNodeKind::Await => "await",
        CfgNodeKind::Defer => "defer",
        CfgNodeKind::RunDefers => "run_defers",
        CfgNodeKind::FinallyEnter => "finally_enter",
        CfgNodeKind::FinallyExit => "finally_exit",
        CfgNodeKind::Synthetic => "synthetic",
        CfgNodeKind::Unsupported => "unsupported",
    }
}

#[allow(
    dead_code,
    reason = "Retained for AnalysisDb CFG metadata until dual accessors are removed."
)]
pub(super) fn basic_block_kind_label(
    kind: crate::analysis::cfg::facts::BasicBlockKind,
) -> &'static str {
    use crate::analysis::cfg::facts::BasicBlockKind;

    match kind {
        BasicBlockKind::Entry => "entry",
        BasicBlockKind::ExitNormal => "exit_normal",
        BasicBlockKind::ExitExceptional => "exit_exceptional",
        BasicBlockKind::StraightLine => "straight_line",
        BasicBlockKind::Branch => "branch",
        BasicBlockKind::LoopHeader => "loop_header",
        BasicBlockKind::LoopBody => "loop_body",
        BasicBlockKind::Join => "join",
        BasicBlockKind::Cleanup => "cleanup",
        BasicBlockKind::Unreachable => "unreachable",
        BasicBlockKind::Synthetic => "synthetic",
    }
}

#[allow(
    dead_code,
    reason = "Retained for AnalysisDb CFG metadata until dual accessors are removed."
)]
pub(super) fn cfg_edge_kind_label(kind: crate::analysis::cfg::facts::CfgEdgeKind) -> &'static str {
    use crate::analysis::cfg::facts::CfgEdgeKind;

    match kind {
        CfgEdgeKind::Normal => "normal",
        CfgEdgeKind::True => "true",
        CfgEdgeKind::False => "false",
        CfgEdgeKind::SwitchCase => "switch_case",
        CfgEdgeKind::DefaultCase => "default_case",
        CfgEdgeKind::LoopEnter => "loop_enter",
        CfgEdgeKind::LoopBack => "loop_back",
        CfgEdgeKind::LoopExit => "loop_exit",
        CfgEdgeKind::Break => "break",
        CfgEdgeKind::Continue => "continue",
        CfgEdgeKind::Goto => "goto",
        CfgEdgeKind::Return => "return",
        CfgEdgeKind::Throw => "throw",
        CfgEdgeKind::ImplicitThrow => "implicit_throw",
        CfgEdgeKind::Panic => "panic",
        CfgEdgeKind::Recover => "recover",
        CfgEdgeKind::Finally => "finally",
        CfgEdgeKind::Cleanup => "cleanup",
        CfgEdgeKind::Defer => "defer",
        CfgEdgeKind::ShortCircuit => "short_circuit",
        CfgEdgeKind::OptionalChain => "optional_chain",
        CfgEdgeKind::Nullish => "nullish",
        CfgEdgeKind::YieldSuspend => "yield_suspend",
        CfgEdgeKind::YieldResume => "yield_resume",
        CfgEdgeKind::AwaitSuspend => "await_suspend",
        CfgEdgeKind::AwaitResume => "await_resume",
        CfgEdgeKind::Spawn => "spawn",
        CfgEdgeKind::Unreachable => "unreachable",
        CfgEdgeKind::Unknown => "unknown",
        CfgEdgeKind::Synthetic => "synthetic",
        CfgEdgeKind::Extension => "extension",
    }
}

pub(super) fn topology_precision_metadata(
    precision: TopologyPrecision,
) -> (FactPrecision, FactConfidence) {
    match precision {
        TopologyPrecision::ExactStatic | TopologyPrecision::ExactLockfile => {
            (FactPrecision::SetupAware, FactConfidence::High)
        }
        TopologyPrecision::Heuristic => (FactPrecision::Heuristic, FactConfidence::Medium),
        TopologyPrecision::Unknown => (FactPrecision::Unresolved, FactConfidence::Low),
        TopologyPrecision::Unsupported => (FactPrecision::Unsupported, FactConfidence::Low),
    }
}

pub(super) fn semantic_status_label(status: SemanticStatus) -> &'static str {
    match status {
        SemanticStatus::Resolved => "resolved",
        SemanticStatus::Ambiguous => "ambiguous",
        SemanticStatus::Unresolved => "unresolved",
        SemanticStatus::Cycle => "cycle",
        SemanticStatus::Generated => "generated",
        SemanticStatus::Dynamic => "dynamic",
        SemanticStatus::External => "external",
        SemanticStatus::SetupMissing => "setup_missing",
        SemanticStatus::Unsupported => "unsupported",
    }
}

pub(super) fn mir_status_label(status: MirStatus) -> &'static str {
    match status {
        MirStatus::Resolved => "resolved",
        MirStatus::Partial => "partial",
        MirStatus::Unknown => "unknown",
        MirStatus::Unsupported => "unsupported",
    }
}

pub(super) fn place_status_label(status: PlaceStatus) -> &'static str {
    match status {
        PlaceStatus::Resolved => "resolved",
        PlaceStatus::Partial => "partial",
        PlaceStatus::Unknown => "unknown",
        PlaceStatus::Unsupported => "unsupported",
    }
}

pub(super) fn syntax_provider_for_language(language: Language) -> &'static str {
    if language.is_ts_family() {
        TS_SYNTAX_PROVIDER_ID
    } else if language == Language::Go {
        GO_SYNTAX_PROVIDER_ID
    } else {
        SOURCE_PROVIDER_ID
    }
}

pub(super) fn syntax_provider_for_file(file: Option<&SourceFile>) -> &'static str {
    file.map(|file| syntax_provider_for_language(file.language))
        .unwrap_or(GO_SYNTAX_PROVIDER_ID)
}

pub(super) fn option_function_id(function: Option<FunctionId>) -> String {
    function
        .map(|function| function.0.to_string())
        .unwrap_or_else(|| "none".to_string())
}

pub(super) fn none_value() -> String {
    "<none>".to_string()
}

pub(super) fn option_bool(value: Option<bool>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

pub(super) fn option_string(value: Option<&str>) -> String {
    value
        .map(|value| format!("some:{value}"))
        .unwrap_or_else(|| "none".to_string())
}
