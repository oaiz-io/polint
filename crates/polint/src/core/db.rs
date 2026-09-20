//! Analysis database types.
//!
//! Extracted from the core monolith without behaviour changes.

use super::POLINT_ABSTRACT_DOMAINS_PROVIDER_ID;
use crate::analysis::access_paths::facts::AccessPathFact;
use crate::analysis::access_paths::store::AccessPathStore;
use crate::analysis::adaptation::facts::{AcceptedModelFact, RejectedModelFact};
use crate::analysis::aliases::facts::AliasAnswerFact;
use crate::analysis::aliases::store::AliasStore;
use crate::analysis::calls::facts::{
    CallSiteFact, CallTargetFact, CallTargetStatus, UnresolvedCallFact, UnresolvedCallReason,
};
use crate::analysis::calls::store::{CallOutput, CallStore};
use crate::analysis::cfg::facts::{
    BasicBlockFact, CfgEdgeFact, CfgFunctionFact, CfgNodeFact, ControlDependenceFact,
    DominatorFact, PostDominatorFact, ReachabilityFact, UnsupportedControlFlowFact,
};
use crate::analysis::cfg::store::CfgOutput;
use crate::analysis::data_flow::facts::{
    DataFlowBudgetFact, DataFlowConfidence, DataFlowEdgeFact, DataFlowModelFact, DataFlowNodeFact,
    DataFlowPrecision, DataFlowStatus, DataFlowValidation,
};
use crate::analysis::data_flow::provider::DATA_FLOW_PROVIDER_ID;
use crate::analysis::data_flow::store::{DataFlowOutput, DataFlowStore};
use crate::analysis::entrypoints::facts::{
    EntrypointFact, EntrypointStatus, FrameworkDispatchEdgeFact, TrustBoundaryFact,
    UnresolvedFrameworkFact,
};
use crate::analysis::entrypoints::store::{EntrypointOutput, EntrypointStore};
use crate::analysis::error::AnalysisError;
use crate::analysis::evidence::facts::{
    EvidenceBundleFact, EvidenceEdgeFact, EvidenceNodeFact, EvidenceOmittedRegionFact,
    EvidencePathFact, EvidencePrecision, EvidenceReplayKeyFact, EvidenceSliceFact,
    EvidenceUnknownFact,
};
use crate::analysis::evidence::provider::EVIDENCE_PROVIDER_ID;
use crate::analysis::evidence::store::{EvidenceOutput, EvidenceStore};
use crate::analysis::extensions::store::{
    AcceptedExtensionFact, ExtensionActivationRow, ExtensionOutput, RejectedExtensionFact,
};
use crate::analysis::identity::facts::{IdentityKind, IdentityRecord};
use crate::analysis::identity::store::{IdentityProviderOutput, IdentityStore};
use crate::analysis::ids::CallSiteId;
use crate::analysis::mir::body::{MirBlock, MirBody, MirOutput, MirStatement, MirTerminator};
use crate::analysis::mir::op::{MirOperation, UnsupportedSemanticFact};
use crate::analysis::places::PlaceFact;
use crate::analysis::points_to::facts::{PointsToConstraintFact, PointsToSetFact, PointsToStatus};
use crate::analysis::points_to::store::PointsToStore;
use crate::analysis::reachability::facts::{ReachabilityRootFact, RootPrecision, RootStatus};
use crate::analysis::reachability::store::{ReachabilityProviderOutput, ReachabilityStore};
use crate::analysis::refined_calls::facts::RefinedCallEdgeFact;
use crate::analysis::refined_calls::provider::REFINED_CALLS_PROVIDER_ID;
use crate::analysis::refined_calls::store::{RefinedCallOutput, RefinedCallStore};
use crate::analysis::semantic_graph::constraints::ConstraintFact;
use crate::analysis::semantic_graph::facts::{SemanticEdgeFact, SemanticNodeFact};
use crate::analysis::semantic_graph::store::{SemanticGraphOutput, SemanticGraphStore};
use crate::analysis::solver::budget::BudgetStatus;
use crate::analysis::solver::facts::DerivedEdgeFact;
use crate::analysis::solver::store::{SolverOutput, SolverStore};
use crate::analysis::store::SemanticStore;
use crate::analysis::summaries::facts::{SummaryEventFact, SummaryFact};
use crate::analysis::summaries::store::{SummaryOutput, SummaryStore};
use crate::analysis::types::facts::{NarrowedTypeFact, TypeFact};
use crate::analysis::types::provider::TYPE_VALUE_ALIAS_PROVIDER_ID;
use crate::analysis::types::store::{TypeStore, TypeValueAliasOutput};
use crate::analysis::values::facts::{AllocationTokenFact, ValueFact};
use crate::analysis::values::store::ValueStore;
use crate::analysis_kernel::{
    FactConfidence, FactFamily, FactMeta, FactMetaStore, FactPrecision, FactRef, MissingFactMeta,
    ValidationStatus, resolution_metadata, resolution_status_metadata, stable_key_text_from_parts,
    symbol_metadata,
};
use crate::analysis_neutral::domains::facts::{DomainEventFact, DomainObservationFact};
use crate::analysis_neutral::domains::store::DomainOutput;
use crate::analysis_neutral::domains::store::DomainStore;
use crate::core::StableKeyInterner;
use crate::diagnostics::fingerprint;
use crate::go::semantic::facts::{
    GoSemanticAddressTakenFact, GoSemanticCallsiteFact, GoSemanticDynamicDispatchFact,
    GoSemanticFunctionFact, GoSemanticInstantiatedTypeFact, GoSemanticMethodSetFact,
    GoSemanticPackageErrorFact, GoSemanticPackageFact,
};
use crate::go::semantic::store::GoSemanticStore;
#[cfg(test)]
use crate::go::semantic::store::{GoSemanticFactsOutput, GoSemanticStoreReport};
use crate::module_graph::topology::{
    DependencyRequirementFact, ImportToPackageFact, RepoTopologyOverlayFact,
    ResolvedDependencyEdgeFact, SourceSetFact, TopologyOutput, TopologyPackageFact,
    WorkspaceRootFact,
};
use crate::symbol_graph::semantic::{
    AliasFact, AliasId, ExportFact, ExportId, GeneratedSymbolFact, GeneratedSymbolId,
    ResolutionFact, ResolutionId, ScopeFact, ScopeId, SemanticImportFact, SemanticImportId,
    SemanticStatus, StableExportId, StableExportIdentity,
};
use crate::ts::object_model::facts::{
    TsObjectAllocationFact, TsPropertyReadFact, TsPropertyWriteFact, TsPrototypeLinkFact,
    TsReceiverBindingFact,
};
use crate::ts::object_model::store::{TsObjectModelOutput, TsObjectModelStore};
use crate::ts::types::facts::{
    TsTypeCallableFact, TsTypeCalleeFact, TsTypeCallsiteFact, TsTypeFileDensityFact,
    TsTypeReceiverFact,
};
use crate::ts::types::store::{TS_TYPES_STORE_FAMILY, TsTypesStore};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use super::fact_index::{DenseFileIndex, DenseIdIndex};
use super::fact_store::{
    ACCESS_PATH_STORE_FAMILY, ADAPTATION_STORE_FAMILY, ALIAS_STORE_FAMILY, AdaptationFactStore,
    CALL_STORE_FAMILY, CFG_STORE_FAMILY, CfgFactStore, DATA_FLOW_STORE_FAMILY, DOMAIN_STORE_FAMILY,
    ENTRYPOINT_STORE_FAMILY, EVIDENCE_STORE_FAMILY, EXTENSION_STORE_FAMILY, ExtensionFactStore,
    FactStore, FactStoreEntry, GO_SEMANTIC_STORE_FAMILY, GO_SYNTAX_STORE_FAMILY, GoSyntaxStore,
    IDENTITY_STORE_FAMILY, METRICS_STORE_FAMILY, MODULE_GRAPH_STORE_FAMILY,
    MODULE_TOPOLOGY_STORE_FAMILY, MetricsStore, ModuleGraphStore, ModuleTopologyStore,
    POINTS_TO_STORE_FAMILY, REACHABILITY_STORE_FAMILY, REFINED_CALL_STORE_FAMILY,
    SEMANTIC_GRAPH_STORE_FAMILY, SEMANTIC_INDEX_STORE_FAMILY, SEMANTIC_MIR_STORE_FAMILY,
    SOLVER_STORE_FAMILY, SUMMARY_STORE_FAMILY, SYMBOL_STORE_FAMILY, SemanticIndexStore,
    SymbolStore, TS_OBJECT_MODEL_STORE_FAMILY, TS_SYNTAX_STORE_FAMILY, TYPE_STORE_FAMILY,
    TsSyntaxStore, VALUE_STORE_FAMILY,
};
use super::facts::{
    BranchObligation, CachedFileFacts, ComplexityMetricFact, CoverageFact, DefinitionFact,
    FileMetricFact, FunctionFact, FunctionMetricFact, GoTypeDeclFact, ImportFact, JsxAttributeFact,
    ModuleEdge, ModuleNode, PackageFact, ReferenceFact, ResolvedImportFact, SourceFile,
    StringLiteralFact, SymbolFact, TestFact, TsClassFact, TsComponentFact,
};
use super::ids::{
    BranchId, FileId, FunctionId, ImportId, ModuleEdgeId, ModuleNodeId, PackageId,
    ResolvedImportId, SymbolId,
};
use super::labels::*;
use super::lang::Language;
use super::metadata::*;
use super::review::ReviewChangeset;
use super::span::Span;
use super::{
    CALLS_PROVIDER_ID, CFG_PROVIDER_ID, CYCLOMATIC_COMPLEXITY_METRIC_NAME, ENTRYPOINTS_PROVIDER_ID,
    FUNCTION_SIZE_METRIC_NAME, GO_SYNTAX_PROVIDER_ID, METRICS_PROVIDER_ID,
    MODULE_GRAPH_PROVIDER_ID, MODULE_TOPOLOGY_PROVIDER_ID, POLINT_DIRECT_SUMMARIES_PROVIDER_ID,
    SEMANTIC_MIR_PROVIDER_ID, SYMBOL_GRAPH_PROVIDER_ID, TS_SYNTAX_PROVIDER_ID,
};

#[derive(Debug, Clone)]
struct FactViewIndexes {
    functions_by_file: DenseFileIndex,
    imports_by_file: DenseFileIndex,
    resolved_imports_by_file: DenseFileIndex,
    module_node_by_file: DenseIdIndex,
    branches_by_file: DenseFileIndex,
    tests_by_file: DenseFileIndex,
    function_metrics_by_file: DenseFileIndex,
    complexity_metrics_by_file: DenseFileIndex,
    file_metric_by_file: DenseIdIndex,
    function_metric_by_function: DenseIdIndex,
    complexity_metric_by_function: DenseIdIndex,
    symbols_by_file: DenseFileIndex,
    definitions_by_file: DenseFileIndex,
    references_by_file: DenseFileIndex,
    ts_components_by_file: DenseFileIndex,
    ts_classes_by_file: DenseFileIndex,
    string_literals_by_file: DenseFileIndex,
    jsx_attributes_by_file: DenseFileIndex,
    go_types_by_file: DenseFileIndex,
}

impl FactViewIndexes {
    fn build(db: &AnalysisDb) -> Self {
        let file_count = db.files().len();
        let function_count = db
            .functions()
            .len()
            .max(db.function_metrics().len())
            .max(db.complexity_metrics().len());
        Self {
            functions_by_file: DenseFileIndex::build(file_count, db.functions(), |fact| {
                Some(fact.file)
            }),
            imports_by_file: DenseFileIndex::build(file_count, db.imports(), |fact| {
                Some(fact.file)
            }),
            resolved_imports_by_file: DenseFileIndex::build(
                file_count,
                db.resolved_imports(),
                |fact| Some(fact.from_file),
            ),
            module_node_by_file: DenseIdIndex::build(file_count, db.module_nodes(), |node| {
                node.file.map(|file| u64::from(file.raw()))
            }),
            branches_by_file: DenseFileIndex::build(file_count, db.branches(), |fact| {
                Some(fact.file)
            }),
            tests_by_file: DenseFileIndex::build(file_count, db.tests(), |fact| Some(fact.file)),
            function_metrics_by_file: DenseFileIndex::build(
                file_count,
                db.function_metrics(),
                |fact| Some(fact.file),
            ),
            complexity_metrics_by_file: DenseFileIndex::build(
                file_count,
                db.complexity_metrics(),
                |fact| Some(fact.file),
            ),
            file_metric_by_file: DenseIdIndex::build(file_count, db.file_metrics(), |fact| {
                Some(u64::from(fact.file.raw()))
            }),
            function_metric_by_function: DenseIdIndex::build(
                function_count,
                db.function_metrics(),
                |fact| Some(fact.function.raw()),
            ),
            complexity_metric_by_function: DenseIdIndex::build(
                function_count,
                db.complexity_metrics(),
                |fact| Some(fact.function.raw()),
            ),
            symbols_by_file: DenseFileIndex::build(file_count, db.symbols(), |fact| fact.file),
            definitions_by_file: DenseFileIndex::build(file_count, db.definitions(), |fact| {
                fact.file
            }),
            references_by_file: DenseFileIndex::build_sorted_by(
                file_count,
                db.references(),
                |fact| fact.file,
                |left, right| left.id.cmp(&right.id),
            ),
            ts_components_by_file: DenseFileIndex::build(file_count, db.ts_components(), |fact| {
                Some(fact.file)
            }),
            ts_classes_by_file: DenseFileIndex::build(file_count, db.ts_classes(), |fact| {
                Some(fact.file)
            }),
            string_literals_by_file: DenseFileIndex::build(
                file_count,
                db.string_literals(),
                |fact| Some(fact.file),
            ),
            jsx_attributes_by_file: DenseFileIndex::build(
                file_count,
                db.jsx_attributes(),
                |fact| Some(fact.file),
            ),
            go_types_by_file: DenseFileIndex::build(file_count, db.go_types(), |fact| {
                Some(fact.file)
            }),
        }
    }
}

#[derive(Debug)]
pub struct AnalysisDb {
    pub(crate) files: Vec<SourceFile>,
    pub(crate) stable_keys: StableKeyInterner,
    pub(crate) fact_meta: FactMetaStore,
    /// Provider-owned stores keyed by primary [`FactFamily`]. Iteration stays ordered.
    pub(crate) fact_stores: BTreeMap<FactFamily, FactStoreEntry>,
    fact_view_indexes: OnceLock<FactViewIndexes>,
    pub(crate) path_contexts: Option<crate::path_context::PathContextIndex>,
    /// Diff-to-target-ref facts, injected by the host for `polint review`.
    ///
    /// This is the first externally injected fact family: it is set by the
    /// runner via [`AnalysisDb::set_changeset`] after the kernel runs, not
    /// derived by a provider. It is `None` under `polint check` (so the
    /// `ChangedFiles` view is empty there) and excluded from all cache digests.
    pub(crate) changeset: Option<ReviewChangeset>,
}

impl Clone for AnalysisDb {
    fn clone(&self) -> Self {
        Self {
            files: self.files.clone(),
            stable_keys: self.stable_keys.detached_clone(),
            fact_meta: self.fact_meta.clone(),
            fact_stores: self.fact_stores.clone(),
            fact_view_indexes: self.fact_view_indexes.clone(),
            path_contexts: self.path_contexts.clone(),
            changeset: self.changeset.clone(),
        }
    }
}

impl Default for AnalysisDb {
    fn default() -> Self {
        let mut fact_stores = BTreeMap::new();
        let mut go_syntax = GoSyntaxStore::default();
        // Trait method is part of the store contract (eviction later). Calling on an
        // empty store at construction keeps the method on the product path.
        FactStore::clear(&mut go_syntax);
        fact_stores.insert(GO_SYNTAX_STORE_FAMILY, FactStoreEntry::new(go_syntax));
        let mut ts_syntax = TsSyntaxStore::default();
        FactStore::clear(&mut ts_syntax);
        fact_stores.insert(TS_SYNTAX_STORE_FAMILY, FactStoreEntry::new(ts_syntax));
        let mut cfg_store = CfgFactStore::default();
        FactStore::clear(&mut cfg_store);
        fact_stores.insert(CFG_STORE_FAMILY, FactStoreEntry::new(cfg_store));
        let mut call_store = CallStore::default();
        FactStore::clear(&mut call_store);
        fact_stores.insert(CALL_STORE_FAMILY, FactStoreEntry::new(call_store));
        let mut go_semantic = GoSemanticStore::default();
        FactStore::clear(&mut go_semantic);
        fact_stores.insert(GO_SEMANTIC_STORE_FAMILY, FactStoreEntry::new(go_semantic));
        let mut module_graph = ModuleGraphStore::default();
        FactStore::clear(&mut module_graph);
        fact_stores.insert(MODULE_GRAPH_STORE_FAMILY, FactStoreEntry::new(module_graph));
        let mut module_topology = ModuleTopologyStore::default();
        FactStore::clear(&mut module_topology);
        fact_stores.insert(
            MODULE_TOPOLOGY_STORE_FAMILY,
            FactStoreEntry::new(module_topology),
        );
        let mut symbol_store = SymbolStore::default();
        FactStore::clear(&mut symbol_store);
        fact_stores.insert(SYMBOL_STORE_FAMILY, FactStoreEntry::new(symbol_store));
        let mut semantic_index = SemanticIndexStore::default();
        FactStore::clear(&mut semantic_index);
        fact_stores.insert(
            SEMANTIC_INDEX_STORE_FAMILY,
            FactStoreEntry::new(semantic_index),
        );
        let mut metrics = MetricsStore::default();
        FactStore::clear(&mut metrics);
        fact_stores.insert(METRICS_STORE_FAMILY, FactStoreEntry::new(metrics));
        let mut ts_object_model = TsObjectModelStore::default();
        FactStore::clear(&mut ts_object_model);
        fact_stores.insert(
            TS_OBJECT_MODEL_STORE_FAMILY,
            FactStoreEntry::new(ts_object_model),
        );
        let mut ts_types = TsTypesStore::default();
        FactStore::clear(&mut ts_types);
        fact_stores.insert(TS_TYPES_STORE_FAMILY, FactStoreEntry::new(ts_types));
        let mut identity = IdentityStore::default();
        FactStore::clear(&mut identity);
        fact_stores.insert(IDENTITY_STORE_FAMILY, FactStoreEntry::new(identity));
        let mut refined_call = RefinedCallStore::default();
        FactStore::clear(&mut refined_call);
        fact_stores.insert(REFINED_CALL_STORE_FAMILY, FactStoreEntry::new(refined_call));
        let mut data_flow = DataFlowStore::default();
        FactStore::clear(&mut data_flow);
        fact_stores.insert(DATA_FLOW_STORE_FAMILY, FactStoreEntry::new(data_flow));
        let mut evidence = EvidenceStore::default();
        FactStore::clear(&mut evidence);
        fact_stores.insert(EVIDENCE_STORE_FAMILY, FactStoreEntry::new(evidence));
        let mut domain = DomainStore::default();
        FactStore::clear(&mut domain);
        fact_stores.insert(DOMAIN_STORE_FAMILY, FactStoreEntry::new(domain));
        let mut summary = SummaryStore::default();
        FactStore::clear(&mut summary);
        fact_stores.insert(SUMMARY_STORE_FAMILY, FactStoreEntry::new(summary));
        let mut entrypoint = EntrypointStore::default();
        FactStore::clear(&mut entrypoint);
        fact_stores.insert(ENTRYPOINT_STORE_FAMILY, FactStoreEntry::new(entrypoint));
        let mut type_store = TypeStore::default();
        FactStore::clear(&mut type_store);
        fact_stores.insert(TYPE_STORE_FAMILY, FactStoreEntry::new(type_store));
        let mut value_store = ValueStore::default();
        FactStore::clear(&mut value_store);
        fact_stores.insert(VALUE_STORE_FAMILY, FactStoreEntry::new(value_store));
        let mut access_path_store = AccessPathStore::default();
        FactStore::clear(&mut access_path_store);
        fact_stores.insert(
            ACCESS_PATH_STORE_FAMILY,
            FactStoreEntry::new(access_path_store),
        );
        let mut points_to_store = PointsToStore::default();
        FactStore::clear(&mut points_to_store);
        fact_stores.insert(POINTS_TO_STORE_FAMILY, FactStoreEntry::new(points_to_store));
        let mut alias_store = AliasStore::default();
        FactStore::clear(&mut alias_store);
        fact_stores.insert(ALIAS_STORE_FAMILY, FactStoreEntry::new(alias_store));
        let mut extension = ExtensionFactStore::default();
        FactStore::clear(&mut extension);
        fact_stores.insert(EXTENSION_STORE_FAMILY, FactStoreEntry::new(extension));
        let mut adaptation = AdaptationFactStore::default();
        FactStore::clear(&mut adaptation);
        fact_stores.insert(ADAPTATION_STORE_FAMILY, FactStoreEntry::new(adaptation));
        let mut reachability = ReachabilityStore::default();
        FactStore::clear(&mut reachability);
        fact_stores.insert(REACHABILITY_STORE_FAMILY, FactStoreEntry::new(reachability));
        let mut semantic_graph = SemanticGraphStore::default();
        FactStore::clear(&mut semantic_graph);
        fact_stores.insert(
            SEMANTIC_GRAPH_STORE_FAMILY,
            FactStoreEntry::new(semantic_graph),
        );
        let mut solver = SolverStore::default();
        FactStore::clear(&mut solver);
        fact_stores.insert(SOLVER_STORE_FAMILY, FactStoreEntry::new(solver));
        let mut semantic_mir = SemanticStore::default();
        FactStore::clear(&mut semantic_mir);
        fact_stores.insert(SEMANTIC_MIR_STORE_FAMILY, FactStoreEntry::new(semantic_mir));
        Self {
            files: Vec::new(),
            stable_keys: {
                #[cfg(test)]
                {
                    crate::core::test_stable_key_interner()
                }
                #[cfg(not(test))]
                {
                    StableKeyInterner::default()
                }
            },
            fact_meta: FactMetaStore::default(),
            fact_stores,
            fact_view_indexes: OnceLock::new(),
            path_contexts: None,
            changeset: None,
        }
    }
}

impl AnalysisDb {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn stable_key_interner(&self) -> StableKeyInterner {
        self.stable_keys.clone()
    }

    pub(crate) fn resolve_stable_key(&self, id: crate::core::StableKeyId) -> Arc<str> {
        self.stable_keys.resolve(id)
    }

    fn go_syntax_store(&self) -> &GoSyntaxStore {
        self.fact_store(GO_SYNTAX_STORE_FAMILY)
            .expect("GoSyntaxStore is installed when AnalysisDb is constructed")
    }

    fn go_syntax_store_mut(&mut self) -> &mut GoSyntaxStore {
        self.fact_store_mut(GO_SYNTAX_STORE_FAMILY)
            .expect("GoSyntaxStore is installed when AnalysisDb is constructed")
    }

    fn ts_syntax_store(&self) -> &TsSyntaxStore {
        self.fact_store(TS_SYNTAX_STORE_FAMILY)
            .expect("TsSyntaxStore is installed when AnalysisDb is constructed")
    }

    fn ts_syntax_store_mut(&mut self) -> &mut TsSyntaxStore {
        self.fact_store_mut(TS_SYNTAX_STORE_FAMILY)
            .expect("TsSyntaxStore is installed when AnalysisDb is constructed")
    }

    fn cfg_store(&self) -> &CfgFactStore {
        self.fact_store(CFG_STORE_FAMILY)
            .expect("CfgFactStore is installed when AnalysisDb is constructed")
    }

    #[allow(
        dead_code,
        reason = "CFG writes go through AnalysisHost in polint-analysis; kept for AnalysisDb test helpers."
    )]
    fn cfg_store_mut(&mut self) -> &mut CfgFactStore {
        self.fact_store_mut(CFG_STORE_FAMILY)
            .expect("CfgFactStore is installed when AnalysisDb is constructed")
    }

    fn calls_store(&self) -> &CallStore {
        self.fact_store(CALL_STORE_FAMILY)
            .expect("CallStore is installed when AnalysisDb is constructed")
    }

    #[allow(
        dead_code,
        reason = "Call writes go through AnalysisHost in polint-analysis; kept for AnalysisDb test helpers."
    )]
    fn calls_store_mut(&mut self) -> &mut CallStore {
        self.fact_store_mut(CALL_STORE_FAMILY)
            .expect("CallStore is installed when AnalysisDb is constructed")
    }

    fn go_semantic_store(&self) -> &GoSemanticStore {
        self.fact_store(GO_SEMANTIC_STORE_FAMILY)
            .expect("GoSemanticStore is installed when AnalysisDb is constructed")
    }

    #[allow(
        dead_code,
        reason = "Go semantic writes now go through polint-go local helpers; kept for AnalysisDb store access."
    )]
    fn go_semantic_store_mut(&mut self) -> &mut GoSemanticStore {
        self.fact_store_mut(GO_SEMANTIC_STORE_FAMILY)
            .expect("GoSemanticStore is installed when AnalysisDb is constructed")
    }

    fn module_graph_store(&self) -> &ModuleGraphStore {
        self.fact_store(MODULE_GRAPH_STORE_FAMILY)
            .expect("ModuleGraphStore is installed when AnalysisDb is constructed")
    }

    fn module_graph_store_mut(&mut self) -> &mut ModuleGraphStore {
        self.fact_store_mut(MODULE_GRAPH_STORE_FAMILY)
            .expect("ModuleGraphStore is installed when AnalysisDb is constructed")
    }

    fn module_topology_store(&self) -> &ModuleTopologyStore {
        self.fact_store(MODULE_TOPOLOGY_STORE_FAMILY)
            .expect("ModuleTopologyStore is installed when AnalysisDb is constructed")
    }

    fn module_topology_store_mut(&mut self) -> &mut ModuleTopologyStore {
        self.fact_store_mut(MODULE_TOPOLOGY_STORE_FAMILY)
            .expect("ModuleTopologyStore is installed when AnalysisDb is constructed")
    }

    fn symbol_store(&self) -> &SymbolStore {
        self.fact_store(SYMBOL_STORE_FAMILY)
            .expect("SymbolStore is installed when AnalysisDb is constructed")
    }

    fn symbol_store_mut(&mut self) -> &mut SymbolStore {
        self.fact_store_mut(SYMBOL_STORE_FAMILY)
            .expect("SymbolStore is installed when AnalysisDb is constructed")
    }

    fn semantic_index_store(&self) -> &SemanticIndexStore {
        self.fact_store(SEMANTIC_INDEX_STORE_FAMILY)
            .expect("SemanticIndexStore is installed when AnalysisDb is constructed")
    }

    fn semantic_index_store_mut(&mut self) -> &mut SemanticIndexStore {
        self.fact_store_mut(SEMANTIC_INDEX_STORE_FAMILY)
            .expect("SemanticIndexStore is installed when AnalysisDb is constructed")
    }

    fn metrics_store(&self) -> &MetricsStore {
        self.fact_store(METRICS_STORE_FAMILY)
            .expect("MetricsStore is installed when AnalysisDb is constructed")
    }

    fn metrics_store_mut(&mut self) -> &mut MetricsStore {
        self.fact_store_mut(METRICS_STORE_FAMILY)
            .expect("MetricsStore is installed when AnalysisDb is constructed")
    }

    fn ts_object_model_store_inner(&self) -> &TsObjectModelStore {
        self.fact_store(TS_OBJECT_MODEL_STORE_FAMILY)
            .expect("TsObjectModelStore is installed when AnalysisDb is constructed")
    }

    fn ts_object_model_store_mut(&mut self) -> &mut TsObjectModelStore {
        self.fact_store_mut(TS_OBJECT_MODEL_STORE_FAMILY)
            .expect("TsObjectModelStore is installed when AnalysisDb is constructed")
    }

    fn ts_types_store(&self) -> &TsTypesStore {
        self.fact_store(TS_TYPES_STORE_FAMILY)
            .expect("TsTypesStore is installed when AnalysisDb is constructed")
    }

    fn identity_store_inner(&self) -> &IdentityStore {
        self.fact_store(IDENTITY_STORE_FAMILY)
            .expect("IdentityStore is installed when AnalysisDb is constructed")
    }

    fn identity_store_mut(&mut self) -> &mut IdentityStore {
        self.fact_store_mut(IDENTITY_STORE_FAMILY)
            .expect("IdentityStore is installed when AnalysisDb is constructed")
    }

    fn refined_call_store_inner(&self) -> &RefinedCallStore {
        self.fact_store(REFINED_CALL_STORE_FAMILY)
            .expect("RefinedCallStore is installed when AnalysisDb is constructed")
    }

    fn refined_call_store_mut(&mut self) -> &mut RefinedCallStore {
        self.fact_store_mut(REFINED_CALL_STORE_FAMILY)
            .expect("RefinedCallStore is installed when AnalysisDb is constructed")
    }

    fn data_flow_store_inner(&self) -> &DataFlowStore {
        self.fact_store(DATA_FLOW_STORE_FAMILY)
            .expect("DataFlowStore is installed when AnalysisDb is constructed")
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    fn data_flow_store_mut(&mut self) -> &mut DataFlowStore {
        self.fact_store_mut(DATA_FLOW_STORE_FAMILY)
            .expect("DataFlowStore is installed when AnalysisDb is constructed")
    }

    fn evidence_store_inner(&self) -> &EvidenceStore {
        self.fact_store(EVIDENCE_STORE_FAMILY)
            .expect("EvidenceStore is installed when AnalysisDb is constructed")
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    fn evidence_store_mut(&mut self) -> &mut EvidenceStore {
        self.fact_store_mut(EVIDENCE_STORE_FAMILY)
            .expect("EvidenceStore is installed when AnalysisDb is constructed")
    }

    fn domain_store_inner(&self) -> &DomainStore {
        self.fact_store(DOMAIN_STORE_FAMILY)
            .expect("DomainStore is installed when AnalysisDb is constructed")
    }

    fn domain_store_mut(&mut self) -> &mut DomainStore {
        self.fact_store_mut(DOMAIN_STORE_FAMILY)
            .expect("DomainStore is installed when AnalysisDb is constructed")
    }

    fn summary_store_inner(&self) -> &SummaryStore {
        self.fact_store(SUMMARY_STORE_FAMILY)
            .expect("SummaryStore is installed when AnalysisDb is constructed")
    }

    fn summary_store_mut(&mut self) -> &mut SummaryStore {
        self.fact_store_mut(SUMMARY_STORE_FAMILY)
            .expect("SummaryStore is installed when AnalysisDb is constructed")
    }

    fn entrypoint_store_inner(&self) -> &EntrypointStore {
        self.fact_store(ENTRYPOINT_STORE_FAMILY)
            .expect("EntrypointStore is installed when AnalysisDb is constructed")
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    fn entrypoint_store_mut(&mut self) -> &mut EntrypointStore {
        self.fact_store_mut(ENTRYPOINT_STORE_FAMILY)
            .expect("EntrypointStore is installed when AnalysisDb is constructed")
    }

    fn type_store_inner(&self) -> &TypeStore {
        self.fact_store(TYPE_STORE_FAMILY)
            .expect("TypeStore is installed when AnalysisDb is constructed")
    }

    fn type_store_mut(&mut self) -> &mut TypeStore {
        self.fact_store_mut(TYPE_STORE_FAMILY)
            .expect("TypeStore is installed when AnalysisDb is constructed")
    }

    fn value_store_inner(&self) -> &ValueStore {
        self.fact_store(VALUE_STORE_FAMILY)
            .expect("ValueStore is installed when AnalysisDb is constructed")
    }

    fn value_store_mut(&mut self) -> &mut ValueStore {
        self.fact_store_mut(VALUE_STORE_FAMILY)
            .expect("ValueStore is installed when AnalysisDb is constructed")
    }

    fn access_path_store_inner(&self) -> &AccessPathStore {
        self.fact_store(ACCESS_PATH_STORE_FAMILY)
            .expect("AccessPathStore is installed when AnalysisDb is constructed")
    }

    fn access_path_store_mut(&mut self) -> &mut AccessPathStore {
        self.fact_store_mut(ACCESS_PATH_STORE_FAMILY)
            .expect("AccessPathStore is installed when AnalysisDb is constructed")
    }

    fn points_to_store_inner(&self) -> &PointsToStore {
        self.fact_store(POINTS_TO_STORE_FAMILY)
            .expect("PointsToStore is installed when AnalysisDb is constructed")
    }

    fn points_to_store_mut(&mut self) -> &mut PointsToStore {
        self.fact_store_mut(POINTS_TO_STORE_FAMILY)
            .expect("PointsToStore is installed when AnalysisDb is constructed")
    }

    fn alias_store_inner(&self) -> &AliasStore {
        self.fact_store(ALIAS_STORE_FAMILY)
            .expect("AliasStore is installed when AnalysisDb is constructed")
    }

    fn alias_store_mut(&mut self) -> &mut AliasStore {
        self.fact_store_mut(ALIAS_STORE_FAMILY)
            .expect("AliasStore is installed when AnalysisDb is constructed")
    }

    fn extension_store_inner(&self) -> &ExtensionFactStore {
        self.fact_store(EXTENSION_STORE_FAMILY)
            .expect("ExtensionFactStore is installed when AnalysisDb is constructed")
    }

    fn extension_store_mut(&mut self) -> &mut ExtensionFactStore {
        self.fact_store_mut(EXTENSION_STORE_FAMILY)
            .expect("ExtensionFactStore is installed when AnalysisDb is constructed")
    }

    fn adaptation_store_inner(&self) -> &AdaptationFactStore {
        self.fact_store(ADAPTATION_STORE_FAMILY)
            .expect("AdaptationFactStore is installed when AnalysisDb is constructed")
    }

    fn adaptation_store_mut(&mut self) -> &mut AdaptationFactStore {
        self.fact_store_mut(ADAPTATION_STORE_FAMILY)
            .expect("AdaptationFactStore is installed when AnalysisDb is constructed")
    }

    fn reachability_store_inner(&self) -> &ReachabilityStore {
        self.fact_store(REACHABILITY_STORE_FAMILY)
            .expect("ReachabilityStore is installed when AnalysisDb is constructed")
    }

    fn reachability_store_mut(&mut self) -> &mut ReachabilityStore {
        self.fact_store_mut(REACHABILITY_STORE_FAMILY)
            .expect("ReachabilityStore is installed when AnalysisDb is constructed")
    }

    fn semantic_graph_store_inner(&self) -> &SemanticGraphStore {
        self.fact_store(SEMANTIC_GRAPH_STORE_FAMILY)
            .expect("SemanticGraphStore is installed when AnalysisDb is constructed")
    }

    fn semantic_graph_store_mut(&mut self) -> &mut SemanticGraphStore {
        self.fact_store_mut(SEMANTIC_GRAPH_STORE_FAMILY)
            .expect("SemanticGraphStore is installed when AnalysisDb is constructed")
    }

    fn solver_store_inner(&self) -> &SolverStore {
        self.fact_store(SOLVER_STORE_FAMILY)
            .expect("SolverStore is installed when AnalysisDb is constructed")
    }

    fn solver_store_mut(&mut self) -> &mut SolverStore {
        self.fact_store_mut(SOLVER_STORE_FAMILY)
            .expect("SolverStore is installed when AnalysisDb is constructed")
    }

    fn semantic_mir_store_inner(&self) -> &SemanticStore {
        self.fact_store(SEMANTIC_MIR_STORE_FAMILY)
            .expect("SemanticStore is installed when AnalysisDb is constructed")
    }

    fn semantic_mir_store_mut(&mut self) -> &mut SemanticStore {
        self.fact_store_mut(SEMANTIC_MIR_STORE_FAMILY)
            .expect("SemanticStore is installed when AnalysisDb is constructed")
    }

    /// Typed downcast helper for registry stores. Returns `None` when the family
    /// is absent or holds a different concrete store type.
    pub(crate) fn fact_store<T: 'static>(&self, family: FactFamily) -> Option<&T> {
        self.fact_stores
            .get(&family)
            .and_then(|entry| entry.as_store().as_any().downcast_ref::<T>())
    }

    /// Mutable typed downcast helper for registry stores.
    pub(crate) fn fact_store_mut<T: 'static>(&mut self, family: FactFamily) -> Option<&mut T> {
        self.invalidate_fact_view_indexes();
        self.fact_stores
            .get_mut(&family)
            .and_then(|entry| entry.as_store_mut().as_any_mut().downcast_mut::<T>())
    }

    pub(crate) fn invalidate_fact_view_indexes(&mut self) {
        let _ = self.fact_view_indexes.take();
    }

    fn fact_view_indexes(&self) -> &FactViewIndexes {
        self.fact_view_indexes
            .get_or_init(|| FactViewIndexes::build(self))
    }

    fn finalize_fact_view_indexes(&mut self) {
        self.fact_view_indexes = OnceLock::from(FactViewIndexes::build(self));
    }

    /// Injects diff-to-target-ref facts for `polint review`.
    ///
    /// Called by the host runner after the kernel runs and before rules
    /// execute, so the `ChangedFiles` fact view can read the diff. The
    /// changeset is excluded from all cache digests by construction (it is set
    /// post-kernel), so a changing diff never busts the analysis cache.
    pub(crate) fn set_changeset(&mut self, changeset: ReviewChangeset) {
        self.changeset = Some(changeset);
    }

    /// Returns the injected changeset, or `None` under `polint check`.
    pub(crate) fn changeset(&self) -> Option<&ReviewChangeset> {
        self.changeset.as_ref()
    }

    pub fn add_file(&mut self, path: PathBuf, relative_path: String, source: String) -> FileId {
        let interner_handle = self.stable_key_interner();
        let interner = &interner_handle;
        let language = Language::from_path(&path);
        let content_hash = fingerprint(&[&source]);
        self.push_source_file(
            interner,
            path,
            relative_path,
            language,
            Arc::from(source),
            content_hash,
        )
    }

    pub fn add_source_file(
        &mut self,
        path: PathBuf,
        relative_path: String,
        language: Language,
        source: Arc<str>,
        content_hash: String,
    ) -> FileId {
        let interner_handle = self.stable_key_interner();
        let interner = &interner_handle;
        self.push_source_file(
            interner,
            path,
            relative_path,
            language,
            source,
            content_hash,
        )
    }

    fn push_source_file(
        &mut self,
        interner: &crate::core::StableKeyInterner,
        path: PathBuf,
        relative_path: String,
        language: Language,
        source: Arc<str>,
        content_hash: String,
    ) -> FileId {
        self.invalidate_fact_view_indexes();
        let id = FileId::from_raw(self.files.len() as u32);
        let metadata = source_file_metadata(interner, &relative_path, language, &content_hash);
        self.files.push(SourceFile::new(
            id,
            path,
            relative_path,
            language,
            source,
            content_hash,
        ));
        self.record_fact_meta(FactFamily::SourceFile, u64::from(id.0), metadata);
        id
    }

    pub fn push_package(&mut self, mut fact: PackageFact) -> PackageId {
        let interner_handle = self.stable_key_interner();
        let interner = &interner_handle;
        fact.id = PackageId::from_raw(self.go_syntax_store().packages().len() as u64);
        let metadata = self.package_metadata(interner, &fact);
        let id = self.go_syntax_store_mut().push_package(fact);
        self.record_fact_meta(FactFamily::Package, id.0, metadata);
        id
    }

    pub fn push_function(&mut self, mut fact: FunctionFact) -> FunctionId {
        let interner_handle = self.stable_key_interner();
        let interner = &interner_handle;
        fact.id = FunctionId::from_raw(self.go_syntax_store().functions().len() as u64);
        let metadata = self.function_metadata(interner, &fact);
        let id = self.go_syntax_store_mut().push_function(fact);
        self.record_fact_meta(FactFamily::Function, id.0, metadata);
        id
    }

    pub fn push_import(&mut self, mut fact: ImportFact) -> ImportId {
        let interner_handle = self.stable_key_interner();
        let interner = &interner_handle;
        fact.id = ImportId::from_raw(self.go_syntax_store().imports().len() as u64);
        let metadata = self.import_metadata(interner, &fact);
        let id = self.go_syntax_store_mut().push_import(fact);
        self.record_fact_meta(FactFamily::Import, id.0, metadata);
        id
    }

    pub fn push_branch(&mut self, mut fact: BranchObligation) -> BranchId {
        let interner_handle = self.stable_key_interner();
        let interner = &interner_handle;
        fact.id = BranchId::from_raw(self.go_syntax_store().branches().len() as u64);
        let metadata = self.branch_metadata(interner, &fact);
        let id = self.go_syntax_store_mut().push_branch(fact);
        self.record_fact_meta(FactFamily::BranchObligation, id.0, metadata);
        id
    }

    pub fn push_test(&mut self, fact: TestFact) {
        let interner_handle = self.stable_key_interner();
        let interner = &interner_handle;
        let metadata = self.test_metadata(interner, &fact);
        let run_id = self.go_syntax_store_mut().push_test(fact);
        self.record_fact_meta(FactFamily::Test, run_id, metadata);
    }

    /// Stores one Go structural type fact. Go type rows are pure syntax facts and
    /// carry no provider-attribution metadata of their own.
    pub fn push_go_type(&mut self, fact: GoTypeDeclFact) {
        self.invalidate_fact_view_indexes();
        self.go_syntax_store_mut().push_go_type(fact);
    }

    pub fn push_coverage(&mut self, fact: CoverageFact) {
        let interner_handle = self.stable_key_interner();
        let interner = &interner_handle;
        let metadata = self.coverage_metadata(interner, &fact);
        let run_id = self.metrics_store_mut().push_coverage(fact);
        self.record_fact_meta(FactFamily::Coverage, run_id, metadata);
    }

    pub(crate) fn replace_metric_facts(
        &mut self,
        file_metrics: Vec<FileMetricFact>,
        function_metrics: Vec<FunctionMetricFact>,
        complexity_metrics: Vec<ComplexityMetricFact>,
    ) {
        let interner_handle = self.stable_key_interner();
        let interner = &interner_handle;
        self.metrics_store_mut().replace_metrics(
            file_metrics,
            function_metrics,
            complexity_metrics,
        );
        self.refresh_metric_metadata(interner);
    }

    pub(crate) fn replace_module_graph_facts(
        &mut self,
        mut resolved_imports: Vec<ResolvedImportFact>,
        mut module_nodes: Vec<ModuleNode>,
        mut module_edges: Vec<ModuleEdge>,
    ) {
        let interner_handle = self.stable_key_interner();
        let interner = &interner_handle;
        let resolved_import_ids = resolved_imports
            .iter()
            .enumerate()
            .map(|(index, fact)| (fact.id, ResolvedImportId::from_raw(index as u64)))
            .collect::<BTreeMap<_, _>>();
        let module_node_ids = module_nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id, ModuleNodeId::from_raw(index as u64)))
            .collect::<BTreeMap<_, _>>();

        for (index, fact) in resolved_imports.iter_mut().enumerate() {
            fact.id = ResolvedImportId::from_raw(index as u64);
            if let Some(target_node) = fact.target_node
                && let Some(remapped) = module_node_ids.get(&target_node)
            {
                fact.target_node = Some(*remapped);
            }
        }
        for (index, node) in module_nodes.iter_mut().enumerate() {
            node.id = ModuleNodeId::from_raw(index as u64);
        }
        for (index, edge) in module_edges.iter_mut().enumerate() {
            edge.id = ModuleEdgeId::from_raw(index as u64);
            if let Some(remapped) = module_node_ids.get(&edge.from) {
                edge.from = *remapped;
            }
            if let Some(remapped) = module_node_ids.get(&edge.to) {
                edge.to = *remapped;
            }
            if let Some(resolved_import) = edge.resolved_import
                && let Some(remapped) = resolved_import_ids.get(&resolved_import)
            {
                edge.resolved_import = Some(*remapped);
            }
        }

        self.module_graph_store_mut()
            .replace(resolved_imports, module_nodes, module_edges);
        self.refresh_module_graph_metadata(interner);
    }

    pub(crate) fn replace_topology_facts(&mut self, output: TopologyOutput) {
        let output = output.normalized(&self.stable_keys);
        *self.module_topology_store_mut() = ModuleTopologyStore::from_output(output);
        self.refresh_topology_metadata();
    }

    pub(crate) fn replace_import_to_package_facts(&mut self, edges: Vec<ImportToPackageFact>) {
        let output = TopologyOutput {
            import_to_package_edges: edges,
            ..TopologyOutput::default()
        }
        .normalized(&self.stable_keys);
        self.module_topology_store_mut()
            .replace_import_to_package_edges(output.import_to_package_edges);
        self.refresh_import_to_package_metadata();
    }

    pub(crate) fn replace_symbol_graph_facts(
        &mut self,
        symbols: Vec<SymbolFact>,
        definitions: Vec<DefinitionFact>,
        references: Vec<ReferenceFact>,
    ) {
        self.symbol_store_mut()
            .replace(symbols, definitions, references);
        self.refresh_symbol_graph_metadata();
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "semantic index replacement accepts every internal semantic row family explicitly"
    )]
    pub(crate) fn replace_semantic_index_facts(
        &mut self,
        mut scopes: Vec<ScopeFact>,
        mut semantic_imports: Vec<SemanticImportFact>,
        mut exports: Vec<ExportFact>,
        mut aliases: Vec<AliasFact>,
        mut resolutions: Vec<ResolutionFact>,
        mut generated_symbols: Vec<GeneratedSymbolFact>,
        mut stable_exports: Vec<StableExportIdentity>,
    ) {
        let interner_handle = self.stable_key_interner();
        let interner = &interner_handle;
        normalize_scope_facts(interner, &mut scopes);
        let scope_ids = scopes
            .iter()
            .enumerate()
            .map(|(index, scope)| (scope.id, ScopeId(index as u64)))
            .collect::<BTreeMap<_, _>>();
        for (index, scope) in scopes.iter_mut().enumerate() {
            scope.id = ScopeId(index as u64);
            if let Some(parent) = scope.parent
                && let Some(remapped) = scope_ids.get(&parent)
            {
                scope.parent = Some(*remapped);
            }
        }

        normalize_semantic_import_facts(interner, &mut semantic_imports);
        for (index, import) in semantic_imports.iter_mut().enumerate() {
            import.id = SemanticImportId(index as u64);
            if let Some(scope) = import.scope
                && let Some(remapped) = scope_ids.get(&scope)
            {
                import.scope = Some(*remapped);
            }
        }

        normalize_export_facts(interner, &mut exports);
        let export_ids = exports
            .iter()
            .enumerate()
            .map(|(index, export)| (export.id, ExportId(index as u64)))
            .collect::<BTreeMap<_, _>>();
        for (index, export) in exports.iter_mut().enumerate() {
            export.id = ExportId(index as u64);
            if let Some(scope) = export.scope
                && let Some(remapped) = scope_ids.get(&scope)
            {
                export.scope = Some(*remapped);
            }
        }

        normalize_alias_facts(interner, &mut aliases);
        for (index, alias) in aliases.iter_mut().enumerate() {
            alias.id = AliasId(index as u64);
        }

        normalize_resolution_facts(interner, &mut resolutions);
        for (index, resolution) in resolutions.iter_mut().enumerate() {
            resolution.id = ResolutionId(index as u64);
        }

        normalize_generated_symbol_facts(interner, &mut generated_symbols);
        for (index, generated) in generated_symbols.iter_mut().enumerate() {
            generated.id = GeneratedSymbolId(index as u64);
        }

        normalize_stable_export_identities(interner, &mut stable_exports);
        for (index, stable_export) in stable_exports.iter_mut().enumerate() {
            stable_export.id = StableExportId(index as u64);
            if let Some(remapped) = export_ids.get(&stable_export.export) {
                stable_export.export = *remapped;
            }
        }

        self.semantic_index_store_mut().replace(
            scopes,
            semantic_imports,
            exports,
            aliases,
            resolutions,
            generated_symbols,
            stable_exports,
        );
        self.refresh_semantic_index_metadata();
    }

    pub(crate) fn replace_semantic_mir(&mut self, output: MirOutput) -> Result<(), AnalysisError> {
        let interner_handle = self.stable_key_interner();
        let interner = &interner_handle;
        *self.semantic_mir_store_mut() = SemanticStore::from_output(output, interner)?;
        self.refresh_semantic_mir_metadata(interner);
        Ok(())
    }

    #[allow(
        dead_code,
        reason = "CFG writes go through AnalysisHost in polint-analysis; kept for AnalysisDb test helpers."
    )]
    pub(crate) fn replace_cfg_facts(&mut self, output: CfgOutput) -> Result<(), AnalysisError> {
        let interner_handle = self.stable_key_interner();
        let output = output.normalized(&interner_handle);
        self.cfg_store_mut().replace(output);
        self.refresh_cfg_metadata();
        Ok(())
    }

    #[allow(
        dead_code,
        reason = "Call writes go through AnalysisHost in polint-analysis; kept for AnalysisDb test helpers."
    )]
    pub(crate) fn replace_call_facts(
        &mut self,
        mut output: CallOutput,
    ) -> Result<(), AnalysisError> {
        let interner_handle = self.stable_key_interner();
        let interner = &interner_handle;
        self.populate_call_owner_symbols(&mut output);
        let store = CallStore::from_output(output, interner)?;
        *self.calls_store_mut() = store;
        self.refresh_call_metadata(interner);
        Ok(())
    }

    fn identity_status_metadata(_record: &IdentityRecord) -> (FactPrecision, FactConfidence) {
        (FactPrecision::SetupAware, FactConfidence::High)
    }

    fn identity_kind_label(kind: IdentityKind) -> &'static str {
        match kind {
            IdentityKind::Function => "function",
            IdentityKind::Callsite => "callsite",
        }
    }

    fn format_signature_digest(
        digest: crate::analysis::identity::facts::SignatureDigest,
    ) -> String {
        digest.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn refresh_identity_metadata(&mut self) {
        let interner = self.stable_key_interner();
        self.fact_meta.remove_family(FactFamily::Identity);
        let records = self.identity_records().to_vec();
        for record in &records {
            let (precision, confidence) = Self::identity_status_metadata(record);
            let payload = stable_parts([
                ("kind", Self::identity_kind_label(record.kind).to_string()),
                ("language", record.language.as_str().to_string()),
                ("file", self.path_for(record.file_id)),
                ("span", span_metadata_value(&record.span)),
                ("package_or_module", record.package_or_module.to_string()),
                ("container_path", record.container_path.to_string()),
                ("display_name", record.display_name.to_string()),
                (
                    "signature_digest",
                    Self::format_signature_digest(record.signature_digest),
                ),
                ("multiplicity", record.multiplicity.to_string()),
                (
                    "originating_call_site",
                    record
                        .originating_call_site_id
                        .map(|id| self.fact_stable_key(FactFamily::CallSite, id.0))
                        .unwrap_or_else(none_value),
                ),
                (
                    "originating_call_target",
                    record
                        .originating_call_target_id
                        .map(|id| self.fact_stable_key(FactFamily::CallTarget, id.0))
                        .unwrap_or_else(none_value),
                ),
            ]);
            self.record_fact_meta(
                FactFamily::Identity,
                record.id.0,
                fact_meta_from_stable_key(
                    &interner,
                    "polint.identity",
                    precision,
                    confidence,
                    record.stable_key,
                    payload,
                ),
            );
        }
        self.finish_fact_meta_insertions(&[FactFamily::Identity]);
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    pub(crate) fn identity_records(&self) -> &[IdentityRecord] {
        self.identity_store_inner().records()
    }

    pub(crate) fn replace_identity_facts(
        &mut self,
        output: IdentityProviderOutput,
    ) -> Result<(), AnalysisError> {
        let valid_sites = self
            .calls_store()
            .sites()
            .iter()
            .map(|site| site.id)
            .collect::<BTreeSet<_>>();
        let valid_targets = self
            .call_targets()
            .iter()
            .map(|target| target.id)
            .collect::<BTreeSet<_>>();
        let interner = self.stable_key_interner();
        let store = IdentityStore::from_output(output, &interner, &valid_sites, &valid_targets)?;
        *self.identity_store_mut() = store;
        self.refresh_identity_metadata();
        Ok(())
    }

    /// Injects identity records directly, bypassing store-level reference
    /// validation, so validation diagnostics (the defense-in-depth layer) can be
    /// exercised even for records that the store would have rejected.
    #[cfg(all(test, feature = "lang-go", feature = "lang-typescript"))]
    pub(crate) fn set_identity_records_for_test(&mut self, records: Vec<IdentityRecord>) {
        let mut store = IdentityStore::default();
        store.records = records;
        *self
            .fact_store_mut(IDENTITY_STORE_FAMILY)
            .expect("IdentityStore is installed when AnalysisDb is constructed") = store;
    }

    #[allow(dead_code)]
    pub(crate) fn identity_store(&self) -> Option<&IdentityStore> {
        Some(self.identity_store_inner())
    }

    #[allow(
        dead_code,
        reason = "Provider hot paths pass normalized output directly; tests and compatibility callers still use the normalizing entry point."
    )]
    pub(crate) fn replace_refined_call_facts(
        &mut self,
        output: RefinedCallOutput,
    ) -> Result<(), AnalysisError> {
        self.replace_normalized_refined_call_facts(output.normalized(&self.stable_key_interner()))
    }

    pub(crate) fn replace_normalized_refined_call_facts(
        &mut self,
        output: RefinedCallOutput,
    ) -> Result<(), AnalysisError> {
        let valid_call_sites = self.call_sites().iter().map(|site| site.id).collect();
        let valid_call_targets = self.call_targets().iter().map(|target| target.id).collect();
        let valid_functions = self
            .functions()
            .iter()
            .map(|function| function.id)
            .collect();
        let valid_symbols = self.symbols().iter().map(|symbol| symbol.id).collect();
        let interner = self.stable_key_interner();
        let store = RefinedCallStore::from_normalized_output(
            output,
            &interner,
            &valid_call_sites,
            &valid_call_targets,
            &valid_functions,
            &valid_symbols,
        )?;
        *self.refined_call_store_mut() = store;
        self.refresh_refined_call_metadata();
        Ok(())
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    pub(crate) fn replace_data_flow_facts(
        &mut self,
        output: DataFlowOutput,
    ) -> Result<(), AnalysisError> {
        let interner = self.stable_key_interner();
        let store = DataFlowStore::from_output(output, &interner)?;
        *self.data_flow_store_mut() = store;
        self.refresh_data_flow_metadata(&interner);
        Ok(())
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    pub(crate) fn replace_evidence_facts(
        &mut self,
        output: EvidenceOutput,
    ) -> Result<(), AnalysisError> {
        let interner = self.stable_key_interner();
        let store = EvidenceStore::from_output(output, &interner)?;
        *self.evidence_store_mut() = store;
        self.refresh_evidence_metadata(&interner);
        Ok(())
    }

    pub(crate) fn replace_abstract_domain_facts(&mut self, output: DomainOutput) {
        let interner = self.stable_key_interner();
        let store = DomainStore::from_output(output, &interner);
        self.replace_abstract_domain_store(store);
    }

    fn replace_abstract_domain_store(&mut self, store: DomainStore) {
        *self.domain_store_mut() = store;
        let interner = self.stable_key_interner();
        self.refresh_abstract_domain_metadata(&interner);
    }

    #[allow(
        dead_code,
        reason = "Call metadata helpers retained for AnalysisDb tests until dual accessors are removed."
    )]
    fn populate_call_owner_symbols(&self, output: &mut CallOutput) {
        if output.sites.iter().all(|site| site.owner_symbol.is_some()) {
            return;
        }

        let function_symbols = self
            .functions()
            .iter()
            .filter_map(|function| {
                let symbol = self
                    .symbols()
                    .iter()
                    .find(|symbol| {
                        symbol.file == Some(function.file)
                            && symbol.name == function.name
                            && symbol.primary_span.as_ref().is_some_and(|span| {
                                span == &function.span || Self::span_is_within(span, &function.span)
                            })
                    })
                    .map(|symbol| symbol.id)
                    .or_else(|| {
                        self.definitions()
                            .iter()
                            .find(|definition| {
                                definition.file == Some(function.file)
                                    && definition.name == function.name
                                    && definition.primary_span.as_ref().is_some_and(|span| {
                                        span == &function.span
                                            || Self::span_is_within(span, &function.span)
                                    })
                            })
                            .map(|definition| definition.symbol)
                    });
                symbol.map(|symbol| (function.id, symbol))
            })
            .collect::<BTreeMap<_, _>>();

        for site in &mut output.sites {
            if site.owner_symbol.is_none() {
                site.owner_symbol = function_symbols.get(&site.caller).copied();
            }
        }
    }

    #[allow(
        dead_code,
        reason = "Call metadata helpers retained for AnalysisDb tests until dual accessors are removed."
    )]
    fn span_is_within(inner: &Span, outer: &Span) -> bool {
        inner.file == outer.file
            && inner.start_byte >= outer.start_byte
            && inner.end_byte <= outer.end_byte
    }

    pub(crate) fn call_sites(&self) -> &[CallSiteFact] {
        self.calls_store().sites()
    }

    pub(crate) fn call_targets(&self) -> &[CallTargetFact] {
        self.calls_store().targets()
    }

    pub(crate) fn unresolved_calls(&self) -> &[UnresolvedCallFact] {
        self.calls_store().unresolved()
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    pub(crate) fn call_store(&self) -> Option<&CallStore> {
        Some(self.calls_store())
    }

    pub(crate) fn refined_call_edges(&self) -> &[RefinedCallEdgeFact] {
        self.refined_call_store_inner().edges()
    }

    #[allow(dead_code)]
    pub(crate) fn refined_call_store(&self) -> Option<&RefinedCallStore> {
        Some(self.refined_call_store_inner())
    }

    pub(crate) fn data_flow_nodes(&self) -> &[DataFlowNodeFact] {
        self.data_flow_store_inner().nodes()
    }

    pub(crate) fn data_flow_edges(&self) -> &[DataFlowEdgeFact] {
        self.data_flow_store_inner().edges()
    }

    pub(crate) fn data_flow_models(&self) -> &[DataFlowModelFact] {
        self.data_flow_store_inner().models()
    }

    pub(crate) fn data_flow_budgets(&self) -> &[DataFlowBudgetFact] {
        self.data_flow_store_inner().budgets()
    }

    #[allow(dead_code)]
    pub(crate) fn data_flow_store(&self) -> Option<&DataFlowStore> {
        Some(self.data_flow_store_inner())
    }

    pub(crate) fn evidence_nodes(&self) -> &[EvidenceNodeFact] {
        self.evidence_store_inner().nodes()
    }

    pub(crate) fn evidence_edges(&self) -> &[EvidenceEdgeFact] {
        self.evidence_store_inner().edges()
    }

    pub(crate) fn evidence_bundles(&self) -> &[EvidenceBundleFact] {
        self.evidence_store_inner().bundles()
    }

    pub(crate) fn evidence_paths(&self) -> &[EvidencePathFact] {
        self.evidence_store_inner().paths()
    }

    pub(crate) fn evidence_slices(&self) -> &[EvidenceSliceFact] {
        self.evidence_store_inner().slices()
    }

    pub(crate) fn evidence_unknowns(&self) -> &[EvidenceUnknownFact] {
        self.evidence_store_inner().unknowns()
    }

    pub(crate) fn evidence_omitted_regions(&self) -> &[EvidenceOmittedRegionFact] {
        self.evidence_store_inner().omitted_regions()
    }

    pub(crate) fn evidence_replay_keys(&self) -> &[EvidenceReplayKeyFact] {
        self.evidence_store_inner().replay_keys()
    }

    #[allow(dead_code)]
    pub(crate) fn evidence_store(&self) -> Option<&EvidenceStore> {
        Some(self.evidence_store_inner())
    }

    #[cfg(test)]
    pub(crate) fn abstract_domain_observations(&self) -> &[DomainObservationFact] {
        self.domain_store_inner().observations()
    }

    #[cfg(test)]
    pub(crate) fn abstract_domain_events(&self) -> &[DomainEventFact] {
        self.domain_store_inner().events()
    }

    #[allow(dead_code)]
    pub(crate) fn abstract_domain_store(&self) -> Option<&DomainStore> {
        Some(self.domain_store_inner())
    }

    pub(crate) fn replace_summary_facts(&mut self, output: SummaryOutput) {
        self.replace_summary_facts_without_metadata(output);
        self.refresh_summary_metadata();
    }

    pub(crate) fn replace_summary_facts_without_metadata(&mut self, output: SummaryOutput) {
        let interner = self.stable_key_interner();
        let store = SummaryStore::from_output(output, &interner)
            .expect("summary output should produce a valid store");
        *self.summary_store_mut() = store;
    }

    #[allow(
        dead_code,
        reason = "Extension fact replacement is wired into the kernel provider in the next plan."
    )]
    pub(crate) fn replace_extension_facts(&mut self, output: ExtensionOutput) {
        let output = output.normalized(&self.stable_key_interner());
        let store = self.extension_store_mut();
        store.activations = output.activations;
        store.accepted = output.accepted;
        store.rejected = output.rejected;
        self.refresh_extension_metadata();
    }

    pub(crate) fn summary_facts(&self) -> &[SummaryFact] {
        self.summary_store_inner().all_summaries()
    }
    pub(crate) fn summary_events(&self) -> &[SummaryEventFact] {
        self.summary_store_inner().all_events()
    }

    #[allow(dead_code)]
    pub(crate) fn summary_store(&self) -> Option<&SummaryStore> {
        Some(self.summary_store_inner())
    }

    pub(crate) fn extension_facts(&self) -> &[AcceptedExtensionFact] {
        &self.extension_store_inner().accepted
    }

    pub(crate) fn extension_activations(&self) -> &[ExtensionActivationRow] {
        &self.extension_store_inner().activations
    }

    #[allow(
        dead_code,
        reason = "Rejected extension audit rows are surfaced by the extension provider/debug wiring in the next plan."
    )]
    pub(crate) fn rejected_extension_facts(&self) -> &[RejectedExtensionFact] {
        &self.extension_store_inner().rejected
    }

    pub(crate) fn replace_adaptation_model_facts(
        &mut self,
        accepted: Vec<AcceptedModelFact>,
        rejected: Vec<RejectedModelFact>,
    ) {
        let store = self.adaptation_store_mut();
        store.accepted = accepted;
        store.rejected = rejected;
        self.refresh_adaptation_model_metadata();
    }

    pub(crate) fn adaptation_model_facts(&self) -> &[AcceptedModelFact] {
        &self.adaptation_store_inner().accepted
    }

    #[allow(
        dead_code,
        reason = "Rejected adaptation model audit rows are surfaced by eval fixture observation wiring."
    )]
    pub(crate) fn rejected_adaptation_model_facts(&self) -> &[RejectedModelFact] {
        &self.adaptation_store_inner().rejected
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    pub(crate) fn replace_entrypoint_facts(
        &mut self,
        output: EntrypointOutput,
    ) -> Result<(), AnalysisError> {
        let interner_handle = self.stable_key_interner();
        let interner = &interner_handle;
        let store = EntrypointStore::from_output(output, interner)?;
        *self.entrypoint_store_mut() = store;
        self.refresh_entrypoint_metadata(interner);
        Ok(())
    }

    pub(crate) fn entrypoint_facts(&self) -> &[EntrypointFact] {
        self.entrypoint_store_inner().entrypoints()
    }

    #[allow(
        dead_code,
        reason = "Reachability fact replacement is wired into the kernel provider in the next  task (provider/kernel splice)."
    )]
    pub(crate) fn replace_reachability_facts(
        &mut self,
        output: ReachabilityProviderOutput,
    ) -> Result<(), AnalysisError> {
        let interner = self.stable_key_interner();
        let valid_function_ids = self
            .functions()
            .iter()
            .map(|row| row.id)
            .collect::<BTreeSet<_>>();
        let valid_entrypoint_ids = self
            .entrypoint_facts()
            .iter()
            .map(|row| row.id)
            .collect::<BTreeSet<_>>();
        let store = ReachabilityStore::from_output(
            output,
            &interner,
            &valid_function_ids,
            &valid_entrypoint_ids,
        )?;
        *self.reachability_store_mut() = store;
        self.refresh_reachability_metadata();
        Ok(())
    }

    #[allow(
        dead_code,
        reason = "Reachability roots are consumed by validation, debug, and the kernel provider wiring in ."
    )]
    pub(crate) fn reachability_roots(&self) -> &[ReachabilityRootFact] {
        self.reachability_store_inner().roots()
    }

    #[allow(
        dead_code,
        reason = "Reachability marks are populated by the marking traversal in  and read by debug/eval."
    )]
    /// Stores the normalized semantic-graph nodes/edges/constraints (GRAPH-01),
    /// mirroring [`Self::replace_reachability_facts`]. Construction runs through
    /// [`SemanticGraphStore::from_output`], which normalizes (stable-key sort + dense
    /// ID assignment) and referentially validates every edge endpoint and constraint
    /// node reference — a dangling reference returns [`AnalysisError::InvalidFact`] so
    /// the db is never left holding a malformed graph.
    #[allow(
        dead_code,
        reason = "Provider hot paths pass normalized output directly; tests and compatibility callers still use the normalizing entry point."
    )]
    pub(crate) fn replace_semantic_graph_facts(
        &mut self,
        output: SemanticGraphOutput,
    ) -> Result<(), AnalysisError> {
        let interner = self.stable_key_interner();
        self.replace_normalized_semantic_graph_facts(output.normalized(&interner))
    }

    pub(crate) fn replace_normalized_semantic_graph_facts(
        &mut self,
        output: SemanticGraphOutput,
    ) -> Result<(), AnalysisError> {
        let interner = self.stable_key_interner();
        *self.semantic_graph_store_mut() =
            SemanticGraphStore::from_normalized_output(output, &interner)?;
        self.refresh_semantic_graph_metadata();
        Ok(())
    }

    pub(crate) fn semantic_nodes(&self) -> &[SemanticNodeFact] {
        self.semantic_graph_store_inner().nodes()
    }

    pub(crate) fn semantic_edges(&self) -> &[SemanticEdgeFact] {
        self.semantic_graph_store_inner().edges()
    }

    pub(crate) fn semantic_constraints(&self) -> &[ConstraintFact] {
        self.semantic_graph_store_inner().constraints()
    }

    /// Stores the private TS object/property/prototype/receiver rows used by the
    /// current semantic-graph lowering. Construction runs through
    /// [`TsObjectModelStore::try_from_output`], which preserves deterministic
    /// normalization and rejects duplicate stable keys before stale rows are replaced.
    pub(crate) fn replace_ts_object_model_facts(
        &mut self,
        output: TsObjectModelOutput,
    ) -> Result<(), AnalysisError> {
        let interner = self.stable_key_interner();
        let store = TsObjectModelStore::try_from_output(output, &interner)
            .map_err(crate::analysis::error_convert::from_ts)?;
        *self.ts_object_model_store_mut() = store;
        Ok(())
    }

    pub(crate) fn ts_object_allocations(&self) -> &[TsObjectAllocationFact] {
        self.ts_object_model_store_inner().allocations()
    }

    pub(crate) fn ts_property_writes(&self) -> &[TsPropertyWriteFact] {
        self.ts_object_model_store_inner().property_writes()
    }

    pub(crate) fn ts_property_reads(&self) -> &[TsPropertyReadFact] {
        self.ts_object_model_store_inner().property_reads()
    }

    pub(crate) fn ts_receiver_bindings(&self) -> &[TsReceiverBindingFact] {
        self.ts_object_model_store_inner().receiver_bindings()
    }

    pub(crate) fn ts_prototype_links(&self) -> &[TsPrototypeLinkFact] {
        self.ts_object_model_store_inner().prototype_links()
    }

    #[allow(
        dead_code,
        reason = "object-model store queries are exercised by storage/regression tests"
    )]
    pub(crate) fn ts_object_model_store(&self) -> Option<&TsObjectModelStore> {
        Some(self.ts_object_model_store_inner())
    }

    /// Stores the normalized solver-derived edges (GRAPH-03/GRAPH-04), mirroring
    /// [`Self::replace_semantic_graph_facts`]. Construction runs through
    /// [`SolverStore::from_output`], which normalizes (stable-key sort + dense ID
    /// assignment) and referentially validates duplicate stable keys + the precision
    /// ceiling (D-06) — a malformed row returns [`AnalysisError::InvalidFact`] so the
    /// db is never left holding a malformed solver output.
    pub(crate) fn replace_solver_facts(
        &mut self,
        output: SolverOutput,
    ) -> Result<(), AnalysisError> {
        let interner = self.stable_key_interner();
        *self.solver_store_mut() = SolverStore::from_output(output, &interner)?;
        self.refresh_solver_metadata();
        Ok(())
    }

    /// The stored solver-derived edges. Consumed by the provider tests today and by
    /// the GRAPH-05 refined_calls rework (which projects over solver output);
    /// no production read exists yet, so the accessor is dead-code in a non-test build
    /// until that consumer lands (the facts are stored unconditionally so the
    /// determinism gate observes them).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn solver_derived_edges(&self) -> &[DerivedEdgeFact] {
        self.solver_store_inner().derived_edges()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn solver_budget_status(&self) -> BudgetStatus {
        self.solver_store_inner().budget_status()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn solver_budget_reasons(&self) -> &BTreeSet<String> {
        self.solver_store_inner().budget_reasons()
    }

    /// Test helper: install Go semantic facts into the facade DB (production writes go through
    /// the `polint-go` provider → fact-store path).
    #[cfg(test)]
    pub(crate) fn replace_go_semantic_facts(
        &mut self,
        output: GoSemanticFactsOutput,
    ) -> Result<GoSemanticStoreReport, AnalysisError> {
        let interner = self.stable_key_interner();
        let store = GoSemanticStore::from_output(output, &interner)
            .map_err(crate::analysis::error_convert::from_go)?;
        let report = store.report();
        *self.go_semantic_store_mut() = store;
        Ok(report)
    }

    pub(crate) fn go_semantic_packages(&self) -> &[GoSemanticPackageFact] {
        &self.go_semantic_store().output().packages
    }

    pub(crate) fn go_semantic_functions(&self) -> &[GoSemanticFunctionFact] {
        &self.go_semantic_store().output().functions
    }

    pub(crate) fn go_semantic_callsites(&self) -> &[GoSemanticCallsiteFact] {
        &self.go_semantic_store().output().callsites
    }

    pub(crate) fn ts_type_callables(&self) -> &[TsTypeCallableFact] {
        &self.ts_types_store().output().callables
    }

    pub(crate) fn ts_type_callsites(&self) -> &[TsTypeCallsiteFact] {
        &self.ts_types_store().output().callsites
    }

    pub(crate) fn ts_type_callees(&self) -> &[TsTypeCalleeFact] {
        &self.ts_types_store().output().callees
    }

    pub(crate) fn ts_type_receivers(&self) -> &[TsTypeReceiverFact] {
        &self.ts_types_store().output().receivers
    }

    pub(crate) fn ts_type_file_densities(&self) -> &[TsTypeFileDensityFact] {
        &self.ts_types_store().output().file_densities
    }

    #[allow(
        dead_code,
        reason = "Method-set facts are stored privately for receiver/RTA expansion."
    )]
    pub(crate) fn go_semantic_method_sets(&self) -> &[GoSemanticMethodSetFact] {
        &self.go_semantic_store().output().method_sets
    }

    #[allow(
        dead_code,
        reason = "Address-taken facts are stored privately for the Plan 2 go_rta dispatch-candidate set (GO-05)."
    )]
    pub(crate) fn go_semantic_address_taken(&self) -> &[GoSemanticAddressTakenFact] {
        &self.go_semantic_store().output().address_taken
    }

    #[allow(
        dead_code,
        reason = "Instantiated-type facts are stored privately for the Plan 2 go_rta rapid-type filter (GO-05)."
    )]
    pub(crate) fn go_semantic_instantiated_types(&self) -> &[GoSemanticInstantiatedTypeFact] {
        &self.go_semantic_store().output().instantiated_types
    }

    #[allow(
        dead_code,
        reason = "Dynamic-dispatch detail is stored privately for the Plan 2 go_rta method-set matching (GO-05)."
    )]
    pub(crate) fn go_semantic_dynamic_dispatch(&self) -> &[GoSemanticDynamicDispatchFact] {
        &self.go_semantic_store().output().dynamic_dispatch
    }

    #[cfg(all(test, feature = "lang-go", feature = "lang-typescript"))]
    pub(crate) fn go_semantic_rta_edges(
        &self,
    ) -> &[crate::go::semantic::facts::GoSemanticRtaEdgeFact] {
        &self.go_semantic_store().output().rta_edges
    }

    #[allow(
        dead_code,
        reason = "Package-load errors are stored privately for capability diagnostics once the provider is kernel-wired."
    )]
    pub(crate) fn go_semantic_package_errors(&self) -> &[GoSemanticPackageErrorFact] {
        &self.go_semantic_store().output().package_errors
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    pub(crate) fn trust_boundary_facts(&self) -> &[TrustBoundaryFact] {
        self.entrypoint_store_inner().trust_boundaries()
    }

    pub(crate) fn dispatch_edge_facts(&self) -> &[FrameworkDispatchEdgeFact] {
        self.entrypoint_store_inner().dispatch_edges()
    }

    pub(crate) fn unresolved_framework_facts(&self) -> &[UnresolvedFrameworkFact] {
        self.entrypoint_store_inner().unresolved()
    }

    #[allow(dead_code)]
    pub(crate) fn entrypoint_store(&self) -> Option<&EntrypointStore> {
        Some(self.entrypoint_store_inner())
    }

    #[allow(
        dead_code,
        reason = "Compatibility callers can still pass unnormalized aggregate output; providers use the normalized fast path."
    )]
    pub(crate) fn replace_type_value_alias_facts(&mut self, output: TypeValueAliasOutput) {
        self.replace_normalized_type_value_alias_facts(
            output.normalized(&self.stable_key_interner()),
        );
    }

    pub(crate) fn replace_normalized_type_value_alias_facts(
        &mut self,
        output: TypeValueAliasOutput,
    ) {
        *self.type_store_mut() = TypeStore::from_normalized_output(output.types);
        *self.value_store_mut() = ValueStore::from_normalized_output(output.values);
        *self.access_path_store_mut() =
            AccessPathStore::from_normalized_output(output.access_paths);
        *self.points_to_store_mut() = PointsToStore::from_normalized_output(output.points_to);
        *self.alias_store_mut() = AliasStore::from_normalized_output(output.aliases);
        self.refresh_type_value_alias_metadata();
    }

    pub(crate) fn type_facts(&self) -> &[TypeFact] {
        self.type_store_inner().types()
    }

    #[allow(dead_code)]
    pub(crate) fn narrowed_type_facts(&self) -> &[NarrowedTypeFact] {
        self.type_store_inner().narrowed()
    }

    pub(crate) fn value_facts(&self) -> &[ValueFact] {
        self.value_store_inner().values()
    }

    #[allow(dead_code)]
    pub(crate) fn allocation_tokens(&self) -> &[AllocationTokenFact] {
        self.value_store_inner().allocations()
    }

    pub(crate) fn access_path_facts(&self) -> &[AccessPathFact] {
        self.access_path_store_inner().access_paths()
    }

    #[allow(dead_code)]
    pub(crate) fn points_to_constraints(&self) -> &[PointsToConstraintFact] {
        self.points_to_store_inner().constraints()
    }

    pub(crate) fn points_to_sets(&self) -> &[PointsToSetFact] {
        self.points_to_store_inner().sets()
    }

    pub(crate) fn alias_answers(&self) -> &[AliasAnswerFact] {
        self.alias_store_inner().answers()
    }

    #[allow(dead_code)]
    pub(crate) fn call_sites_by_caller(&self, caller: FunctionId) -> Vec<&CallSiteFact> {
        self.calls_store().sites_by_caller(caller)
    }

    #[allow(dead_code)]
    pub(crate) fn call_targets_by_site(&self, site: CallSiteId) -> Vec<&CallTargetFact> {
        self.calls_store().targets_by_site(site)
    }

    #[allow(dead_code)]
    pub(crate) fn outgoing_calls_by_function(&self, caller: FunctionId) -> Vec<&CallTargetFact> {
        self.calls_store().outgoing_by_function(caller)
    }

    #[allow(dead_code)]
    pub(crate) fn outgoing_calls_by_symbol(&self, caller: SymbolId) -> Vec<&CallTargetFact> {
        self.calls_store().outgoing_by_symbol(caller)
    }

    #[allow(dead_code)]
    pub(crate) fn incoming_calls_by_symbol(&self, target: SymbolId) -> Vec<&CallTargetFact> {
        self.calls_store().incoming_by_symbol(target)
    }

    #[allow(dead_code)]
    pub(crate) fn incoming_calls_by_function(&self, target: FunctionId) -> Vec<&CallTargetFact> {
        self.calls_store().incoming_by_function(target)
    }

    #[allow(dead_code)]
    pub(crate) fn unresolved_calls_by_reason(
        &self,
        reason: UnresolvedCallReason,
    ) -> Vec<&UnresolvedCallFact> {
        self.calls_store().unresolved_by_reason(reason)
    }

    #[allow(dead_code)]
    pub(crate) fn unresolved_calls_by_status(
        &self,
        status: CallTargetStatus,
    ) -> Vec<&UnresolvedCallFact> {
        self.calls_store().unresolved_by_status(status)
    }

    #[allow(
        dead_code,
        reason = "Call metadata helpers retained for AnalysisDb tests until dual accessors are removed."
    )]
    fn refresh_call_metadata(&mut self, interner: &crate::core::StableKeyInterner) {
        self.fact_meta.remove_family(FactFamily::CallSite);
        self.fact_meta.remove_family(FactFamily::CallTarget);
        self.fact_meta.remove_family(FactFamily::UnresolvedCall);

        let call_sites = self.call_sites().to_vec();
        let call_targets = self.call_targets().to_vec();
        let unresolved_calls = self.unresolved_calls().to_vec();

        for fact in &call_sites {
            let metadata = self.call_site_metadata(interner, fact);
            self.record_fact_meta(FactFamily::CallSite, fact.id.0, metadata);
        }

        for fact in &call_targets {
            let metadata = self.call_target_metadata(interner, fact);
            self.record_fact_meta(FactFamily::CallTarget, fact.id.0, metadata);
        }

        for (index, fact) in unresolved_calls.iter().enumerate() {
            let metadata = self.unresolved_call_metadata(interner, fact);
            self.record_fact_meta(FactFamily::UnresolvedCall, index as u64, metadata);
        }

        self.finish_fact_meta_insertions(&[
            FactFamily::CallSite,
            FactFamily::CallTarget,
            FactFamily::UnresolvedCall,
        ]);
    }

    fn refresh_refined_call_metadata(&mut self) {
        self.fact_meta.remove_family(FactFamily::RefinedCallEdge);

        let edges = self.refined_call_edges().to_vec();
        for fact in &edges {
            let metadata = self.refined_call_edge_metadata(fact);
            self.record_fact_meta(FactFamily::RefinedCallEdge, fact.id.0, metadata);
        }
        self.finish_fact_meta_insertions(&[FactFamily::RefinedCallEdge]);
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    fn refresh_data_flow_metadata(&mut self, interner: &crate::core::StableKeyInterner) {
        self.fact_meta.remove_family(FactFamily::DataFlowNode);
        self.fact_meta.remove_family(FactFamily::DataFlowEdge);
        self.fact_meta.remove_family(FactFamily::DataFlowModel);
        self.fact_meta.remove_family(FactFamily::DataFlowBudget);

        let nodes = self.data_flow_nodes().to_vec();
        let edges = self.data_flow_edges().to_vec();
        let models = self.data_flow_models().to_vec();
        let budgets = self.data_flow_budgets().to_vec();

        for fact in &nodes {
            let metadata = self.data_flow_node_metadata(interner, fact);
            self.record_fact_meta(FactFamily::DataFlowNode, fact.id.0, metadata);
        }
        for fact in &edges {
            let metadata = self.data_flow_edge_metadata(interner, fact);
            self.record_fact_meta(FactFamily::DataFlowEdge, fact.id.0, metadata);
        }
        for fact in &models {
            let metadata = self.data_flow_model_metadata(interner, fact);
            self.record_fact_meta(FactFamily::DataFlowModel, fact.id.0, metadata);
        }
        for fact in &budgets {
            let metadata = self.data_flow_budget_metadata(interner, fact);
            self.record_fact_meta(FactFamily::DataFlowBudget, fact.id.0, metadata);
        }

        self.finish_fact_meta_insertions(&[
            FactFamily::DataFlowNode,
            FactFamily::DataFlowEdge,
            FactFamily::DataFlowModel,
            FactFamily::DataFlowBudget,
        ]);
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    fn refresh_evidence_metadata(&mut self, interner: &crate::core::StableKeyInterner) {
        for family in [
            FactFamily::EvidenceNode,
            FactFamily::EvidenceEdge,
            FactFamily::EvidenceBundle,
            FactFamily::EvidencePath,
            FactFamily::EvidenceSlice,
            FactFamily::EvidenceUnknown,
            FactFamily::EvidenceOmittedRegion,
            FactFamily::EvidenceReplayKey,
        ] {
            self.fact_meta.remove_family(family);
        }

        let nodes = self.evidence_nodes().to_vec();
        let edges = self.evidence_edges().to_vec();
        let bundles = self.evidence_bundles().to_vec();
        let paths = self.evidence_paths().to_vec();
        let slices = self.evidence_slices().to_vec();
        let unknowns = self.evidence_unknowns().to_vec();
        let omitted = self.evidence_omitted_regions().to_vec();
        let replay_keys = self.evidence_replay_keys().to_vec();

        for fact in &nodes {
            let metadata = self.evidence_node_metadata(interner, fact);
            self.record_fact_meta(FactFamily::EvidenceNode, fact.id.0, metadata);
        }
        for fact in &edges {
            let metadata = self.evidence_edge_metadata(interner, fact);
            self.record_fact_meta(FactFamily::EvidenceEdge, fact.id.0, metadata);
        }
        for fact in &bundles {
            let metadata = self.evidence_bundle_metadata(interner, fact);
            self.record_fact_meta(FactFamily::EvidenceBundle, fact.id.0, metadata);
        }
        for fact in &paths {
            let metadata = self.evidence_path_metadata(interner, fact);
            self.record_fact_meta(FactFamily::EvidencePath, fact.id.0, metadata);
        }
        for fact in &slices {
            let metadata = self.evidence_slice_metadata(interner, fact);
            self.record_fact_meta(FactFamily::EvidenceSlice, fact.id.0, metadata);
        }
        for (index, fact) in unknowns.iter().enumerate() {
            let metadata = self.evidence_unknown_metadata(interner, fact);
            self.record_fact_meta(FactFamily::EvidenceUnknown, index as u64, metadata);
        }
        for fact in &omitted {
            let metadata = self.evidence_omitted_region_metadata(interner, fact);
            self.record_fact_meta(FactFamily::EvidenceOmittedRegion, fact.id.0, metadata);
        }
        for (index, fact) in replay_keys.iter().enumerate() {
            let metadata = self.evidence_replay_key_metadata(interner, fact);
            self.record_fact_meta(FactFamily::EvidenceReplayKey, index as u64, metadata);
        }

        self.finish_fact_meta_insertions(&[
            FactFamily::EvidenceNode,
            FactFamily::EvidenceEdge,
            FactFamily::EvidenceBundle,
            FactFamily::EvidencePath,
            FactFamily::EvidenceSlice,
            FactFamily::EvidenceUnknown,
            FactFamily::EvidenceOmittedRegion,
            FactFamily::EvidenceReplayKey,
        ]);
    }
    fn refresh_abstract_domain_metadata(&mut self, interner: &crate::core::StableKeyInterner) {
        self.fact_meta.remove_family(FactFamily::DomainObservation);
        self.fact_meta.remove_family(FactFamily::DomainEvent);

        let observations = self.domain_store_inner().observations().to_vec();
        let events = self.domain_store_inner().events().to_vec();
        for fact in &observations {
            let metadata = self.domain_observation_metadata(interner, fact);
            self.record_fact_meta(FactFamily::DomainObservation, fact.id.0, metadata);
        }
        for fact in &events {
            let metadata = self.domain_event_metadata(interner, fact);
            self.record_fact_meta(FactFamily::DomainEvent, fact.id.0, metadata);
        }
        self.finish_fact_meta_insertions(&[FactFamily::DomainObservation, FactFamily::DomainEvent]);
    }

    fn refresh_summary_metadata(&mut self) {
        self.fact_meta.remove_family(FactFamily::SummaryControl);
        self.fact_meta.remove_family(FactFamily::SummaryCall);
        self.fact_meta.remove_family(FactFamily::SummaryMemory);
        self.fact_meta.remove_family(FactFamily::SummaryTito);
        self.fact_meta.remove_family(FactFamily::SummaryEvent);

        let summaries = self.summary_facts().to_vec();
        let events = self.summary_events().to_vec();
        for fact in &summaries {
            let family = summary_domain_to_fact_family(fact.domain);
            let metadata = self.summary_fact_metadata(fact);
            self.record_fact_meta(family, fact.id.0, metadata);
        }
        for fact in &events {
            let metadata = self.summary_event_metadata(fact);
            self.record_fact_meta(FactFamily::SummaryEvent, fact.id.0, metadata);
        }
        self.finish_fact_meta_insertions(&[
            FactFamily::SummaryControl,
            FactFamily::SummaryCall,
            FactFamily::SummaryMemory,
            FactFamily::SummaryTito,
            FactFamily::SummaryEvent,
        ]);
    }

    #[allow(
        dead_code,
        reason = "Extension metadata refresh is reached through extension provider wiring in the next plan."
    )]
    fn refresh_extension_metadata(&mut self) {
        self.fact_meta.remove_family(FactFamily::ExtensionFact);
        let interner = self.stable_key_interner();
        let facts = self.extension_facts().to_vec();
        for (index, fact) in facts.iter().enumerate() {
            let metadata = extension_fact_metadata(&interner, fact);
            self.record_fact_meta(FactFamily::ExtensionFact, index as u64, metadata);
        }
        self.finish_fact_meta_insertions(&[FactFamily::ExtensionFact]);
    }

    fn refresh_adaptation_model_metadata(&mut self) {
        self.fact_meta.remove_family(FactFamily::AdaptationModel);
        let interner = self.stable_key_interner();
        let facts = self.adaptation_model_facts().to_vec();
        for (index, fact) in facts.iter().enumerate() {
            let metadata = adaptation_model_fact_metadata(&interner, fact);
            self.record_fact_meta(FactFamily::AdaptationModel, index as u64, metadata);
        }
        self.finish_fact_meta_insertions(&[FactFamily::AdaptationModel]);
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    fn refresh_entrypoint_metadata(&mut self, interner: &crate::core::StableKeyInterner) {
        self.fact_meta.remove_family(FactFamily::Entrypoint);
        self.fact_meta.remove_family(FactFamily::TrustBoundary);
        self.fact_meta.remove_family(FactFamily::DispatchEdge);
        self.fact_meta
            .remove_family(FactFamily::UnresolvedFramework);

        let entrypoints = self.entrypoint_facts().to_vec();
        let trust_boundaries = self.trust_boundary_facts().to_vec();
        let dispatch_edges = self.dispatch_edge_facts().to_vec();
        let unresolved = self.unresolved_framework_facts().to_vec();

        for fact in &entrypoints {
            let metadata = self.entrypoint_fact_metadata(interner, fact);
            self.record_fact_meta(FactFamily::Entrypoint, fact.id.0, metadata);
        }
        for fact in &trust_boundaries {
            let metadata = self.trust_boundary_metadata(interner, fact);
            self.record_fact_meta(FactFamily::TrustBoundary, fact.id.0, metadata);
        }
        for fact in &dispatch_edges {
            let metadata = self.dispatch_edge_metadata(interner, fact);
            self.record_fact_meta(FactFamily::DispatchEdge, fact.id.0, metadata);
        }
        for fact in &unresolved {
            let metadata = self.unresolved_framework_metadata(interner, fact);
            self.record_fact_meta(FactFamily::UnresolvedFramework, fact.id.0, metadata);
        }
        self.finish_fact_meta_insertions(&[
            FactFamily::Entrypoint,
            FactFamily::TrustBoundary,
            FactFamily::DispatchEdge,
            FactFamily::UnresolvedFramework,
        ]);
    }

    fn refresh_type_value_alias_metadata(&mut self) {
        for family in [
            FactFamily::Type,
            FactFamily::NarrowedType,
            FactFamily::Value,
            FactFamily::AllocationToken,
            FactFamily::AccessPath,
            FactFamily::PointsToConstraint,
            FactFamily::PointsToSet,
            FactFamily::AliasAnswer,
        ] {
            self.fact_meta.remove_family(family);
        }

        let types = self.type_facts().to_vec();
        let narrowed = self.narrowed_type_facts().to_vec();
        let values = self.value_facts().to_vec();
        let allocations = self.allocation_tokens().to_vec();
        let access_paths = self.access_path_facts().to_vec();
        let constraints = self.points_to_constraints().to_vec();
        let sets = self.points_to_sets().to_vec();
        let aliases = self.alias_answers().to_vec();

        for fact in &types {
            let metadata = self.type_fact_metadata(fact);
            self.record_fact_meta(FactFamily::Type, fact.id.0, metadata);
        }
        for fact in &narrowed {
            let metadata = self.narrowed_type_metadata(fact);
            self.record_fact_meta(FactFamily::NarrowedType, fact.id.0, metadata);
        }
        for fact in &values {
            let metadata = self.value_fact_metadata(fact);
            self.record_fact_meta(FactFamily::Value, fact.id.0, metadata);
        }
        for fact in &allocations {
            let metadata = self.allocation_token_metadata(fact);
            self.record_fact_meta(FactFamily::AllocationToken, fact.id.0, metadata);
        }
        for fact in &access_paths {
            let metadata = self.access_path_metadata(fact);
            self.record_fact_meta(FactFamily::AccessPath, fact.id.0, metadata);
        }
        for fact in &constraints {
            let metadata = self.points_to_constraint_metadata(fact);
            self.record_fact_meta(FactFamily::PointsToConstraint, fact.id.0, metadata);
        }
        for fact in &sets {
            let metadata = self.points_to_set_metadata(fact);
            self.record_fact_meta(FactFamily::PointsToSet, fact.id.0, metadata);
        }
        for fact in &aliases {
            let metadata = self.alias_answer_metadata(fact);
            self.record_fact_meta(FactFamily::AliasAnswer, fact.id.0, metadata);
        }

        self.finish_fact_meta_insertions(&[
            FactFamily::Type,
            FactFamily::NarrowedType,
            FactFamily::Value,
            FactFamily::AllocationToken,
            FactFamily::AccessPath,
            FactFamily::PointsToConstraint,
            FactFamily::PointsToSet,
            FactFamily::AliasAnswer,
        ]);
    }

    fn type_fact_metadata(&self, fact: &TypeFact) -> FactMeta {
        let (precision, confidence) =
            type_metadata_precision(fact.status, fact.precision, Some(fact.confidence));
        fact_meta_from_stable_key(
            &self.stable_keys,
            TYPE_VALUE_ALIAS_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("status", format!("{:?}", fact.status)),
                ("precision", format!("{:?}", fact.precision)),
                ("phase", format!("{:?}", fact.phase)),
                ("language", language_label(fact.language).to_string()),
                ("file_key", self.option_source_file_key(fact.file)),
                (
                    "place_key",
                    fact.place
                        .map(|place| self.fact_stable_key(FactFamily::Place, place.0))
                        .unwrap_or_else(none_value),
                ),
                ("subject", format!("{:?}", fact.subject)),
                ("shape", format!("{:?}", fact.shape)),
                ("provenance", format!("{:?}", fact.provenance)),
            ]),
        )
    }

    fn narrowed_type_metadata(&self, fact: &NarrowedTypeFact) -> FactMeta {
        let (precision, confidence) = type_metadata_precision(fact.status, fact.precision, None);
        fact_meta_from_stable_key(
            &self.stable_keys,
            TYPE_VALUE_ALIAS_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("status", format!("{:?}", fact.status)),
                ("precision", format!("{:?}", fact.precision)),
                ("language", language_label(fact.language).to_string()),
                (
                    "place_key",
                    self.fact_stable_key(FactFamily::Place, fact.place.0),
                ),
                (
                    "operation_key",
                    fact.operation
                        .map(|operation| {
                            self.fact_stable_key(FactFamily::MirOperation, operation.0)
                        })
                        .unwrap_or_else(none_value),
                ),
                ("evidence", fact.evidence.clone()),
            ]),
        )
    }

    fn value_fact_metadata(&self, fact: &ValueFact) -> FactMeta {
        let (precision, confidence) = value_metadata_precision(fact.status, fact.precision);
        fact_meta_from_stable_key(
            &self.stable_keys,
            TYPE_VALUE_ALIAS_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("status", format!("{:?}", fact.status)),
                ("precision", format!("{:?}", fact.precision)),
                ("language", language_label(fact.language).to_string()),
                ("subject", format!("{:?}", fact.subject)),
                ("kind", format!("{:?}", fact.kind)),
                ("provenance", format!("{:?}", fact.provenance)),
            ]),
        )
    }

    fn allocation_token_metadata(&self, fact: &AllocationTokenFact) -> FactMeta {
        fact_meta_from_stable_key(
            &self.stable_keys,
            TYPE_VALUE_ALIAS_PROVIDER_ID,
            FactPrecision::SetupAware,
            FactConfidence::Medium,
            fact.stable_key,
            stable_parts([
                ("kind", format!("{:?}", fact.kind)),
                ("language", language_label(fact.language).to_string()),
                (
                    "source_place",
                    fact.source_place
                        .map(|place| self.fact_stable_key(FactFamily::Place, place.0))
                        .unwrap_or_else(none_value),
                ),
                ("provenance", format!("{:?}", fact.provenance)),
            ]),
        )
    }

    fn access_path_metadata(&self, fact: &AccessPathFact) -> FactMeta {
        let precision = match fact.status {
            crate::analysis::access_paths::facts::AccessPathStatus::Resolved => {
                FactPrecision::SetupAware
            }
            crate::analysis::access_paths::facts::AccessPathStatus::Partial => {
                FactPrecision::Ambiguous
            }
            crate::analysis::access_paths::facts::AccessPathStatus::Unknown => {
                FactPrecision::Unresolved
            }
            crate::analysis::access_paths::facts::AccessPathStatus::Unsupported => {
                FactPrecision::Unsupported
            }
            crate::analysis::access_paths::facts::AccessPathStatus::BudgetExceeded => {
                FactPrecision::Heuristic
            }
        };
        fact_meta_from_stable_key(
            &self.stable_keys,
            TYPE_VALUE_ALIAS_PROVIDER_ID,
            precision,
            FactConfidence::Medium,
            fact.stable_key,
            stable_parts([
                ("status", format!("{:?}", fact.status)),
                ("language", language_label(fact.language).to_string()),
                (
                    "base_key",
                    self.fact_stable_key(FactFamily::Place, fact.base.0),
                ),
                ("projection_count", fact.projections.len().to_string()),
            ]),
        )
    }

    fn points_to_constraint_metadata(&self, fact: &PointsToConstraintFact) -> FactMeta {
        let (precision, confidence) = points_to_metadata_precision(fact.status, fact.precision);
        fact_meta_from_stable_key(
            &self.stable_keys,
            TYPE_VALUE_ALIAS_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("status", format!("{:?}", fact.status)),
                ("precision", format!("{:?}", fact.precision)),
                ("kind", format!("{:?}", fact.kind)),
            ]),
        )
    }

    fn points_to_set_metadata(&self, fact: &PointsToSetFact) -> FactMeta {
        let (precision, confidence) = points_to_metadata_precision(fact.status, fact.precision);
        fact_meta_from_stable_key(
            &self.stable_keys,
            TYPE_VALUE_ALIAS_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("status", format!("{:?}", fact.status)),
                ("precision", format!("{:?}", fact.precision)),
                ("budget", format!("{:?}", fact.budget)),
                ("variable", format!("{:?}", fact.variable)),
                ("objects", format!("{:?}", fact.objects)),
            ]),
        )
    }

    fn alias_answer_metadata(&self, fact: &AliasAnswerFact) -> FactMeta {
        let (precision, confidence) = alias_metadata_precision(fact.status, fact.precision);
        fact_meta_from_stable_key(
            &self.stable_keys,
            TYPE_VALUE_ALIAS_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("status", format!("{:?}", fact.status)),
                ("precision", format!("{:?}", fact.precision)),
                ("reason", format!("{:?}", fact.reason)),
                ("left", format!("{:?}", fact.left)),
                ("right", format!("{:?}", fact.right)),
                ("evidence", fact.evidence.join("\n")),
            ]),
        )
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    fn entrypoint_fact_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &EntrypointFact,
    ) -> FactMeta {
        let (precision, confidence) = entrypoint_precision_metadata(fact.status, fact.precision);
        fact_meta_from_stable_key(
            interner,
            ENTRYPOINTS_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("status", format!("{:?}", fact.status)),
                ("precision", format!("{:?}", fact.precision)),
                ("kind", format!("{:?}", fact.kind)),
                ("language", language_label(fact.language).to_string()),
                ("framework", fact.framework_id.clone()),
                ("file_key", self.source_file_key(fact.registration_file)),
                (
                    "function_key",
                    self.function_key(
                        interner,
                        fact.target_function,
                        &fact.framework_id,
                        &fact.registration_span,
                    ),
                ),
                ("provenance", format!("{:?}", fact.provenance)),
            ]),
        )
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    fn trust_boundary_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &TrustBoundaryFact,
    ) -> FactMeta {
        let (precision, confidence) =
            entrypoint_precision_metadata(EntrypointStatus::Resolved, fact.precision);
        fact_meta_from_stable_key(
            interner,
            ENTRYPOINTS_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("source_kind", format!("{:?}", fact.source_kind)),
                (
                    "entrypoint_key",
                    interner.resolve(fact.entrypoint_stable_key).to_string(),
                ),
                ("precision", format!("{:?}", fact.precision)),
                ("language", language_label(fact.language).to_string()),
                ("file_key", self.source_file_key(fact.file)),
            ]),
        )
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    fn dispatch_edge_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &FrameworkDispatchEdgeFact,
    ) -> FactMeta {
        let (precision, confidence) =
            entrypoint_precision_metadata(EntrypointStatus::Resolved, fact.precision);
        fact_meta_from_stable_key(
            interner,
            ENTRYPOINTS_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("edge_kind", format!("{:?}", fact.edge_kind)),
                ("from_source", fact.from_source.clone()),
                ("precision", format!("{:?}", fact.precision)),
                ("language", language_label(fact.language).to_string()),
                ("file_key", self.source_file_key(fact.file)),
            ]),
        )
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    fn unresolved_framework_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &UnresolvedFrameworkFact,
    ) -> FactMeta {
        fact_meta_from_stable_key(
            interner,
            ENTRYPOINTS_PROVIDER_ID,
            FactPrecision::SetupAware,
            FactConfidence::Medium,
            fact.stable_key,
            stable_parts([
                ("reason", format!("{:?}", fact.reason)),
                ("framework", fact.framework_id.clone()),
                ("precision", format!("{:?}", fact.precision)),
                ("language", language_label(fact.language).to_string()),
                ("file_key", self.source_file_key(fact.file)),
            ]),
        )
    }

    fn summary_fact_metadata(&self, fact: &SummaryFact) -> FactMeta {
        let (precision, confidence) = summary_precision_metadata(fact.status, fact.precision);
        FactMeta {
            stable_key: fact.stable_key,
            producer_id: POLINT_DIRECT_SUMMARIES_PROVIDER_ID,
            layer_id: POLINT_DIRECT_SUMMARIES_PROVIDER_ID,
            precision,
            confidence,
            validation: ValidationStatus::NativeTrusted,
            payload_digest: summary_fact_payload_metadata_digest(&self.stable_keys, fact),
        }
    }

    fn summary_event_metadata(&self, fact: &SummaryEventFact) -> FactMeta {
        let (precision, confidence) = summary_precision_metadata(fact.status, fact.precision);
        FactMeta {
            stable_key: fact.stable_key,
            producer_id: POLINT_DIRECT_SUMMARIES_PROVIDER_ID,
            layer_id: POLINT_DIRECT_SUMMARIES_PROVIDER_ID,
            precision,
            confidence,
            validation: ValidationStatus::NativeTrusted,
            payload_digest: summary_event_payload_metadata_digest(&self.stable_keys, fact),
        }
    }

    #[allow(
        dead_code,
        reason = "CFG writes go through AnalysisHost in polint-analysis; kept for AnalysisDb test helpers."
    )]
    fn refresh_cfg_metadata(&mut self) {
        self.fact_meta.remove_family(FactFamily::CfgFunction);
        self.fact_meta.remove_family(FactFamily::CfgNode);
        self.fact_meta.remove_family(FactFamily::BasicBlock);
        self.fact_meta.remove_family(FactFamily::CfgEdge);
        self.fact_meta.remove_family(FactFamily::CfgReachability);
        self.fact_meta.remove_family(FactFamily::CfgDominator);
        self.fact_meta.remove_family(FactFamily::CfgPostDominator);
        self.fact_meta
            .remove_family(FactFamily::CfgControlDependence);
        self.fact_meta
            .remove_family(FactFamily::UnsupportedControlFlow);

        let cfg_functions = self.cfg_functions().to_vec();
        let cfg_nodes = self.cfg_nodes().to_vec();
        let cfg_blocks = self.cfg_blocks().to_vec();
        let cfg_edges = self.cfg_edges().to_vec();
        let cfg_reachability = self.cfg_reachability().to_vec();
        let cfg_dominators = self.cfg_dominators().to_vec();
        let cfg_postdominators = self.cfg_postdominators().to_vec();
        let cfg_control_dependence = self.cfg_control_dependence().to_vec();
        let unsupported_control_flow = self.unsupported_control_flow().to_vec();

        for fact in &cfg_functions {
            let metadata = self.cfg_function_metadata(fact);
            self.record_fact_meta(FactFamily::CfgFunction, fact.id.0, metadata);
        }

        for fact in &cfg_nodes {
            let metadata = self.cfg_node_metadata(fact);
            self.record_fact_meta(FactFamily::CfgNode, fact.id.0, metadata);
        }

        for fact in &cfg_blocks {
            let metadata = self.cfg_block_metadata(fact);
            self.record_fact_meta(FactFamily::BasicBlock, fact.id.0, metadata);
        }

        for fact in &cfg_edges {
            let metadata = self.cfg_edge_metadata(fact);
            self.record_fact_meta(FactFamily::CfgEdge, fact.id.0, metadata);
        }

        for fact in &cfg_reachability {
            let metadata = self.cfg_reachability_metadata(fact);
            self.record_fact_meta(FactFamily::CfgReachability, fact.id.0, metadata);
        }

        for fact in &cfg_dominators {
            let metadata = self.cfg_dominator_metadata(fact);
            self.record_fact_meta(FactFamily::CfgDominator, fact.id.0, metadata);
        }

        for fact in &cfg_postdominators {
            let metadata = self.cfg_postdominator_metadata(fact);
            self.record_fact_meta(FactFamily::CfgPostDominator, fact.id.0, metadata);
        }

        for fact in &cfg_control_dependence {
            let metadata = self.cfg_control_dependence_metadata(fact);
            self.record_fact_meta(FactFamily::CfgControlDependence, fact.id.0, metadata);
        }

        for fact in &unsupported_control_flow {
            let metadata = self.unsupported_control_flow_metadata(fact);
            self.record_fact_meta(FactFamily::UnsupportedControlFlow, fact.id.0, metadata);
        }

        self.finish_fact_meta_insertions(&[
            FactFamily::CfgFunction,
            FactFamily::CfgNode,
            FactFamily::BasicBlock,
            FactFamily::CfgEdge,
            FactFamily::CfgReachability,
            FactFamily::CfgDominator,
            FactFamily::CfgPostDominator,
            FactFamily::CfgControlDependence,
            FactFamily::UnsupportedControlFlow,
        ]);
    }

    fn refresh_semantic_mir_metadata(&mut self, interner: &crate::core::StableKeyInterner) {
        self.fact_meta.remove_family(FactFamily::MirBody);
        self.fact_meta.remove_family(FactFamily::MirOperation);
        self.fact_meta.remove_family(FactFamily::Place);
        self.fact_meta
            .remove_family(FactFamily::UnsupportedSemantic);

        for index in 0..self.mir_bodies().len() {
            let (run_id, metadata) = {
                let body = &self.mir_bodies()[index];
                (body.id.0, self.mir_body_metadata(interner, body))
            };
            self.record_fact_meta(FactFamily::MirBody, run_id, metadata);
        }

        for index in 0..self.mir_operations().len() {
            let (run_id, metadata) = {
                let operation = &self.mir_operations()[index];
                (operation.id.0, self.mir_operation_metadata(operation))
            };
            self.record_fact_meta(FactFamily::MirOperation, run_id, metadata);
        }

        for index in 0..self.mir_places().len() {
            let (run_id, metadata) = {
                let place = &self.mir_places()[index];
                (place.id.0, self.place_metadata(place))
            };
            self.record_fact_meta(FactFamily::Place, run_id, metadata);
        }

        for index in 0..self.unsupported_semantics().len() {
            let (run_id, metadata) = {
                let row = &self.unsupported_semantics()[index];
                (row.id.0, self.unsupported_semantic_metadata(row))
            };
            self.record_fact_meta(FactFamily::UnsupportedSemantic, run_id, metadata);
        }

        self.finish_fact_meta_insertions(&[
            FactFamily::MirBody,
            FactFamily::MirOperation,
            FactFamily::Place,
            FactFamily::UnsupportedSemantic,
        ]);
    }
}

impl AnalysisDb {
    fn refresh_module_graph_metadata(&mut self, interner: &crate::core::StableKeyInterner) {
        self.fact_meta.remove_family(FactFamily::ModuleNode);
        self.fact_meta.remove_family(FactFamily::ResolvedImport);
        self.fact_meta.remove_family(FactFamily::ModuleEdge);

        let module_nodes = self.module_nodes().to_vec();
        let resolved_imports = self.resolved_imports().to_vec();
        let module_edges = self.module_edges().to_vec();

        let node_metadata = module_nodes
            .iter()
            .map(|node| (node.id.0, self.module_node_metadata(interner, node)))
            .collect::<Vec<_>>();
        for (run_id, metadata) in node_metadata {
            self.record_fact_meta(FactFamily::ModuleNode, run_id, metadata);
        }

        let resolved_metadata = resolved_imports
            .iter()
            .map(|fact| (fact.id.0, self.resolved_import_metadata(interner, fact)))
            .collect::<Vec<_>>();
        for (run_id, metadata) in resolved_metadata {
            self.record_fact_meta(FactFamily::ResolvedImport, run_id, metadata);
        }

        let edge_metadata = module_edges
            .iter()
            .map(|edge| (edge.id.0, self.module_edge_metadata(interner, edge)))
            .collect::<Vec<_>>();
        for (run_id, metadata) in edge_metadata {
            self.record_fact_meta(FactFamily::ModuleEdge, run_id, metadata);
        }
        self.finish_fact_meta_insertions(&[
            FactFamily::ModuleNode,
            FactFamily::ResolvedImport,
            FactFamily::ModuleEdge,
        ]);
    }

    fn refresh_topology_metadata(&mut self) {
        let interner = self.stable_key_interner();
        self.fact_meta.remove_family(FactFamily::WorkspaceRoot);
        self.fact_meta.remove_family(FactFamily::TopologyPackage);
        self.fact_meta.remove_family(FactFamily::SourceSet);
        self.fact_meta
            .remove_family(FactFamily::DependencyRequirement);
        self.fact_meta
            .remove_family(FactFamily::ResolvedDependencyEdge);
        self.fact_meta
            .remove_family(FactFamily::RepoTopologyOverlay);
        self.refresh_import_to_package_metadata();

        let root_metadata = self
            .workspace_roots()
            .iter()
            .map(|fact| {
                (
                    fact.id.0,
                    topology_fact_metadata(
                        &interner,
                        MODULE_GRAPH_PROVIDER_ID,
                        fact.precision,
                        fact.stable_key,
                    ),
                )
            })
            .collect::<Vec<_>>();
        for (run_id, metadata) in root_metadata {
            self.record_fact_meta(FactFamily::WorkspaceRoot, run_id, metadata);
        }

        let package_metadata = self
            .topology_packages()
            .iter()
            .map(|fact| {
                (
                    fact.id.0,
                    topology_fact_metadata(
                        &interner,
                        MODULE_GRAPH_PROVIDER_ID,
                        fact.precision,
                        fact.stable_key,
                    ),
                )
            })
            .collect::<Vec<_>>();
        for (run_id, metadata) in package_metadata {
            self.record_fact_meta(FactFamily::TopologyPackage, run_id, metadata);
        }

        let source_set_metadata = self
            .source_sets()
            .iter()
            .map(|fact| {
                (
                    fact.id.0,
                    topology_fact_metadata(
                        &interner,
                        MODULE_GRAPH_PROVIDER_ID,
                        fact.precision,
                        fact.stable_key,
                    ),
                )
            })
            .collect::<Vec<_>>();
        for (run_id, metadata) in source_set_metadata {
            self.record_fact_meta(FactFamily::SourceSet, run_id, metadata);
        }

        let requirement_metadata = self
            .dependency_requirements()
            .iter()
            .map(|fact| {
                (
                    fact.id.0,
                    topology_fact_metadata(
                        &interner,
                        MODULE_GRAPH_PROVIDER_ID,
                        fact.precision,
                        fact.stable_key,
                    ),
                )
            })
            .collect::<Vec<_>>();
        for (run_id, metadata) in requirement_metadata {
            self.record_fact_meta(FactFamily::DependencyRequirement, run_id, metadata);
        }

        let resolved_metadata = self
            .resolved_dependency_edges()
            .iter()
            .map(|fact| {
                (
                    fact.id.0,
                    topology_fact_metadata(
                        &interner,
                        MODULE_GRAPH_PROVIDER_ID,
                        fact.precision,
                        fact.stable_key,
                    ),
                )
            })
            .collect::<Vec<_>>();
        for (run_id, metadata) in resolved_metadata {
            self.record_fact_meta(FactFamily::ResolvedDependencyEdge, run_id, metadata);
        }

        let overlay_metadata = self
            .repo_topology_overlays()
            .iter()
            .map(|fact| {
                (
                    fact.id.0,
                    topology_fact_metadata(
                        &interner,
                        MODULE_GRAPH_PROVIDER_ID,
                        fact.precision,
                        fact.stable_key,
                    ),
                )
            })
            .collect::<Vec<_>>();
        for (run_id, metadata) in overlay_metadata {
            self.record_fact_meta(FactFamily::RepoTopologyOverlay, run_id, metadata);
        }
        self.finish_fact_meta_insertions(&[
            FactFamily::WorkspaceRoot,
            FactFamily::TopologyPackage,
            FactFamily::SourceSet,
            FactFamily::DependencyRequirement,
            FactFamily::ResolvedDependencyEdge,
            FactFamily::RepoTopologyOverlay,
        ]);
    }

    fn refresh_import_to_package_metadata(&mut self) {
        let interner = self.stable_key_interner();
        self.fact_meta.remove_family(FactFamily::ImportToPackage);

        let metadata = self
            .import_to_package_edges()
            .iter()
            .map(|fact| {
                (
                    fact.id.0,
                    topology_fact_metadata(
                        &interner,
                        MODULE_TOPOLOGY_PROVIDER_ID,
                        fact.precision,
                        fact.stable_key,
                    ),
                )
            })
            .collect::<Vec<_>>();
        for (run_id, metadata) in metadata {
            self.record_fact_meta(FactFamily::ImportToPackage, run_id, metadata);
        }
        self.finish_fact_meta_insertions(&[FactFamily::ImportToPackage]);
    }

    fn refresh_semantic_index_metadata(&mut self) {
        self.fact_meta.remove_family(FactFamily::Scope);
        self.fact_meta.remove_family(FactFamily::SemanticImport);
        self.fact_meta.remove_family(FactFamily::Export);
        self.fact_meta.remove_family(FactFamily::Alias);
        self.fact_meta.remove_family(FactFamily::Resolution);
        self.fact_meta.remove_family(FactFamily::GeneratedSymbol);
        self.fact_meta.remove_family(FactFamily::StableExport);

        let scopes = self.scopes().to_vec();
        let semantic_imports = self.semantic_imports().to_vec();
        let exports = self.exports().to_vec();
        let aliases = self.aliases().to_vec();
        let resolution_facts = self.resolution_facts().to_vec();
        let generated_symbols = self.generated_symbols().to_vec();
        let stable_exports = self.stable_exports().to_vec();

        let scope_metadata = scopes
            .iter()
            .map(|scope| {
                (
                    scope.id.0,
                    self.semantic_fact_metadata(scope.stable_key, scope.status),
                )
            })
            .collect::<Vec<_>>();
        for (run_id, metadata) in scope_metadata {
            self.record_fact_meta(FactFamily::Scope, run_id, metadata);
        }

        let import_metadata = semantic_imports
            .iter()
            .map(|fact| {
                (
                    fact.id.0,
                    self.semantic_fact_metadata(fact.stable_key, fact.status),
                )
            })
            .collect::<Vec<_>>();
        for (run_id, metadata) in import_metadata {
            self.record_fact_meta(FactFamily::SemanticImport, run_id, metadata);
        }

        let export_metadata = exports
            .iter()
            .map(|fact| {
                (
                    fact.id.0,
                    self.semantic_fact_metadata(fact.stable_key, fact.status),
                )
            })
            .collect::<Vec<_>>();
        for (run_id, metadata) in export_metadata {
            self.record_fact_meta(FactFamily::Export, run_id, metadata);
        }

        let alias_metadata = aliases
            .iter()
            .map(|fact| {
                (
                    fact.id.0,
                    self.semantic_fact_metadata(fact.stable_key, fact.status),
                )
            })
            .collect::<Vec<_>>();
        for (run_id, metadata) in alias_metadata {
            self.record_fact_meta(FactFamily::Alias, run_id, metadata);
        }

        let resolution_metadata = resolution_facts
            .iter()
            .map(|fact| {
                (
                    fact.id.0,
                    self.semantic_fact_metadata(fact.stable_key, fact.status),
                )
            })
            .collect::<Vec<_>>();
        for (run_id, metadata) in resolution_metadata {
            self.record_fact_meta(FactFamily::Resolution, run_id, metadata);
        }

        let generated_metadata = generated_symbols
            .iter()
            .map(|fact| {
                (
                    fact.id.0,
                    self.semantic_fact_metadata(fact.stable_key, fact.status),
                )
            })
            .collect::<Vec<_>>();
        for (run_id, metadata) in generated_metadata {
            self.record_fact_meta(FactFamily::GeneratedSymbol, run_id, metadata);
        }

        let stable_export_metadata = stable_exports
            .iter()
            .map(|fact| {
                (
                    fact.id.0,
                    self.semantic_fact_metadata(fact.stable_key, fact.status),
                )
            })
            .collect::<Vec<_>>();
        for (run_id, metadata) in stable_export_metadata {
            self.record_fact_meta(FactFamily::StableExport, run_id, metadata);
        }
        self.finish_fact_meta_insertions(&[
            FactFamily::Scope,
            FactFamily::SemanticImport,
            FactFamily::Export,
            FactFamily::Alias,
            FactFamily::Resolution,
            FactFamily::GeneratedSymbol,
            FactFamily::StableExport,
        ]);
    }

    fn refresh_symbol_graph_metadata(&mut self) {
        self.fact_meta.remove_family(FactFamily::Symbol);
        self.fact_meta.remove_family(FactFamily::Definition);
        self.fact_meta.remove_family(FactFamily::Reference);

        let symbols = self.symbols().to_vec();
        let definitions = self.definitions().to_vec();
        let references = self.references().to_vec();

        for symbol in &symbols {
            let metadata = self.symbol_fact_metadata(symbol);
            self.record_fact_meta(FactFamily::Symbol, symbol.id.0, metadata);
        }

        for definition in &definitions {
            let metadata = self.definition_fact_metadata(definition);
            self.record_fact_meta(FactFamily::Definition, definition.id.0, metadata);
        }

        for reference in &references {
            let metadata = self.reference_fact_metadata(reference);
            self.record_fact_meta(FactFamily::Reference, reference.id.0, metadata);
        }
        self.finish_fact_meta_insertions(&[
            FactFamily::Symbol,
            FactFamily::Definition,
            FactFamily::Reference,
        ]);
    }

    fn refresh_metric_metadata(&mut self, interner: &crate::core::StableKeyInterner) {
        self.fact_meta.remove_family(FactFamily::FileMetric);
        self.fact_meta.remove_family(FactFamily::FunctionMetric);
        self.fact_meta.remove_family(FactFamily::ComplexityMetric);

        let file_metrics = self.file_metrics().to_vec();
        let function_metrics = self.function_metrics().to_vec();
        let complexity_metrics = self.complexity_metrics().to_vec();

        let file_metadata = file_metrics
            .iter()
            .enumerate()
            .map(|(index, fact)| (index as u64, self.file_metric_metadata(interner, fact)))
            .collect::<Vec<_>>();
        for (run_id, metadata) in file_metadata {
            self.record_fact_meta(FactFamily::FileMetric, run_id, metadata);
        }

        let function_metadata = function_metrics
            .iter()
            .enumerate()
            .map(|(index, fact)| (index as u64, self.function_metric_metadata(interner, fact)))
            .collect::<Vec<_>>();
        for (run_id, metadata) in function_metadata {
            self.record_fact_meta(FactFamily::FunctionMetric, run_id, metadata);
        }

        let complexity_metadata = complexity_metrics
            .iter()
            .enumerate()
            .map(|(index, fact)| {
                (
                    index as u64,
                    self.complexity_metric_metadata(interner, fact),
                )
            })
            .collect::<Vec<_>>();
        for (run_id, metadata) in complexity_metadata {
            self.record_fact_meta(FactFamily::ComplexityMetric, run_id, metadata);
        }
        self.finish_fact_meta_insertions(&[
            FactFamily::FileMetric,
            FactFamily::FunctionMetric,
            FactFamily::ComplexityMetric,
        ]);
    }

    pub fn push_ts_component(&mut self, fact: TsComponentFact) {
        let interner_handle = self.stable_key_interner();
        let interner = &interner_handle;
        let metadata = self.ts_component_metadata(interner, &fact);
        let run_id = self.ts_syntax_store_mut().push_ts_component(fact);
        self.record_fact_meta(FactFamily::TsComponent, run_id, metadata);
    }

    pub fn push_ts_class(&mut self, fact: TsClassFact) {
        let interner_handle = self.stable_key_interner();
        let interner = &interner_handle;
        let metadata = self.ts_class_metadata(interner, &fact);
        let run_id = self.ts_syntax_store_mut().push_ts_class(fact);
        self.record_fact_meta(FactFamily::TsClass, run_id, metadata);
    }

    pub fn push_string_literal(&mut self, fact: StringLiteralFact) {
        let interner_handle = self.stable_key_interner();
        let interner = &interner_handle;
        let metadata = self.string_literal_metadata(interner, &fact);
        let run_id = self.ts_syntax_store_mut().push_string_literal(fact);
        self.record_fact_meta(FactFamily::StringLiteral, run_id, metadata);
    }

    pub fn push_jsx_attribute(&mut self, fact: JsxAttributeFact) {
        let interner_handle = self.stable_key_interner();
        let interner = &interner_handle;
        let metadata = self.jsx_attribute_metadata(interner, &fact);
        let run_id = self.ts_syntax_store_mut().push_jsx_attribute(fact);
        self.record_fact_meta(FactFamily::JsxAttribute, run_id, metadata);
    }

    pub(crate) fn fact_meta(&self) -> &FactMetaStore {
        &self.fact_meta
    }

    #[cfg(test)]
    pub(crate) fn fact_meta_mut_for_test(&mut self) -> &mut FactMetaStore {
        &mut self.fact_meta
    }

    #[cfg(test)]
    pub(crate) fn remove_fact_metadata_for_test(&mut self, fact_ref: FactRef) -> Option<FactMeta> {
        self.fact_meta.remove_for_test(fact_ref)
    }

    pub(crate) fn metadata_for(&self, fact_ref: FactRef) -> Option<&FactMeta> {
        self.fact_meta().get(fact_ref)
    }
}

impl AnalysisDb {
    pub(crate) fn missing_fact_metadata(&self) -> Vec<MissingFactMeta> {
        let mut missing = Vec::new();

        for file in self.files() {
            self.push_missing_fact_metadata(
                &mut missing,
                FactFamily::SourceFile,
                u64::from(file.id.0),
            );
        }
        for package in self.packages() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::Package, package.id.0);
        }
        for function in self.functions() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::Function, function.id.0);
        }
        for import in self.imports() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::Import, import.id.0);
        }
        for resolved_import in self.resolved_imports() {
            self.push_missing_fact_metadata(
                &mut missing,
                FactFamily::ResolvedImport,
                resolved_import.id.0,
            );
        }
        for module_node in self.module_nodes() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::ModuleNode, module_node.id.0);
        }
        for module_edge in self.module_edges() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::ModuleEdge, module_edge.id.0);
        }
        for root in self.workspace_roots() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::WorkspaceRoot, root.id.0);
        }
        for package in self.topology_packages() {
            self.push_missing_fact_metadata(
                &mut missing,
                FactFamily::TopologyPackage,
                package.id.0,
            );
        }
        for source_set in self.source_sets() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::SourceSet, source_set.id.0);
        }
        for requirement in self.dependency_requirements() {
            self.push_missing_fact_metadata(
                &mut missing,
                FactFamily::DependencyRequirement,
                requirement.id.0,
            );
        }
        for edge in self.resolved_dependency_edges() {
            self.push_missing_fact_metadata(
                &mut missing,
                FactFamily::ResolvedDependencyEdge,
                edge.id.0,
            );
        }
        for edge in self.import_to_package_edges() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::ImportToPackage, edge.id.0);
        }
        for overlay in self.repo_topology_overlays() {
            self.push_missing_fact_metadata(
                &mut missing,
                FactFamily::RepoTopologyOverlay,
                overlay.id.0,
            );
        }
        for scope in self.scopes() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::Scope, scope.id.0);
        }
        for import in self.semantic_imports() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::SemanticImport, import.id.0);
        }
        for export in self.exports() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::Export, export.id.0);
        }
        for alias in self.aliases() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::Alias, alias.id.0);
        }
        for resolution in self.resolution_facts() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::Resolution, resolution.id.0);
        }
        for generated in self.generated_symbols() {
            self.push_missing_fact_metadata(
                &mut missing,
                FactFamily::GeneratedSymbol,
                generated.id.0,
            );
        }
        for stable_export in self.stable_exports() {
            self.push_missing_fact_metadata(
                &mut missing,
                FactFamily::StableExport,
                stable_export.id.0,
            );
        }
        for body in self.mir_bodies() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::MirBody, body.id.0);
        }
        for operation in self.mir_operations() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::MirOperation, operation.id.0);
        }
        for place in self.mir_places() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::Place, place.id.0);
        }
        for row in self.unsupported_semantics() {
            self.push_missing_fact_metadata(
                &mut missing,
                FactFamily::UnsupportedSemantic,
                row.id.0,
            );
        }
        for site in self.call_sites() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::CallSite, site.id.0);
        }
        for target in self.call_targets() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::CallTarget, target.id.0);
        }
        for (run_id, _unresolved) in self.unresolved_calls().iter().enumerate() {
            self.push_missing_fact_metadata(
                &mut missing,
                FactFamily::UnresolvedCall,
                run_id as u64,
            );
        }
        for edge in self.refined_call_edges() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::RefinedCallEdge, edge.id.0);
        }
        for node in self.data_flow_nodes() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::DataFlowNode, node.id.0);
        }
        for edge in self.data_flow_edges() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::DataFlowEdge, edge.id.0);
        }
        for model in self.data_flow_models() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::DataFlowModel, model.id.0);
        }
        for budget in self.data_flow_budgets() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::DataFlowBudget, budget.id.0);
        }
        for node in self.evidence_nodes() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::EvidenceNode, node.id.0);
        }
        for edge in self.evidence_edges() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::EvidenceEdge, edge.id.0);
        }
        for bundle in self.evidence_bundles() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::EvidenceBundle, bundle.id.0);
        }
        for path in self.evidence_paths() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::EvidencePath, path.id.0);
        }
        for slice in self.evidence_slices() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::EvidenceSlice, slice.id.0);
        }
        for (run_id, _unknown) in self.evidence_unknowns().iter().enumerate() {
            self.push_missing_fact_metadata(
                &mut missing,
                FactFamily::EvidenceUnknown,
                run_id as u64,
            );
        }
        for omitted in self.evidence_omitted_regions() {
            self.push_missing_fact_metadata(
                &mut missing,
                FactFamily::EvidenceOmittedRegion,
                omitted.id.0,
            );
        }
        for (run_id, _replay) in self.evidence_replay_keys().iter().enumerate() {
            self.push_missing_fact_metadata(
                &mut missing,
                FactFamily::EvidenceReplayKey,
                run_id as u64,
            );
        }
        for symbol in self.symbols() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::Symbol, symbol.id.0);
        }
        for definition in self.definitions() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::Definition, definition.id.0);
        }
        for reference in self.references() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::Reference, reference.id.0);
        }
        for branch in self.branches() {
            self.push_missing_fact_metadata(
                &mut missing,
                FactFamily::BranchObligation,
                branch.id.0,
            );
        }
        for (run_id, _test) in self.tests().iter().enumerate() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::Test, run_id as u64);
        }
        for (run_id, _coverage) in self.coverage().iter().enumerate() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::Coverage, run_id as u64);
        }
        for (run_id, _file_metric) in self.file_metrics().iter().enumerate() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::FileMetric, run_id as u64);
        }
        for (run_id, _function_metric) in self.function_metrics().iter().enumerate() {
            self.push_missing_fact_metadata(
                &mut missing,
                FactFamily::FunctionMetric,
                run_id as u64,
            );
        }
        for (run_id, _complexity_metric) in self.complexity_metrics().iter().enumerate() {
            self.push_missing_fact_metadata(
                &mut missing,
                FactFamily::ComplexityMetric,
                run_id as u64,
            );
        }
        for (run_id, _component) in self.ts_components().iter().enumerate() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::TsComponent, run_id as u64);
        }
        for (run_id, _class) in self.ts_classes().iter().enumerate() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::TsClass, run_id as u64);
        }
        for (run_id, _literal) in self.string_literals().iter().enumerate() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::StringLiteral, run_id as u64);
        }
        for (run_id, _attribute) in self.jsx_attributes().iter().enumerate() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::JsxAttribute, run_id as u64);
        }
        for (run_id, _fact) in self.extension_facts().iter().enumerate() {
            self.push_missing_fact_metadata(&mut missing, FactFamily::ExtensionFact, run_id as u64);
        }
        for (run_id, _fact) in self.adaptation_model_facts().iter().enumerate() {
            self.push_missing_fact_metadata(
                &mut missing,
                FactFamily::AdaptationModel,
                run_id as u64,
            );
        }

        missing.sort_by(|left, right| {
            (left.family.label(), left.run_id).cmp(&(right.family.label(), right.run_id))
        });
        missing
    }

    fn push_missing_fact_metadata(
        &self,
        missing: &mut Vec<MissingFactMeta>,
        family: FactFamily,
        run_id: u64,
    ) {
        if self.metadata_for(FactRef::new(family, run_id)).is_none() {
            missing.push(MissingFactMeta { family, run_id });
        }
    }

    pub fn file(&self, id: FileId) -> Option<&SourceFile> {
        self.files.get(id.0 as usize)
    }

    pub(crate) fn set_path_contexts(&mut self, index: crate::path_context::PathContextIndex) {
        self.path_contexts = Some(index);
    }

    /// Repo-relative paths paired with `relative_path` (see `.polint.toml` `[path_contexts]`).
    pub fn path_context_related(&self, pair_name: &str, relative_path: &str) -> Vec<String> {
        self.path_contexts
            .as_ref()
            .map(|ix| ix.related_paths(pair_name, relative_path))
            .unwrap_or_default()
    }

    pub fn files(&self) -> &[SourceFile] {
        &self.files
    }

    /// Relative paths as in diagnostics (`SourceFile.relative_path`) → full source text.
    pub fn sources_by_relative_path(&self) -> BTreeMap<String, Arc<str>> {
        self.files
            .iter()
            .map(|file| (file.relative_path.clone(), Arc::clone(&file.source)))
            .collect()
    }

    pub fn packages(&self) -> &[PackageFact] {
        self.go_syntax_store().packages()
    }

    pub fn functions(&self) -> &[FunctionFact] {
        self.go_syntax_store().functions()
    }

    pub(crate) fn functions_for_file(
        &self,
        file: FileId,
    ) -> impl Iterator<Item = &FunctionFact> + '_ {
        self.fact_view_indexes()
            .functions_by_file
            .facts(file, self.functions())
    }

    pub fn go_types(&self) -> &[GoTypeDeclFact] {
        self.go_syntax_store().go_types()
    }

    pub(crate) fn go_types_for_file(
        &self,
        file: FileId,
    ) -> impl Iterator<Item = &GoTypeDeclFact> + '_ {
        self.fact_view_indexes()
            .go_types_by_file
            .facts(file, self.go_types())
    }

    pub fn imports(&self) -> &[ImportFact] {
        self.go_syntax_store().imports()
    }

    pub(crate) fn imports_for_file(&self, file: FileId) -> impl Iterator<Item = &ImportFact> + '_ {
        self.fact_view_indexes()
            .imports_by_file
            .facts(file, self.imports())
    }

    pub fn resolved_imports(&self) -> &[ResolvedImportFact] {
        self.module_graph_store().resolved_imports()
    }

    pub(crate) fn resolved_imports_for_file(
        &self,
        file: FileId,
    ) -> impl Iterator<Item = &ResolvedImportFact> + '_ {
        self.fact_view_indexes()
            .resolved_imports_by_file
            .facts(file, self.resolved_imports())
    }

    pub fn module_nodes(&self) -> &[ModuleNode] {
        self.module_graph_store().module_nodes()
    }

    pub(crate) fn module_node_for_file(&self, file: FileId) -> Option<&ModuleNode> {
        self.fact_view_indexes()
            .module_node_by_file
            .get(u64::from(file.raw()), self.module_nodes())
    }

    pub fn module_edges(&self) -> &[ModuleEdge] {
        self.module_graph_store().module_edges()
    }

    pub(crate) fn workspace_roots(&self) -> &[WorkspaceRootFact] {
        self.module_topology_store().workspace_roots()
    }

    pub(crate) fn topology_packages(&self) -> &[TopologyPackageFact] {
        self.module_topology_store().topology_packages()
    }

    pub(crate) fn source_sets(&self) -> &[SourceSetFact] {
        self.module_topology_store().source_sets()
    }

    pub(crate) fn dependency_requirements(&self) -> &[DependencyRequirementFact] {
        self.module_topology_store().dependency_requirements()
    }

    pub(crate) fn resolved_dependency_edges(&self) -> &[ResolvedDependencyEdgeFact] {
        self.module_topology_store().resolved_dependency_edges()
    }

    pub(crate) fn import_to_package_edges(&self) -> &[ImportToPackageFact] {
        self.module_topology_store().import_to_package_edges()
    }

    pub(crate) fn repo_topology_overlays(&self) -> &[RepoTopologyOverlayFact] {
        self.module_topology_store().repo_topology_overlays()
    }

    pub(crate) fn scopes(&self) -> &[ScopeFact] {
        self.semantic_index_store().scopes()
    }

    pub(crate) fn semantic_imports(&self) -> &[SemanticImportFact] {
        self.semantic_index_store().semantic_imports()
    }

    pub(crate) fn exports(&self) -> &[ExportFact] {
        self.semantic_index_store().exports()
    }

    pub(crate) fn aliases(&self) -> &[AliasFact] {
        self.semantic_index_store().aliases()
    }

    pub(crate) fn resolution_facts(&self) -> &[ResolutionFact] {
        self.semantic_index_store().resolution_facts()
    }

    pub(crate) fn generated_symbols(&self) -> &[GeneratedSymbolFact] {
        self.semantic_index_store().generated_symbols()
    }

    pub(crate) fn stable_exports(&self) -> &[StableExportIdentity] {
        self.semantic_index_store().stable_exports()
    }

    #[allow(dead_code)]
    pub(crate) fn semantic_store(&self) -> Option<&SemanticStore> {
        Some(self.semantic_mir_store_inner())
    }

    pub(crate) fn mir_bodies(&self) -> &[MirBody] {
        self.semantic_mir_store_inner().mir_bodies()
    }

    pub(crate) fn mir_operations(&self) -> &[MirOperation] {
        self.semantic_mir_store_inner().mir_operations()
    }

    #[allow(
        dead_code,
        reason = "MIR accessors retained for tests; production reads go through AnalysisHost."
    )]
    pub(crate) fn mir_blocks(&self) -> &[MirBlock] {
        self.semantic_mir_store_inner().mir_blocks()
    }

    #[allow(
        dead_code,
        reason = "MIR accessors retained for tests; production reads go through AnalysisHost."
    )]
    pub(crate) fn mir_statements(&self) -> &[MirStatement] {
        self.semantic_mir_store_inner().mir_statements()
    }

    #[allow(
        dead_code,
        reason = "MIR accessors retained for tests; production reads go through AnalysisHost."
    )]
    pub(crate) fn mir_terminators(&self) -> &[MirTerminator] {
        self.semantic_mir_store_inner().mir_terminators()
    }

    pub(crate) fn mir_places(&self) -> &[PlaceFact] {
        self.semantic_mir_store_inner().places()
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    pub(crate) fn mir_place_types(&self) -> &[crate::analysis::places::PlaceTypeFact] {
        self.semantic_mir_store_inner().place_types()
    }

    pub(crate) fn unsupported_semantics(&self) -> &[UnsupportedSemanticFact] {
        self.semantic_mir_store_inner().unsupported_semantics()
    }

    pub(crate) fn cfg_functions(&self) -> &[CfgFunctionFact] {
        self.cfg_store().functions()
    }

    pub(crate) fn cfg_nodes(&self) -> &[CfgNodeFact] {
        self.cfg_store().nodes()
    }

    pub(crate) fn cfg_blocks(&self) -> &[BasicBlockFact] {
        self.cfg_store().blocks()
    }

    pub(crate) fn cfg_edges(&self) -> &[CfgEdgeFact] {
        self.cfg_store().edges()
    }

    pub(crate) fn cfg_reachability(&self) -> &[ReachabilityFact] {
        self.cfg_store().reachability()
    }

    pub(crate) fn cfg_dominators(&self) -> &[DominatorFact] {
        self.cfg_store().dominators()
    }

    pub(crate) fn cfg_postdominators(&self) -> &[PostDominatorFact] {
        self.cfg_store().postdominators()
    }

    pub(crate) fn cfg_control_dependence(&self) -> &[ControlDependenceFact] {
        self.cfg_store().control_dependence()
    }

    pub(crate) fn unsupported_control_flow(&self) -> &[UnsupportedControlFlowFact] {
        self.cfg_store().unsupported()
    }

    pub fn symbols(&self) -> &[SymbolFact] {
        self.symbol_store().symbols()
    }

    pub(crate) fn symbols_for_file(&self, file: FileId) -> impl Iterator<Item = &SymbolFact> + '_ {
        self.fact_view_indexes()
            .symbols_by_file
            .facts(file, self.symbols())
    }

    pub fn definitions(&self) -> &[DefinitionFact] {
        self.symbol_store().definitions()
    }

    pub(crate) fn definitions_for_file(
        &self,
        file: FileId,
    ) -> impl Iterator<Item = &DefinitionFact> + '_ {
        self.fact_view_indexes()
            .definitions_by_file
            .facts(file, self.definitions())
    }

    pub fn references(&self) -> &[ReferenceFact] {
        self.symbol_store().references()
    }

    pub(crate) fn symbol_by_id(&self, id: SymbolId) -> Option<&SymbolFact> {
        self.symbol_store().symbol_by_id(id)
    }

    pub(crate) fn definition_for_symbol(&self, symbol: SymbolId) -> Option<&DefinitionFact> {
        let mut definitions = self.definitions_for_symbol(symbol);
        let first = definitions.next();
        first
            .filter(|definition| definition.is_primary)
            .or_else(|| definitions.find(|definition| definition.is_primary))
            .or(first)
    }

    pub(crate) fn definitions_for_symbol(
        &self,
        symbol: SymbolId,
    ) -> impl Iterator<Item = &DefinitionFact> + '_ {
        self.symbol_store().definitions_for_symbol(symbol)
    }

    pub(crate) fn references_for_file(
        &self,
        file: FileId,
    ) -> impl Iterator<Item = &ReferenceFact> + '_ {
        self.fact_view_indexes()
            .references_by_file
            .facts(file, self.references())
    }

    pub fn branches(&self) -> &[BranchObligation] {
        self.go_syntax_store().branches()
    }

    pub(crate) fn branches_for_file(
        &self,
        file: FileId,
    ) -> impl Iterator<Item = &BranchObligation> + '_ {
        self.fact_view_indexes()
            .branches_by_file
            .facts(file, self.branches())
    }

    pub fn tests(&self) -> &[TestFact] {
        self.go_syntax_store().tests()
    }

    pub(crate) fn tests_for_file(&self, file: FileId) -> impl Iterator<Item = &TestFact> + '_ {
        self.fact_view_indexes()
            .tests_by_file
            .facts(file, self.tests())
    }

    pub fn coverage(&self) -> &[CoverageFact] {
        self.metrics_store().coverage()
    }

    pub fn file_metrics(&self) -> &[FileMetricFact] {
        self.metrics_store().file_metrics()
    }

    pub(crate) fn file_metric_for_file(&self, file: FileId) -> Option<&FileMetricFact> {
        self.fact_view_indexes()
            .file_metric_by_file
            .get(u64::from(file.raw()), self.file_metrics())
    }

    pub fn function_metrics(&self) -> &[FunctionMetricFact] {
        self.metrics_store().function_metrics()
    }

    pub(crate) fn function_metrics_for_file(
        &self,
        file: FileId,
    ) -> impl Iterator<Item = &FunctionMetricFact> + '_ {
        self.fact_view_indexes()
            .function_metrics_by_file
            .facts(file, self.function_metrics())
    }

    pub(crate) fn function_metric_for_function(
        &self,
        function: FunctionId,
    ) -> Option<&FunctionMetricFact> {
        self.fact_view_indexes()
            .function_metric_by_function
            .get(function.raw(), self.function_metrics())
    }

    pub fn complexity_metrics(&self) -> &[ComplexityMetricFact] {
        self.metrics_store().complexity_metrics()
    }

    pub(crate) fn complexity_metrics_for_file(
        &self,
        file: FileId,
    ) -> impl Iterator<Item = &ComplexityMetricFact> + '_ {
        self.fact_view_indexes()
            .complexity_metrics_by_file
            .facts(file, self.complexity_metrics())
    }

    pub(crate) fn complexity_metric_for_function(
        &self,
        function: FunctionId,
    ) -> Option<&ComplexityMetricFact> {
        self.fact_view_indexes()
            .complexity_metric_by_function
            .get(function.raw(), self.complexity_metrics())
    }

    pub fn ts_components(&self) -> &[TsComponentFact] {
        self.ts_syntax_store().ts_components()
    }

    pub(crate) fn ts_components_for_file(
        &self,
        file: FileId,
    ) -> impl Iterator<Item = &TsComponentFact> + '_ {
        self.fact_view_indexes()
            .ts_components_by_file
            .facts(file, self.ts_components())
    }

    pub fn ts_classes(&self) -> &[TsClassFact] {
        self.ts_syntax_store().ts_classes()
    }

    pub(crate) fn ts_classes_for_file(
        &self,
        file: FileId,
    ) -> impl Iterator<Item = &TsClassFact> + '_ {
        self.fact_view_indexes()
            .ts_classes_by_file
            .facts(file, self.ts_classes())
    }

    pub fn string_literals(&self) -> &[StringLiteralFact] {
        self.ts_syntax_store().string_literals()
    }

    pub(crate) fn string_literals_for_file(
        &self,
        file: FileId,
    ) -> impl Iterator<Item = &StringLiteralFact> + '_ {
        self.fact_view_indexes()
            .string_literals_by_file
            .facts(file, self.string_literals())
    }

    pub fn jsx_attributes(&self) -> &[JsxAttributeFact] {
        self.ts_syntax_store().jsx_attributes()
    }

    pub(crate) fn jsx_attributes_for_file(
        &self,
        file: FileId,
    ) -> impl Iterator<Item = &JsxAttributeFact> + '_ {
        self.fact_view_indexes()
            .jsx_attributes_by_file
            .facts(file, self.jsx_attributes())
    }

    pub fn path_for(&self, file: FileId) -> String {
        self.path_str_for(file).to_string()
    }

    /// [`AnalysisDb::path_for`] without the copy, for callers that only read the
    /// path — fact metadata builds one per fact.
    pub(crate) fn path_str_for(&self, file: FileId) -> &str {
        self.file(file)
            .map(|file| file.relative_path.as_str())
            .unwrap_or("<unknown>")
    }
}

impl AnalysisDb {
    pub fn facts_for_file(&self, file: FileId) -> CachedFileFacts {
        let branch_ids = self
            .branches()
            .iter()
            .filter(|branch| branch.file == file)
            .map(|branch| branch.id)
            .collect::<BTreeSet<_>>();
        CachedFileFacts {
            packages: self
                .packages()
                .iter()
                .filter(|fact| fact.file == file)
                .cloned()
                .collect(),
            functions: self
                .functions()
                .iter()
                .filter(|fact| fact.file == file)
                .cloned()
                .collect(),
            imports: self
                .imports()
                .iter()
                .filter(|fact| fact.file == file)
                .cloned()
                .collect(),
            branches: self
                .branches()
                .iter()
                .filter(|fact| fact.file == file)
                .cloned()
                .collect(),
            tests: self
                .tests()
                .iter()
                .filter(|fact| fact.file == file)
                .cloned()
                .collect(),
            coverage: self
                .coverage()
                .iter()
                .filter(|fact| branch_ids.contains(&fact.branch))
                .cloned()
                .collect(),
            ts_components: self
                .ts_components()
                .iter()
                .filter(|fact| fact.file == file)
                .cloned()
                .collect(),
            ts_classes: self
                .ts_classes()
                .iter()
                .filter(|fact| fact.file == file)
                .cloned()
                .collect(),
            string_literals: self
                .string_literals()
                .iter()
                .filter(|fact| fact.file == file)
                .cloned()
                .collect(),
            jsx_attributes: self
                .jsx_attributes()
                .iter()
                .filter(|fact| fact.file == file)
                .cloned()
                .collect(),
            go_types: self
                .go_types()
                .iter()
                .filter(|fact| fact.file == file)
                .cloned()
                .collect(),
        }
    }

    pub fn restore_file_facts(&mut self, file: FileId, facts: CachedFileFacts) {
        let mut function_ids = BTreeMap::new();
        let mut branch_ids = BTreeMap::new();

        for mut package in facts.packages {
            package.file = file;
            package.span.file = file;
            self.push_package(package);
        }

        for mut function in facts.functions {
            let cached_id = function.id;
            function.file = file;
            function.span.file = file;
            let restored_id = self.push_function(function);
            function_ids.insert(cached_id, restored_id);
        }

        for mut import in facts.imports {
            import.file = file;
            import.span.file = file;
            self.push_import(import);
        }

        for mut branch in facts.branches {
            let cached_id = branch.id;
            branch.file = file;
            branch.function = branch
                .function
                .and_then(|function| function_ids.get(&function).copied());
            branch.decision_span.file = file;
            let restored_id = self.push_branch(branch);
            branch_ids.insert(cached_id, restored_id);
        }

        for mut test in facts.tests {
            test.file = file;
            test.function = test
                .function
                .and_then(|function| function_ids.get(&function).copied());
            test.span.file = file;
            self.push_test(test);
        }

        for mut coverage in facts.coverage {
            if let Some(branch) = branch_ids.get(&coverage.branch).copied() {
                coverage.branch = branch;
                self.push_coverage(coverage);
            }
        }

        for mut component in facts.ts_components {
            component.file = file;
            component.function = component
                .function
                .and_then(|function| function_ids.get(&function).copied());
            component.span.file = file;
            self.push_ts_component(component);
        }

        for mut class in facts.ts_classes {
            class.file = file;
            class.span.file = file;
            self.push_ts_class(class);
        }

        for mut literal in facts.string_literals {
            literal.file = file;
            literal.span.file = file;
            self.push_string_literal(literal);
        }

        for mut attribute in facts.jsx_attributes {
            attribute.file = file;
            attribute.span.file = file;
            self.push_jsx_attribute(attribute);
        }

        for mut go_type in facts.go_types {
            go_type.file = file;
            go_type.span.file = file;
            self.push_go_type(go_type);
        }
    }

    fn record_fact_meta(&mut self, family: FactFamily, run_id: u64, meta: FactMeta) {
        let reference = FactRef::new(family, run_id);
        let _insert = self.fact_meta.insert(reference, meta);
        debug_assert!(self.metadata_for(reference).is_some());
    }

    fn finish_fact_meta_insertions(&mut self, families: &[FactFamily]) {
        for family in families {
            self.fact_meta.finish_family_insertions(*family);
        }
    }

    pub(crate) fn finish_all_fact_meta_insertions(&mut self) {
        self.fact_meta.finish_all_insertions();
        self.finalize_fact_view_indexes();
    }

    fn package_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &PackageFact,
    ) -> FactMeta {
        let span = span_metadata_value(&fact.span);
        fact_meta_from_borrowed_parts(
            interner,
            FactFamily::Package,
            syntax_provider_for_language(fact.language),
            FactPrecision::Syntax,
            FactConfidence::High,
            stable_parts([
                ("path", self.path_str_for(fact.file)),
                ("language", language_label(fact.language)),
                ("name", fact.name.as_str()),
                ("span", span.as_str()),
            ]),
            stable_parts([]),
        )
    }

    fn function_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &FunctionFact,
    ) -> FactMeta {
        let span = span_metadata_value(&fact.span);
        let cyclomatic_complexity = fact.cyclomatic_complexity.to_string();
        let calls = fact.calls.join("\n");
        fact_meta_from_borrowed_parts(
            interner,
            FactFamily::Function,
            syntax_provider_for_language(fact.language),
            FactPrecision::Syntax,
            FactConfidence::High,
            stable_parts([
                ("path", self.path_str_for(fact.file)),
                ("language", language_label(fact.language)),
                ("name", fact.name.as_str()),
                ("span", span.as_str()),
            ]),
            stable_parts([
                ("is_test", bool_label(fact.is_test)),
                ("is_exported", bool_label(fact.is_exported)),
                ("cyclomatic_complexity", cyclomatic_complexity.as_str()),
                ("calls", calls.as_str()),
            ]),
        )
    }

    fn import_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &ImportFact,
    ) -> FactMeta {
        let span = span_metadata_value(&fact.span);
        fact_meta_from_borrowed_parts(
            interner,
            FactFamily::Import,
            syntax_provider_for_language(fact.language),
            FactPrecision::Syntax,
            FactConfidence::High,
            stable_parts([
                ("path", self.path_str_for(fact.file)),
                ("language", language_label(fact.language)),
                ("import_path", fact.path.as_str()),
                ("span", span.as_str()),
            ]),
            stable_parts([("package", fact.package.as_deref().unwrap_or("none"))]),
        )
    }

    fn branch_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &BranchObligation,
    ) -> FactMeta {
        let function = option_function_id(fact.function);
        let span = span_metadata_value(&fact.decision_span);
        fact_meta_from_borrowed_parts(
            interner,
            FactFamily::BranchObligation,
            GO_SYNTAX_PROVIDER_ID,
            FactPrecision::Heuristic,
            FactConfidence::Medium,
            stable_parts([
                ("path", self.path_str_for(fact.file)),
                ("stable_fingerprint", fact.stable_fingerprint.as_str()),
            ]),
            stable_parts([
                ("function", function.as_str()),
                ("span", span.as_str()),
                ("condition_text", fact.condition_text.as_str()),
                ("edge_label", fact.edge_label.as_str()),
                ("is_error_path", bool_label(fact.is_error_path)),
            ]),
        )
    }

    fn test_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &TestFact,
    ) -> FactMeta {
        let span = span_metadata_value(&fact.span);
        let function = option_function_id(fact.function);
        let evidence_terms = fact.evidence_terms.join("\n");
        let assertion_count = fact.assertion_count.to_string();
        let subtest_count = fact.subtest_count.to_string();
        let subtest_names = fact.subtest_names.join("\n");
        let table_rows = fact.table_rows.to_string();
        fact_meta_from_borrowed_parts(
            interner,
            FactFamily::Test,
            GO_SYNTAX_PROVIDER_ID,
            FactPrecision::Heuristic,
            FactConfidence::Medium,
            stable_parts([
                ("path", self.path_str_for(fact.file)),
                ("name", fact.name.as_str()),
                ("span", span.as_str()),
            ]),
            stable_parts([
                ("function", function.as_str()),
                ("evidence_terms", evidence_terms.as_str()),
                ("assertion_count", assertion_count.as_str()),
                ("subtest_count", subtest_count.as_str()),
                ("subtest_names", subtest_names.as_str()),
                ("table_rows", table_rows.as_str()),
            ]),
        )
    }

    fn coverage_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &CoverageFact,
    ) -> FactMeta {
        let branch = self
            .branches()
            .iter()
            .find(|branch| branch.id == fact.branch);
        let unresolved_fingerprint;
        let (path, branch_fingerprint, precision, confidence) = if let Some(branch) = branch {
            (
                self.path_str_for(branch.file),
                branch.stable_fingerprint.as_str(),
                FactPrecision::SetupAware,
                FactConfidence::Medium,
            )
        } else {
            unresolved_fingerprint = format!("unresolved:{}", fact.branch.0);
            (
                "<unknown>",
                unresolved_fingerprint.as_str(),
                FactPrecision::Unsupported,
                FactConfidence::Low,
            )
        };
        let branch_id = fact.branch.0.to_string();
        let covered = option_bool(fact.covered);

        fact_meta_from_borrowed_parts(
            interner,
            FactFamily::Coverage,
            branch
                .map(|branch| syntax_provider_for_file(self.file(branch.file)))
                .unwrap_or(GO_SYNTAX_PROVIDER_ID),
            precision,
            confidence,
            stable_parts([
                ("path", path),
                ("branch_fingerprint", branch_fingerprint),
                ("source", fact.source.as_str()),
            ]),
            stable_parts([
                ("branch", branch_id.as_str()),
                ("covered", covered.as_str()),
            ]),
        )
    }

    fn file_metric_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &FileMetricFact,
    ) -> FactMeta {
        fact_meta_from_parts(
            interner,
            FactFamily::FileMetric,
            METRICS_PROVIDER_ID,
            FactPrecision::Syntax,
            FactConfidence::High,
            stable_parts([("file_key", self.source_file_key(fact.file))]),
            stable_parts([
                ("language", language_label(fact.language).to_string()),
                ("line_count", fact.line_count.to_string()),
                (
                    "non_empty_line_count",
                    fact.non_empty_line_count.to_string(),
                ),
                ("byte_count", fact.byte_count.to_string()),
                ("function_count", fact.function_count.to_string()),
            ]),
        )
    }

    fn function_metric_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &FunctionMetricFact,
    ) -> FactMeta {
        fact_meta_from_parts(
            interner,
            FactFamily::FunctionMetric,
            METRICS_PROVIDER_ID,
            FactPrecision::Syntax,
            FactConfidence::High,
            stable_parts([
                (
                    "function_key",
                    self.function_key(interner, fact.function, &fact.name, &fact.span),
                ),
                ("metric_name", FUNCTION_SIZE_METRIC_NAME.to_string()),
            ]),
            stable_parts([
                ("file_key", self.source_file_key(fact.file)),
                ("language", language_label(fact.language).to_string()),
                ("line_count", fact.line_count.to_string()),
                ("byte_count", fact.byte_count.to_string()),
            ]),
        )
    }

    fn complexity_metric_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &ComplexityMetricFact,
    ) -> FactMeta {
        fact_meta_from_parts(
            interner,
            FactFamily::ComplexityMetric,
            METRICS_PROVIDER_ID,
            FactPrecision::Syntax,
            FactConfidence::High,
            stable_parts([
                (
                    "function_key",
                    self.function_key(interner, fact.function, &fact.name, &fact.span),
                ),
                ("metric_name", CYCLOMATIC_COMPLEXITY_METRIC_NAME.to_string()),
            ]),
            stable_parts([
                ("file_key", self.source_file_key(fact.file)),
                ("language", language_label(fact.language).to_string()),
                (
                    "cyclomatic_complexity",
                    fact.cyclomatic_complexity.to_string(),
                ),
            ]),
        )
    }

    fn module_node_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        node: &ModuleNode,
    ) -> FactMeta {
        fact_meta_from_parts(
            interner,
            FactFamily::ModuleNode,
            MODULE_GRAPH_PROVIDER_ID,
            FactPrecision::SetupAware,
            FactConfidence::High,
            stable_parts([
                ("kind", module_node_kind_label(node.kind).to_string()),
                ("label", node.label.clone()),
                ("path", option_file_path(self, node.file)),
                (
                    "package_key",
                    node.package
                        .map(|package| self.fact_stable_key(FactFamily::Package, package.0))
                        .unwrap_or_else(none_value),
                ),
                (
                    "language",
                    node.language
                        .map(|language| language_label(language).to_string())
                        .unwrap_or_else(none_value),
                ),
            ]),
            stable_parts([("id", node.id.0.to_string())]),
        )
    }

    fn resolved_import_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &ResolvedImportFact,
    ) -> FactMeta {
        let (precision, confidence) = resolution_metadata(fact.precision, fact.status);
        fact_meta_from_parts(
            interner,
            FactFamily::ResolvedImport,
            MODULE_GRAPH_PROVIDER_ID,
            precision,
            confidence,
            stable_parts([
                (
                    "import_key",
                    self.fact_stable_key(FactFamily::Import, fact.import.0),
                ),
                ("from_path", self.path_for(fact.from_file)),
                (
                    "target_node_key",
                    fact.target_node
                        .map(|node| self.fact_stable_key(FactFamily::ModuleNode, node.0))
                        .unwrap_or_else(none_value),
                ),
                ("status", resolution_status_label(fact.status).to_string()),
                (
                    "precision",
                    resolution_precision_label(fact.precision).to_string(),
                ),
                (
                    "reason",
                    fact.reason
                        .map(|reason| unresolved_reason_label(reason).to_string())
                        .unwrap_or_else(none_value),
                ),
            ]),
            stable_parts([
                ("id", fact.id.0.to_string()),
                ("import", fact.import.0.to_string()),
                ("from_file", u64::from(fact.from_file.0).to_string()),
                (
                    "target_node",
                    fact.target_node
                        .map(|node| node.0.to_string())
                        .unwrap_or_else(none_value),
                ),
            ]),
        )
    }

    fn module_edge_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        edge: &ModuleEdge,
    ) -> FactMeta {
        let (precision, confidence) = resolution_status_metadata(edge.status);
        fact_meta_from_parts(
            interner,
            FactFamily::ModuleEdge,
            MODULE_GRAPH_PROVIDER_ID,
            precision,
            confidence,
            stable_parts([
                (
                    "from_node_key",
                    self.fact_stable_key(FactFamily::ModuleNode, edge.from.0),
                ),
                (
                    "to_node_key",
                    self.fact_stable_key(FactFamily::ModuleNode, edge.to.0),
                ),
                (
                    "import_key",
                    edge.import
                        .map(|import| self.fact_stable_key(FactFamily::Import, import.0))
                        .unwrap_or_else(none_value),
                ),
                (
                    "resolved_import_key",
                    edge.resolved_import
                        .map(|resolved| {
                            self.fact_stable_key(FactFamily::ResolvedImport, resolved.0)
                        })
                        .unwrap_or_else(none_value),
                ),
                ("kind", module_edge_kind_label(edge.kind).to_string()),
                ("status", resolution_status_label(edge.status).to_string()),
            ]),
            stable_parts([
                ("id", edge.id.0.to_string()),
                ("from", edge.from.0.to_string()),
                ("to", edge.to.0.to_string()),
            ]),
        )
    }

    fn symbol_fact_metadata(&self, fact: &SymbolFact) -> FactMeta {
        let (precision, confidence) = symbol_metadata(fact.precision);
        fact_meta_from_stable_key(
            &self.stable_keys,
            SYMBOL_GRAPH_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("id", fact.id.0.to_string()),
                ("language", language_label(fact.language).to_string()),
                ("name", fact.name.clone()),
                ("qualified_name", fact.qualified_name.clone()),
                (
                    "precision",
                    symbol_precision_label(fact.precision).to_string(),
                ),
                ("path", option_file_path(self, fact.file)),
                (
                    "span",
                    option_span_metadata_value(fact.primary_span.as_ref()),
                ),
            ]),
        )
    }

    fn definition_fact_metadata(&self, fact: &DefinitionFact) -> FactMeta {
        let (precision, confidence) = symbol_metadata(fact.precision);
        fact_meta_from_stable_key(
            &self.stable_keys,
            SYMBOL_GRAPH_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("id", fact.id.0.to_string()),
                ("symbol", fact.symbol.0.to_string()),
                ("language", language_label(fact.language).to_string()),
                ("name", fact.name.clone()),
                ("qualified_name", fact.qualified_name.clone()),
                (
                    "precision",
                    symbol_precision_label(fact.precision).to_string(),
                ),
                ("path", option_file_path(self, fact.file)),
                (
                    "span",
                    option_span_metadata_value(fact.primary_span.as_ref()),
                ),
            ]),
        )
    }

    fn reference_fact_metadata(&self, fact: &ReferenceFact) -> FactMeta {
        let (precision, confidence) = symbol_metadata(fact.precision);
        fact_meta_from_stable_key(
            &self.stable_keys,
            SYMBOL_GRAPH_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("id", fact.id.0.to_string()),
                ("language", language_label(fact.language).to_string()),
                ("name", fact.name.clone()),
                ("qualified_name", fact.qualified_name.clone()),
                (
                    "precision",
                    symbol_precision_label(fact.precision).to_string(),
                ),
                (
                    "status",
                    symbol_resolution_status_label(fact.status).to_string(),
                ),
                ("path", option_file_path(self, fact.file)),
                (
                    "span",
                    option_span_metadata_value(fact.primary_span.as_ref()),
                ),
                (
                    "target",
                    fact.target
                        .map(|target| target.0.to_string())
                        .unwrap_or_else(none_value),
                ),
            ]),
        )
    }

    fn semantic_fact_metadata(
        &self,
        stable_key: crate::core::StableKeyId,
        status: SemanticStatus,
    ) -> FactMeta {
        let (precision, confidence) = semantic_status_metadata(status);
        fact_meta_from_stable_key(
            &self.stable_keys,
            SYMBOL_GRAPH_PROVIDER_ID,
            precision,
            confidence,
            stable_key,
            stable_parts([("status", semantic_status_label(status).to_string())]),
        )
    }

    fn mir_body_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        body: &MirBody,
    ) -> FactMeta {
        let (precision, confidence) = mir_status_metadata(body.status);
        fact_meta_from_stable_key(
            interner,
            SEMANTIC_MIR_PROVIDER_ID,
            precision,
            confidence,
            body.stable_key,
            stable_parts([
                ("status", mir_status_label(body.status).to_string()),
                ("language", language_label(body.language).to_string()),
                ("file_key", self.source_file_key(body.file)),
                (
                    "function_key",
                    self.function_key(interner, body.function, "", &body.span),
                ),
                (
                    "owner_stable_key",
                    interner.resolve(body.owner_stable_key).to_string(),
                ),
                (
                    "package",
                    body.package
                        .map(|package| package.0.to_string())
                        .unwrap_or_else(none_value),
                ),
                (
                    "module",
                    body.module
                        .map(|module| module.0.to_string())
                        .unwrap_or_else(none_value),
                ),
                ("span", span_metadata_value(&body.span)),
            ]),
        )
    }

    fn mir_operation_metadata(&self, operation: &MirOperation) -> FactMeta {
        let (precision, confidence) = mir_status_metadata(operation.status);
        fact_meta_from_stable_key(
            &self.stable_keys,
            SEMANTIC_MIR_PROVIDER_ID,
            precision,
            confidence,
            operation.stable_key,
            stable_parts([
                ("status", mir_status_label(operation.status).to_string()),
                (
                    "body_key",
                    self.fact_stable_key(FactFamily::MirBody, operation.body.0),
                ),
                ("ordinal", operation.ordinal.to_string()),
                ("span", span_metadata_value(&operation.span)),
            ]),
        )
    }
}

impl AnalysisDb {
    fn place_metadata(&self, place: &PlaceFact) -> FactMeta {
        let (precision, confidence) = place_status_metadata(place.status);
        fact_meta_from_stable_key(
            &self.stable_keys,
            SEMANTIC_MIR_PROVIDER_ID,
            precision,
            confidence,
            place.stable_key,
            stable_parts([
                ("status", place_status_label(place.status).to_string()),
                ("language", language_label(place.language).to_string()),
                ("path", option_file_path(self, place.file)),
                ("function", option_function_id(place.function)),
                ("projection_count", place.projections.len().to_string()),
            ]),
        )
    }

    fn unsupported_semantic_metadata(&self, row: &UnsupportedSemanticFact) -> FactMeta {
        let (precision, confidence) = mir_status_metadata(row.status);
        fact_meta_from_stable_key(
            &self.stable_keys,
            SEMANTIC_MIR_PROVIDER_ID,
            precision,
            confidence,
            row.stable_key,
            stable_parts([
                ("status", mir_status_label(row.status).to_string()),
                ("language", language_label(row.language).to_string()),
                ("path", self.path_for(row.file)),
                ("span", span_metadata_value(&row.span)),
                ("construct", row.construct.clone()),
                ("source_evidence", row.source_evidence.clone()),
                (
                    "body_key",
                    row.body
                        .map(|body| self.fact_stable_key(FactFamily::MirBody, body.0))
                        .unwrap_or_else(none_value),
                ),
                (
                    "operation_key",
                    row.operation
                        .map(|operation| {
                            self.fact_stable_key(FactFamily::MirOperation, operation.0)
                        })
                        .unwrap_or_else(none_value),
                ),
                (
                    "affected_places",
                    row.affected_places
                        .iter()
                        .map(|place| self.fact_stable_key(FactFamily::Place, place.0))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
            ]),
        )
    }

    #[allow(
        dead_code,
        reason = "Call metadata helpers retained for AnalysisDb tests until dual accessors are removed."
    )]
    fn call_site_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &CallSiteFact,
    ) -> FactMeta {
        let (precision, confidence) = call_status_metadata(fact.status, fact.precision);
        fact_meta_from_stable_key(
            interner,
            CALLS_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("status", call_status_label(fact.status).to_string()),
                (
                    "precision",
                    call_precision_label(fact.precision).to_string(),
                ),
                ("kind", call_syntax_kind_label(fact.kind).to_string()),
                ("language", language_label(fact.language).to_string()),
                ("file_key", self.source_file_key(fact.file)),
                (
                    "caller_key",
                    self.function_key(interner, fact.caller, "", &fact.span),
                ),
                (
                    "owner_symbol_key",
                    fact.owner_symbol
                        .map(|symbol| self.fact_stable_key(FactFamily::Symbol, symbol.0))
                        .unwrap_or_else(none_value),
                ),
                (
                    "body_key",
                    self.fact_stable_key(FactFamily::MirBody, fact.body.0),
                ),
                (
                    "operation_key",
                    self.fact_stable_key(FactFamily::MirOperation, fact.operation.0),
                ),
                ("span", span_metadata_value(&fact.span)),
            ]),
        )
    }

    #[allow(
        dead_code,
        reason = "Call metadata helpers retained for AnalysisDb tests until dual accessors are removed."
    )]
    fn call_target_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &CallTargetFact,
    ) -> FactMeta {
        let (precision, confidence) = call_status_metadata(fact.status, fact.precision);
        fact_meta_from_stable_key(
            interner,
            CALLS_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("status", call_status_label(fact.status).to_string()),
                (
                    "precision",
                    call_precision_label(fact.precision).to_string(),
                ),
                (
                    "algorithm",
                    call_algorithm_label(fact.algorithm).to_string(),
                ),
                (
                    "edge_kind",
                    call_edge_kind_label(fact.edge_kind).to_string(),
                ),
                (
                    "reason",
                    fact.reason
                        .map(call_unresolved_reason_label)
                        .map(str::to_string)
                        .unwrap_or_else(none_value),
                ),
                (
                    "site_key",
                    self.fact_stable_key(FactFamily::CallSite, fact.site.0),
                ),
                (
                    "caller_key",
                    self.fact_stable_key(FactFamily::Function, fact.caller.0),
                ),
                (
                    "target_function_key",
                    fact.target_function
                        .map(|function| self.fact_stable_key(FactFamily::Function, function.0))
                        .unwrap_or_else(none_value),
                ),
                (
                    "target_symbol_key",
                    fact.target_symbol
                        .map(|symbol| self.fact_stable_key(FactFamily::Symbol, symbol.0))
                        .unwrap_or_else(none_value),
                ),
            ]),
        )
    }

    fn refined_call_edge_metadata(&self, fact: &RefinedCallEdgeFact) -> FactMeta {
        let (precision, status_confidence) = call_status_metadata(fact.status, fact.precision);
        let confidence = refined_call_confidence_metadata(fact.confidence, status_confidence);
        let validation = refined_call_validation_metadata(fact.validation);
        fact_meta_from_stable_key_with_validation(
            &self.stable_keys,
            REFINED_CALLS_PROVIDER_ID,
            precision,
            confidence,
            validation,
            fact.stable_key,
            stable_parts([
                ("status", call_status_label(fact.status).to_string()),
                (
                    "precision",
                    call_precision_label(fact.precision).to_string(),
                ),
                (
                    "algorithm",
                    call_algorithm_label(fact.algorithm).to_string(),
                ),
                (
                    "edge_kind",
                    call_edge_kind_label(fact.edge_kind).to_string(),
                ),
                ("tier", refined_call_tier_label(fact.tier).to_string()),
                (
                    "validation",
                    refined_call_validation_label(fact.validation).to_string(),
                ),
                (
                    "reason",
                    fact.reason
                        .map(call_unresolved_reason_label)
                        .map(str::to_string)
                        .unwrap_or_else(none_value),
                ),
                (
                    "site_key",
                    self.fact_stable_key(FactFamily::CallSite, fact.site.0),
                ),
                (
                    "base_target_key",
                    fact.base_target
                        .map(|target| self.fact_stable_key(FactFamily::CallTarget, target.0))
                        .unwrap_or_else(none_value),
                ),
                (
                    "caller_key",
                    self.fact_stable_key(FactFamily::Function, fact.caller.0),
                ),
                (
                    "target_function_key",
                    fact.target_function
                        .map(|function| self.fact_stable_key(FactFamily::Function, function.0))
                        .unwrap_or_else(none_value),
                ),
                (
                    "target_symbol_key",
                    fact.target_symbol
                        .map(|symbol| self.fact_stable_key(FactFamily::Symbol, symbol.0))
                        .unwrap_or_else(none_value),
                ),
                (
                    "synthetic_target",
                    fact.synthetic_target.clone().unwrap_or_else(none_value),
                ),
                ("evidence", fact.evidence.join("\n")),
                ("inputs", fact.input_stable_keys.join("\n")),
            ]),
        )
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    fn data_flow_node_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &DataFlowNodeFact,
    ) -> FactMeta {
        let model = fact
            .model
            .and_then(|id| self.data_flow_models().iter().find(|model| model.id == id));
        let (status, data_flow_precision, data_flow_confidence, data_flow_validation, model_key) =
            model.map_or(
                (
                    DataFlowStatus::Present,
                    DataFlowPrecision::Syntax,
                    DataFlowConfidence::High,
                    DataFlowValidation::Native,
                    none_value(),
                ),
                |model| {
                    (
                        model.status,
                        model.precision,
                        model.confidence,
                        model.validation,
                        interner.resolve(model.stable_key).to_string(),
                    )
                },
            );
        let (precision, status_confidence) = data_flow_status_metadata(status, data_flow_precision);
        let confidence = data_flow_confidence_metadata(data_flow_confidence, status_confidence);
        let validation = data_flow_validation_metadata(data_flow_validation);
        fact_meta_from_stable_key_with_validation(
            interner,
            DATA_FLOW_PROVIDER_ID,
            precision,
            confidence,
            validation,
            fact.stable_key,
            stable_parts([
                ("kind", format!("{:?}", fact.kind)),
                ("status", data_flow_status_label(status).to_string()),
                (
                    "precision",
                    data_flow_precision_label(data_flow_precision).to_string(),
                ),
                (
                    "validation",
                    data_flow_validation_label(data_flow_validation).to_string(),
                ),
                ("language", language_label(fact.language).to_string()),
                (
                    "file_key",
                    fact.file
                        .map(|file| self.source_file_key(file))
                        .unwrap_or_else(none_value),
                ),
                (
                    "function_key",
                    fact.function
                        .map(|function| self.fact_stable_key(FactFamily::Function, function.0))
                        .unwrap_or_else(none_value),
                ),
                (
                    "place_key",
                    fact.place
                        .map(|place| self.fact_stable_key(FactFamily::Place, place.0))
                        .unwrap_or_else(none_value),
                ),
                (
                    "symbol_key",
                    fact.symbol
                        .map(|symbol| self.fact_stable_key(FactFamily::Symbol, symbol.0))
                        .unwrap_or_else(none_value),
                ),
                (
                    "reference_key",
                    fact.reference
                        .map(|reference| self.fact_stable_key(FactFamily::Reference, reference.0))
                        .unwrap_or_else(none_value),
                ),
                (
                    "call_site_key",
                    fact.call_site
                        .map(|site| self.fact_stable_key(FactFamily::CallSite, site.0))
                        .unwrap_or_else(none_value),
                ),
                ("model_key", model_key),
                ("span", option_span_metadata_value(fact.span.as_ref())),
            ]),
        )
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    fn data_flow_edge_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &DataFlowEdgeFact,
    ) -> FactMeta {
        let (precision, status_confidence) = data_flow_status_metadata(fact.status, fact.precision);
        let confidence = data_flow_confidence_metadata(fact.confidence, status_confidence);
        let validation = data_flow_validation_metadata(fact.validation);
        fact_meta_from_stable_key_with_validation(
            interner,
            DATA_FLOW_PROVIDER_ID,
            precision,
            confidence,
            validation,
            fact.stable_key,
            stable_parts([
                ("kind", format!("{:?}", fact.kind)),
                ("algorithm", format!("{:?}", fact.algorithm)),
                ("status", data_flow_status_label(fact.status).to_string()),
                (
                    "precision",
                    data_flow_precision_label(fact.precision).to_string(),
                ),
                (
                    "validation",
                    data_flow_validation_label(fact.validation).to_string(),
                ),
                (
                    "from_key",
                    self.fact_stable_key(FactFamily::DataFlowNode, fact.from.0),
                ),
                (
                    "to_key",
                    self.fact_stable_key(FactFamily::DataFlowNode, fact.to.0),
                ),
                (
                    "call_site_key",
                    fact.call_site
                        .map(|site| self.fact_stable_key(FactFamily::CallSite, site.0))
                        .unwrap_or_else(none_value),
                ),
                (
                    "call_target_key",
                    fact.call_target
                        .map(|target| self.fact_stable_key(FactFamily::CallTarget, target.0))
                        .unwrap_or_else(none_value),
                ),
                (
                    "refined_call_key",
                    fact.refined_call
                        .map(|edge| self.fact_stable_key(FactFamily::RefinedCallEdge, edge.0))
                        .unwrap_or_else(none_value),
                ),
                ("evidence", fact.evidence.join("\n")),
                ("inputs", fact.input_stable_keys.join("\n")),
            ]),
        )
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    fn data_flow_model_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &DataFlowModelFact,
    ) -> FactMeta {
        let (precision, status_confidence) = data_flow_status_metadata(fact.status, fact.precision);
        let confidence = data_flow_confidence_metadata(fact.confidence, status_confidence);
        let validation = data_flow_validation_metadata(fact.validation);
        fact_meta_from_stable_key_with_validation(
            interner,
            DATA_FLOW_PROVIDER_ID,
            precision,
            confidence,
            validation,
            fact.stable_key,
            stable_parts([
                ("kind", format!("{:?}", fact.kind)),
                ("language", language_label(fact.language).to_string()),
                ("provider_id", fact.provider_id.clone()),
                ("model_id", fact.model_id.clone().unwrap_or_else(none_value)),
                (
                    "source_key",
                    fact.source_stable_key.clone().unwrap_or_else(none_value),
                ),
                ("evidence", fact.evidence.join("\n")),
                ("payload_labels", fact.payload_labels.join("\n")),
            ]),
        )
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    fn data_flow_budget_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &DataFlowBudgetFact,
    ) -> FactMeta {
        fact_meta_from_stable_key(
            interner,
            DATA_FLOW_PROVIDER_ID,
            FactPrecision::Heuristic,
            FactConfidence::Medium,
            fact.stable_key,
            stable_parts([
                ("reason", format!("{:?}", fact.reason)),
                ("status", data_flow_status_label(fact.status).to_string()),
                ("limit", fact.limit.to_string()),
                ("observed", fact.observed.to_string()),
            ]),
        )
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    fn evidence_node_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &EvidenceNodeFact,
    ) -> FactMeta {
        let (precision, status_confidence) = evidence_status_metadata(fact.status, fact.precision);
        let confidence = evidence_confidence_metadata(fact.confidence, status_confidence);
        let validation = evidence_validation_metadata(fact.validation);
        fact_meta_from_stable_key_with_validation(
            interner,
            EVIDENCE_PROVIDER_ID,
            precision,
            confidence,
            validation,
            fact.stable_key,
            stable_parts([
                ("kind", format!("{:?}", fact.kind)),
                ("status", evidence_status_label(fact.status).to_string()),
                (
                    "precision",
                    evidence_precision_label(fact.precision).to_string(),
                ),
                (
                    "provenance",
                    evidence_provenance_label(fact.provenance).to_string(),
                ),
                (
                    "validation",
                    evidence_validation_label(fact.validation).to_string(),
                ),
                ("language", language_label(fact.language).to_string()),
                (
                    "file_key",
                    fact.file
                        .map(|file| self.source_file_key(file))
                        .unwrap_or_else(none_value),
                ),
                (
                    "function_key",
                    fact.function
                        .map(|function| self.fact_stable_key(FactFamily::Function, function.0))
                        .unwrap_or_else(none_value),
                ),
                (
                    "place_key",
                    fact.place
                        .map(|place| self.fact_stable_key(FactFamily::Place, place.0))
                        .unwrap_or_else(none_value),
                ),
                (
                    "operation_key",
                    fact.operation
                        .map(|operation| {
                            self.fact_stable_key(FactFamily::MirOperation, operation.0)
                        })
                        .unwrap_or_else(none_value),
                ),
                ("span", option_span_metadata_value(fact.span.as_ref())),
                ("sources", fact.source_fact_stable_keys.join("\n")),
            ]),
        )
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    fn evidence_edge_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &EvidenceEdgeFact,
    ) -> FactMeta {
        let (precision, status_confidence) = evidence_status_metadata(fact.status, fact.precision);
        let confidence = evidence_confidence_metadata(fact.confidence, status_confidence);
        let validation = evidence_validation_metadata(fact.validation);
        fact_meta_from_stable_key_with_validation(
            interner,
            EVIDENCE_PROVIDER_ID,
            precision,
            confidence,
            validation,
            fact.stable_key,
            stable_parts([
                ("kind", format!("{:?}", fact.kind)),
                ("query_mode", format!("{:?}", fact.query_mode)),
                ("status", evidence_status_label(fact.status).to_string()),
                (
                    "precision",
                    evidence_precision_label(fact.precision).to_string(),
                ),
                (
                    "provenance",
                    evidence_provenance_label(fact.provenance).to_string(),
                ),
                (
                    "validation",
                    evidence_validation_label(fact.validation).to_string(),
                ),
                (
                    "from_key",
                    self.fact_stable_key(FactFamily::EvidenceNode, fact.from.0),
                ),
                (
                    "to_key",
                    self.fact_stable_key(FactFamily::EvidenceNode, fact.to.0),
                ),
                (
                    "call_site_key",
                    fact.call_site
                        .map(|site| self.fact_stable_key(FactFamily::CallSite, site.0))
                        .unwrap_or_else(none_value),
                ),
                (
                    "summary_key",
                    fact.summary_stable_key
                        .as_ref()
                        .map_or_else(none_value, |key| key.to_string()),
                ),
                ("sources", fact.source_fact_stable_keys.join("\n")),
            ]),
        )
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    fn evidence_bundle_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &EvidenceBundleFact,
    ) -> FactMeta {
        let (precision, status_confidence) = evidence_status_metadata(fact.status, fact.precision);
        let confidence = evidence_confidence_metadata(fact.confidence, status_confidence);
        let validation = evidence_validation_metadata(fact.validation);
        fact_meta_from_stable_key_with_validation(
            interner,
            EVIDENCE_PROVIDER_ID,
            precision,
            confidence,
            validation,
            fact.stable_key,
            stable_parts([
                (
                    "diagnostic_key",
                    interner.resolve(fact.diagnostic_stable_key).to_string(),
                ),
                ("query_mode", format!("{:?}", fact.query_mode)),
                ("status", evidence_status_label(fact.status).to_string()),
                (
                    "precision",
                    evidence_precision_label(fact.precision).to_string(),
                ),
                ("selected_paths", fact.selected_paths.len().to_string()),
                ("selected_slices", fact.selected_slices.len().to_string()),
                (
                    "replay_key",
                    fact.replay_key.clone().unwrap_or_else(none_value),
                ),
            ]),
        )
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    fn evidence_path_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &EvidencePathFact,
    ) -> FactMeta {
        let (precision, confidence) =
            evidence_status_metadata(fact.status, EvidencePrecision::Heuristic);
        fact_meta_from_stable_key(
            interner,
            EVIDENCE_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("query_mode", format!("{:?}", fact.query_mode)),
                ("status", evidence_status_label(fact.status).to_string()),
                ("rank", fact.rank.to_string()),
                ("node_count", fact.nodes.len().to_string()),
                ("edge_count", fact.edges.len().to_string()),
                ("hidden_node_count", fact.hidden_node_count.to_string()),
            ]),
        )
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    fn evidence_slice_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &EvidenceSliceFact,
    ) -> FactMeta {
        let (precision, confidence) =
            evidence_status_metadata(fact.status, EvidencePrecision::Heuristic);
        fact_meta_from_stable_key(
            interner,
            EVIDENCE_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("query_mode", format!("{:?}", fact.query_mode)),
                ("status", evidence_status_label(fact.status).to_string()),
                ("root_count", fact.root_nodes.len().to_string()),
                ("node_count", fact.nodes.len().to_string()),
                ("edge_count", fact.edges.len().to_string()),
            ]),
        )
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    fn evidence_unknown_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &EvidenceUnknownFact,
    ) -> FactMeta {
        fact_meta_from_stable_key(
            interner,
            EVIDENCE_PROVIDER_ID,
            FactPrecision::Unresolved,
            FactConfidence::Low,
            fact.stable_key,
            stable_parts([
                ("reason", format!("{:?}", fact.reason)),
                ("message", fact.message.clone()),
                ("sources", fact.source_fact_stable_keys.join("\n")),
            ]),
        )
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    fn evidence_omitted_region_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &EvidenceOmittedRegionFact,
    ) -> FactMeta {
        fact_meta_from_stable_key(
            interner,
            EVIDENCE_PROVIDER_ID,
            FactPrecision::Unresolved,
            FactConfidence::Low,
            fact.stable_key,
            stable_parts([
                ("reason", format!("{:?}", fact.reason)),
                ("hidden_node_count", fact.hidden_node_count.to_string()),
                ("hidden_edge_count", fact.hidden_edge_count.to_string()),
                (
                    "budget_label",
                    fact.budget_label.clone().unwrap_or_else(none_value),
                ),
            ]),
        )
    }

    #[allow(
        dead_code,
        reason = "Retained for AnalysisDb until dual accessors are removed."
    )]
    fn evidence_replay_key_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &EvidenceReplayKeyFact,
    ) -> FactMeta {
        fact_meta_from_stable_key(
            interner,
            EVIDENCE_PROVIDER_ID,
            FactPrecision::Heuristic,
            FactConfidence::Medium,
            fact.stable_key,
            stable_parts([
                ("query_mode", format!("{:?}", fact.query_mode)),
                ("graph_schema", fact.graph_schema.clone()),
                ("max_paths", fact.query_budget.max_paths.to_string()),
                ("max_nodes", fact.query_budget.max_nodes.to_string()),
                ("max_edges", fact.query_budget.max_edges.to_string()),
                ("max_depth", fact.query_budget.max_depth.to_string()),
                ("ranking", format!("{:?}", fact.ranking)),
                ("renderer", format!("{:?}", fact.renderer)),
                ("upstream", fact.upstream_digest_keys.join("\n")),
            ]),
        )
    }
}

impl AnalysisDb {
    #[allow(
        dead_code,
        reason = "Call metadata helpers retained for AnalysisDb tests until dual accessors are removed."
    )]
    fn unresolved_call_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &UnresolvedCallFact,
    ) -> FactMeta {
        let (precision, confidence) = call_status_metadata(fact.status, fact.precision);
        fact_meta_from_stable_key(
            interner,
            CALLS_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("status", call_status_label(fact.status).to_string()),
                (
                    "precision",
                    call_precision_label(fact.precision).to_string(),
                ),
                (
                    "algorithm",
                    call_algorithm_label(fact.algorithm).to_string(),
                ),
                (
                    "reason",
                    call_unresolved_reason_label(fact.reason).to_string(),
                ),
                (
                    "site_key",
                    self.fact_stable_key(FactFamily::CallSite, fact.site.0),
                ),
                (
                    "caller_key",
                    self.fact_stable_key(FactFamily::Function, fact.caller.0),
                ),
            ]),
        )
    }

    fn domain_observation_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &DomainObservationFact,
    ) -> FactMeta {
        let (precision, confidence) = domain_status_metadata(fact.status, fact.precision);
        fact_meta_from_stable_key(
            interner,
            POLINT_ABSTRACT_DOMAINS_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("status", fact.status.as_str().to_string()),
                ("precision", fact.precision.as_str().to_string()),
                ("slot", fact.slot.as_str().to_string()),
                ("location", fact.location.as_str().to_string()),
                (
                    "body_key",
                    self.fact_stable_key(FactFamily::MirBody, fact.body.0),
                ),
                (
                    "block",
                    fact.block
                        .map(|block| block.0.to_string())
                        .unwrap_or_else(none_value),
                ),
                (
                    "operation_key",
                    fact.operation
                        .map(|operation| {
                            self.fact_stable_key(FactFamily::MirOperation, operation.0)
                        })
                        .unwrap_or_else(none_value),
                ),
                (
                    "place_key",
                    fact.place
                        .map(|place| self.fact_stable_key(FactFamily::Place, place.0))
                        .unwrap_or_else(none_value),
                ),
                ("value", fact.value.stable_parts().join("\n")),
            ]),
        )
    }

    fn domain_event_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &DomainEventFact,
    ) -> FactMeta {
        let (precision, confidence) = domain_status_metadata(fact.status, fact.precision);
        fact_meta_from_stable_key(
            interner,
            POLINT_ABSTRACT_DOMAINS_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("status", fact.status.as_str().to_string()),
                ("precision", fact.precision.as_str().to_string()),
                (
                    "slot",
                    fact.slot
                        .map(|slot| slot.as_str().to_string())
                        .unwrap_or_else(none_value),
                ),
                ("reason", fact.reason.clone()),
                (
                    "body_key",
                    self.fact_stable_key(FactFamily::MirBody, fact.body.0),
                ),
                (
                    "block",
                    fact.block
                        .map(|block| block.0.to_string())
                        .unwrap_or_else(none_value),
                ),
                (
                    "operation_key",
                    fact.operation
                        .map(|operation| {
                            self.fact_stable_key(FactFamily::MirOperation, operation.0)
                        })
                        .unwrap_or_else(none_value),
                ),
            ]),
        )
    }

    #[allow(
        dead_code,
        reason = "CFG metadata helpers retained for AnalysisDb tests until dual accessors are removed."
    )]
    fn cfg_function_metadata(&self, fact: &CfgFunctionFact) -> FactMeta {
        let (precision, confidence) = cfg_status_metadata(fact.status, fact.precision);
        fact_meta_from_stable_key(
            &self.stable_keys,
            CFG_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("status", cfg_status_label(fact.status).to_string()),
                ("precision", cfg_precision_label(fact.precision).to_string()),
                (
                    "body_key",
                    self.fact_stable_key(FactFamily::MirBody, fact.body.0),
                ),
                ("language", language_label(fact.language).to_string()),
                ("path", self.path_for(fact.file)),
                ("span", span_metadata_value(&fact.span)),
            ]),
        )
    }

    #[allow(
        dead_code,
        reason = "CFG metadata helpers retained for AnalysisDb tests until dual accessors are removed."
    )]
    fn cfg_node_metadata(&self, fact: &CfgNodeFact) -> FactMeta {
        let (precision, confidence) = cfg_status_metadata(fact.status, fact.precision);
        fact_meta_from_stable_key(
            &self.stable_keys,
            CFG_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("status", cfg_status_label(fact.status).to_string()),
                ("precision", cfg_precision_label(fact.precision).to_string()),
                ("kind", cfg_node_kind_label(fact.kind).to_string()),
                (
                    "function_key",
                    self.fact_stable_key(FactFamily::CfgFunction, fact.cfg_function.0),
                ),
                ("operation_ordinal", fact.operation_ordinal.to_string()),
                ("span", option_span_metadata_value(fact.span.as_ref())),
            ]),
        )
    }

    #[allow(
        dead_code,
        reason = "CFG metadata helpers retained for AnalysisDb tests until dual accessors are removed."
    )]
    fn cfg_block_metadata(&self, fact: &BasicBlockFact) -> FactMeta {
        let (precision, confidence) = cfg_status_metadata(fact.status, fact.precision);
        fact_meta_from_stable_key(
            &self.stable_keys,
            CFG_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("status", cfg_status_label(fact.status).to_string()),
                ("precision", cfg_precision_label(fact.precision).to_string()),
                ("kind", basic_block_kind_label(fact.kind).to_string()),
                (
                    "function_key",
                    self.fact_stable_key(FactFamily::CfgFunction, fact.cfg_function.0),
                ),
                ("reachable", fact.reachable.to_string()),
                ("reverse_postorder", fact.reverse_postorder.to_string()),
            ]),
        )
    }

    #[allow(
        dead_code,
        reason = "CFG metadata helpers retained for AnalysisDb tests until dual accessors are removed."
    )]
    fn cfg_edge_metadata(&self, fact: &CfgEdgeFact) -> FactMeta {
        let (precision, confidence) = cfg_status_metadata(fact.status, fact.precision);
        fact_meta_from_stable_key(
            &self.stable_keys,
            CFG_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("status", cfg_status_label(fact.status).to_string()),
                ("precision", cfg_precision_label(fact.precision).to_string()),
                ("view", cfg_view_label(fact.view).to_string()),
                ("kind", cfg_edge_kind_label(fact.kind).to_string()),
                (
                    "function_key",
                    self.fact_stable_key(FactFamily::CfgFunction, fact.cfg_function.0),
                ),
                ("from_block", fact.from_block.0.to_string()),
                ("to_block", fact.to_block.0.to_string()),
            ]),
        )
    }

    #[allow(
        dead_code,
        reason = "CFG metadata helpers retained for AnalysisDb tests until dual accessors are removed."
    )]
    fn cfg_reachability_metadata(&self, fact: &ReachabilityFact) -> FactMeta {
        let (precision, confidence) = cfg_status_metadata(fact.status, fact.precision);
        fact_meta_from_stable_key(
            &self.stable_keys,
            CFG_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("status", cfg_status_label(fact.status).to_string()),
                ("precision", cfg_precision_label(fact.precision).to_string()),
                ("view", cfg_view_label(fact.view).to_string()),
                ("block", fact.block.0.to_string()),
                ("reachable", fact.reachable.to_string()),
            ]),
        )
    }

    #[allow(
        dead_code,
        reason = "CFG metadata helpers retained for AnalysisDb tests until dual accessors are removed."
    )]
    fn cfg_dominator_metadata(&self, fact: &DominatorFact) -> FactMeta {
        let (precision, confidence) = cfg_status_metadata(fact.status, fact.precision);
        fact_meta_from_stable_key(
            &self.stable_keys,
            CFG_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("status", cfg_status_label(fact.status).to_string()),
                ("precision", cfg_precision_label(fact.precision).to_string()),
                ("view", cfg_view_label(fact.view).to_string()),
                ("dominator", fact.dominator.0.to_string()),
                ("dominated", fact.dominated.0.to_string()),
                ("immediate", fact.immediate.to_string()),
            ]),
        )
    }

    #[allow(
        dead_code,
        reason = "CFG metadata helpers retained for AnalysisDb tests until dual accessors are removed."
    )]
    fn cfg_postdominator_metadata(&self, fact: &PostDominatorFact) -> FactMeta {
        let (precision, confidence) = cfg_status_metadata(fact.status, fact.precision);
        fact_meta_from_stable_key(
            &self.stable_keys,
            CFG_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("status", cfg_status_label(fact.status).to_string()),
                ("precision", cfg_precision_label(fact.precision).to_string()),
                ("view", cfg_view_label(fact.view).to_string()),
                ("postdominator", fact.postdominator.0.to_string()),
                ("postdominated", fact.postdominated.0.to_string()),
                ("immediate", fact.immediate.to_string()),
            ]),
        )
    }

    #[allow(
        dead_code,
        reason = "CFG metadata helpers retained for AnalysisDb tests until dual accessors are removed."
    )]
    fn cfg_control_dependence_metadata(&self, fact: &ControlDependenceFact) -> FactMeta {
        let (precision, confidence) = cfg_status_metadata(fact.status, fact.precision);
        fact_meta_from_stable_key(
            &self.stable_keys,
            CFG_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("status", cfg_status_label(fact.status).to_string()),
                ("precision", cfg_precision_label(fact.precision).to_string()),
                ("view", cfg_view_label(fact.view).to_string()),
                ("edge", fact.controlling_edge.0.to_string()),
                (
                    "edge_kind",
                    cfg_edge_kind_label(fact.controlling_edge_kind).to_string(),
                ),
                ("controlled_block", fact.controlled_block.0.to_string()),
            ]),
        )
    }

    #[allow(
        dead_code,
        reason = "CFG metadata helpers retained for AnalysisDb tests until dual accessors are removed."
    )]
    fn unsupported_control_flow_metadata(&self, fact: &UnsupportedControlFlowFact) -> FactMeta {
        let (precision, confidence) = cfg_status_metadata(fact.status, fact.precision);
        fact_meta_from_stable_key(
            &self.stable_keys,
            CFG_PROVIDER_ID,
            precision,
            confidence,
            fact.stable_key,
            stable_parts([
                ("status", cfg_status_label(fact.status).to_string()),
                ("precision", cfg_precision_label(fact.precision).to_string()),
                ("language", language_label(fact.language).to_string()),
                ("path", self.path_for(fact.file)),
                ("span", span_metadata_value(&fact.span)),
                ("construct", fact.construct.clone()),
                ("source_evidence", fact.source_evidence.clone()),
            ]),
        )
    }

    fn fact_stable_key(&self, family: FactFamily, run_id: u64) -> String {
        self.metadata_for(FactRef::new(family, run_id))
            .map(|metadata| self.resolve_stable_key(metadata.stable_key).to_string())
            .unwrap_or_else(|| format!("<missing:{}:{run_id}>", family.label()))
    }

    fn source_file_key(&self, file: FileId) -> String {
        self.metadata_for(FactRef::new(FactFamily::SourceFile, u64::from(file.0)))
            .map(|metadata| self.resolve_stable_key(metadata.stable_key).to_string())
            .unwrap_or_else(|| self.path_for(file).replace('\\', "/"))
    }

    fn option_source_file_key(&self, file: Option<FileId>) -> String {
        file.map(|file| self.source_file_key(file))
            .unwrap_or_else(none_value)
    }

    fn function_key(
        &self,
        interner: &crate::core::StableKeyInterner,
        function: FunctionId,
        name: &str,
        span: &Span,
    ) -> String {
        self.metadata_for(FactRef::new(FactFamily::Function, function.0))
            .map(|metadata| interner.resolve(metadata.stable_key).to_string())
            .unwrap_or_else(|| {
                stable_key_text_from_parts(
                    FactFamily::Function,
                    &[
                        ("path", self.path_for(span.file)),
                        ("name", name.to_string()),
                        ("span", span_metadata_value(span)),
                    ],
                )
            })
    }

    fn ts_component_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &TsComponentFact,
    ) -> FactMeta {
        let span = span_metadata_value(&fact.span);
        let function = option_function_id(fact.function);
        fact_meta_from_borrowed_parts(
            interner,
            FactFamily::TsComponent,
            TS_SYNTAX_PROVIDER_ID,
            FactPrecision::Heuristic,
            FactConfidence::Medium,
            stable_parts([
                ("path", self.path_str_for(fact.file)),
                ("name", fact.name.as_str()),
                ("span", span.as_str()),
            ]),
            stable_parts([("function", function.as_str())]),
        )
    }

    fn ts_class_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &TsClassFact,
    ) -> FactMeta {
        let span = span_metadata_value(&fact.span);
        fact_meta_from_borrowed_parts(
            interner,
            FactFamily::TsClass,
            TS_SYNTAX_PROVIDER_ID,
            FactPrecision::Syntax,
            FactConfidence::High,
            stable_parts([
                ("path", self.path_str_for(fact.file)),
                ("name", fact.name.as_str()),
                ("span", span.as_str()),
            ]),
            stable_parts([
                ("is_exported", bool_label(fact.is_exported)),
                ("is_component_like", bool_label(fact.is_component_like)),
            ]),
        )
    }

    fn string_literal_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &StringLiteralFact,
    ) -> FactMeta {
        let span = span_metadata_value(&fact.span);
        fact_meta_from_borrowed_parts(
            interner,
            FactFamily::StringLiteral,
            syntax_provider_for_language(fact.language),
            FactPrecision::Syntax,
            FactConfidence::High,
            stable_parts([
                ("path", self.path_str_for(fact.file)),
                ("language", language_label(fact.language)),
                ("value", fact.value.as_str()),
                ("span", span.as_str()),
            ]),
            stable_parts([]),
        )
    }

    fn jsx_attribute_metadata(
        &self,
        interner: &crate::core::StableKeyInterner,
        fact: &JsxAttributeFact,
    ) -> FactMeta {
        let value = option_string(fact.value.as_deref());
        let span = span_metadata_value(&fact.span);
        fact_meta_from_borrowed_parts(
            interner,
            FactFamily::JsxAttribute,
            TS_SYNTAX_PROVIDER_ID,
            FactPrecision::Syntax,
            FactConfidence::High,
            stable_parts([
                ("path", self.path_str_for(fact.file)),
                ("name", fact.name.as_str()),
                ("value", value.as_str()),
                ("span", span.as_str()),
            ]),
            stable_parts([]),
        )
    }
}

fn option_file_path(db: &AnalysisDb, file: Option<FileId>) -> String {
    file.map(|file| db.path_for(file))
        .unwrap_or_else(none_value)
}

impl AnalysisDb {
    fn refresh_reachability_metadata(&mut self) {
        self.fact_meta.remove_family(FactFamily::Reachability);
        let interner = self.stable_key_interner();
        let roots = self.reachability_roots().to_vec();
        for root in &roots {
            let precision = match root.precision {
                // Reachability combines roots with setup-dependent call and entrypoint
                // facts, so even a statically discovered root cannot exceed the
                // provider's setup-aware precision ceiling.
                RootPrecision::ResolvedStatic | RootPrecision::SetupAware => {
                    FactPrecision::SetupAware
                }
                RootPrecision::Heuristic | RootPrecision::Conservative => FactPrecision::Heuristic,
                RootPrecision::Unknown => FactPrecision::Unresolved,
            };
            let confidence = match root.status {
                RootStatus::Resolved => FactConfidence::High,
                RootStatus::Partial => FactConfidence::Medium,
                RootStatus::Unresolved | RootStatus::SetupMissing | RootStatus::Unsupported => {
                    FactConfidence::Low
                }
            };
            let metadata = fact_meta_from_stable_key(
                &interner,
                "polint.reachability",
                precision,
                confidence,
                root.stable_key,
                stable_parts([
                    ("kind", root.kind.as_str().to_string()),
                    ("language", language_label(root.language).to_string()),
                    (
                        "target_function",
                        self.fact_stable_key(FactFamily::Function, root.target_function.0),
                    ),
                    (
                        "target_symbol",
                        root.target_symbol
                            .map(|id| self.fact_stable_key(FactFamily::Symbol, id.0))
                            .unwrap_or_else(none_value),
                    ),
                    (
                        "originating_entrypoint",
                        root.originating_entrypoint
                            .map(|id| self.fact_stable_key(FactFamily::Entrypoint, id.0))
                            .unwrap_or_else(none_value),
                    ),
                    ("path", self.path_for(root.file)),
                    ("span", span_metadata_value(&root.span)),
                    ("precision", root.precision.as_str().to_string()),
                    ("provenance", root.provenance.as_str().to_string()),
                    ("status", root.status.as_str().to_string()),
                ]),
            );
            self.record_fact_meta(FactFamily::Reachability, root.id.0, metadata);
        }
        self.finish_fact_meta_insertions(&[FactFamily::Reachability]);
    }

    fn refresh_solver_metadata(&mut self) {
        self.fact_meta.remove_family(FactFamily::SolverDerivedEdge);
        let interner = self.stable_key_interner();
        let edges = self.solver_derived_edges().to_vec();
        for edge in &edges {
            let precision = crate::analysis_neutral::solver::facts::derived_edge_precision_ceiling(
                edge.precision,
            );
            let confidence = match edge.status {
                PointsToStatus::Present => FactConfidence::High,
                PointsToStatus::BudgetExceeded => FactConfidence::Medium,
                PointsToStatus::Unknown
                | PointsToStatus::Unsupported
                | PointsToStatus::SetupMissing => FactConfidence::Low,
            };
            let metadata = fact_meta_from_stable_key(
                &interner,
                "polint.solver",
                precision,
                confidence,
                edge.stable_key,
                stable_parts([
                    ("status", format!("{:?}", edge.status)),
                    ("precision", format!("{:?}", edge.precision)),
                    (
                        "provenance",
                        serde_json::to_string(&edge.provenance.stable_payload(&interner))
                            .expect("solver provenance stable payload serializes"),
                    ),
                ]),
            );
            self.record_fact_meta(FactFamily::SolverDerivedEdge, edge.id.0, metadata);
        }
        self.finish_fact_meta_insertions(&[FactFamily::SolverDerivedEdge]);
    }

    fn refresh_semantic_graph_metadata(&mut self) {
        self.fact_meta.remove_family(FactFamily::SemanticGraph);
        let interner = self.stable_key_interner();
        let mut stable_keys = self
            .semantic_nodes()
            .iter()
            .map(|row| {
                format!(
                    "node|{}|{}|{}",
                    interner.resolve(row.stable_key),
                    row.kind.as_str(),
                    row.precision.as_str()
                )
            })
            .collect::<Vec<_>>();
        stable_keys.extend(self.semantic_edges().iter().map(|row| {
            format!(
                "edge|{}|{}|{}",
                interner.resolve(row.stable_key),
                row.kind.as_str(),
                row.precision.as_str()
            )
        }));
        stable_keys.extend(self.semantic_constraints().iter().map(|row| {
            format!(
                "constraint|{}|{}|{:?}|{:?}",
                interner.resolve(row.stable_key),
                row.kind.as_str(),
                row.status,
                row.precision
            )
        }));
        stable_keys.sort();
        let aggregate_text = stable_key_text_from_parts(
            FactFamily::SemanticGraph,
            &[("rows", stable_keys.join("\n"))],
        );
        let aggregate_key = interner.intern(aggregate_text);
        let metadata = fact_meta_from_stable_key(
            &interner,
            "polint.semantic_graph",
            FactPrecision::Heuristic,
            FactConfidence::Medium,
            aggregate_key,
            stable_parts([
                ("nodes", self.semantic_nodes().len().to_string()),
                ("edges", self.semantic_edges().len().to_string()),
                ("constraints", self.semantic_constraints().len().to_string()),
                ("stable_rows", stable_keys.join("\n")),
            ]),
        );
        self.record_fact_meta(FactFamily::SemanticGraph, 0, metadata);
        self.finish_fact_meta_insertions(&[FactFamily::SemanticGraph]);
    }
}

impl crate::analysis_neutral::AnalysisHost for AnalysisDb {
    fn semantic_scopes(&self) -> &[ScopeFact] {
        self.scopes()
    }

    /// Route the trait's symbol and reference lookups to the indexed inherent
    /// methods.
    ///
    /// Every lowerer is generic over `impl AnalysisHost`, so it reached the trait
    /// defaults in `analysis_neutral/host.rs`, which filter the whole reference
    /// and definition tables once per call — the closure-capture scan in each
    /// lowerer calls them once per closure literal. `AnalysisDb` has had the
    /// indexes all along (`references_by_file`, `definitions_by_symbol`); only
    /// the inherent methods reached them, and an inherent method is out of scope
    /// inside a generic function.
    ///
    /// Identity: `definitions_by_symbol` is built by walking the definition table
    /// in order, so the override yields the same rows in the same order as the
    /// default's filter. `references_by_file` is sorted by `ReferenceId`, which
    /// the default's table order need not follow; every trait-generic caller
    /// either folds the rows into a `BTreeSet` (both lowerers' closure captures)
    /// or accepts exactly one match and rejects two (`calls/direct.rs`'s
    /// `unique_reference_by_site_name`), so no output depends on the order.
    /// The defaults stay for `LocalAnalysisDb` and the two `LocalFactDb` test
    /// databases, which carry no index.
    fn references_for_file(
        &self,
        file: crate::internal_core::FileId,
    ) -> Vec<&crate::analysis_api::ReferenceFact> {
        AnalysisDb::references_for_file(self, file).collect()
    }

    fn definitions_for_symbol(
        &self,
        symbol: SymbolId,
    ) -> Box<dyn Iterator<Item = &crate::analysis_api::DefinitionFact> + '_> {
        Box::new(AnalysisDb::definitions_for_symbol(self, symbol))
    }

    fn definition_for_symbol(
        &self,
        symbol: SymbolId,
    ) -> Option<&crate::analysis_api::DefinitionFact> {
        AnalysisDb::definition_for_symbol(self, symbol)
    }

    fn replace_summary_facts(&mut self, output: SummaryOutput) {
        AnalysisDb::replace_summary_facts(self, output);
    }

    /// Route the SCC closure's bulk refresh to `AnalysisDb`'s digest recipe.
    ///
    /// `close_summaries_by_scc` is generic over `impl AnalysisHost`, so inherent
    /// methods are out of scope inside it: it calls this trait method, and
    /// without this override the trait default ran, which re-records every
    /// summary and event row as the verbatim text `summary:<SummaryId>` /
    /// `summary-event:<SummaryEventId>`. That is what the five summary families'
    /// `FactMeta::payload_digest` column held on every scan whose closure
    /// updated anything: not a digest of the fact's parts, unmoved by any parts
    /// change and moved by any id reassignment. The default stays for
    /// `LocalAnalysisDb` and the language-local fact databases, which have no
    /// recipe of their own.
    ///
    /// No provider output digest folds `FactMeta::payload_digest`, so this moves
    /// no `digest=` anywhere; it moves the second column of those five families'
    /// fact rows, and nothing else.
    fn refresh_summary_metadata_after_bulk_update(&mut self) {
        self.refresh_summary_metadata();
    }

    fn replace_call_facts(&mut self, output: CallOutput) -> Result<(), AnalysisError> {
        AnalysisDb::replace_call_facts(self, output)
    }

    fn replace_cfg_facts(&mut self, output: CfgOutput) -> Result<(), AnalysisError> {
        AnalysisDb::replace_cfg_facts(self, output)
    }

    fn replace_semantic_mir(&mut self, output: MirOutput) -> Result<(), AnalysisError> {
        AnalysisDb::replace_semantic_mir(self, output)
    }

    fn replace_refined_call_facts(
        &mut self,
        output: RefinedCallOutput,
    ) -> Result<(), AnalysisError> {
        AnalysisDb::replace_refined_call_facts(self, output)
    }

    fn replace_entrypoint_facts(&mut self, output: EntrypointOutput) -> Result<(), AnalysisError> {
        AnalysisDb::replace_entrypoint_facts(self, output)
    }

    fn replace_abstract_domain_facts(&mut self, output: DomainOutput) {
        AnalysisDb::replace_abstract_domain_facts(self, output);
    }

    fn replace_data_flow_facts(&mut self, output: DataFlowOutput) -> Result<(), AnalysisError> {
        AnalysisDb::replace_data_flow_facts(self, output)
    }

    fn replace_evidence_facts(&mut self, output: EvidenceOutput) -> Result<(), AnalysisError> {
        AnalysisDb::replace_evidence_facts(self, output)
    }

    fn replace_reachability_facts(
        &mut self,
        output: ReachabilityProviderOutput,
    ) -> Result<(), AnalysisError> {
        AnalysisDb::replace_reachability_facts(self, output)
    }

    fn replace_solver_facts(&mut self, output: SolverOutput) -> Result<(), AnalysisError> {
        AnalysisDb::replace_solver_facts(self, output)
    }

    fn replace_type_value_alias_facts(&mut self, output: TypeValueAliasOutput) {
        AnalysisDb::replace_type_value_alias_facts(self, output);
    }

    fn replace_extension_facts(&mut self, output: ExtensionOutput) {
        AnalysisDb::replace_extension_facts(self, output);
    }

    fn replace_semantic_graph_facts(
        &mut self,
        output: SemanticGraphOutput,
    ) -> Result<(), AnalysisError> {
        AnalysisDb::replace_semantic_graph_facts(self, output)
    }

    fn replace_identity_facts(
        &mut self,
        output: IdentityProviderOutput,
    ) -> Result<(), AnalysisError> {
        AnalysisDb::replace_identity_facts(self, output)
    }

    fn replace_normalized_type_value_alias_facts(&mut self, output: TypeValueAliasOutput) {
        AnalysisDb::replace_normalized_type_value_alias_facts(self, output);
    }
}
