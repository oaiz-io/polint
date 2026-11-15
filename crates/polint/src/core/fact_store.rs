//! Provider-owned fact stores behind a keyed registry on [`super::AnalysisDb`].
//!
//! Analysis-family stores and their [`FactStore`] impls live in `polint-analysis`.
//! This module keeps module/symbol/metrics stores plus frontend family re-exports.

use std::any::Any;
use std::collections::BTreeMap;

use crate::core::facts::{
    ComplexityMetricFact, CoverageFact, DefinitionFact, FileMetricFact, FunctionMetricFact,
    ModuleEdge, ModuleNode, ReferenceFact, ResolvedImportFact, SymbolFact,
};
use crate::core::ids::SymbolId;
pub(crate) use crate::go::semantic::store::GO_SEMANTIC_STORE_FAMILY;
pub(crate) use crate::go::{GO_SYNTAX_STORE_FAMILY, GoSyntaxStore};
use crate::module_graph::topology::{
    DependencyRequirementFact, ImportToPackageFact, RepoTopologyOverlayFact,
    ResolvedDependencyEdgeFact, SourceSetFact, TopologyOutput, TopologyPackageFact,
    WorkspaceRootFact,
};
use crate::symbol_graph::semantic::{
    AliasFact, AliasId, ExportFact, ExportId, GeneratedSymbolFact, GeneratedSymbolId,
    ResolutionFact, ResolutionId, ScopeFact, ScopeId, SemanticImportFact, SemanticImportId,
    StableExportId, StableExportIdentity, ValueFunctionFact,
};
pub(crate) use crate::ts::object_model::store::TS_OBJECT_MODEL_STORE_FAMILY;
pub(crate) use crate::ts::{TS_SYNTAX_STORE_FAMILY, TsSyntaxStore};

use crate::analysis_api::FactFamily;
pub(crate) use crate::analysis_api::{FactStore, FactStoreEntry};
pub(crate) use crate::analysis_neutral::fact_store::{
    ACCESS_PATH_STORE_FAMILY, ADAPTATION_STORE_FAMILY, ALIAS_STORE_FAMILY, AdaptationFactStore,
    CALL_STORE_FAMILY, CFG_STORE_FAMILY, CfgFactStore, DATA_FLOW_STORE_FAMILY, DOMAIN_STORE_FAMILY,
    ENTRYPOINT_STORE_FAMILY, EVIDENCE_STORE_FAMILY, EXTENSION_STORE_FAMILY, ExtensionFactStore,
    IDENTITY_STORE_FAMILY, POINTS_TO_STORE_FAMILY, REACHABILITY_STORE_FAMILY,
    REFINED_CALL_STORE_FAMILY, SEMANTIC_GRAPH_STORE_FAMILY, SEMANTIC_MIR_STORE_FAMILY,
    SOLVER_STORE_FAMILY, SUMMARY_STORE_FAMILY, TYPE_STORE_FAMILY, VALUE_STORE_FAMILY,
};

/// Module-graph facts produced by `polint.module_graph`.
#[derive(Debug, Clone, Default)]
pub(crate) struct ModuleGraphStore {
    pub(crate) resolved_imports: Vec<ResolvedImportFact>,
    pub(crate) module_nodes: Vec<ModuleNode>,
    pub(crate) module_edges: Vec<ModuleEdge>,
}

impl ModuleGraphStore {
    pub(crate) fn resolved_imports(&self) -> &[ResolvedImportFact] {
        &self.resolved_imports
    }

    pub(crate) fn module_nodes(&self) -> &[ModuleNode] {
        &self.module_nodes
    }

    pub(crate) fn module_edges(&self) -> &[ModuleEdge] {
        &self.module_edges
    }

    pub(crate) fn replace(
        &mut self,
        resolved_imports: Vec<ResolvedImportFact>,
        module_nodes: Vec<ModuleNode>,
        module_edges: Vec<ModuleEdge>,
    ) {
        self.resolved_imports = resolved_imports;
        self.module_nodes = module_nodes;
        self.module_edges = module_edges;
    }
}

impl FactStore for ModuleGraphStore {
    fn family(&self) -> FactFamily {
        FactFamily::ResolvedImport
    }

    fn clear(&mut self) {
        *self = ModuleGraphStore::default();
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn FactStore> {
        Box::new(self.clone())
    }
}

/// Registry key used for [`ModuleGraphStore`] in `AnalysisDb::fact_stores`.
pub(crate) const MODULE_GRAPH_STORE_FAMILY: FactFamily = FactFamily::ResolvedImport;

/// Topology facts produced by `polint.module_topology`.
#[derive(Debug, Clone, Default)]
pub(crate) struct ModuleTopologyStore {
    output: TopologyOutput,
}

impl ModuleTopologyStore {
    pub(crate) fn from_output(output: TopologyOutput) -> Self {
        Self { output }
    }

    pub(crate) fn workspace_roots(&self) -> &[WorkspaceRootFact] {
        &self.output.workspace_roots
    }

    pub(crate) fn topology_packages(&self) -> &[TopologyPackageFact] {
        &self.output.packages
    }

    pub(crate) fn source_sets(&self) -> &[SourceSetFact] {
        &self.output.source_sets
    }

    pub(crate) fn dependency_requirements(&self) -> &[DependencyRequirementFact] {
        &self.output.dependency_requirements
    }

    pub(crate) fn resolved_dependency_edges(&self) -> &[ResolvedDependencyEdgeFact] {
        &self.output.resolved_dependency_edges
    }

    pub(crate) fn import_to_package_edges(&self) -> &[ImportToPackageFact] {
        &self.output.import_to_package_edges
    }

    pub(crate) fn repo_topology_overlays(&self) -> &[RepoTopologyOverlayFact] {
        &self.output.overlays
    }

    pub(crate) fn replace_import_to_package_edges(&mut self, edges: Vec<ImportToPackageFact>) {
        self.output.import_to_package_edges = edges;
    }
}

impl FactStore for ModuleTopologyStore {
    fn family(&self) -> FactFamily {
        FactFamily::WorkspaceRoot
    }

    fn clear(&mut self) {
        *self = ModuleTopologyStore::default();
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn FactStore> {
        Box::new(self.clone())
    }
}

/// Registry key used for [`ModuleTopologyStore`] in `AnalysisDb::fact_stores`.
pub(crate) const MODULE_TOPOLOGY_STORE_FAMILY: FactFamily = FactFamily::WorkspaceRoot;

/// Symbol-graph facts and derived indexes produced by `polint.symbol_graph`.
///
/// `symbols_by_name` stays keyed by `String` (not StableKeyId) until a later
/// coordinated interning change.
#[derive(Debug, Clone, Default)]
pub(crate) struct SymbolStore {
    pub(crate) symbols: Vec<SymbolFact>,
    pub(crate) definitions: Vec<DefinitionFact>,
    pub(crate) references: Vec<ReferenceFact>,
    pub(crate) symbols_by_id: BTreeMap<SymbolId, usize>,
    pub(crate) definitions_by_symbol: BTreeMap<SymbolId, Vec<usize>>,
}

impl SymbolStore {
    pub(crate) fn symbols(&self) -> &[SymbolFact] {
        &self.symbols
    }

    pub(crate) fn definitions(&self) -> &[DefinitionFact] {
        &self.definitions
    }

    pub(crate) fn references(&self) -> &[ReferenceFact] {
        &self.references
    }

    pub(crate) fn replace(
        &mut self,
        symbols: Vec<SymbolFact>,
        definitions: Vec<DefinitionFact>,
        references: Vec<ReferenceFact>,
    ) {
        self.symbols = symbols;
        self.definitions = definitions;
        self.references = references;
        self.rebuild_indexes();
    }

    pub(crate) fn symbol_by_id(&self, id: SymbolId) -> Option<&SymbolFact> {
        self.symbols_by_id
            .get(&id)
            .and_then(|index| self.symbols.get(*index))
    }

    pub(crate) fn definitions_for_symbol(
        &self,
        symbol: SymbolId,
    ) -> impl Iterator<Item = &DefinitionFact> + '_ {
        self.definitions_by_symbol
            .get(&symbol)
            .into_iter()
            .flat_map(|indexes| {
                indexes
                    .iter()
                    .filter_map(|index| self.definitions.get(*index))
            })
    }

    fn rebuild_indexes(&mut self) {
        self.symbols_by_id.clear();
        self.definitions_by_symbol.clear();

        for (index, symbol) in self.symbols.iter().enumerate() {
            self.symbols_by_id.insert(symbol.id, index);
        }

        for (index, definition) in self.definitions.iter().enumerate() {
            self.definitions_by_symbol
                .entry(definition.symbol)
                .or_default()
                .push(index);
        }

        let definitions = &self.definitions;
        for indexes in self.definitions_by_symbol.values_mut() {
            indexes.sort_by_key(|index| definitions[*index].id);
        }
    }
}

impl FactStore for SymbolStore {
    fn family(&self) -> FactFamily {
        FactFamily::Symbol
    }

    fn clear(&mut self) {
        *self = SymbolStore::default();
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn FactStore> {
        Box::new(self.clone())
    }
}

/// Registry key used for [`SymbolStore`] in `AnalysisDb::fact_stores`.
pub(crate) const SYMBOL_STORE_FAMILY: FactFamily = FactFamily::Symbol;

/// Semantic-index facts and by-id indexes produced by the symbol-graph semantic layer.
#[derive(Debug, Clone, Default)]
pub(crate) struct SemanticIndexStore {
    pub(crate) scopes: Vec<ScopeFact>,
    pub(crate) semantic_imports: Vec<SemanticImportFact>,
    pub(crate) exports: Vec<ExportFact>,
    pub(crate) aliases: Vec<AliasFact>,
    pub(crate) resolution_facts: Vec<ResolutionFact>,
    pub(crate) generated_symbols: Vec<GeneratedSymbolFact>,
    pub(crate) stable_exports: Vec<StableExportIdentity>,
    pub(crate) value_functions: Vec<ValueFunctionFact>,
    pub(crate) scopes_by_id: BTreeMap<ScopeId, usize>,
    pub(crate) semantic_imports_by_id: BTreeMap<SemanticImportId, usize>,
    pub(crate) exports_by_id: BTreeMap<ExportId, usize>,
    pub(crate) aliases_by_id: BTreeMap<AliasId, usize>,
    pub(crate) resolution_facts_by_id: BTreeMap<ResolutionId, usize>,
    pub(crate) generated_symbols_by_id: BTreeMap<GeneratedSymbolId, usize>,
    pub(crate) stable_exports_by_id: BTreeMap<StableExportId, usize>,
}

impl SemanticIndexStore {
    pub(crate) fn scopes(&self) -> &[ScopeFact] {
        &self.scopes
    }

    pub(crate) fn semantic_imports(&self) -> &[SemanticImportFact] {
        &self.semantic_imports
    }

    pub(crate) fn exports(&self) -> &[ExportFact] {
        &self.exports
    }

    pub(crate) fn aliases(&self) -> &[AliasFact] {
        &self.aliases
    }

    pub(crate) fn resolution_facts(&self) -> &[ResolutionFact] {
        &self.resolution_facts
    }

    pub(crate) fn generated_symbols(&self) -> &[GeneratedSymbolFact] {
        &self.generated_symbols
    }

    pub(crate) fn stable_exports(&self) -> &[StableExportIdentity] {
        &self.stable_exports
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "semantic index replacement accepts every internal semantic row family explicitly"
    )]
    pub(crate) fn replace(
        &mut self,
        scopes: Vec<ScopeFact>,
        semantic_imports: Vec<SemanticImportFact>,
        exports: Vec<ExportFact>,
        aliases: Vec<AliasFact>,
        resolution_facts: Vec<ResolutionFact>,
        generated_symbols: Vec<GeneratedSymbolFact>,
        stable_exports: Vec<StableExportIdentity>,
    ) {
        self.scopes = scopes;
        self.semantic_imports = semantic_imports;
        self.exports = exports;
        self.aliases = aliases;
        self.resolution_facts = resolution_facts;
        self.generated_symbols = generated_symbols;
        self.stable_exports = stable_exports;
        // Value-function rows describe the same derivation; a fresh semantic
        // index drops them until the producer supplies its own.
        self.value_functions.clear();
        self.rebuild_indexes();
    }

    pub(crate) fn value_functions(&self) -> &[ValueFunctionFact] {
        &self.value_functions
    }

    pub(crate) fn replace_value_functions(&mut self, value_functions: Vec<ValueFunctionFact>) {
        self.value_functions = value_functions;
    }

    fn rebuild_indexes(&mut self) {
        self.scopes_by_id.clear();
        self.semantic_imports_by_id.clear();
        self.exports_by_id.clear();
        self.aliases_by_id.clear();
        self.resolution_facts_by_id.clear();
        self.generated_symbols_by_id.clear();
        self.stable_exports_by_id.clear();

        for (index, scope) in self.scopes.iter().enumerate() {
            self.scopes_by_id.insert(scope.id, index);
        }
        for (index, import) in self.semantic_imports.iter().enumerate() {
            self.semantic_imports_by_id.insert(import.id, index);
        }
        for (index, export) in self.exports.iter().enumerate() {
            self.exports_by_id.insert(export.id, index);
        }
        for (index, alias) in self.aliases.iter().enumerate() {
            self.aliases_by_id.insert(alias.id, index);
        }
        for (index, resolution) in self.resolution_facts.iter().enumerate() {
            self.resolution_facts_by_id.insert(resolution.id, index);
        }
        for (index, generated) in self.generated_symbols.iter().enumerate() {
            self.generated_symbols_by_id.insert(generated.id, index);
        }
        for (index, stable_export) in self.stable_exports.iter().enumerate() {
            self.stable_exports_by_id.insert(stable_export.id, index);
        }
    }
}

impl FactStore for SemanticIndexStore {
    fn family(&self) -> FactFamily {
        FactFamily::Scope
    }

    fn clear(&mut self) {
        *self = SemanticIndexStore::default();
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn FactStore> {
        Box::new(self.clone())
    }
}

/// Registry key used for [`SemanticIndexStore`] in `AnalysisDb::fact_stores`.
pub(crate) const SEMANTIC_INDEX_STORE_FAMILY: FactFamily = FactFamily::Scope;

/// Coverage and metric facts produced by the metrics/coverage providers.
#[derive(Debug, Clone, Default)]
pub(crate) struct MetricsStore {
    pub(crate) coverage: Vec<CoverageFact>,
    pub(crate) file_metrics: Vec<FileMetricFact>,
    pub(crate) function_metrics: Vec<FunctionMetricFact>,
    pub(crate) complexity_metrics: Vec<ComplexityMetricFact>,
}

impl MetricsStore {
    pub(crate) fn coverage(&self) -> &[CoverageFact] {
        &self.coverage
    }

    pub(crate) fn file_metrics(&self) -> &[FileMetricFact] {
        &self.file_metrics
    }

    pub(crate) fn function_metrics(&self) -> &[FunctionMetricFact] {
        &self.function_metrics
    }

    pub(crate) fn complexity_metrics(&self) -> &[ComplexityMetricFact] {
        &self.complexity_metrics
    }

    pub(crate) fn push_coverage(&mut self, fact: CoverageFact) -> u64 {
        let run_id = self.coverage.len() as u64;
        self.coverage.push(fact);
        run_id
    }

    pub(crate) fn replace_metrics(
        &mut self,
        file_metrics: Vec<FileMetricFact>,
        function_metrics: Vec<FunctionMetricFact>,
        complexity_metrics: Vec<ComplexityMetricFact>,
    ) {
        self.file_metrics = file_metrics;
        self.function_metrics = function_metrics;
        self.complexity_metrics = complexity_metrics;
    }
}

impl FactStore for MetricsStore {
    fn family(&self) -> FactFamily {
        FactFamily::FileMetric
    }

    fn clear(&mut self) {
        *self = MetricsStore::default();
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn FactStore> {
        Box::new(self.clone())
    }
}

/// Registry key used for [`MetricsStore`] in `AnalysisDb::fact_stores`.
pub(crate) const METRICS_STORE_FAMILY: FactFamily = FactFamily::FileMetric;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{AnalysisDb, FileId, Language, PackageFact, PackageId, Span};
    use std::path::PathBuf;

    #[test]
    fn analysis_db_serves_go_syntax_facts_from_registry_store() {
        let mut db = AnalysisDb::new();
        assert!(
            db.fact_store::<GoSyntaxStore>(GO_SYNTAX_STORE_FAMILY)
                .is_some(),
            "GoSyntaxStore must be registered at construction"
        );

        let file = db.add_file(
            PathBuf::from("pkg/x.go"),
            "pkg/x.go".to_string(),
            "package x\n".to_string(),
        );
        let id = db.push_package(PackageFact::new(
            PackageId::from_raw(0),
            file,
            "x".to_string(),
            Span::point(file, 1, 1),
            Language::Go,
        ));
        assert_eq!(id, PackageId::from_raw(0));
        assert_eq!(db.packages().len(), 1);
        assert_eq!(
            db.fact_store::<GoSyntaxStore>(GO_SYNTAX_STORE_FAMILY)
                .map(|store| store.packages().len()),
            Some(1)
        );
        assert_eq!(db.packages()[0].name, "x");
    }

    #[test]
    fn go_syntax_store_clear_empties_owned_vectors() {
        let mut store = GoSyntaxStore::default();
        store.packages.push(PackageFact::new(
            PackageId::from_raw(0),
            FileId::from_raw(0),
            "x".to_string(),
            Span::point(FileId::from_raw(0), 1, 1),
            Language::Go,
        ));
        let erased: &mut dyn FactStore = &mut store;
        erased.clear();
        assert_eq!(erased.family(), FactFamily::Package);
        assert!(store.packages.is_empty());
        assert!(store.functions.is_empty());
        assert!(store.imports.is_empty());
        assert!(store.branches.is_empty());
        assert!(store.tests.is_empty());
    }
}
