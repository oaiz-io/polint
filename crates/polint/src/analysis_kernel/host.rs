//! Facade-owned host session for [`crate::analysis_api::ProviderCtx`].
//!
//! `ProviderCtx` only carries fact-db / digest contracts. Facade providers reach
//! cache/config/plan state through a thread-local owned session installed by the
//! composition root for the duration of scheduled provider execution.

use std::any::Any;
use std::cell::RefCell;

use crate::analysis::summaries::provider::SccClosureProviderOutput;
use crate::analysis_kernel::incremental::InputSnapshot;
use crate::analysis_plan::AnalysisPlan;
use crate::cache::Cache;
use crate::config::LoadedConfig;
use std::path::PathBuf;
use std::sync::Arc;

use crate::analysis_api::{
    AnalysisCache, BranchObligation, CachedFileFacts, CoverageFact, FactDatabase, FactFamily,
    FactMetaStore, FactStore, FunctionFact, ImportFact, JsxAttributeFact, NullHostAttachment,
    PackageFact, ProviderHostServices, SourceFile, StringLiteralFact, TestFact, TsClassFact,
    TsComponentFact,
};
use crate::core::{AnalysisDb, CapabilitySupportView, StableKeyInterner};
use crate::internal_core::{BranchId, FileId, FunctionId, ImportId, Language, PackageId};

thread_local! {
    static PROVIDER_HOST_SESSION: RefCell<Option<ProviderHostSession>> = const { RefCell::new(None) };
}

/// Owned host state cloned into each provider handle (cache is path metadata only).
pub(crate) struct ProviderHostSession {
    pub(crate) cache: Cache,
    pub(crate) loaded: LoadedConfig,
    pub(crate) input_snapshot: InputSnapshot,
    pub(crate) plan: AnalysisPlan,
    pub(crate) capability_support: CapabilitySupportView,
    pub(crate) scc_closure: Option<SccClosureProviderOutput>,
    /// A Go semantic sidecar the schedule started before `polint.go.semantic`
    /// was reached, handed to that provider and to nobody else.
    pub(crate) go_semantic_prefetch: Option<crate::go::semantic::prefetch::GoSemanticPrefetch>,
}

/// Installs a provider host session for the current thread and restores the previous
/// value (if any) when the scope ends — including on panic.
pub(crate) struct ProviderHostSessionScope {
    previous: Option<ProviderHostSession>,
    active: bool,
}

impl ProviderHostSessionScope {
    pub(crate) fn install(session: ProviderHostSession) -> Self {
        let previous = PROVIDER_HOST_SESSION.with(|slot| slot.borrow_mut().replace(session));
        Self {
            previous,
            active: true,
        }
    }

    /// Take the active session and restore any previous session. Disarms [`Drop`].
    pub(crate) fn take(mut self) -> ProviderHostSession {
        self.active = false;
        let current = PROVIDER_HOST_SESSION
            .with(|slot| slot.borrow_mut().take())
            .expect("provider host session installed by analysis kernel");
        if let Some(previous) = self.previous.take() {
            PROVIDER_HOST_SESSION.with(|slot| {
                *slot.borrow_mut() = Some(previous);
            });
        }
        current
    }
}

impl Drop for ProviderHostSessionScope {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let _ = PROVIDER_HOST_SESSION.with(|slot| slot.borrow_mut().take());
        if let Some(previous) = self.previous.take() {
            PROVIDER_HOST_SESSION.with(|slot| {
                *slot.borrow_mut() = Some(previous);
            });
        }
    }
}

pub(crate) fn with_provider_host_session_mut<R>(
    f: impl FnOnce(&mut ProviderHostSession) -> R,
) -> R {
    PROVIDER_HOST_SESSION.with(|slot| {
        let mut slot = slot.borrow_mut();
        let session = slot
            .as_mut()
            .expect("provider host session installed by analysis kernel");
        f(session)
    })
}

#[cfg(test)]
fn provider_host_session_is_installed() -> bool {
    PROVIDER_HOST_SESSION.with(|slot| slot.borrow().is_some())
}

/// Host services object for `ProviderCtx::host`, carrying plan digest + cache handle.
#[derive(Default)]
pub(crate) struct FacadeHostServices {
    pub(crate) plan_digest: String,
    pub(crate) analysis_cache: Option<Arc<dyn AnalysisCache>>,
}

impl ProviderHostServices for FacadeHostServices {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn plan_digest(&self) -> &str {
        &self.plan_digest
    }

    fn analysis_cache(&self) -> Option<&dyn AnalysisCache> {
        self.analysis_cache.as_deref()
    }
}

pub(crate) type FacadeHostAttachment = NullHostAttachment;

impl FactDatabase for AnalysisDb {
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
        self.invalidate_fact_view_indexes();
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
        AnalysisDb::files(self)
    }

    fn file(&self, id: FileId) -> Option<&SourceFile> {
        AnalysisDb::file(self, id)
    }

    fn path_for(&self, file: FileId) -> String {
        AnalysisDb::path_for(self, file)
    }

    fn add_file(&mut self, path: PathBuf, relative_path: String, source: String) -> FileId {
        AnalysisDb::add_file(self, path, relative_path, source)
    }

    fn add_source_file(
        &mut self,
        path: PathBuf,
        relative_path: String,
        language: Language,
        source: Arc<str>,
        content_hash: String,
    ) -> FileId {
        AnalysisDb::add_source_file(self, path, relative_path, language, source, content_hash)
    }

    fn push_package(&mut self, fact: PackageFact) -> PackageId {
        AnalysisDb::push_package(self, fact)
    }

    fn push_function(&mut self, fact: FunctionFact) -> FunctionId {
        AnalysisDb::push_function(self, fact)
    }

    fn push_import(&mut self, fact: ImportFact) -> ImportId {
        AnalysisDb::push_import(self, fact)
    }

    fn push_branch(&mut self, fact: BranchObligation) -> BranchId {
        AnalysisDb::push_branch(self, fact)
    }

    fn push_test(&mut self, fact: TestFact) {
        AnalysisDb::push_test(self, fact)
    }

    fn push_coverage(&mut self, fact: CoverageFact) {
        AnalysisDb::push_coverage(self, fact)
    }

    fn push_string_literal(&mut self, fact: StringLiteralFact) {
        AnalysisDb::push_string_literal(self, fact)
    }

    fn push_ts_component(&mut self, fact: TsComponentFact) {
        AnalysisDb::push_ts_component(self, fact)
    }

    fn push_ts_class(&mut self, fact: TsClassFact) {
        AnalysisDb::push_ts_class(self, fact)
    }

    fn push_jsx_attribute(&mut self, fact: JsxAttributeFact) {
        AnalysisDb::push_jsx_attribute(self, fact)
    }

    fn push_go_type(&mut self, fact: crate::analysis_api::GoTypeDeclFact) {
        AnalysisDb::push_go_type(self, fact)
    }

    fn packages(&self) -> &[PackageFact] {
        AnalysisDb::packages(self)
    }

    fn functions(&self) -> &[FunctionFact] {
        AnalysisDb::functions(self)
    }

    fn imports(&self) -> &[ImportFact] {
        AnalysisDb::imports(self)
    }

    fn branches(&self) -> &[BranchObligation] {
        AnalysisDb::branches(self)
    }

    fn tests(&self) -> &[TestFact] {
        AnalysisDb::tests(self)
    }

    fn coverage(&self) -> &[CoverageFact] {
        AnalysisDb::coverage(self)
    }

    fn string_literals(&self) -> &[StringLiteralFact] {
        AnalysisDb::string_literals(self)
    }

    fn ts_components(&self) -> &[TsComponentFact] {
        AnalysisDb::ts_components(self)
    }

    fn ts_classes(&self) -> &[TsClassFact] {
        AnalysisDb::ts_classes(self)
    }

    fn go_types(&self) -> &[crate::analysis_api::GoTypeDeclFact] {
        AnalysisDb::go_types(self)
    }

    fn jsx_attributes(&self) -> &[JsxAttributeFact] {
        AnalysisDb::jsx_attributes(self)
    }

    fn module_nodes(&self) -> &[crate::analysis_api::ModuleNode] {
        AnalysisDb::module_nodes(self)
    }

    fn resolved_imports(&self) -> &[crate::analysis_api::ResolvedImportFact] {
        AnalysisDb::resolved_imports(self)
    }

    fn symbols(&self) -> &[crate::analysis_api::SymbolFact] {
        AnalysisDb::symbols(self)
    }

    fn definitions(&self) -> &[crate::analysis_api::DefinitionFact] {
        AnalysisDb::definitions(self)
    }

    fn references(&self) -> &[crate::analysis_api::ReferenceFact] {
        AnalysisDb::references(self)
    }

    fn replace_symbol_facts(
        &mut self,
        symbols: Vec<crate::analysis_api::SymbolFact>,
        definitions: Vec<crate::analysis_api::DefinitionFact>,
        references: Vec<crate::analysis_api::ReferenceFact>,
    ) {
        AnalysisDb::replace_symbol_graph_facts(self, symbols, definitions, references);
    }

    fn semantic_imports(&self) -> &[crate::analysis_api::SemanticImportFact] {
        AnalysisDb::semantic_imports(self)
    }

    fn file_metrics(&self) -> &[crate::analysis_api::FileMetricFact] {
        AnalysisDb::file_metrics(self)
    }

    fn function_metrics(&self) -> &[crate::analysis_api::FunctionMetricFact] {
        AnalysisDb::function_metrics(self)
    }

    fn complexity_metrics(&self) -> &[crate::analysis_api::ComplexityMetricFact] {
        AnalysisDb::complexity_metrics(self)
    }

    fn facts_for_file(&self, file: FileId) -> CachedFileFacts {
        AnalysisDb::facts_for_file(self, file)
    }

    fn restore_file_facts(&mut self, file: FileId, facts: CachedFileFacts) {
        AnalysisDb::restore_file_facts(self, file, facts)
    }
}

impl crate::analysis_api::CaptureEnrichment for AnalysisDb {
    fn primary_definition_outside_span(
        &self,
        _file: crate::internal_core::FileId,
        symbol: crate::internal_core::SymbolId,
        span_start: u32,
        span_end: u32,
    ) -> Option<String> {
        let definition = self.definition_for_symbol(symbol)?;
        let definition_span = definition.primary_span.as_ref()?;
        (definition_span.start_byte < span_start || definition_span.end_byte > span_end)
            .then(|| definition.name.clone())
    }

    fn reference_targets_in_span(
        &self,
        file: crate::internal_core::FileId,
        span_start: u32,
        span_end: u32,
    ) -> Vec<(String, Option<crate::internal_core::SymbolId>)> {
        self.references_for_file(file)
            .filter(|reference| {
                reference
                    .primary_span
                    .as_ref()
                    .is_some_and(|reference_span| {
                        reference_span.start_byte >= span_start
                            && reference_span.end_byte <= span_end
                    })
            })
            .map(|reference| (reference.name.clone(), reference.target))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_kernel::AnalysisKernel;
    use crate::config::load_config;

    fn test_session(marker: &str) -> ProviderHostSession {
        let temp = tempfile::tempdir().expect("tempdir");
        let _ = marker;
        let loaded = load_config(temp.path()).expect("config");
        let cache = Cache::new("", false);
        let plan = AnalysisPlan::empty();
        let db = AnalysisDb::default();
        let input_snapshot = crate::analysis_kernel::incremental::input_snapshot_from_run_inputs(
            &loaded,
            &db,
            "config",
            "rules",
            plan.digest(),
            AnalysisKernel::provider_manifests(),
        );
        ProviderHostSession {
            cache,
            loaded,
            input_snapshot,
            capability_support: plan.support_view().clone(),
            plan,
            scc_closure: None,
            go_semantic_prefetch: None,
        }
    }

    #[test]
    fn provider_host_session_scope_clears_on_drop() {
        assert!(!provider_host_session_is_installed());
        {
            let _scope = ProviderHostSessionScope::install(test_session("outer"));
            assert!(provider_host_session_is_installed());
        }
        assert!(!provider_host_session_is_installed());
    }

    #[test]
    fn provider_host_session_scope_restores_previous_on_drop() {
        assert!(!provider_host_session_is_installed());
        let outer = ProviderHostSessionScope::install(test_session("outer"));
        assert!(provider_host_session_is_installed());
        {
            let _inner = ProviderHostSessionScope::install(test_session("inner"));
            assert!(provider_host_session_is_installed());
        }
        assert!(provider_host_session_is_installed());
        let _ = outer.take();
        assert!(!provider_host_session_is_installed());
    }

    #[test]
    fn provider_host_session_scope_take_disarms_drop() {
        assert!(!provider_host_session_is_installed());
        let scope = ProviderHostSessionScope::install(test_session("take"));
        let session = scope.take();
        assert!(!provider_host_session_is_installed());
        assert!(session.scc_closure.is_none());
    }
}
