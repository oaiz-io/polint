//! Minimal [`FactDatabase`] used by analysis unit tests in place of the facade
//! `AnalysisDb`.

use std::any::Any;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::analysis_api::{
    BranchObligation, CachedFileFacts, ComplexityMetricFact, CoverageFact, DefinitionFact,
    FactDatabase, FactFamily, FactMetaStore, FactStore, FactStoreEntry, FileMetricFact,
    FunctionFact, FunctionMetricFact, ImportFact, JsxAttributeFact, ModuleNode, PackageFact,
    ReferenceFact, ResolvedImportFact, SemanticImportFact, SourceFile, StringLiteralFact,
    SymbolFact, TestFact, TsClassFact, TsComponentFact,
};
#[cfg(test)]
use crate::internal_core::test_stable_key_interner;
use crate::internal_core::{
    BranchId, FileId, FunctionId, ImportId, Language, PackageId, StableKeyInterner, fingerprint,
};

use crate::analysis_neutral::access_paths::store::AccessPathStore;
use crate::analysis_neutral::aliases::store::AliasStore;
use crate::analysis_neutral::calls::store::CallStore;
use crate::analysis_neutral::data_flow::store::DataFlowStore;
use crate::analysis_neutral::domains::store::DomainStore;
use crate::analysis_neutral::entrypoints::store::EntrypointStore;
use crate::analysis_neutral::evidence::store::EvidenceStore;
use crate::analysis_neutral::fact_store::{
    ACCESS_PATH_STORE_FAMILY, ADAPTATION_STORE_FAMILY, ALIAS_STORE_FAMILY, AdaptationFactStore,
    CALL_STORE_FAMILY, CFG_STORE_FAMILY, CfgFactStore, DATA_FLOW_STORE_FAMILY, DOMAIN_STORE_FAMILY,
    ENTRYPOINT_STORE_FAMILY, EVIDENCE_STORE_FAMILY, EXTENSION_STORE_FAMILY, ExtensionFactStore,
    IDENTITY_STORE_FAMILY, POINTS_TO_STORE_FAMILY, REACHABILITY_STORE_FAMILY,
    REFINED_CALL_STORE_FAMILY, SEMANTIC_GRAPH_STORE_FAMILY, SEMANTIC_MIR_STORE_FAMILY,
    SOLVER_STORE_FAMILY, SUMMARY_STORE_FAMILY, TYPE_STORE_FAMILY, VALUE_STORE_FAMILY,
};
use crate::analysis_neutral::identity::store::IdentityStore;
use crate::analysis_neutral::points_to::store::PointsToStore;
use crate::analysis_neutral::reachability::store::ReachabilityStore;
use crate::analysis_neutral::refined_calls::store::RefinedCallStore;
use crate::analysis_neutral::semantic_graph::store::SemanticGraphStore;
use crate::analysis_neutral::solver::store::SolverStore;
use crate::analysis_neutral::store::SemanticStore;
use crate::analysis_neutral::summaries::store::SummaryStore;
use crate::analysis_neutral::symbol_graph::semantic::{NamespaceMemberCallFact, ValueFunctionFact};
use crate::analysis_neutral::types::store::TypeStore;
use crate::analysis_neutral::values::store::ValueStore;

#[derive(Debug)]
pub struct LocalAnalysisDb {
    files: Vec<SourceFile>,
    stable_keys: StableKeyInterner,
    fact_meta: FactMetaStore,
    fact_stores: BTreeMap<FactFamily, FactStoreEntry>,
    packages: Vec<PackageFact>,
    functions: Vec<FunctionFact>,
    imports: Vec<ImportFact>,
    resolved_imports: Vec<ResolvedImportFact>,
    module_nodes: Vec<ModuleNode>,
    branches: Vec<BranchObligation>,
    tests: Vec<TestFact>,
    coverage: Vec<CoverageFact>,
    string_literals: Vec<StringLiteralFact>,
    ts_components: Vec<TsComponentFact>,
    ts_classes: Vec<TsClassFact>,
    jsx_attributes: Vec<JsxAttributeFact>,
    go_types: Vec<crate::analysis_api::GoTypeDeclFact>,
    symbols: Vec<SymbolFact>,
    definitions: Vec<DefinitionFact>,
    references: Vec<ReferenceFact>,
    semantic_imports: Vec<SemanticImportFact>,
    value_functions: Vec<ValueFunctionFact>,
    namespace_member_calls: Vec<NamespaceMemberCallFact>,
    file_metrics: Vec<FileMetricFact>,
    function_metrics: Vec<FunctionMetricFact>,
    complexity_metrics: Vec<ComplexityMetricFact>,
}

impl Default for LocalAnalysisDb {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalAnalysisDb {
    pub fn new() -> Self {
        let mut fact_stores = BTreeMap::new();
        macro_rules! put {
            ($fam:expr, $store:expr) => {{
                let mut store = $store;
                FactStore::clear(&mut store);
                fact_stores.insert($fam, FactStoreEntry::new(store));
            }};
        }
        put!(CFG_STORE_FAMILY, CfgFactStore::default());
        put!(CALL_STORE_FAMILY, CallStore::default());
        put!(IDENTITY_STORE_FAMILY, IdentityStore::default());
        put!(REFINED_CALL_STORE_FAMILY, RefinedCallStore::default());
        put!(DATA_FLOW_STORE_FAMILY, DataFlowStore::default());
        put!(EVIDENCE_STORE_FAMILY, EvidenceStore::default());
        put!(DOMAIN_STORE_FAMILY, DomainStore::default());
        put!(SUMMARY_STORE_FAMILY, SummaryStore::default());
        put!(ENTRYPOINT_STORE_FAMILY, EntrypointStore::default());
        put!(TYPE_STORE_FAMILY, TypeStore::default());
        put!(VALUE_STORE_FAMILY, ValueStore::default());
        put!(ACCESS_PATH_STORE_FAMILY, AccessPathStore::default());
        put!(POINTS_TO_STORE_FAMILY, PointsToStore::default());
        put!(ALIAS_STORE_FAMILY, AliasStore::default());
        put!(EXTENSION_STORE_FAMILY, ExtensionFactStore::default());
        put!(ADAPTATION_STORE_FAMILY, AdaptationFactStore::default());
        put!(REACHABILITY_STORE_FAMILY, ReachabilityStore::default());
        put!(SEMANTIC_GRAPH_STORE_FAMILY, SemanticGraphStore::default());
        put!(SOLVER_STORE_FAMILY, SolverStore::default());
        put!(SEMANTIC_MIR_STORE_FAMILY, SemanticStore::default());

        Self {
            files: Vec::new(),
            stable_keys: {
                #[cfg(test)]
                {
                    test_stable_key_interner()
                }
                #[cfg(not(test))]
                {
                    StableKeyInterner::default()
                }
            },
            fact_meta: FactMetaStore::default(),
            fact_stores,
            packages: Vec::new(),
            functions: Vec::new(),
            imports: Vec::new(),
            resolved_imports: Vec::new(),
            module_nodes: Vec::new(),
            branches: Vec::new(),
            tests: Vec::new(),
            coverage: Vec::new(),
            string_literals: Vec::new(),
            ts_components: Vec::new(),
            ts_classes: Vec::new(),
            jsx_attributes: Vec::new(),
            go_types: Vec::new(),
            symbols: Vec::new(),
            definitions: Vec::new(),
            references: Vec::new(),
            semantic_imports: Vec::new(),
            value_functions: Vec::new(),
            namespace_member_calls: Vec::new(),
            file_metrics: Vec::new(),
            function_metrics: Vec::new(),
            complexity_metrics: Vec::new(),
        }
    }

    /// Inherent helpers so unit-test fixtures do not need [`FactDatabase`] in scope.
    pub fn add_file(&mut self, path: PathBuf, relative_path: String, source: String) -> FileId {
        FactDatabase::add_file(self, path, relative_path, source)
    }

    pub fn add_source_file(
        &mut self,
        path: PathBuf,
        relative_path: String,
        language: Language,
        source: Arc<str>,
        content_hash: String,
    ) -> FileId {
        FactDatabase::add_source_file(self, path, relative_path, language, source, content_hash)
    }

    pub fn semantic_imports(&self) -> &[SemanticImportFact] {
        FactDatabase::semantic_imports(self)
    }

    pub fn file(&self, id: FileId) -> Option<&SourceFile> {
        FactDatabase::file(self, id)
    }

    pub fn stable_key_interner(&self) -> StableKeyInterner {
        FactDatabase::stable_key_interner(self)
    }

    pub fn functions(&self) -> &[FunctionFact] {
        FactDatabase::functions(self)
    }

    pub fn push_function(&mut self, fact: FunctionFact) -> FunctionId {
        FactDatabase::push_function(self, fact)
    }

    pub fn push_import(&mut self, fact: ImportFact) -> ImportId {
        FactDatabase::push_import(self, fact)
    }

    pub fn push_package(&mut self, fact: PackageFact) -> PackageId {
        FactDatabase::push_package(self, fact)
    }

    pub fn fact_meta(&self) -> &FactMetaStore {
        FactDatabase::fact_meta(self)
    }

    pub fn resolve_stable_key(&self, id: crate::internal_core::StableKeyId) -> std::sync::Arc<str> {
        crate::analysis_neutral::AnalysisHost::resolve_stable_key(self, id)
    }

    pub fn replace_summary_facts(
        &mut self,
        output: crate::analysis_neutral::summaries::store::SummaryOutput,
    ) {
        crate::analysis_neutral::AnalysisHost::replace_summary_facts(self, output)
    }

    pub fn summary_facts(&self) -> &[crate::analysis_neutral::summaries::facts::SummaryFact] {
        crate::analysis_neutral::AnalysisHost::summary_facts(self)
    }

    pub fn summary_events(&self) -> &[crate::analysis_neutral::summaries::facts::SummaryEventFact] {
        crate::analysis_neutral::AnalysisHost::summary_events(self)
    }

    pub fn summary_store(
        &self,
    ) -> Option<&crate::analysis_neutral::summaries::store::SummaryStore> {
        Some(crate::analysis_neutral::AnalysisHost::summary_store(self))
    }

    pub fn replace_abstract_domain_facts(
        &mut self,
        output: crate::analysis_neutral::domains::store::DomainOutput,
    ) {
        crate::analysis_neutral::AnalysisHost::replace_abstract_domain_facts(self, output)
    }

    pub fn abstract_domain_observations(
        &self,
    ) -> &[crate::analysis_neutral::domains::facts::DomainObservationFact] {
        crate::analysis_neutral::AnalysisHost::domain_store_inner(self).observations()
    }

    pub fn fact_meta_mut_for_test(&mut self) -> &mut FactMetaStore {
        crate::analysis_neutral::AnalysisHost::fact_meta_mut_for_test(self)
    }

    pub fn files(&self) -> &[SourceFile] {
        FactDatabase::files(self)
    }

    pub fn path_for(&self, file: FileId) -> String {
        FactDatabase::path_for(self, file)
    }

    pub fn packages(&self) -> &[PackageFact] {
        FactDatabase::packages(self)
    }

    pub fn imports(&self) -> &[ImportFact] {
        FactDatabase::imports(self)
    }

    pub fn resolved_imports(&self) -> &[ResolvedImportFact] {
        FactDatabase::resolved_imports(self)
    }

    pub fn module_nodes(&self) -> &[ModuleNode] {
        FactDatabase::module_nodes(self)
    }

    pub fn replace_module_graph_facts(
        &mut self,
        resolved_imports: Vec<ResolvedImportFact>,
        module_nodes: Vec<ModuleNode>,
        _module_edges: Vec<crate::analysis_api::ModuleEdge>,
    ) {
        self.resolved_imports = resolved_imports;
        self.module_nodes = module_nodes;
    }

    pub fn mir_bodies(&self) -> &[crate::analysis_neutral::mir_body::MirBody] {
        crate::analysis_neutral::AnalysisHost::mir_bodies(self)
    }

    pub fn mir_operations(&self) -> &[crate::analysis_neutral::mir_op::MirOperation] {
        crate::analysis_neutral::AnalysisHost::mir_operations(self)
    }

    pub fn mir_blocks(&self) -> &[crate::analysis_neutral::mir_body::MirBlock] {
        crate::analysis_neutral::AnalysisHost::mir_blocks(self)
    }

    pub fn mir_places(&self) -> &[crate::analysis_neutral::places::PlaceFact] {
        crate::analysis_neutral::AnalysisHost::mir_places(self)
    }

    pub fn mir_place_types(&self) -> &[crate::analysis_neutral::places::PlaceTypeFact] {
        crate::analysis_neutral::AnalysisHost::mir_place_types(self)
    }

    pub fn call_sites(&self) -> &[crate::analysis_neutral::calls::facts::CallSiteFact] {
        crate::analysis_neutral::AnalysisHost::call_sites(self)
    }

    pub fn call_targets(&self) -> &[crate::analysis_neutral::calls::facts::CallTargetFact] {
        crate::analysis_neutral::AnalysisHost::call_targets(self)
    }

    pub fn unresolved_calls(&self) -> &[crate::analysis_neutral::calls::facts::UnresolvedCallFact] {
        crate::analysis_neutral::AnalysisHost::unresolved_calls(self)
    }

    pub fn unsupported_semantics(
        &self,
    ) -> &[crate::analysis_neutral::mir_op::UnsupportedSemanticFact] {
        crate::analysis_neutral::AnalysisHost::unsupported_semantics(self)
    }

    pub fn cfg_functions(&self) -> &[crate::analysis_neutral::cfg::facts::CfgFunctionFact] {
        crate::analysis_neutral::AnalysisHost::cfg_functions(self)
    }
    pub fn cfg_nodes(&self) -> &[crate::analysis_neutral::cfg::facts::CfgNodeFact] {
        crate::analysis_neutral::AnalysisHost::cfg_nodes(self)
    }
    pub fn cfg_edges(&self) -> &[crate::analysis_neutral::cfg::facts::CfgEdgeFact] {
        crate::analysis_neutral::AnalysisHost::cfg_edges(self)
    }
    pub fn refined_call_edges(
        &self,
    ) -> &[crate::analysis_neutral::refined_calls::facts::RefinedCallEdgeFact] {
        crate::analysis_neutral::AnalysisHost::refined_call_edges(self)
    }
    pub fn replace_call_facts(
        &mut self,
        output: crate::analysis_neutral::calls::store::CallOutput,
    ) -> Result<(), crate::analysis_neutral::AnalysisError> {
        crate::analysis_neutral::AnalysisHost::replace_call_facts(self, output)
    }
    pub fn replace_refined_call_facts(
        &mut self,
        output: crate::analysis_neutral::refined_calls::store::RefinedCallOutput,
    ) -> Result<(), crate::analysis_neutral::AnalysisError> {
        crate::analysis_neutral::AnalysisHost::replace_refined_call_facts(self, output)
    }
    pub fn metadata_for(
        &self,
        fact_ref: crate::analysis_api::FactRef,
    ) -> Option<&crate::analysis_api::FactMeta> {
        crate::analysis_neutral::AnalysisHost::metadata_for(self, fact_ref)
    }

    pub fn replace_semantic_mir(
        &mut self,
        output: crate::analysis_neutral::mir_body::MirOutput,
    ) -> Result<(), crate::analysis_neutral::AnalysisError> {
        crate::analysis_neutral::AnalysisHost::replace_semantic_mir(self, output)
    }

    pub fn replace_symbol_graph_facts(
        &mut self,
        symbols: Vec<SymbolFact>,
        definitions: Vec<DefinitionFact>,
        references: Vec<ReferenceFact>,
    ) {
        FactDatabase::replace_symbol_facts(self, symbols, definitions, references);
    }

    pub fn replace_semantic_imports(&mut self, imports: Vec<SemanticImportFact>) {
        FactDatabase::replace_semantic_imports(self, imports);
    }

    pub fn replace_value_functions(&mut self, value_functions: Vec<ValueFunctionFact>) {
        self.value_functions = value_functions;
    }

    pub fn value_functions(&self) -> &[ValueFunctionFact] {
        &self.value_functions
    }

    pub fn replace_namespace_member_calls(
        &mut self,
        namespace_member_calls: Vec<NamespaceMemberCallFact>,
    ) {
        self.namespace_member_calls = namespace_member_calls;
    }

    pub fn namespace_member_calls(&self) -> &[NamespaceMemberCallFact] {
        &self.namespace_member_calls
    }

    pub fn symbols(&self) -> &[SymbolFact] {
        FactDatabase::symbols(self)
    }

    pub fn references(&self) -> &[ReferenceFact] {
        FactDatabase::references(self)
    }
}

impl FactDatabase for LocalAnalysisDb {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn store(&self, family: FactFamily) -> Option<&dyn FactStore> {
        self.fact_stores.get(&family).map(|entry| entry.as_store())
    }

    fn store_mut(&mut self, family: FactFamily) -> Option<&mut dyn FactStore> {
        self.fact_stores
            .get_mut(&family)
            .map(|entry| entry.as_store_mut())
    }

    fn fact_meta(&self) -> &FactMetaStore {
        &self.fact_meta
    }

    fn fact_meta_mut(&mut self) -> &mut FactMetaStore {
        &mut self.fact_meta
    }

    fn stable_key_interner(&self) -> StableKeyInterner {
        self.stable_keys.clone()
    }

    fn files(&self) -> &[SourceFile] {
        &self.files
    }

    fn file(&self, id: FileId) -> Option<&SourceFile> {
        self.files.get(id.0 as usize)
    }

    fn path_for(&self, file: FileId) -> String {
        self.file(file)
            .map(|file| file.relative_path.clone())
            .unwrap_or_else(|| "<unknown>".to_string())
    }

    fn add_file(&mut self, path: PathBuf, relative_path: String, source: String) -> FileId {
        let language = Language::from_path(&path);
        let content_hash = fingerprint(&[&source]);
        self.add_source_file(
            path,
            relative_path,
            language,
            Arc::from(source),
            content_hash,
        )
    }

    fn add_source_file(
        &mut self,
        path: PathBuf,
        relative_path: String,
        language: Language,
        source: Arc<str>,
        content_hash: String,
    ) -> FileId {
        let id = FileId::from_raw(self.files.len() as u32);
        self.files.push(SourceFile::new(
            id,
            path,
            relative_path,
            language,
            source,
            content_hash,
        ));
        id
    }

    fn push_package(&mut self, mut fact: PackageFact) -> PackageId {
        let id = PackageId::from_raw(self.packages.len() as u64);
        fact.id = id;
        self.packages.push(fact);
        id
    }

    fn push_function(&mut self, mut fact: FunctionFact) -> FunctionId {
        let id = FunctionId::from_raw(self.functions.len() as u64);
        fact.id = id;
        self.functions.push(fact);
        id
    }

    fn push_import(&mut self, mut fact: ImportFact) -> ImportId {
        let id = ImportId::from_raw(self.imports.len() as u64);
        fact.id = id;
        self.imports.push(fact);
        id
    }

    fn push_branch(&mut self, mut fact: BranchObligation) -> BranchId {
        let id = BranchId::from_raw(self.branches.len() as u64);
        fact.id = id;
        self.branches.push(fact);
        id
    }

    fn push_test(&mut self, fact: TestFact) {
        self.tests.push(fact);
    }

    fn push_coverage(&mut self, fact: CoverageFact) {
        self.coverage.push(fact);
    }

    fn push_string_literal(&mut self, fact: StringLiteralFact) {
        self.string_literals.push(fact);
    }

    fn push_ts_component(&mut self, fact: TsComponentFact) {
        self.ts_components.push(fact);
    }

    fn push_go_type(&mut self, fact: crate::analysis_api::GoTypeDeclFact) {
        self.go_types.push(fact);
    }

    fn push_ts_class(&mut self, fact: TsClassFact) {
        self.ts_classes.push(fact);
    }

    fn push_jsx_attribute(&mut self, fact: JsxAttributeFact) {
        self.jsx_attributes.push(fact);
    }

    fn packages(&self) -> &[PackageFact] {
        &self.packages
    }

    fn functions(&self) -> &[FunctionFact] {
        &self.functions
    }

    fn imports(&self) -> &[ImportFact] {
        &self.imports
    }

    fn branches(&self) -> &[BranchObligation] {
        &self.branches
    }

    fn tests(&self) -> &[TestFact] {
        &self.tests
    }

    fn coverage(&self) -> &[CoverageFact] {
        &self.coverage
    }

    fn string_literals(&self) -> &[StringLiteralFact] {
        &self.string_literals
    }

    fn ts_components(&self) -> &[TsComponentFact] {
        &self.ts_components
    }

    fn ts_classes(&self) -> &[TsClassFact] {
        &self.ts_classes
    }

    fn jsx_attributes(&self) -> &[JsxAttributeFact] {
        &self.jsx_attributes
    }

    fn go_types(&self) -> &[crate::analysis_api::GoTypeDeclFact] {
        &self.go_types
    }

    fn module_nodes(&self) -> &[crate::analysis_api::ModuleNode] {
        &self.module_nodes
    }

    fn resolved_imports(&self) -> &[ResolvedImportFact] {
        &self.resolved_imports
    }

    fn symbols(&self) -> &[SymbolFact] {
        &self.symbols
    }

    fn definitions(&self) -> &[DefinitionFact] {
        &self.definitions
    }

    fn references(&self) -> &[ReferenceFact] {
        &self.references
    }

    fn replace_symbol_facts(
        &mut self,
        symbols: Vec<SymbolFact>,
        definitions: Vec<DefinitionFact>,
        references: Vec<ReferenceFact>,
    ) {
        self.symbols = symbols;
        self.definitions = definitions;
        self.references = references;
    }

    fn semantic_imports(&self) -> &[SemanticImportFact] {
        &self.semantic_imports
    }

    fn replace_semantic_imports(&mut self, imports: Vec<SemanticImportFact>) {
        self.semantic_imports = imports;
    }

    fn file_metrics(&self) -> &[FileMetricFact] {
        &self.file_metrics
    }

    fn function_metrics(&self) -> &[FunctionMetricFact] {
        &self.function_metrics
    }

    fn complexity_metrics(&self) -> &[ComplexityMetricFact] {
        &self.complexity_metrics
    }

    fn facts_for_file(&self, file: FileId) -> CachedFileFacts {
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
            coverage: self.coverage().to_vec(),
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

    fn restore_file_facts(&mut self, file: FileId, facts: CachedFileFacts) {
        for mut package in facts.packages {
            package.file = file;
            package.span.file = file;
            self.push_package(package);
        }
        for mut function in facts.functions {
            function.file = file;
            function.span.file = file;
            self.push_function(function);
        }
        for mut import in facts.imports {
            import.file = file;
            import.span.file = file;
            self.push_import(import);
        }
        for mut branch in facts.branches {
            branch.file = file;
            branch.decision_span.file = file;
            self.push_branch(branch);
        }
        for mut test in facts.tests {
            test.file = file;
            test.span.file = file;
            self.push_test(test);
        }
        for coverage in facts.coverage {
            self.push_coverage(coverage);
        }
        for mut component in facts.ts_components {
            component.file = file;
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
    }
}
