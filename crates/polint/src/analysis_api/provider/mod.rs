//! Provider and host-context contracts.

use std::any::Any;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::internal_core::{
    BranchId, Diagnostic, FileId, FunctionId, ImportId, Language, LanguageId, PackageId,
    StableKeyInterner,
};

use crate::analysis_api::digest::{CacheStats, Digest, DigestKind};
use crate::analysis_api::fact_store::FactStore;
use crate::analysis_api::metadata::{FactFamily, FactMetaStore};
use crate::analysis_api::source_file::SourceFile;
use crate::analysis_api::syntax_facts::{
    BranchObligation, CachedFileFacts, CoverageFact, FunctionFact, ImportFact, JsxAttributeFact,
    PackageFact, StringLiteralFact, TestFact, TsClassFact, TsComponentFact,
};

/// Optional symbol-aware enrichment for MIR closure-capture lowering.
///
/// Frontends and analysis crates must not depend on facade symbol stores; the
/// composition root implements this when symbol facts are available. Default
/// methods return no enrichment so unit fixtures can omit symbols.
pub trait CaptureEnrichment {
    fn primary_definition_outside_span(
        &self,
        _file: FileId,
        _symbol: crate::internal_core::SymbolId,
        _span_start: u32,
        _span_end: u32,
    ) -> Option<String> {
        None
    }

    fn reference_targets_in_span(
        &self,
        _file: FileId,
        _span_start: u32,
        _span_end: u32,
    ) -> Vec<(String, Option<crate::internal_core::SymbolId>)> {
        Vec::new()
    }
}

/// Empty enrichment used by unit fixtures without symbol facts.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullCaptureEnrichment;

impl CaptureEnrichment for NullCaptureEnrichment {}

/// Host fact registry neck used by providers and frontends.
pub trait FactDatabase: Any + Send {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn store(&self, family: FactFamily) -> Option<&dyn FactStore>;
    fn store_mut(&mut self, family: FactFamily) -> Option<&mut dyn FactStore>;
    fn fact_meta(&self) -> &FactMetaStore;
    fn fact_meta_mut(&mut self) -> &mut FactMetaStore;
    fn stable_key_interner(&self) -> StableKeyInterner;

    /// Source-file spine shared by language frontends.
    fn files(&self) -> &[SourceFile];
    fn file(&self, id: FileId) -> Option<&SourceFile>;
    fn path_for(&self, file: FileId) -> String;
    fn add_file(&mut self, path: PathBuf, relative_path: String, source: String) -> FileId;
    fn add_source_file(
        &mut self,
        path: PathBuf,
        relative_path: String,
        language: Language,
        source: Arc<str>,
        content_hash: String,
    ) -> FileId;

    fn push_package(&mut self, fact: PackageFact) -> PackageId;
    fn push_function(&mut self, fact: FunctionFact) -> FunctionId;
    fn push_import(&mut self, fact: ImportFact) -> ImportId;
    fn push_branch(&mut self, fact: BranchObligation) -> BranchId;
    fn push_test(&mut self, fact: TestFact);
    fn push_coverage(&mut self, fact: CoverageFact);
    fn push_string_literal(&mut self, fact: StringLiteralFact);
    fn push_ts_component(&mut self, fact: TsComponentFact);
    fn push_ts_class(&mut self, fact: TsClassFact);
    fn push_jsx_attribute(&mut self, fact: JsxAttributeFact);
    fn push_go_type(&mut self, fact: crate::analysis_api::GoTypeDeclFact);

    fn packages(&self) -> &[PackageFact];
    fn functions(&self) -> &[FunctionFact];
    fn imports(&self) -> &[ImportFact];
    fn branches(&self) -> &[BranchObligation];
    fn tests(&self) -> &[TestFact];
    fn coverage(&self) -> &[CoverageFact];
    fn string_literals(&self) -> &[StringLiteralFact];
    fn ts_components(&self) -> &[TsComponentFact];
    fn ts_classes(&self) -> &[TsClassFact];
    /// Go structural type facts (declarations and anonymous struct occurrences).
    fn go_types(&self) -> &[crate::analysis_api::GoTypeDeclFact];
    fn jsx_attributes(&self) -> &[JsxAttributeFact];

    /// Module graph nodes when the composition root has installed them.
    fn module_nodes(&self) -> &[crate::analysis_api::module_facts::ModuleNode];

    /// Resolved import rows when the composition root has installed them.
    fn resolved_imports(&self) -> &[crate::analysis_api::module_facts::ResolvedImportFact] {
        &[]
    }

    /// Symbol-graph rows when the composition root has installed them.
    fn symbols(&self) -> &[crate::analysis_api::symbol_facts::SymbolFact];
    fn definitions(&self) -> &[crate::analysis_api::symbol_facts::DefinitionFact];
    fn references(&self) -> &[crate::analysis_api::symbol_facts::ReferenceFact];
    fn replace_symbol_facts(
        &mut self,
        symbols: Vec<crate::analysis_api::symbol_facts::SymbolFact>,
        definitions: Vec<crate::analysis_api::symbol_facts::DefinitionFact>,
        references: Vec<crate::analysis_api::symbol_facts::ReferenceFact>,
    );
    /// Semantic import bindings used by call-target / module linking. Default empty.
    fn semantic_imports(&self) -> &[crate::analysis_api::symbol_facts::SemanticImportFact] {
        &[]
    }
    fn replace_semantic_imports(
        &mut self,
        _imports: Vec<crate::analysis_api::symbol_facts::SemanticImportFact>,
    ) {
    }
    fn file_metrics(&self) -> &[crate::analysis_api::symbol_facts::FileMetricFact] {
        &[]
    }
    fn function_metrics(&self) -> &[crate::analysis_api::symbol_facts::FunctionMetricFact] {
        &[]
    }
    fn complexity_metrics(&self) -> &[crate::analysis_api::symbol_facts::ComplexityMetricFact] {
        &[]
    }

    fn facts_for_file(&self, file: FileId) -> CachedFileFacts;
    fn restore_file_facts(&mut self, file: FileId, facts: CachedFileFacts);
}

/// Facade/host services providers need beyond the fact registry (cache, config, plan).
pub trait ProviderHostServices: Any + Send {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// Plan digest string for frontend file-cache keys. Empty when unavailable.
    fn plan_digest(&self) -> &str {
        ""
    }

    /// Opaque frontend analysis cache (implemented by the composition root).
    fn analysis_cache(&self) -> Option<&dyn crate::analysis_api::cache_api::AnalysisCache> {
        None
    }
}

/// Opaque host side-channel (for example SCC-closure outputs owned by the composition root).
pub trait HostAttachment: Any + Send {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// Empty attachment used when a provider run does not need a side channel.
#[derive(Debug, Default)]
pub struct NullHostAttachment;

impl HostAttachment for NullHostAttachment {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderManifest {
    pub id: &'static str,
    pub kind: ProviderKind,
    pub inputs: &'static [&'static str],
    pub outputs: &'static [&'static str],
    pub language_ids: &'static [LanguageId],
    pub cache_policy: CachePolicy,
    pub schema_versions: &'static [SchemaVersion],
    pub precision_ceiling: PrecisionCeiling,
}

/// Typed execution status for a provider stage.
///
/// Diagnostics are ordinary analysis output and do not by themselves change this
/// status. Providers set a failure only when they could not certify the output
/// state they expose (for example, a validated store replacement was rejected or
/// required setup was unavailable).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProviderExecution {
    #[default]
    Succeeded,
    Failed {
        stage: ProviderFailureStage,
        reason: ProviderFailureReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderFailureStage {
    Setup,
    Execution,
    Validation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderFailureReason {
    Unsupported,
    SetupMissing,
    ExecutionFailed,
    ValidationRejected,
}

/// Result of running a single analysis provider stage.
pub struct ProviderRunResult {
    pub diagnostics: Vec<Diagnostic>,
    pub cache_stats: CacheStats,
    pub output_digest: Option<Digest>,
    pub execution: ProviderExecution,
    /// Stage-specific counters a provider chose to report, such as a sidecar's
    /// per-phase timings. Empty for providers with nothing to add.
    pub counts: std::collections::BTreeMap<String, u64>,
}

pub trait Provider: Send + Sync {
    fn manifest(&self) -> &'static ProviderManifest;
    fn run(&self, ctx: &mut ProviderCtx<'_>) -> ProviderRunResult;
}

pub struct ProviderCtx<'a> {
    pub facts: &'a mut dyn FactDatabase,
    pub host: &'a mut dyn ProviderHostServices,
    pub config_digest: &'a str,
    pub rule_digest: &'a str,
    pub parallel: bool,
    /// Output digests of already-run providers, keyed by provider id.
    pub upstream_digests: &'a BTreeMap<&'static str, Digest>,
    /// Opaque host side-channel filled by selected providers for composition-root plumbing.
    pub host_attachment: &'a mut dyn HostAttachment,
}

impl ProviderCtx<'_> {
    pub fn dependency_digest(&self, provider_id: &'static str) -> Digest {
        self.upstream_digests
            .get(provider_id)
            .cloned()
            .unwrap_or_else(|| Digest::absent(DigestKind::ProviderOutput, provider_id))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    SourceDiscovery,
    LanguageSyntax,
    WholeRepoDerived,
    MetricsDerived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CachePolicy {
    NoCache,
    ExistingFileFactCache { schema: &'static str },
    InMemoryDerived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaVersion {
    pub name: &'static str,
    pub version: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrecisionCeiling {
    Exact,
    Syntax,
    SetupAware,
}

impl ProviderManifest {
    pub fn provider_version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    pub fn primary_schema_label(&self) -> String {
        let mut labels = self
            .schema_versions
            .iter()
            .map(|schema| format!("{}:{}", schema.name, schema.version))
            .collect::<Vec<_>>();
        labels.sort();
        labels.join(",")
    }

    pub fn language_scope_label(&self) -> &'static str {
        match self.language_ids {
            [] => "workspace",
            [id] if *id == LanguageId::GO => "go",
            [id] if *id == LanguageId::TS => "typescript_javascript",
            _ => "multi_language",
        }
    }

    pub fn cache_policy_label(&self) -> String {
        match self.cache_policy {
            CachePolicy::NoCache => "no_cache".to_string(),
            CachePolicy::ExistingFileFactCache { schema } => {
                format!("existing_file_fact_cache:{schema}")
            }
            CachePolicy::InMemoryDerived => "in_memory_derived".to_string(),
        }
    }
}
