//! Owner-side typed accessors over [`crate::analysis_api::FactDatabase`].
//!
//! Concrete composition roots (facade `AnalysisDb`, [`crate::analysis_neutral::LocalAnalysisDb`])
//! implement [`FactDatabase`]; analysis algorithms call these default methods so
//! they never name the facade database type.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::analysis_api::{
    FactConfidence, FactDatabase, FactFamily, FactMeta, FactMetaStore, FactPrecision, FactRef,
    FactStore, ValidationStatus, stable_key_text_from_parts,
};
use crate::internal_core::{Language, StableKeyId, StableKeyInterner};

use crate::analysis_neutral::summaries::facts::SummaryDomainKind;
use crate::analysis_neutral::symbol_graph::semantic::ScopeFact;
use crate::analysis_neutral::{
    CALLS_PROVIDER_ID, POLINT_ABSTRACT_DOMAINS_PROVIDER_ID, POLINT_DIRECT_SUMMARIES_PROVIDER_ID,
};

use crate::analysis_neutral::access_paths::facts::AccessPathFact;
use crate::analysis_neutral::access_paths::store::AccessPathStore;
use crate::analysis_neutral::aliases::facts::AliasAnswerFact;
use crate::analysis_neutral::aliases::store::AliasStore;
use crate::analysis_neutral::calls::facts::{
    CallAlgorithm, CallEdgeKind, CallPrecision, CallSiteFact, CallSyntaxKind, CallTargetFact,
    CallTargetStatus, UnresolvedCallFact, UnresolvedCallReason,
};
use crate::analysis_neutral::calls::store::{CallOutput, CallStore};
use crate::analysis_neutral::cfg::facts::{
    BasicBlockFact, CfgEdgeFact, CfgFunctionFact, CfgNodeFact, ControlDependenceFact,
    DominatorFact, PostDominatorFact, ReachabilityFact, UnsupportedControlFlowFact,
};
use crate::analysis_neutral::cfg::store::CfgOutput;
use crate::analysis_neutral::data_flow::facts::{
    DataFlowBudgetFact, DataFlowEdgeFact, DataFlowModelFact, DataFlowNodeFact,
};
use crate::analysis_neutral::data_flow::store::{DataFlowOutput, DataFlowStore};
use crate::analysis_neutral::domains::facts::{DomainEventFact, DomainObservationFact};
use crate::analysis_neutral::domains::store::{DomainOutput, DomainStore};
use crate::analysis_neutral::entrypoints::facts::{
    EntrypointFact, FrameworkDispatchEdgeFact, TrustBoundaryFact, UnresolvedFrameworkFact,
};
use crate::analysis_neutral::entrypoints::store::{EntrypointOutput, EntrypointStore};
use crate::analysis_neutral::error::AnalysisError;
use crate::analysis_neutral::evidence::facts::{
    EvidenceBundleFact, EvidenceEdgeFact, EvidenceNodeFact, EvidenceOmittedRegionFact,
    EvidencePathFact, EvidenceReplayKeyFact, EvidenceSliceFact, EvidenceUnknownFact,
};
use crate::analysis_neutral::evidence::store::{EvidenceOutput, EvidenceStore};
use crate::analysis_neutral::extensions::store::{
    AcceptedExtensionFact, ExtensionActivationRow, ExtensionOutput, RejectedExtensionFact,
};
use crate::analysis_neutral::fact_store::{
    ACCESS_PATH_STORE_FAMILY, ADAPTATION_STORE_FAMILY, ALIAS_STORE_FAMILY, AdaptationFactStore,
    CALL_STORE_FAMILY, CFG_STORE_FAMILY, CfgFactStore, DATA_FLOW_STORE_FAMILY, DOMAIN_STORE_FAMILY,
    ENTRYPOINT_STORE_FAMILY, EVIDENCE_STORE_FAMILY, EXTENSION_STORE_FAMILY, ExtensionFactStore,
    IDENTITY_STORE_FAMILY, POINTS_TO_STORE_FAMILY, REACHABILITY_STORE_FAMILY,
    REFINED_CALL_STORE_FAMILY, SEMANTIC_GRAPH_STORE_FAMILY, SEMANTIC_MIR_STORE_FAMILY,
    SOLVER_STORE_FAMILY, SUMMARY_STORE_FAMILY, TYPE_STORE_FAMILY, VALUE_STORE_FAMILY,
};
use crate::analysis_neutral::identity::facts::IdentityRecord;
use crate::analysis_neutral::identity::store::{IdentityProviderOutput, IdentityStore};
use crate::analysis_neutral::ids::CallSiteId;
use crate::analysis_neutral::local_db::LocalAnalysisDb;
use crate::analysis_neutral::mir_body::MirOutput;
use crate::analysis_neutral::mir_body::{MirBlock, MirBody};
use crate::analysis_neutral::mir_body::{MirStatement, MirTerminator};
use crate::analysis_neutral::mir_op::MirOperation;
use crate::analysis_neutral::mir_op::UnsupportedSemanticFact;
use crate::analysis_neutral::places::{PlaceFact, PlaceTypeFact};
use crate::analysis_neutral::points_to::facts::{PointsToConstraintFact, PointsToSetFact};
use crate::analysis_neutral::points_to::store::PointsToStore;
use crate::analysis_neutral::reachability::facts::ReachabilityRootFact;
use crate::analysis_neutral::reachability::store::{ReachabilityProviderOutput, ReachabilityStore};
use crate::analysis_neutral::refined_calls::facts::{
    RefinedCallConfidence, RefinedCallEdgeFact, RefinedCallTier, RefinedCallValidation,
};
use crate::analysis_neutral::refined_calls::store::{RefinedCallOutput, RefinedCallStore};
use crate::analysis_neutral::semantic_graph::constraints::ConstraintFact;
use crate::analysis_neutral::semantic_graph::facts::{SemanticEdgeFact, SemanticNodeFact};
use crate::analysis_neutral::semantic_graph::store::{SemanticGraphOutput, SemanticGraphStore};
use crate::analysis_neutral::solver::budget::BudgetStatus;
use crate::analysis_neutral::solver::facts::DerivedEdgeFact;
use crate::analysis_neutral::solver::store::{SolverOutput, SolverStore};
use crate::analysis_neutral::store::SemanticStore;
use crate::analysis_neutral::summaries::facts::{SummaryEventFact, SummaryFact};
use crate::analysis_neutral::summaries::store::{SummaryOutput, SummaryStore};
use crate::analysis_neutral::types::facts::{NarrowedTypeFact, TypeFact};
use crate::analysis_neutral::types::store::{TypeStore, TypeValueAliasOutput};
use crate::analysis_neutral::values::facts::{AllocationTokenFact, ValueFact};
use crate::analysis_neutral::values::store::ValueStore;

fn store_ref<T: FactStore + 'static>(db: &(impl FactDatabase + ?Sized), family: FactFamily) -> &T {
    db.store(family)
        .and_then(|entry| entry.as_any().downcast_ref::<T>())
        .unwrap_or_else(|| panic!("analysis store {family:?} is installed"))
}

fn store_mut<T: FactStore + 'static>(
    db: &mut (impl FactDatabase + ?Sized),
    family: FactFamily,
) -> &mut T {
    db.store_mut(family)
        .and_then(|entry| entry.as_any_mut().downcast_mut::<T>())
        .unwrap_or_else(|| panic!("analysis store {family:?} is installed"))
}

/// Typed analysis store access and replace helpers for any [`FactDatabase`].
pub trait AnalysisHost: FactDatabase {
    fn resolve_stable_key(&self, id: StableKeyId) -> Arc<str> {
        self.stable_key_interner().resolve(id)
    }

    fn replace_symbol_graph_facts(
        &mut self,
        symbols: Vec<crate::analysis_api::SymbolFact>,
        definitions: Vec<crate::analysis_api::DefinitionFact>,
        references: Vec<crate::analysis_api::ReferenceFact>,
    ) {
        FactDatabase::replace_symbol_facts(self, symbols, definitions, references);
    }

    fn replace_semantic_imports(&mut self, imports: Vec<crate::analysis_api::SemanticImportFact>) {
        FactDatabase::replace_semantic_imports(self, imports);
    }

    fn references_for_file(
        &self,
        file: crate::internal_core::FileId,
    ) -> Vec<&crate::analysis_api::ReferenceFact> {
        FactDatabase::references(self)
            .iter()
            .filter(|reference| reference.file == Some(file))
            .collect()
    }

    /// Every definition of a symbol, in database order.
    ///
    /// The default filters the whole definition table, which is the scan
    /// `definition_for_symbol` used to perform inline. `AnalysisDb` overrides it
    /// with its `definitions_by_symbol` index; that index is built by walking the
    /// same table in order and pushing positions, so both orders are the table's
    /// and the first-primary-else-first rule below picks the same row either way.
    fn definitions_for_symbol(
        &self,
        symbol: crate::internal_core::SymbolId,
    ) -> Box<dyn Iterator<Item = &crate::analysis_api::DefinitionFact> + '_> {
        Box::new(
            FactDatabase::definitions(self)
                .iter()
                .filter(move |definition| definition.symbol == symbol),
        )
    }

    /// Return the primary definition for a symbol, falling back to the first
    /// definition when no primary marker is present.
    fn definition_for_symbol(
        &self,
        symbol: crate::internal_core::SymbolId,
    ) -> Option<&crate::analysis_api::DefinitionFact> {
        let mut definitions = self.definitions_for_symbol(symbol);
        let first = definitions.next();
        first
            .filter(|definition| definition.is_primary)
            .or_else(|| definitions.find(|definition| definition.is_primary))
            .or(first)
    }

    fn fact_meta_mut_for_test(&mut self) -> &mut FactMetaStore {
        self.fact_meta_mut()
    }

    fn cfg_store(&self) -> &CfgFactStore {
        store_ref(self, CFG_STORE_FAMILY)
    }
    fn cfg_store_mut(&mut self) -> &mut CfgFactStore {
        store_mut(self, CFG_STORE_FAMILY)
    }
    fn calls_store(&self) -> &CallStore {
        store_ref(self, CALL_STORE_FAMILY)
    }
    fn calls_store_mut(&mut self) -> &mut CallStore {
        store_mut(self, CALL_STORE_FAMILY)
    }
    fn summary_store(&self) -> &SummaryStore {
        store_ref(self, SUMMARY_STORE_FAMILY)
    }
    fn summary_store_mut(&mut self) -> &mut SummaryStore {
        store_mut(self, SUMMARY_STORE_FAMILY)
    }
    fn semantic_mir_store(&self) -> &SemanticStore {
        store_ref(self, SEMANTIC_MIR_STORE_FAMILY)
    }
    fn semantic_mir_store_mut(&mut self) -> &mut SemanticStore {
        store_mut(self, SEMANTIC_MIR_STORE_FAMILY)
    }
    fn identity_store_inner(&self) -> &IdentityStore {
        store_ref(self, IDENTITY_STORE_FAMILY)
    }
    fn refined_call_store_inner(&self) -> &RefinedCallStore {
        store_ref(self, REFINED_CALL_STORE_FAMILY)
    }
    fn data_flow_store_inner(&self) -> &DataFlowStore {
        store_ref(self, DATA_FLOW_STORE_FAMILY)
    }
    fn evidence_store_inner(&self) -> &EvidenceStore {
        store_ref(self, EVIDENCE_STORE_FAMILY)
    }
    fn domain_store_inner(&self) -> &DomainStore {
        store_ref(self, DOMAIN_STORE_FAMILY)
    }
    fn entrypoint_store_inner(&self) -> &EntrypointStore {
        store_ref(self, ENTRYPOINT_STORE_FAMILY)
    }
    fn type_store_inner(&self) -> &TypeStore {
        store_ref(self, TYPE_STORE_FAMILY)
    }
    fn value_store_inner(&self) -> &ValueStore {
        store_ref(self, VALUE_STORE_FAMILY)
    }
    fn access_path_store_inner(&self) -> &AccessPathStore {
        store_ref(self, ACCESS_PATH_STORE_FAMILY)
    }
    fn points_to_store_inner(&self) -> &PointsToStore {
        store_ref(self, POINTS_TO_STORE_FAMILY)
    }
    fn alias_store_inner(&self) -> &AliasStore {
        store_ref(self, ALIAS_STORE_FAMILY)
    }
    fn extension_store_inner(&self) -> &ExtensionFactStore {
        store_ref(self, EXTENSION_STORE_FAMILY)
    }
    fn adaptation_store_inner(&self) -> &AdaptationFactStore {
        store_ref(self, ADAPTATION_STORE_FAMILY)
    }
    fn reachability_store_inner(&self) -> &ReachabilityStore {
        store_ref(self, REACHABILITY_STORE_FAMILY)
    }
    fn semantic_graph_store_inner(&self) -> &SemanticGraphStore {
        store_ref(self, SEMANTIC_GRAPH_STORE_FAMILY)
    }
    fn solver_store_inner(&self) -> &SolverStore {
        store_ref(self, SOLVER_STORE_FAMILY)
    }

    fn summary_facts(&self) -> &[SummaryFact] {
        self.summary_store().all_summaries()
    }
    fn summary_events(&self) -> &[SummaryEventFact] {
        self.summary_store().all_events()
    }

    fn unresolved_calls(&self) -> &[UnresolvedCallFact] {
        self.calls_store().unresolved()
    }

    fn unsupported_semantics(&self) -> &[UnsupportedSemanticFact] {
        self.semantic_mir_store().unsupported_semantics()
    }

    fn mir_bodies(&self) -> &[MirBody] {
        self.semantic_mir_store().mir_bodies()
    }

    fn mir_operations(&self) -> &[MirOperation] {
        self.semantic_mir_store().mir_operations()
    }

    fn mir_blocks(&self) -> &[MirBlock] {
        self.semantic_mir_store().mir_blocks()
    }

    fn mir_places(&self) -> &[PlaceFact] {
        self.semantic_mir_store().places()
    }

    fn mir_place_types(&self) -> &[PlaceTypeFact] {
        self.semantic_mir_store().place_types()
    }

    fn call_sites(&self) -> &[CallSiteFact] {
        self.calls_store().sites()
    }

    /// Language-neutral semantic scopes installed by the composition root.
    fn semantic_scopes(&self) -> &[ScopeFact] {
        &[]
    }

    fn call_targets(&self) -> &[CallTargetFact] {
        self.calls_store().targets()
    }

    fn replace_summary_facts(&mut self, output: SummaryOutput) {
        let interner = self.stable_key_interner();
        let store = SummaryStore::from_output(output, &interner)
            .expect("summary output should produce a valid store");
        *self.summary_store_mut() = store;
        refresh_summary_metadata(self);
    }

    fn replace_call_facts(&mut self, mut output: CallOutput) -> Result<(), AnalysisError> {
        populate_call_owner_symbols(self, &mut output);
        let interner = self.stable_key_interner();
        let store = CallStore::from_output(output, &interner)?;
        *self.calls_store_mut() = store;
        refresh_call_metadata(self);
        Ok(())
    }

    fn replace_cfg_facts(&mut self, output: CfgOutput) -> Result<(), AnalysisError> {
        let interner = self.stable_key_interner();
        self.cfg_store_mut().replace(output.normalized(&interner));
        refresh_cfg_metadata(self);
        Ok(())
    }

    fn replace_semantic_mir(&mut self, output: MirOutput) -> Result<(), AnalysisError> {
        let interner = self.stable_key_interner();
        *self.semantic_mir_store_mut() = SemanticStore::from_output(output, &interner)?;
        Ok(())
    }

    fn replace_refined_call_facts(
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
        *store_mut::<RefinedCallStore>(self, REFINED_CALL_STORE_FAMILY) =
            RefinedCallStore::from_output(
                output,
                &interner,
                &valid_call_sites,
                &valid_call_targets,
                &valid_functions,
                &valid_symbols,
            )?;
        refresh_refined_call_metadata(self);
        Ok(())
    }

    fn replace_entrypoint_facts(&mut self, output: EntrypointOutput) -> Result<(), AnalysisError> {
        let interner = self.stable_key_interner();
        *store_mut::<EntrypointStore>(self, ENTRYPOINT_STORE_FAMILY) =
            EntrypointStore::from_output(output, &interner)?;
        Ok(())
    }

    fn replace_abstract_domain_facts(&mut self, output: DomainOutput) {
        let interner = self.stable_key_interner();
        *store_mut::<DomainStore>(self, DOMAIN_STORE_FAMILY) =
            DomainStore::from_output(output, &interner);
        refresh_abstract_domain_metadata(self);
    }

    fn mir_statements(&self) -> &[MirStatement] {
        self.semantic_mir_store().mir_statements()
    }

    fn mir_terminators(&self) -> &[MirTerminator] {
        self.semantic_mir_store().mir_terminators()
    }

    fn cfg_functions(&self) -> &[CfgFunctionFact] {
        self.cfg_store().functions()
    }
    fn cfg_nodes(&self) -> &[CfgNodeFact] {
        self.cfg_store().nodes()
    }
    fn cfg_blocks(&self) -> &[BasicBlockFact] {
        self.cfg_store().blocks()
    }
    fn cfg_edges(&self) -> &[CfgEdgeFact] {
        self.cfg_store().edges()
    }
    fn cfg_reachability(&self) -> &[ReachabilityFact] {
        self.cfg_store().reachability()
    }
    fn cfg_dominators(&self) -> &[DominatorFact] {
        self.cfg_store().dominators()
    }
    fn cfg_postdominators(&self) -> &[PostDominatorFact] {
        self.cfg_store().postdominators()
    }
    fn cfg_control_dependence(&self) -> &[ControlDependenceFact] {
        self.cfg_store().control_dependence()
    }
    fn unsupported_control_flow(&self) -> &[UnsupportedControlFlowFact] {
        self.cfg_store().unsupported()
    }

    fn refined_call_edges(&self) -> &[RefinedCallEdgeFact] {
        self.refined_call_store_inner().edges()
    }

    fn data_flow_nodes(&self) -> &[DataFlowNodeFact] {
        self.data_flow_store_inner().nodes()
    }
    fn data_flow_edges(&self) -> &[DataFlowEdgeFact] {
        self.data_flow_store_inner().edges()
    }
    fn data_flow_models(&self) -> &[DataFlowModelFact] {
        self.data_flow_store_inner().models()
    }
    fn data_flow_budgets(&self) -> &[DataFlowBudgetFact] {
        self.data_flow_store_inner().budgets()
    }

    fn evidence_nodes(&self) -> &[EvidenceNodeFact] {
        self.evidence_store_inner().nodes()
    }
    fn evidence_edges(&self) -> &[EvidenceEdgeFact] {
        self.evidence_store_inner().edges()
    }
    fn evidence_bundles(&self) -> &[EvidenceBundleFact] {
        self.evidence_store_inner().bundles()
    }
    fn evidence_paths(&self) -> &[EvidencePathFact] {
        self.evidence_store_inner().paths()
    }
    fn evidence_slices(&self) -> &[EvidenceSliceFact] {
        self.evidence_store_inner().slices()
    }
    fn evidence_unknowns(&self) -> &[EvidenceUnknownFact] {
        self.evidence_store_inner().unknowns()
    }
    fn evidence_omitted_regions(&self) -> &[EvidenceOmittedRegionFact] {
        self.evidence_store_inner().omitted_regions()
    }
    fn evidence_replay_keys(&self) -> &[EvidenceReplayKeyFact] {
        self.evidence_store_inner().replay_keys()
    }

    fn abstract_domain_observations(&self) -> &[DomainObservationFact] {
        self.domain_store_inner().observations()
    }
    fn abstract_domain_events(&self) -> &[DomainEventFact] {
        self.domain_store_inner().events()
    }

    fn entrypoint_facts(&self) -> &[EntrypointFact] {
        self.entrypoint_store_inner().entrypoints()
    }
    fn trust_boundary_facts(&self) -> &[TrustBoundaryFact] {
        self.entrypoint_store_inner().trust_boundaries()
    }
    fn dispatch_edge_facts(&self) -> &[FrameworkDispatchEdgeFact] {
        self.entrypoint_store_inner().dispatch_edges()
    }
    fn unresolved_framework_facts(&self) -> &[UnresolvedFrameworkFact] {
        self.entrypoint_store_inner().unresolved()
    }

    fn type_facts(&self) -> &[TypeFact] {
        self.type_store_inner().types()
    }
    fn narrowed_type_facts(&self) -> &[NarrowedTypeFact] {
        self.type_store_inner().narrowed()
    }
    fn value_facts(&self) -> &[ValueFact] {
        self.value_store_inner().values()
    }
    fn allocation_tokens(&self) -> &[AllocationTokenFact] {
        self.value_store_inner().allocations()
    }
    fn access_path_facts(&self) -> &[AccessPathFact] {
        self.access_path_store_inner().access_paths()
    }
    fn points_to_constraints(&self) -> &[PointsToConstraintFact] {
        self.points_to_store_inner().constraints()
    }
    fn points_to_sets(&self) -> &[PointsToSetFact] {
        self.points_to_store_inner().sets()
    }
    fn alias_answers(&self) -> &[AliasAnswerFact] {
        self.alias_store_inner().answers()
    }

    fn identity_records(&self) -> &[IdentityRecord] {
        self.identity_store_inner().records()
    }

    fn reachability_roots(&self) -> &[ReachabilityRootFact] {
        self.reachability_store_inner().roots()
    }

    fn semantic_nodes(&self) -> &[SemanticNodeFact] {
        self.semantic_graph_store_inner().nodes()
    }
    fn semantic_edges(&self) -> &[SemanticEdgeFact] {
        self.semantic_graph_store_inner().edges()
    }
    fn semantic_constraints(&self) -> &[ConstraintFact] {
        self.semantic_graph_store_inner().constraints()
    }

    fn solver_derived_edges(&self) -> &[DerivedEdgeFact] {
        self.solver_store_inner().derived_edges()
    }
    fn solver_budget_status(&self) -> BudgetStatus {
        self.solver_store_inner().budget_status()
    }
    fn solver_budget_reasons(&self) -> &std::collections::BTreeSet<String> {
        self.solver_store_inner().budget_reasons()
    }

    fn extension_facts(&self) -> &[AcceptedExtensionFact] {
        self.extension_store_inner().accepted.as_slice()
    }
    fn extension_activations(&self) -> &[ExtensionActivationRow] {
        self.extension_store_inner().activations.as_slice()
    }
    fn rejected_extension_facts(&self) -> &[RejectedExtensionFact] {
        self.extension_store_inner().rejected.as_slice()
    }

    fn call_sites_by_caller(&self, caller: crate::internal_core::FunctionId) -> Vec<&CallSiteFact> {
        self.calls_store().sites_by_caller(caller)
    }
    fn call_targets_by_site(&self, site: CallSiteId) -> Vec<&CallTargetFact> {
        self.calls_store().targets_by_site(site)
    }

    fn metadata_for(&self, fact_ref: FactRef) -> Option<&FactMeta> {
        self.fact_meta().get(fact_ref)
    }

    fn replace_data_flow_facts(&mut self, output: DataFlowOutput) -> Result<(), AnalysisError> {
        let interner = self.stable_key_interner();
        *store_mut::<DataFlowStore>(self, DATA_FLOW_STORE_FAMILY) =
            DataFlowStore::from_output(output, &interner)?;
        Ok(())
    }

    fn replace_evidence_facts(&mut self, output: EvidenceOutput) -> Result<(), AnalysisError> {
        let interner = self.stable_key_interner();
        *store_mut::<EvidenceStore>(self, EVIDENCE_STORE_FAMILY) =
            EvidenceStore::from_output(output, &interner)?;
        Ok(())
    }

    fn replace_reachability_facts(
        &mut self,
        output: ReachabilityProviderOutput,
    ) -> Result<(), AnalysisError> {
        let interner = self.stable_key_interner();
        let valid_function_ids = self
            .functions()
            .iter()
            .map(|row| row.id)
            .collect::<std::collections::BTreeSet<_>>();
        let valid_entrypoint_ids = self
            .entrypoint_facts()
            .iter()
            .map(|row| row.id)
            .collect::<std::collections::BTreeSet<_>>();
        let store = ReachabilityStore::from_output(
            output,
            &interner,
            &valid_function_ids,
            &valid_entrypoint_ids,
        )?;
        *store_mut::<ReachabilityStore>(self, REACHABILITY_STORE_FAMILY) = store;
        Ok(())
    }

    fn replace_solver_facts(&mut self, output: SolverOutput) -> Result<(), AnalysisError> {
        let interner = self.stable_key_interner();
        *store_mut::<SolverStore>(self, SOLVER_STORE_FAMILY) =
            SolverStore::from_output(output, &interner)?;
        Ok(())
    }

    fn replace_type_value_alias_facts(&mut self, output: TypeValueAliasOutput) {
        let interner = self.stable_key_interner();
        let output = output.normalized(&interner);
        *store_mut::<TypeStore>(self, TYPE_STORE_FAMILY) =
            TypeStore::from_normalized_output(output.types);
        *store_mut::<ValueStore>(self, VALUE_STORE_FAMILY) =
            ValueStore::from_normalized_output(output.values);
        *store_mut::<AccessPathStore>(self, ACCESS_PATH_STORE_FAMILY) =
            AccessPathStore::from_normalized_output(output.access_paths);
        *store_mut::<PointsToStore>(self, POINTS_TO_STORE_FAMILY) =
            PointsToStore::from_normalized_output(output.points_to);
        *store_mut::<AliasStore>(self, ALIAS_STORE_FAMILY) =
            AliasStore::from_normalized_output(output.aliases);
    }

    fn replace_extension_facts(&mut self, output: ExtensionOutput) {
        let output = output.normalized(&self.stable_key_interner());
        let store = store_mut::<ExtensionFactStore>(self, EXTENSION_STORE_FAMILY);
        store.activations = output.activations;
        store.accepted = output.accepted;
        store.rejected = output.rejected;
    }

    fn replace_semantic_graph_facts(
        &mut self,
        output: SemanticGraphOutput,
    ) -> Result<(), AnalysisError> {
        let interner = self.stable_key_interner();
        *store_mut::<SemanticGraphStore>(self, SEMANTIC_GRAPH_STORE_FAMILY) =
            SemanticGraphStore::from_output(output, &interner)?;
        Ok(())
    }
    fn identity_store_mut(&mut self) -> &mut IdentityStore {
        store_mut(self, IDENTITY_STORE_FAMILY)
    }

    fn replace_identity_facts(
        &mut self,
        output: IdentityProviderOutput,
    ) -> Result<(), AnalysisError> {
        let valid_sites = self
            .calls_store()
            .sites()
            .iter()
            .map(|site| site.id)
            .collect::<std::collections::BTreeSet<_>>();
        let valid_targets = self
            .call_targets()
            .iter()
            .map(|target| target.id)
            .collect::<std::collections::BTreeSet<_>>();
        let interner = self.stable_key_interner();
        let store = IdentityStore::from_output(output, &interner, &valid_sites, &valid_targets)?;
        *self.identity_store_mut() = store;
        Ok(())
    }

    #[cfg(test)]
    fn set_identity_records_for_test(
        &mut self,
        records: Vec<crate::analysis_neutral::identity::facts::IdentityRecord>,
    ) {
        let mut store = IdentityStore::default();
        store.records = records;
        *self.identity_store_mut() = store;
    }

    fn merge_summary_facts_without_metadata(
        &mut self,
        summaries: &[SummaryFact],
        events: &[SummaryEventFact],
    ) {
        let interner = self.stable_key_interner();
        self.summary_store_mut()
            .merge_updates(summaries, events, &interner);
    }

    fn refresh_summary_metadata_after_bulk_update(&mut self) {
        refresh_summary_metadata(self);
    }

    fn replace_normalized_type_value_alias_facts(&mut self, output: TypeValueAliasOutput) {
        *store_mut::<TypeStore>(self, TYPE_STORE_FAMILY) =
            TypeStore::from_normalized_output(output.types);
        *store_mut::<ValueStore>(self, VALUE_STORE_FAMILY) =
            ValueStore::from_normalized_output(output.values);
        *store_mut::<AccessPathStore>(self, ACCESS_PATH_STORE_FAMILY) =
            AccessPathStore::from_normalized_output(output.access_paths);
        *store_mut::<PointsToStore>(self, POINTS_TO_STORE_FAMILY) =
            PointsToStore::from_normalized_output(output.points_to);
        *store_mut::<AliasStore>(self, ALIAS_STORE_FAMILY) =
            AliasStore::from_normalized_output(output.aliases);
    }
}

impl AnalysisHost for LocalAnalysisDb {}

fn host_fact_meta(
    producer_id: &'static str,
    stable_key: StableKeyId,
    payload_digest: String,
) -> FactMeta {
    FactMeta {
        stable_key,
        producer_id,
        layer_id: producer_id,
        precision: FactPrecision::Heuristic,
        confidence: FactConfidence::Medium,
        validation: ValidationStatus::NativeTrusted,
        payload_digest,
    }
}

fn summary_domain_family(domain: SummaryDomainKind) -> FactFamily {
    match domain {
        SummaryDomainKind::ControlEffects => FactFamily::SummaryControl,
        SummaryDomainKind::CallEffects => FactFamily::SummaryCall,
        SummaryDomainKind::MemoryEffects => FactFamily::SummaryMemory,
        SummaryDomainKind::DataFlowTito => FactFamily::SummaryTito,
    }
}

fn populate_call_owner_symbols(db: &(impl AnalysisHost + ?Sized), output: &mut CallOutput) {
    if output.sites.iter().all(|site| site.owner_symbol.is_some()) {
        return;
    }

    let function_symbols = db
        .functions()
        .iter()
        .filter_map(|function| {
            let symbol = db
                .symbols()
                .iter()
                .find(|symbol| {
                    symbol.file == Some(function.file)
                        && symbol.name == function.name
                        && symbol.primary_span.as_ref().is_some_and(|span| {
                            span == &function.span || span_is_within(span, &function.span)
                        })
                })
                .map(|symbol| symbol.id)
                .or_else(|| {
                    db.definitions()
                        .iter()
                        .find(|definition| {
                            definition.file == Some(function.file)
                                && definition.name == function.name
                                && definition.primary_span.as_ref().is_some_and(|span| {
                                    span == &function.span || span_is_within(span, &function.span)
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

fn span_is_within(inner: &crate::internal_core::Span, outer: &crate::internal_core::Span) -> bool {
    inner.file == outer.file
        && inner.start_byte >= outer.start_byte
        && inner.end_byte <= outer.end_byte
}

fn refresh_summary_metadata(db: &mut (impl AnalysisHost + ?Sized)) {
    let summaries = db.summary_facts().to_vec();
    let events = db.summary_events().to_vec();

    {
        let meta = db.fact_meta_mut();
        meta.remove_family(FactFamily::SummaryControl);
        meta.remove_family(FactFamily::SummaryCall);
        meta.remove_family(FactFamily::SummaryMemory);
        meta.remove_family(FactFamily::SummaryTito);
        meta.remove_family(FactFamily::SummaryEvent);
        for fact in &summaries {
            let family = summary_domain_family(fact.domain);
            meta.insert(
                FactRef::new(family, fact.id.0),
                host_fact_meta(
                    POLINT_DIRECT_SUMMARIES_PROVIDER_ID,
                    fact.stable_key,
                    format!("summary:{}", fact.id.0),
                ),
            );
        }
        for fact in &events {
            meta.insert(
                FactRef::new(FactFamily::SummaryEvent, fact.id.0),
                host_fact_meta(
                    POLINT_DIRECT_SUMMARIES_PROVIDER_ID,
                    fact.stable_key,
                    format!("summary-event:{}", fact.id.0),
                ),
            );
        }
        for family in [
            FactFamily::SummaryControl,
            FactFamily::SummaryCall,
            FactFamily::SummaryMemory,
            FactFamily::SummaryTito,
            FactFamily::SummaryEvent,
        ] {
            meta.finish_family_insertions(family);
        }
    }
}

fn refresh_abstract_domain_metadata(db: &mut (impl AnalysisHost + ?Sized)) {
    let observations = db.domain_store_inner().observations().to_vec();
    let events = db.domain_store_inner().events().to_vec();

    {
        let meta = db.fact_meta_mut();
        meta.remove_family(FactFamily::DomainObservation);
        meta.remove_family(FactFamily::DomainEvent);
        for fact in &observations {
            meta.insert(
                FactRef::new(FactFamily::DomainObservation, fact.id.0),
                host_fact_meta(
                    POLINT_ABSTRACT_DOMAINS_PROVIDER_ID,
                    fact.stable_key,
                    format!("domain-obs:{}", fact.id.0),
                ),
            );
        }
        for fact in &events {
            meta.insert(
                FactRef::new(FactFamily::DomainEvent, fact.id.0),
                host_fact_meta(
                    POLINT_ABSTRACT_DOMAINS_PROVIDER_ID,
                    fact.stable_key,
                    format!("domain-event:{}", fact.id.0),
                ),
            );
        }
        meta.finish_family_insertions(FactFamily::DomainObservation);
        meta.finish_family_insertions(FactFamily::DomainEvent);
    }
}

fn refresh_cfg_metadata(db: &mut (impl AnalysisHost + ?Sized)) {
    let interner = db.stable_key_interner();
    let families = [
        FactFamily::CfgFunction,
        FactFamily::CfgNode,
        FactFamily::BasicBlock,
        FactFamily::CfgEdge,
        FactFamily::CfgReachability,
        FactFamily::CfgDominator,
        FactFamily::CfgPostDominator,
        FactFamily::CfgControlDependence,
        FactFamily::UnsupportedControlFlow,
    ];
    {
        let metadata = db.fact_meta_mut();
        for family in families {
            metadata.remove_family(family);
        }
    }

    let functions = db.cfg_functions().to_vec();
    let nodes = db.cfg_nodes().to_vec();
    let blocks = db.cfg_blocks().to_vec();
    let edges = db.cfg_edges().to_vec();
    let reachability = db.cfg_reachability().to_vec();
    let dominators = db.cfg_dominators().to_vec();
    let postdominators = db.cfg_postdominators().to_vec();
    let control_dependence = db.cfg_control_dependence().to_vec();
    let unsupported = db.unsupported_control_flow().to_vec();

    for fact in &functions {
        let metadata = cfg_function_metadata(db, &interner, fact);
        db.fact_meta_mut()
            .insert(FactRef::new(FactFamily::CfgFunction, fact.id.0), metadata);
    }
    for fact in &nodes {
        let metadata = cfg_node_metadata(db, &interner, fact);
        db.fact_meta_mut()
            .insert(FactRef::new(FactFamily::CfgNode, fact.id.0), metadata);
    }
    for fact in &blocks {
        let metadata = cfg_block_metadata(db, &interner, fact);
        db.fact_meta_mut()
            .insert(FactRef::new(FactFamily::BasicBlock, fact.id.0), metadata);
    }
    for fact in &edges {
        let metadata = cfg_edge_metadata(db, &interner, fact);
        db.fact_meta_mut()
            .insert(FactRef::new(FactFamily::CfgEdge, fact.id.0), metadata);
    }
    for fact in &reachability {
        let metadata = cfg_reachability_metadata(db, &interner, fact);
        db.fact_meta_mut().insert(
            FactRef::new(FactFamily::CfgReachability, fact.id.0),
            metadata,
        );
    }
    for fact in &dominators {
        let metadata = cfg_dominator_metadata(db, &interner, fact);
        db.fact_meta_mut()
            .insert(FactRef::new(FactFamily::CfgDominator, fact.id.0), metadata);
    }
    for fact in &postdominators {
        let metadata = cfg_postdominator_metadata(db, &interner, fact);
        db.fact_meta_mut().insert(
            FactRef::new(FactFamily::CfgPostDominator, fact.id.0),
            metadata,
        );
    }
    for fact in &control_dependence {
        let metadata = cfg_control_dependence_metadata(db, &interner, fact);
        db.fact_meta_mut().insert(
            FactRef::new(FactFamily::CfgControlDependence, fact.id.0),
            metadata,
        );
    }
    for fact in &unsupported {
        let metadata = unsupported_control_flow_metadata(db, &interner, fact);
        db.fact_meta_mut().insert(
            FactRef::new(FactFamily::UnsupportedControlFlow, fact.id.0),
            metadata,
        );
    }

    let metadata = db.fact_meta_mut();
    for family in families {
        metadata.finish_family_insertions(family);
    }
}

fn cfg_fact_metadata<const N: usize>(
    interner: &StableKeyInterner,
    stable_key: StableKeyId,
    precision: FactPrecision,
    confidence: FactConfidence,
    payload_parts: [(&str, String); N],
) -> FactMeta {
    FactMeta {
        stable_key,
        producer_id: crate::analysis_neutral::CFG_PROVIDER_ID,
        layer_id: crate::analysis_neutral::CFG_PROVIDER_ID,
        precision,
        confidence,
        validation: ValidationStatus::NativeTrusted,
        payload_digest: metadata_payload_digest(interner, stable_key, &payload_parts),
    }
}

fn cfg_status_metadata(
    status: crate::analysis_neutral::cfg::facts::CfgStatus,
    precision: crate::analysis_neutral::cfg::facts::CfgPrecision,
) -> (FactPrecision, FactConfidence) {
    use crate::analysis_neutral::cfg::facts::{CfgPrecision, CfgStatus};

    let fact_precision = match (status, precision) {
        (CfgStatus::Resolved, CfgPrecision::ExactSyntax) => FactPrecision::Syntax,
        (CfgStatus::Resolved, CfgPrecision::ExactLowered | CfgPrecision::SetupAware) => {
            FactPrecision::SetupAware
        }
        (_, CfgPrecision::Conservative | CfgPrecision::Heuristic) => FactPrecision::Heuristic,
        (CfgStatus::Partial, _) => FactPrecision::Heuristic,
        (CfgStatus::Unknown, _) | (_, CfgPrecision::Unknown) => FactPrecision::Unresolved,
        (CfgStatus::Unsupported, _) | (_, CfgPrecision::Unsupported) => FactPrecision::Unsupported,
    };
    let confidence = match status {
        CfgStatus::Resolved => FactConfidence::High,
        CfgStatus::Partial => FactConfidence::Medium,
        CfgStatus::Unknown | CfgStatus::Unsupported => FactConfidence::Low,
    };
    (fact_precision, confidence)
}

fn cfg_function_metadata(
    db: &(impl AnalysisHost + ?Sized),
    interner: &StableKeyInterner,
    fact: &CfgFunctionFact,
) -> FactMeta {
    let (precision, confidence) = cfg_status_metadata(fact.status, fact.precision);
    cfg_fact_metadata(
        interner,
        fact.stable_key,
        precision,
        confidence,
        [
            ("status", cfg_status_label(fact.status).to_string()),
            ("precision", cfg_precision_label(fact.precision).to_string()),
            (
                "body_key",
                fact_stable_key(db, FactFamily::MirBody, fact.body.0),
            ),
            ("language", language_label(fact.language).to_string()),
            ("path", db.path_for(fact.file)),
            ("span", span_metadata_value(&fact.span)),
        ],
    )
}

fn cfg_node_metadata(
    db: &(impl AnalysisHost + ?Sized),
    interner: &StableKeyInterner,
    fact: &CfgNodeFact,
) -> FactMeta {
    let (precision, confidence) = cfg_status_metadata(fact.status, fact.precision);
    cfg_fact_metadata(
        interner,
        fact.stable_key,
        precision,
        confidence,
        [
            ("status", cfg_status_label(fact.status).to_string()),
            ("precision", cfg_precision_label(fact.precision).to_string()),
            ("kind", cfg_node_kind_label(fact.kind).to_string()),
            (
                "function_key",
                fact_stable_key(db, FactFamily::CfgFunction, fact.cfg_function.0),
            ),
            ("operation_ordinal", fact.operation_ordinal.to_string()),
            ("span", option_span_metadata_value(fact.span.as_ref())),
        ],
    )
}

fn cfg_block_metadata(
    db: &(impl AnalysisHost + ?Sized),
    interner: &StableKeyInterner,
    fact: &BasicBlockFact,
) -> FactMeta {
    let (precision, confidence) = cfg_status_metadata(fact.status, fact.precision);
    cfg_fact_metadata(
        interner,
        fact.stable_key,
        precision,
        confidence,
        [
            ("status", cfg_status_label(fact.status).to_string()),
            ("precision", cfg_precision_label(fact.precision).to_string()),
            ("kind", basic_block_kind_label(fact.kind).to_string()),
            (
                "function_key",
                fact_stable_key(db, FactFamily::CfgFunction, fact.cfg_function.0),
            ),
            ("reachable", fact.reachable.to_string()),
            ("reverse_postorder", fact.reverse_postorder.to_string()),
        ],
    )
}

fn cfg_edge_metadata(
    db: &(impl AnalysisHost + ?Sized),
    interner: &StableKeyInterner,
    fact: &CfgEdgeFact,
) -> FactMeta {
    let (precision, confidence) = cfg_status_metadata(fact.status, fact.precision);
    cfg_fact_metadata(
        interner,
        fact.stable_key,
        precision,
        confidence,
        [
            ("status", cfg_status_label(fact.status).to_string()),
            ("precision", cfg_precision_label(fact.precision).to_string()),
            ("view", cfg_view_label(fact.view).to_string()),
            ("kind", cfg_edge_kind_label(fact.kind).to_string()),
            (
                "function_key",
                fact_stable_key(db, FactFamily::CfgFunction, fact.cfg_function.0),
            ),
            ("from_block", fact.from_block.0.to_string()),
            ("to_block", fact.to_block.0.to_string()),
        ],
    )
}

fn cfg_reachability_metadata(
    _db: &(impl AnalysisHost + ?Sized),
    interner: &StableKeyInterner,
    fact: &ReachabilityFact,
) -> FactMeta {
    let (precision, confidence) = cfg_status_metadata(fact.status, fact.precision);
    cfg_fact_metadata(
        interner,
        fact.stable_key,
        precision,
        confidence,
        [
            ("status", cfg_status_label(fact.status).to_string()),
            ("precision", cfg_precision_label(fact.precision).to_string()),
            ("view", cfg_view_label(fact.view).to_string()),
            ("block", fact.block.0.to_string()),
            ("reachable", fact.reachable.to_string()),
        ],
    )
}

fn cfg_dominator_metadata(
    _db: &(impl AnalysisHost + ?Sized),
    interner: &StableKeyInterner,
    fact: &DominatorFact,
) -> FactMeta {
    let (precision, confidence) = cfg_status_metadata(fact.status, fact.precision);
    cfg_fact_metadata(
        interner,
        fact.stable_key,
        precision,
        confidence,
        [
            ("status", cfg_status_label(fact.status).to_string()),
            ("precision", cfg_precision_label(fact.precision).to_string()),
            ("view", cfg_view_label(fact.view).to_string()),
            ("dominator", fact.dominator.0.to_string()),
            ("dominated", fact.dominated.0.to_string()),
            ("immediate", fact.immediate.to_string()),
        ],
    )
}

fn cfg_postdominator_metadata(
    _db: &(impl AnalysisHost + ?Sized),
    interner: &StableKeyInterner,
    fact: &PostDominatorFact,
) -> FactMeta {
    let (precision, confidence) = cfg_status_metadata(fact.status, fact.precision);
    cfg_fact_metadata(
        interner,
        fact.stable_key,
        precision,
        confidence,
        [
            ("status", cfg_status_label(fact.status).to_string()),
            ("precision", cfg_precision_label(fact.precision).to_string()),
            ("view", cfg_view_label(fact.view).to_string()),
            ("postdominator", fact.postdominator.0.to_string()),
            ("postdominated", fact.postdominated.0.to_string()),
            ("immediate", fact.immediate.to_string()),
        ],
    )
}

fn cfg_control_dependence_metadata(
    _db: &(impl AnalysisHost + ?Sized),
    interner: &StableKeyInterner,
    fact: &ControlDependenceFact,
) -> FactMeta {
    let (precision, confidence) = cfg_status_metadata(fact.status, fact.precision);
    cfg_fact_metadata(
        interner,
        fact.stable_key,
        precision,
        confidence,
        [
            ("status", cfg_status_label(fact.status).to_string()),
            ("precision", cfg_precision_label(fact.precision).to_string()),
            ("view", cfg_view_label(fact.view).to_string()),
            ("edge", fact.controlling_edge.0.to_string()),
            (
                "edge_kind",
                cfg_edge_kind_label(fact.controlling_edge_kind).to_string(),
            ),
            ("controlled_block", fact.controlled_block.0.to_string()),
        ],
    )
}

fn unsupported_control_flow_metadata(
    db: &(impl AnalysisHost + ?Sized),
    interner: &StableKeyInterner,
    fact: &UnsupportedControlFlowFact,
) -> FactMeta {
    let (precision, confidence) = cfg_status_metadata(fact.status, fact.precision);
    cfg_fact_metadata(
        interner,
        fact.stable_key,
        precision,
        confidence,
        [
            ("status", cfg_status_label(fact.status).to_string()),
            ("precision", cfg_precision_label(fact.precision).to_string()),
            ("language", language_label(fact.language).to_string()),
            ("path", db.path_for(fact.file)),
            ("span", span_metadata_value(&fact.span)),
            ("construct", fact.construct.clone()),
            ("source_evidence", fact.source_evidence.clone()),
        ],
    )
}

fn option_span_metadata_value(span: Option<&crate::internal_core::Span>) -> String {
    span.map(span_metadata_value).unwrap_or_else(none_value)
}

fn cfg_status_label(status: crate::analysis_neutral::cfg::facts::CfgStatus) -> &'static str {
    match status {
        crate::analysis_neutral::cfg::facts::CfgStatus::Resolved => "resolved",
        crate::analysis_neutral::cfg::facts::CfgStatus::Partial => "partial",
        crate::analysis_neutral::cfg::facts::CfgStatus::Unknown => "unknown",
        crate::analysis_neutral::cfg::facts::CfgStatus::Unsupported => "unsupported",
    }
}

fn cfg_precision_label(
    precision: crate::analysis_neutral::cfg::facts::CfgPrecision,
) -> &'static str {
    match precision {
        crate::analysis_neutral::cfg::facts::CfgPrecision::ExactSyntax => "exact_syntax",
        crate::analysis_neutral::cfg::facts::CfgPrecision::ExactLowered => "exact_lowered",
        crate::analysis_neutral::cfg::facts::CfgPrecision::SetupAware => "setup_aware",
        crate::analysis_neutral::cfg::facts::CfgPrecision::Conservative => "conservative",
        crate::analysis_neutral::cfg::facts::CfgPrecision::Heuristic => "heuristic",
        crate::analysis_neutral::cfg::facts::CfgPrecision::Unknown => "unknown",
        crate::analysis_neutral::cfg::facts::CfgPrecision::Unsupported => "unsupported",
    }
}

fn cfg_view_label(view: crate::analysis_neutral::cfg::facts::CfgView) -> &'static str {
    match view {
        crate::analysis_neutral::cfg::facts::CfgView::NormalControl => "normal_control",
        crate::analysis_neutral::cfg::facts::CfgView::AbruptAware => "abrupt_aware",
        crate::analysis_neutral::cfg::facts::CfgView::ExceptionConservative => {
            "exception_conservative"
        }
    }
}

fn cfg_node_kind_label(kind: crate::analysis_neutral::cfg::facts::CfgNodeKind) -> &'static str {
    use crate::analysis_neutral::cfg::facts::CfgNodeKind;

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

fn basic_block_kind_label(
    kind: crate::analysis_neutral::cfg::facts::BasicBlockKind,
) -> &'static str {
    use crate::analysis_neutral::cfg::facts::BasicBlockKind;

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

fn cfg_edge_kind_label(kind: crate::analysis_neutral::cfg::facts::CfgEdgeKind) -> &'static str {
    use crate::analysis_neutral::cfg::facts::CfgEdgeKind;

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

fn refresh_refined_call_metadata(db: &mut (impl AnalysisHost + ?Sized)) {
    let interner = db.stable_key_interner();
    let edges = db.refined_call_edges().to_vec();
    {
        let metadata = db.fact_meta_mut();
        metadata.remove_family(FactFamily::RefinedCallEdge);
    }

    for fact in &edges {
        let (precision, status_confidence) = call_status_metadata(fact.status, fact.precision);
        let confidence = refined_call_confidence_metadata(fact.confidence, status_confidence);
        let validation = refined_call_validation_metadata(fact.validation);
        let metadata = FactMeta {
            stable_key: fact.stable_key,
            producer_id: "polint.refined_calls",
            layer_id: "polint.refined_calls",
            precision,
            confidence,
            validation,
            payload_digest: metadata_payload_digest(
                &interner,
                fact.stable_key,
                &[
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
                        fact_stable_key(db, FactFamily::CallSite, fact.site.0),
                    ),
                    (
                        "base_target_key",
                        fact.base_target
                            .map(|target| fact_stable_key(db, FactFamily::CallTarget, target.0))
                            .unwrap_or_else(none_value),
                    ),
                    (
                        "caller_key",
                        fact_stable_key(db, FactFamily::Function, fact.caller.0),
                    ),
                    (
                        "target_function_key",
                        fact.target_function
                            .map(|function| fact_stable_key(db, FactFamily::Function, function.0))
                            .unwrap_or_else(none_value),
                    ),
                    (
                        "target_symbol_key",
                        fact.target_symbol
                            .map(|symbol| fact_stable_key(db, FactFamily::Symbol, symbol.0))
                            .unwrap_or_else(none_value),
                    ),
                    (
                        "synthetic_target",
                        fact.synthetic_target.clone().unwrap_or_else(none_value),
                    ),
                    ("evidence", fact.evidence.join("\n")),
                    ("inputs", fact.input_stable_keys.join("\n")),
                ],
            ),
        };
        db.fact_meta_mut().insert(
            FactRef::new(FactFamily::RefinedCallEdge, fact.id.0),
            metadata,
        );
    }
    db.fact_meta_mut()
        .finish_family_insertions(FactFamily::RefinedCallEdge);
}

fn refined_call_confidence_metadata(
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

fn refined_call_validation_metadata(validation: RefinedCallValidation) -> ValidationStatus {
    match validation {
        RefinedCallValidation::Native => ValidationStatus::NativeTrusted,
        RefinedCallValidation::ReferentiallyValidated => ValidationStatus::ReferentiallyValidated,
        RefinedCallValidation::ExtensionValidated => ValidationStatus::SchemaValidated,
        RefinedCallValidation::Rejected => ValidationStatus::ConflictRejected,
    }
}

fn refined_call_tier_label(tier: RefinedCallTier) -> &'static str {
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

fn refined_call_validation_label(validation: RefinedCallValidation) -> &'static str {
    match validation {
        RefinedCallValidation::Native => "native",
        RefinedCallValidation::ReferentiallyValidated => "referentially_validated",
        RefinedCallValidation::ExtensionValidated => "extension_validated",
        RefinedCallValidation::Rejected => "rejected",
    }
}

fn refresh_call_metadata(db: &mut (impl AnalysisHost + ?Sized)) {
    let interner = db.stable_key_interner();
    let call_sites = db.call_sites().to_vec();
    let call_targets = db.call_targets().to_vec();
    let unresolved_calls = db.unresolved_calls().to_vec();

    {
        let meta = db.fact_meta_mut();
        for family in [
            FactFamily::CallSite,
            FactFamily::CallTarget,
            FactFamily::UnresolvedCall,
        ] {
            meta.remove_family(family);
        }
    }

    let site_metadata = call_sites
        .iter()
        .map(|fact| call_site_metadata(db, &interner, fact))
        .collect::<Vec<_>>();
    {
        let meta = db.fact_meta_mut();
        for (fact, metadata) in call_sites.iter().zip(site_metadata) {
            meta.insert(FactRef::new(FactFamily::CallSite, fact.id.0), metadata);
        }
    }

    let target_metadata = call_targets
        .iter()
        .map(|fact| call_target_metadata(db, &interner, fact))
        .collect::<Vec<_>>();
    {
        let meta = db.fact_meta_mut();
        for (fact, metadata) in call_targets.iter().zip(target_metadata) {
            meta.insert(FactRef::new(FactFamily::CallTarget, fact.id.0), metadata);
        }
    }

    let unresolved_metadata = unresolved_calls
        .iter()
        .map(|fact| unresolved_call_metadata(db, &interner, fact))
        .collect::<Vec<_>>();
    {
        let meta = db.fact_meta_mut();
        for (run_id, metadata) in unresolved_metadata.into_iter().enumerate() {
            meta.insert(
                FactRef::new(FactFamily::UnresolvedCall, run_id as u64),
                metadata,
            );
        }
        for family in [
            FactFamily::CallSite,
            FactFamily::CallTarget,
            FactFamily::UnresolvedCall,
        ] {
            meta.finish_family_insertions(family);
        }
    }
}

fn call_site_metadata(
    db: &(impl AnalysisHost + ?Sized),
    interner: &StableKeyInterner,
    fact: &CallSiteFact,
) -> FactMeta {
    let (precision, confidence) = call_status_metadata(fact.status, fact.precision);
    call_fact_metadata(
        interner,
        precision,
        confidence,
        fact.stable_key,
        [
            ("status", call_status_label(fact.status).to_string()),
            (
                "precision",
                call_precision_label(fact.precision).to_string(),
            ),
            ("kind", call_syntax_kind_label(fact.kind).to_string()),
            ("language", language_label(fact.language).to_string()),
            ("file_key", source_file_key(db, fact.file)),
            ("caller_key", function_key(db, fact.caller, "", &fact.span)),
            (
                "owner_symbol_key",
                fact.owner_symbol
                    .map(|symbol| fact_stable_key(db, FactFamily::Symbol, symbol.0))
                    .unwrap_or_else(none_value),
            ),
            (
                "body_key",
                fact_stable_key(db, FactFamily::MirBody, fact.body.0),
            ),
            (
                "operation_key",
                fact_stable_key(db, FactFamily::MirOperation, fact.operation.0),
            ),
            ("span", span_metadata_value(&fact.span)),
        ],
    )
}

fn call_target_metadata(
    db: &(impl AnalysisHost + ?Sized),
    interner: &StableKeyInterner,
    fact: &CallTargetFact,
) -> FactMeta {
    let (precision, confidence) = call_status_metadata(fact.status, fact.precision);
    call_fact_metadata(
        interner,
        precision,
        confidence,
        fact.stable_key,
        [
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
                fact_stable_key(db, FactFamily::CallSite, fact.site.0),
            ),
            (
                "caller_key",
                fact_stable_key(db, FactFamily::Function, fact.caller.0),
            ),
            (
                "target_function_key",
                fact.target_function
                    .map(|function| fact_stable_key(db, FactFamily::Function, function.0))
                    .unwrap_or_else(none_value),
            ),
            (
                "target_symbol_key",
                fact.target_symbol
                    .map(|symbol| fact_stable_key(db, FactFamily::Symbol, symbol.0))
                    .unwrap_or_else(none_value),
            ),
        ],
    )
}

fn unresolved_call_metadata(
    db: &(impl AnalysisHost + ?Sized),
    interner: &StableKeyInterner,
    fact: &UnresolvedCallFact,
) -> FactMeta {
    let (precision, confidence) = call_status_metadata(fact.status, fact.precision);
    call_fact_metadata(
        interner,
        precision,
        confidence,
        fact.stable_key,
        [
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
                fact_stable_key(db, FactFamily::CallSite, fact.site.0),
            ),
            (
                "caller_key",
                fact_stable_key(db, FactFamily::Function, fact.caller.0),
            ),
        ],
    )
}

fn call_fact_metadata<const N: usize>(
    interner: &StableKeyInterner,
    precision: FactPrecision,
    confidence: FactConfidence,
    stable_key: StableKeyId,
    payload_parts: [(&str, String); N],
) -> FactMeta {
    FactMeta {
        stable_key,
        producer_id: CALLS_PROVIDER_ID,
        layer_id: CALLS_PROVIDER_ID,
        precision,
        confidence,
        validation: ValidationStatus::NativeTrusted,
        payload_digest: metadata_payload_digest(interner, stable_key, &payload_parts),
    }
}

fn metadata_payload_digest(
    interner: &StableKeyInterner,
    stable_key: StableKeyId,
    payload_parts: &[(&str, String)],
) -> String {
    let mut normalized = payload_parts
        .iter()
        .map(|(label, value)| format!("{label}={}", value.replace('\\', "/")))
        .collect::<Vec<_>>();
    normalized.sort();

    let stable_key = interner.resolve(stable_key);
    let mut parts = Vec::with_capacity(normalized.len() + 1);
    parts.push(stable_key.as_ref());
    parts.extend(normalized.iter().map(String::as_str));
    crate::analysis_neutral::hash::stable_hash(&parts)
}

fn fact_stable_key(db: &(impl AnalysisHost + ?Sized), family: FactFamily, run_id: u64) -> String {
    db.metadata_for(FactRef::new(family, run_id))
        .map(|metadata| db.resolve_stable_key(metadata.stable_key).to_string())
        .unwrap_or_else(|| format!("<missing:{}:{run_id}>", family.label()))
}

fn source_file_key(
    db: &(impl AnalysisHost + ?Sized),
    file: crate::internal_core::FileId,
) -> String {
    db.metadata_for(FactRef::new(FactFamily::SourceFile, u64::from(file.0)))
        .map(|metadata| db.resolve_stable_key(metadata.stable_key).to_string())
        .unwrap_or_else(|| db.path_for(file).replace('\\', "/"))
}

fn function_key(
    db: &(impl AnalysisHost + ?Sized),
    function: crate::internal_core::FunctionId,
    name: &str,
    span: &crate::internal_core::Span,
) -> String {
    db.metadata_for(FactRef::new(FactFamily::Function, function.0))
        .map(|metadata| db.resolve_stable_key(metadata.stable_key).to_string())
        .unwrap_or_else(|| {
            stable_key_text_from_parts(
                FactFamily::Function,
                &[
                    ("path", db.path_for(span.file)),
                    ("name", name.to_string()),
                    ("span", span_metadata_value(span)),
                ],
            )
        })
}

fn span_metadata_value(span: &crate::internal_core::Span) -> String {
    format!(
        "{}-{}:{}:{}-{}:{}",
        span.start_byte,
        span.end_byte,
        span.start_line,
        span.start_col,
        span.end_line,
        span.end_col
    )
}

fn none_value() -> String {
    "none".to_string()
}

fn language_label(language: Language) -> &'static str {
    match language {
        Language::Go => "go",
        Language::TypeScript => "typescript",
        Language::Tsx => "tsx",
        Language::JavaScript => "javascript",
        Language::Jsx => "jsx",
        Language::Unknown => "unknown",
        _ => "unknown",
    }
}

fn call_status_metadata(
    status: CallTargetStatus,
    precision: CallPrecision,
) -> (FactPrecision, FactConfidence) {
    let fact_precision = match status {
        CallTargetStatus::Resolved => match precision {
            CallPrecision::Exact | CallPrecision::SetupAware => FactPrecision::SetupAware,
            CallPrecision::Conservative | CallPrecision::Heuristic => FactPrecision::Heuristic,
            CallPrecision::Ambiguous => FactPrecision::Ambiguous,
            CallPrecision::Unknown => FactPrecision::Unresolved,
            CallPrecision::Unsupported => FactPrecision::Unsupported,
        },
        CallTargetStatus::Ambiguous => FactPrecision::Ambiguous,
        CallTargetStatus::Unresolved | CallTargetStatus::BudgetExceeded => {
            FactPrecision::Unresolved
        }
        CallTargetStatus::Unsupported | CallTargetStatus::Rejected => FactPrecision::Unsupported,
        CallTargetStatus::SetupMissing => FactPrecision::SetupMissing,
    };
    let confidence = match status {
        CallTargetStatus::Resolved => FactConfidence::High,
        CallTargetStatus::Ambiguous => FactConfidence::Medium,
        CallTargetStatus::Unresolved
        | CallTargetStatus::Unsupported
        | CallTargetStatus::SetupMissing
        | CallTargetStatus::BudgetExceeded
        | CallTargetStatus::Rejected => FactConfidence::Low,
    };
    (fact_precision, confidence)
}

fn call_status_label(status: CallTargetStatus) -> &'static str {
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

fn call_precision_label(precision: CallPrecision) -> &'static str {
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

fn call_syntax_kind_label(kind: CallSyntaxKind) -> &'static str {
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

fn call_edge_kind_label(kind: CallEdgeKind) -> &'static str {
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

fn call_algorithm_label(algorithm: CallAlgorithm) -> &'static str {
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

fn call_unresolved_reason_label(reason: UnresolvedCallReason) -> &'static str {
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

#[cfg(test)]
mod tests {
    use crate::analysis_api::{FactFamily, FactRef};
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::calls::facts::{
        CallCallee, CallEdgeKind, CallPrecision, CallProvenance, CallSiteFact, CallSyntaxKind,
        CallTargetFact, CallTargetStatus, UnresolvedCallFact, UnresolvedCallReason,
    };
    use crate::analysis_neutral::calls::store::CallOutput;
    use crate::analysis_neutral::ids::{CallSiteId, CallTargetId, MirBodyId, MirOpId};
    use crate::internal_core::{FunctionId, Language, Span};
    use std::path::PathBuf;

    #[test]
    fn replacing_calls_refreshes_metadata_for_all_call_families() {
        let mut db = LocalAnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/app.ts"),
            "src/app.ts".to_string(),
            "function app() { target(); }\n".to_string(),
        );
        let interner = db.stable_key_interner();
        let site_key = interner.intern("call-site:metadata");
        let target_key = interner.intern("call-target:metadata");
        let unresolved_key = interner.intern("unresolved:metadata");
        let site = CallSiteFact {
            id: CallSiteId(0),
            language: Language::TypeScript,
            file,
            caller: FunctionId::from_raw(0),
            owner_symbol: None,
            body: MirBodyId(0),
            operation: MirOpId(0),
            span: Span::point(file, 1, 24),
            kind: CallSyntaxKind::Function,
            callee: CallCallee::Identifier {
                reference: None,
                name: "target".to_string(),
            },
            receiver: None,
            arguments: Vec::new(),
            result: None,
            status: CallTargetStatus::Unsupported,
            precision: CallPrecision::Unsupported,
            in_throw: false,
            stable_key: site_key,
        };
        let target = CallTargetFact {
            id: CallTargetId(0),
            site: site.id,
            caller: site.caller,
            target_function: None,
            target_symbol: None,
            edge_kind: CallEdgeKind::Unknown,
            algorithm: crate::analysis_neutral::calls::facts::CallAlgorithm::Unsupported,
            status: CallTargetStatus::SetupMissing,
            reason: Some(UnresolvedCallReason::SetupMissing),
            provenance: CallProvenance::Native,
            precision: CallPrecision::Unknown,
            stable_key: target_key,
        };
        let unresolved = UnresolvedCallFact {
            site: site.id,
            caller: site.caller,
            status: CallTargetStatus::Unresolved,
            reason: UnresolvedCallReason::Unknown,
            algorithm: crate::analysis_neutral::calls::facts::CallAlgorithm::SyntaxOnly,
            provenance: CallProvenance::MirShape,
            precision: CallPrecision::Unknown,
            stable_key: unresolved_key,
        };

        db.replace_call_facts(CallOutput {
            sites: vec![site],
            targets: vec![target],
            unresolved: vec![unresolved],
        })
        .expect("call facts should install");

        for (family, stable_key) in [
            (FactFamily::CallSite, "call-site:metadata"),
            (FactFamily::CallTarget, "call-target:metadata"),
            (FactFamily::UnresolvedCall, "unresolved:metadata"),
        ] {
            let metadata = db
                .metadata_for(FactRef::new(family, 0))
                .expect("call metadata should be installed");
            assert_eq!(
                metadata.producer_id,
                crate::analysis_neutral::CALLS_PROVIDER_ID
            );
            assert_eq!(
                metadata.layer_id,
                crate::analysis_neutral::CALLS_PROVIDER_ID
            );
            assert_eq!(
                metadata.validation,
                crate::analysis_api::ValidationStatus::NativeTrusted
            );
            assert_ne!(
                metadata.precision,
                crate::analysis_api::FactPrecision::Exact
            );
            assert_eq!(
                db.resolve_stable_key(metadata.stable_key).as_ref(),
                stable_key
            );
            assert!(!metadata.payload_digest.is_empty());
        }

        db.replace_call_facts(CallOutput::empty())
            .expect("empty call facts should install");
        for family in [
            FactFamily::CallSite,
            FactFamily::CallTarget,
            FactFamily::UnresolvedCall,
        ] {
            assert!(
                db.metadata_for(FactRef::new(family, 0)).is_none(),
                "stale metadata should be removed for {family:?}"
            );
        }
        assert!(db.call_sites().is_empty());
    }
}
