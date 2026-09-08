#[cfg(test)]
use crate::analysis::data_flow::facts::{
    DataFlowConfidence, DataFlowModelFact, DataFlowNodeFact, DataFlowPrecision, DataFlowStatus,
    DataFlowValidation,
};
#[cfg(test)]
use crate::analysis::evidence::facts::{EvidenceNodeFact, EvidencePrecision};

#[cfg(test)]
use crate::analysis::data_flow::store::DataFlowOutput;
#[cfg(test)]
use crate::analysis::extensions::store::{
    AcceptedExtensionFact, ExtensionActivationRow, ExtensionOutput, RejectedExtensionFact,
};
#[cfg(test)]
use crate::analysis::solver::budget::BudgetStatus;
#[cfg(test)]
use crate::analysis::solver::store::SolverOutput;
#[cfg(test)]
use crate::analysis::types::store::TypeValueAliasOutput;
#[cfg(test)]
use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
use std::path::PathBuf;

#[cfg(test)]
use crate::analysis_kernel::MissingFactMeta;
#[cfg(test)]
use std::sync::Arc;

#[cfg(test)]
use crate::analysis::summaries::facts::{SummaryEventFact, SummaryFact};
#[cfg(test)]
use crate::analysis::values::facts::ValueFact;

pub(super) const SOURCE_PROVIDER_ID: &str = "polint.source";
pub(super) const GO_SYNTAX_PROVIDER_ID: &str = "polint.go.syntax";
pub(super) const TS_SYNTAX_PROVIDER_ID: &str = "polint.ts.syntax";
pub(super) const MODULE_GRAPH_PROVIDER_ID: &str = "polint.module_graph";
pub(super) const MODULE_TOPOLOGY_PROVIDER_ID: &str = "polint.module_topology";
pub(super) const SYMBOL_GRAPH_PROVIDER_ID: &str = "polint.symbol_graph";
pub(crate) const SEMANTIC_MIR_PROVIDER_ID: &str = "polint.semantic_mir";
#[allow(
    dead_code,
    reason = "Used by AnalysisDb CFG metadata writers retained until dual accessors are removed."
)]
pub(crate) const CFG_PROVIDER_ID: &str = "polint.cfg";
#[allow(
    dead_code,
    reason = "Used by AnalysisDb call-fact metadata writers retained for tests until dual accessors are removed."
)]
pub(crate) const CALLS_PROVIDER_ID: &str = "polint.calls";
pub(crate) const POLINT_ABSTRACT_DOMAINS_PROVIDER_ID: &str = "polint.abstract_domains";
pub(crate) const POLINT_DIRECT_SUMMARIES_PROVIDER_ID: &str = "polint.direct_summaries";
#[allow(
    dead_code,
    reason = "Entrypoint metadata writers retained until dual accessors are removed."
)]
pub(crate) const ENTRYPOINTS_PROVIDER_ID: &str = "polint.entrypoints";
pub(super) const METRICS_PROVIDER_ID: &str = "polint.metrics";
pub(super) const FUNCTION_SIZE_METRIC_NAME: &str = "function_size";
pub(super) const CYCLOMATIC_COMPLEXITY_METRIC_NAME: &str = "cyclomatic_complexity";

#[cfg(test)]
use metadata::*;

mod capability;
mod completeness;
mod db;
mod fact_index;
mod fact_store;
mod facts;
mod ids;
mod labels;
mod lang;
mod metadata;
mod review;
pub(crate) mod rule;
mod span;
mod stable_key;

pub use facts::{
    BranchObligation, ComplexityMetricFact, CoverageFact, DefinitionFact, DefinitionKind,
    FileMetricFact, FunctionFact, FunctionMetricFact, GoTypeDeclFact, GoTypeDeclKind, ImportFact,
    JsxAttributeFact, ModuleEdge, ModuleEdgeKind, ModuleNode, ModuleNodeKind, PackageFact,
    ReferenceFact, ReferenceKind, ResolutionPrecision, ResolutionStatus, ResolvedImportFact,
    SourceFile, StringLiteralFact, SymbolFact, SymbolKind, SymbolNamespace, SymbolPrecision,
    SymbolResolutionStatus, TestFact, TsClassFact, TsComponentFact, UnresolvedReason,
};
#[cfg(test)]
pub(crate) use facts::{CachedFileAnalysis, CachedFileFacts, TS_JS_MODULE_FUNCTION_NAME};
pub use ids::{
    BranchId, DefinitionId, FileId, FunctionId, ImportId, ModuleEdgeId, ModuleNodeId, NodeId,
    PackageId, ReferenceId, ResolvedImportId, RuleId, SymbolId,
};
pub use lang::Language;
pub use span::{Span, TextRange};
pub(crate) use stable_key::{StableKeyId, StableKeyInterner};
#[cfg(test)]
pub(crate) use stable_key::{stable_key_for_test, test_stable_key_interner};

pub use review::ChangeStatus;
pub(crate) use review::{ChangedFile, ReviewChangeset};

pub use capability::{
    Capabilities, CapabilitySupport, CapabilitySupportStatus, CapabilitySupportView,
};
pub use completeness::{CapabilityCompleteness, CapabilityCompletenessStatus, CompletenessView};
pub use db::AnalysisDb;
#[cfg(test)]
pub(crate) use rule::RuleRegistry;
#[cfg(test)]
pub(crate) use rule::run_rules_with_capability_support;
#[cfg(test)]
pub(crate) use rule::span_from_byte_range;
pub use rule::{Rule, RuleConfigValue, RuleCtx, RuleKind, RuleMeta, RuleOptions};
pub(crate) use rule::{
    RuleRuntimeViews, rule_id_matches, run_rules, run_rules_with_runtime_provider_blockers,
};

#[cfg(test)]
mod tests {
    include!("tests/batch1.rs");

    include!("tests/batch2.rs");

    include!("tests/batch3.rs");

    include!("tests/batch4.rs");

    include!("tests/batch5.rs");
}
