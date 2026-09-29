//! Cross-crate analysis contracts for polint.
//!
//! Depends only on `polint-core` and `polint-ir`. Must not import concrete analyses or frontends.

mod cache_api;
mod callable_names;
mod digest;
mod fact_store;
mod metadata;
mod module_facts;
mod parser_identity;
mod provider;
mod source_file;
mod symbol_facts;
mod syntax_facts;

pub use cache_api::{
    AnalysisCache, DisabledAnalysisCache, FileCacheKeyParts, FileCacheReadOutcome,
    FileCacheReadStatus, LayerCacheEntryDigests, LayerCacheKeyParts, LayerCacheKind,
    LayerCachePrecision, LayerCacheReadOutcome, LayerCacheReadStatus, LayerCacheWriteStatus,
};
pub use callable_names::{
    ANONYMOUS_CALLABLE_PREFIX, anonymous_callable_name, is_anonymous_callable_name,
};
pub use digest::{
    CacheStats, Digest, DigestBuilder, DigestKind, FileSnapshot, GoLifecycleSnapshot,
    INPUT_SNAPSHOT_SCHEMA_VERSION, InputComponent, InputComponentStatus, InputSnapshot, LayerKind,
    PrecisionTier, ProviderSchemaSnapshot, QueryKey, TsJsLifecycleSnapshot,
};
pub use fact_store::{FactStore, FactStoreEntry};
pub(crate) use metadata::stable_key_from_key_parts;
pub use metadata::{
    FactConfidence, FactFamily, FactMeta, FactMetaInsert, FactMetaStore, FactPrecision, FactRef,
    MissingFactMeta, StableKeyConflict, StableKeyOwner, ValidationStatus, stable_key_from_parts,
    stable_key_text_from_parts, write_stable_key_text,
};
pub use module_facts::{
    ModuleEdge, ModuleEdgeKind, ModuleNode, ModuleNodeKind, ResolutionPrecision, ResolutionStatus,
    ResolvedImportFact, UnresolvedReason,
};
pub use parser_identity::{
    GO_PARSER_BACKEND, GO_PARSER_GRAMMAR, TS_MODULE_RESOLVER, TS_PARSER_BACKEND,
    engine_parser_identity,
};
pub use provider::{
    CachePolicy, CaptureEnrichment, FactDatabase, HostAttachment, NullCaptureEnrichment,
    NullHostAttachment, PrecisionCeiling, Provider, ProviderCtx, ProviderExecution,
    ProviderFailureReason, ProviderFailureStage, ProviderHostServices, ProviderKind,
    ProviderManifest, ProviderRunResult, SchemaVersion,
};
pub use source_file::SourceFile;
pub use symbol_facts::{
    ComplexityMetricFact, DefinitionFact, DefinitionKind, FileMetricFact, FunctionMetricFact,
    ReferenceFact, ReferenceKind, ScopeId, SemanticImportFact, SemanticImportId,
    SemanticImportKind, SemanticStatus, SymbolFact, SymbolKind, SymbolNamespace, SymbolPrecision,
    SymbolResolutionStatus,
};
pub use syntax_facts::{
    BranchObligation, CachedFileAnalysis, CachedFileFacts, CoverageFact, FunctionFact,
    GoTypeDeclFact, GoTypeDeclKind, ImportFact, JsxAttributeFact, PackageFact, StringLiteralFact,
    TS_JS_MODULE_FUNCTION_NAME, TestFact, TsClassFact, TsComponentFact,
    is_synthetic_ts_js_module_function,
};

/// MIR identifiers shared with analysis contracts.
pub use crate::ir::{MirBodyId, PlaceId};
