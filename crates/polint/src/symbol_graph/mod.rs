pub(crate) use crate::go::symbol_graph as go;
pub(crate) mod model;
pub(crate) mod query;
pub(crate) mod semantic;
#[cfg(feature = "lang-typescript")]
pub(crate) use crate::ts::symbol_graph as ts;

use crate::analysis_kernel::ProviderManifest;
use crate::analysis_kernel::incremental::{
    CacheNode, CacheStats, DependencyEdge, DependencyKind, Digest, DigestKind, InputComponent,
    InputSnapshot, LayerCacheManifest, LayerCacheReadStatus, LayerCacheStore,
    LayerCacheWriteStatus, LayerKey, LayerKind, PrecisionTier, ShapeKind, dependency_layer_digest,
    relative_manifest_dependency_source, semantic_provider_parameter_digest,
};
use crate::analysis_plan::AnalysisPlan;
use crate::cache::Cache;
use crate::config::LoadedConfig;
use crate::core::{
    AnalysisDb, CapabilitySupport, CapabilitySupportStatus, CapabilitySupportView, DefinitionFact,
    FunctionFact, ImportFact, Language, PackageFact, ReferenceFact, SourceFile, Span, SymbolFact,
};
use crate::diagnostics::{Diagnostic, TextRange};
use model::{
    CachedDefinitionFact, CachedReferenceFact, CachedSymbolFact, SYMBOL_GRAPH_LAYER_SCHEMA,
    SymbolGraphBuilder, SymbolGraphLayerPayload,
};
use semantic::{
    AliasFact, CachedSemanticIndexOutput, ExportFact, GeneratedSymbolFact, ResolutionFact,
    ScopeFact, SemanticImportFact, SemanticIndexOutput, StableExportIdentity,
    alias_reexport_closure, emit_native_generated_symbol_hooks,
};

const SYMBOL_GRAPH_CAPABILITIES: &[&str] = &["symbols", "references"];
const SYMBOL_FACTS_DOCS_PATH: &str = "docs/facts/symbols-and-references.md";

#[derive(Debug, Clone, Default)]
pub(crate) struct SymbolGraphDerivation {
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) capability_support: Vec<CapabilitySupport>,
    pub(crate) cache_stats: CacheStats,
    pub(crate) output_digest: Option<Digest>,
}

impl SymbolGraphDerivation {
    pub(crate) fn support_view(&self, base: &CapabilitySupportView) -> CapabilitySupportView {
        let mut entries = base.entries().to_vec();
        for override_entry in &self.capability_support {
            if let Some(existing) = entries.iter_mut().find(|entry| {
                entry.capability == override_entry.capability
                    && entry.language == override_entry.language
            }) {
                *existing = override_entry.clone();
            } else {
                entries.push(override_entry.clone());
            }
        }
        CapabilitySupportView::new(entries)
    }
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Compatibility wrapper remains for direct in-crate symbol derivation callers while the kernel uses the stats-returning cache path."
    )
)]
pub(crate) fn derive_requested_symbols(
    db: &mut AnalysisDb,
    loaded: &LoadedConfig,
    plan: &AnalysisPlan,
) -> SymbolGraphDerivation {
    derive_requested_symbols_uncached(db, loaded, plan)
}

#[expect(
    clippy::too_many_arguments,
    reason = "The layer key consumes every input the derivation reads, including what the plan asks of it."
)]
pub(crate) fn symbol_graph_layer_key(
    db: &AnalysisDb,
    manifest: &ProviderManifest,
    config_digest: Digest,
    go_lifecycle_digest: Digest,
    ts_js_lifecycle_digest: Digest,
    module_graph_output_digest: Digest,
    upstream_syntax_output_digests: Vec<Digest>,
    selection: &SymbolGraphSelection,
) -> LayerKey {
    LayerKey::symbol_graph_layer_key(
        manifest,
        symbol_graph_source_function_digests(db),
        symbol_graph_package_context_digests(db),
        symbol_graph_import_shape_digests(db),
        config_digest,
        go_lifecycle_digest,
        ts_js_lifecycle_digest,
        module_graph_output_digest,
        upstream_syntax_output_digests,
        symbol_graph_parameter_digest(selection),
    )
}

/// What the plan asks the symbol graph for, per language.
///
/// TypeScript and JavaScript symbols and references are built for any plan that
/// needs them, directly or through a capability that depends on them. The Go
/// symbol sidecar runs only for the symbols and references a rule asks for
/// itself: a deep analysis depends on symbols and references, but its Go call
/// sites are resolved from the semantic sidecar's typed facts, and on a large
/// module the symbol sidecar alone outweighs the whole deep analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SymbolGraphSelection {
    request: crate::analysis_neutral::symbol_graph::SymbolGraphRequest,
    go_request: crate::analysis_neutral::symbol_graph::SymbolGraphRequest,
    go_reference_files: std::collections::BTreeSet<String>,
}

impl SymbolGraphSelection {
    pub(crate) fn from_plan(db: &AnalysisDb, plan: &AnalysisPlan) -> Self {
        let go_files = db
            .files()
            .iter()
            .filter(|file| file.language == Language::Go)
            .collect::<Vec<_>>();
        let go_reference_files = go_files
            .iter()
            .filter(|file| {
                !plan
                    .rules_requesting_capability_directly_matching_files(
                        "references",
                        std::slice::from_ref(*file),
                    )
                    .is_empty()
            })
            .map(|file| file.relative_path.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let go_references = !go_reference_files.is_empty();
        let go_symbols = go_references
            || !plan
                .rules_requesting_capability_directly_matching_files("symbols", &go_files)
                .is_empty();
        Self {
            request: crate::analysis_neutral::symbol_graph::SymbolGraphRequest::new(
                plan.requests_capability("symbols"),
                plan.requests_capability("references"),
            ),
            go_request: crate::analysis_neutral::symbol_graph::SymbolGraphRequest::new(
                go_symbols,
                go_references,
            ),
            go_reference_files,
        }
    }

    fn digest_parts(&self) -> Vec<String> {
        let mut parts = vec![
            format!(
                "request=symbols:{}/references:{}",
                self.request.symbols, self.request.references
            ),
            format!(
                "go_request=symbols:{}/references:{}",
                self.go_request.symbols, self.go_request.references
            ),
        ];
        parts.extend(
            self.go_reference_files
                .iter()
                .map(|file| format!("go_reference_file={file}")),
        );
        parts
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Symbol graph cache identity consumes cache, snapshot, provider manifest, module graph output, and upstream syntax outputs explicitly."
)]
pub(crate) fn derive_requested_symbols_with_cache_stats(
    db: &mut AnalysisDb,
    loaded: &LoadedConfig,
    plan: &AnalysisPlan,
    cache: &Cache,
    input_snapshot: &InputSnapshot,
    manifest: &ProviderManifest,
    module_graph_output_digest: Digest,
    upstream_syntax_output_digests: Vec<Digest>,
) -> SymbolGraphDerivation {
    if !plan.requests_any_capability(SYMBOL_GRAPH_CAPABILITIES) {
        return SymbolGraphDerivation::default();
    }

    let config_digest = input_snapshot.config.digest.clone();
    let go_lifecycle_digest = lifecycle_component_digest(
        DigestKind::GoLifecycle,
        "symbol_graph_go_lifecycle",
        &input_snapshot.go_lifecycle.components,
    );
    let ts_js_lifecycle_digest = lifecycle_component_digest(
        DigestKind::TsJsLifecycle,
        "symbol_graph_ts_js_lifecycle",
        &input_snapshot.ts_js_lifecycle.components,
    );
    let selection = SymbolGraphSelection::from_plan(db, plan);
    let layer_key = symbol_graph_layer_key(
        db,
        manifest,
        config_digest.clone(),
        go_lifecycle_digest.clone(),
        ts_js_lifecycle_digest.clone(),
        module_graph_output_digest.clone(),
        upstream_syntax_output_digests.clone(),
        &selection,
    );
    let store = cache.layer_cache_store();
    let interner = db.stable_key_interner();
    let mut cache_stats = CacheStats::default();
    let read = store
        .read_json_validated::<SymbolGraphLayerPayload, _>(&layer_key, |payload, manifest| {
            validate_symbol_graph_layer_payload(&interner, payload, manifest)
        });

    match read.status {
        LayerCacheReadStatus::Hit => {
            cache_stats.record_hit();
            cache_stats.record_verified_reuse();
            let payload = read
                .value
                .expect("layer cache hit should include symbol graph payload");
            restore_symbol_graph_layer_payload(db, &payload);
            SymbolGraphDerivation {
                diagnostics: payload.diagnostics,
                capability_support: payload.capability_support,
                cache_stats,
                output_digest: read.output_digest,
            }
        }
        LayerCacheReadStatus::BypassedDisabled => {
            cache_stats.record_disabled_bypass();
            cache_stats.record_recompute();
            let (mut derivation, _) =
                derive_requested_symbols_uncached_with_payload(db, loaded, plan);
            derivation.cache_stats = cache_stats;
            derivation
        }
        LayerCacheReadStatus::Miss | LayerCacheReadStatus::InvalidEvicted => {
            if read.status == LayerCacheReadStatus::Miss {
                cache_stats.record_miss();
            } else {
                cache_stats.record_invalid_evicted_read();
            }
            cache_stats.record_recompute();
            let (mut derivation, payload) =
                derive_requested_symbols_uncached_with_payload(db, loaded, plan);
            let payload = payload.unwrap_or_else(|| symbol_graph_layer_payload(db, &derivation));
            if has_setup_missing_support(&payload) {
                // A setup-missing graph records a failure of the environment
                // (toolchain, network, module download), not of the sources the key
                // covers. Caching it would replay the failure after the environment
                // is fixed, so it is recomputed on every run until it loads.
                derivation.output_digest = Some(symbol_graph_output_digest_for_payload(
                    &payload,
                    Some(&layer_key),
                ));
                derivation.cache_stats = cache_stats;
                return derivation;
            }
            let dependencies = symbol_graph_layer_dependency_edges(
                db,
                &layer_key,
                manifest,
                &module_graph_output_digest,
                &upstream_syntax_output_digests,
                config_digest,
                go_lifecycle_digest,
                ts_js_lifecycle_digest,
            );
            derivation.output_digest = write_symbol_graph_layer_payload(
                &store,
                layer_key,
                &payload,
                dependencies,
                &mut cache_stats,
                &mut derivation.diagnostics,
            );
            derivation.cache_stats = cache_stats;
            derivation
        }
    }
}

fn has_setup_missing_support(payload: &SymbolGraphLayerPayload) -> bool {
    payload
        .capability_support
        .iter()
        .any(|support| support.status == CapabilitySupportStatus::SetupMissing)
}

fn derive_requested_symbols_uncached(
    db: &mut AnalysisDb,
    loaded: &LoadedConfig,
    plan: &AnalysisPlan,
) -> SymbolGraphDerivation {
    derive_requested_symbols_uncached_with_payload(db, loaded, plan).0
}

fn derive_requested_symbols_uncached_with_payload(
    db: &mut AnalysisDb,
    loaded: &LoadedConfig,
    plan: &AnalysisPlan,
) -> (SymbolGraphDerivation, Option<SymbolGraphLayerPayload>) {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    if !plan.requests_any_capability(SYMBOL_GRAPH_CAPABILITIES) {
        return (SymbolGraphDerivation::default(), None);
    }

    let mut builder = SymbolGraphBuilder::new(interner_handle.clone());
    let mut derivation = SymbolGraphDerivation::default();
    let mut semantic_output = SemanticIndexOutput::default();

    let selection = SymbolGraphSelection::from_plan(db, plan);
    let request = selection.request;
    #[cfg(feature = "lang-typescript")]
    merge_language_output(
        &mut derivation,
        &mut semantic_output,
        ts::derive_ts_symbols(
            &mut builder,
            db,
            crate::ts::symbol_graph::TsSymbolOptions {
                request,
                resolved_imports: db.resolved_imports().to_vec(),
                module_nodes: db.module_nodes().to_vec(),
            },
        ),
        db,
        plan,
    );
    merge_language_output(
        &mut derivation,
        &mut semantic_output,
        go::derive_go_symbols(
            &mut builder,
            db,
            &crate::go::symbol_graph::GoSymbolOptions {
                root: loaded.root.clone(),
                settings: loaded.config.languages.go.clone(),
                request: selection.go_request,
                reference_files: Some(selection.go_reference_files),
            },
        ),
        db,
        plan,
    );

    let output = builder.finish();
    let closure = alias_reexport_closure(
        interner,
        &semantic_output.aliases,
        &semantic_output.exports,
        &semantic_output.stable_exports,
    );
    semantic_output.aliases = closure.aliases;
    semantic_output.resolutions.extend(closure.resolutions);
    let generated_hooks = emit_native_generated_symbol_hooks(interner, &semantic_output);
    semantic_output
        .generated_symbols
        .extend(generated_hooks.generated_symbols);
    semantic_output
        .resolutions
        .extend(generated_hooks.resolutions);
    derivation.diagnostics.extend(generated_hooks.diagnostics);
    db.replace_semantic_index_facts(
        semantic_output.scopes,
        semantic_output.semantic_imports,
        semantic_output.exports,
        semantic_output.aliases,
        semantic_output.resolutions,
        semantic_output.generated_symbols,
        semantic_output.stable_exports,
    );
    db.replace_symbol_graph_facts(output.symbols, output.definitions, output.references);
    derivation.diagnostics.extend(output.diagnostics);
    derivation
        .diagnostics
        .extend(capability_diagnostics(&derivation.capability_support));
    sort_symbol_derivation(&mut derivation);
    let payload = symbol_graph_layer_payload(db, &derivation);
    derivation.output_digest = Some(symbol_graph_output_digest_for_payload(&payload, None));
    (derivation, Some(payload))
}

#[expect(
    clippy::too_many_arguments,
    reason = "Symbol graph cache identity consumes provider, module graph, syntax, config, and language lifecycle inputs explicitly."
)]
fn symbol_graph_layer_dependency_edges(
    db: &AnalysisDb,
    key: &LayerKey,
    manifest: &ProviderManifest,
    module_graph_output_digest: &Digest,
    upstream_syntax_output_digests: &[Digest],
    config_digest: Digest,
    go_lifecycle_digest: Digest,
    ts_js_lifecycle_digest: Digest,
) -> Vec<DependencyEdge> {
    let from = CacheNode::Layer(key.clone());
    let mut edges = Vec::new();

    for file in sorted_files(db) {
        edges.push(dependency_edge(
            &from,
            CacheNode::Input(format!(
                "source:{}:{}",
                normalized_file_path(file),
                file.content_hash
            )),
            DependencyKind::SourceText,
            ShapeKind::Content,
        ));
    }

    for function in sorted_functions(db) {
        edges.push(dependency_edge(
            &from,
            CacheNode::Input(format!(
                "function:{}:{}:{}:{}:{}",
                db.path_for(function.file),
                function.name,
                function.span.start_byte,
                function.span.end_byte,
                language_cache_label(function.language)
            )),
            DependencyKind::Input,
            ShapeKind::Syntax,
        ));
    }

    for package in sorted_packages(db) {
        edges.push(dependency_edge(
            &from,
            CacheNode::Input(format!(
                "package:{}:{}:{}",
                db.path_for(package.file),
                language_cache_label(package.language),
                package.name
            )),
            DependencyKind::Input,
            ShapeKind::ModuleTopology,
        ));
    }

    for import in sorted_imports(db) {
        edges.push(dependency_edge(
            &from,
            CacheNode::Input(format!(
                "import:{}:{}:{}:{}",
                db.path_for(import.file),
                import.path,
                import.span.start_byte,
                language_cache_label(import.language)
            )),
            DependencyKind::ImportShape,
            ShapeKind::Import,
        ));
    }

    edges.push(dependency_edge(
        &from,
        CacheNode::Input(format!("config:{}", config_digest)),
        DependencyKind::Config,
        ShapeKind::Unknown,
    ));
    edges.push(dependency_edge(
        &from,
        CacheNode::Input(format!("lifecycle:go:{}", go_lifecycle_digest)),
        DependencyKind::Lifecycle,
        ShapeKind::Lifecycle,
    ));
    edges.push(dependency_edge(
        &from,
        CacheNode::Input(format!("lifecycle:ts_js:{}", ts_js_lifecycle_digest)),
        DependencyKind::Lifecycle,
        ShapeKind::Lifecycle,
    ));
    edges.push(dependency_edge(
        &from,
        CacheNode::Input(format!(
            "provider_schema:{}:{}",
            manifest.id,
            manifest.primary_schema_label()
        )),
        DependencyKind::ProviderSchema,
        ShapeKind::ProviderVersion,
    ));
    edges.push(dependency_edge(
        &from,
        CacheNode::Input("toolchain:symbol_graph:absent".to_string()),
        DependencyKind::Toolchain,
        ShapeKind::Toolchain,
    ));
    edges.push(dependency_edge(
        &from,
        CacheNode::Layer(upstream_layer_key(
            LayerKind::ModuleGraph,
            "polint.module_graph",
            module_graph_output_digest.clone(),
        )),
        DependencyKind::UpstreamLayer,
        ShapeKind::Output,
    ));

    for (index, output_digest) in upstream_syntax_output_digests.iter().cloned().enumerate() {
        let (layer_kind, provider_id) = match index {
            0 => (LayerKind::GoSyntax, "polint.go.syntax"),
            1 => (LayerKind::TsSyntax, "polint.ts.syntax"),
            _ => (LayerKind::Extension, "polint.unknown_upstream"),
        };
        edges.push(dependency_edge(
            &from,
            CacheNode::Layer(upstream_layer_key(layer_kind, provider_id, output_digest)),
            DependencyKind::UpstreamLayer,
            ShapeKind::Output,
        ));
    }

    edges.sort();
    edges.dedup();
    edges
}

fn symbol_graph_source_function_digests(db: &AnalysisDb) -> Vec<Digest> {
    let mut digests = sorted_files(db)
        .into_iter()
        .map(|file| {
            let parts = [
                normalized_file_path(file),
                file.content_hash.clone(),
                language_cache_label(file.language).to_string(),
            ];
            let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
            Digest::from_parts(DigestKind::SourceText, "source_text", &refs)
        })
        .collect::<Vec<_>>();
    digests.extend(sorted_functions(db).into_iter().map(|function| {
        let mut calls = function.calls.clone();
        calls.sort();
        calls.dedup();
        let parts = [
            db.path_for(function.file),
            function.name.clone(),
            function.span.start_byte.to_string(),
            function.span.end_byte.to_string(),
            function.is_test.to_string(),
            function.is_exported.to_string(),
            function.cyclomatic_complexity.to_string(),
            language_cache_label(function.language).to_string(),
            calls.join("\n"),
        ];
        let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
        Digest::from_parts(DigestKind::ProviderParameters, "function_fact", &refs)
    }));
    digests.sort();
    digests
}

fn symbol_graph_package_context_digests(db: &AnalysisDb) -> Vec<Digest> {
    sorted_packages(db)
        .into_iter()
        .map(|package| {
            let parts = [
                db.path_for(package.file),
                package.name.clone(),
                language_cache_label(package.language).to_string(),
            ];
            let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
            Digest::from_parts(DigestKind::ProviderParameters, "package_context", &refs)
        })
        .collect()
}

fn symbol_graph_import_shape_digests(db: &AnalysisDb) -> Vec<Digest> {
    sorted_imports(db)
        .into_iter()
        .map(|import| {
            let parts = [
                db.path_for(import.file),
                import.path.clone(),
                import.package.clone().unwrap_or_default(),
                import.span.start_byte.to_string(),
                import.span.end_byte.to_string(),
                language_cache_label(import.language).to_string(),
            ];
            let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
            Digest::from_parts(DigestKind::ProviderParameters, "import_shape", &refs)
        })
        .collect()
}

fn symbol_graph_parameter_digest(selection: &SymbolGraphSelection) -> Digest {
    let selection_parts = selection.digest_parts();
    let selection_parts = selection_parts
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    Digest::from_unordered(
        DigestKind::ProviderParameters,
        "symbol_graph_parameters",
        vec![
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "symbol_graph_outputs",
                &["output=symbols", "output=definitions", "output=references"],
            ),
            semantic_provider_parameter_digest(),
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "symbol_graph_selection",
                &selection_parts,
            ),
        ],
    )
}

fn symbol_graph_layer_payload(
    db: &AnalysisDb,
    derivation: &SymbolGraphDerivation,
) -> SymbolGraphLayerPayload {
    let interner = db.stable_key_interner();
    let mut symbols = db.symbols().to_vec();
    let mut definitions = db.definitions().to_vec();
    let mut references = db.references().to_vec();
    sort_symbol_facts(&interner, &mut symbols);
    sort_definition_facts(&interner, &mut definitions);
    sort_reference_facts(&interner, &mut references);

    SymbolGraphLayerPayload {
        schema: SYMBOL_GRAPH_LAYER_SCHEMA.to_string(),
        diagnostics: derivation.diagnostics.clone(),
        capability_support: derivation.capability_support.clone(),
        symbols: symbols
            .iter()
            .map(|fact| CachedSymbolFact::from_fact(&interner, fact))
            .collect(),
        definitions: definitions
            .iter()
            .map(|fact| CachedDefinitionFact::from_fact(&interner, fact))
            .collect(),
        references: references
            .iter()
            .map(|fact| CachedReferenceFact::from_fact(&interner, fact))
            .collect(),
        semantic_index: CachedSemanticIndexOutput::from_output(
            &interner,
            &semantic_index_payload(db),
        ),
    }
}

fn restore_symbol_graph_layer_payload(db: &mut AnalysisDb, payload: &SymbolGraphLayerPayload) {
    let interner = db.stable_key_interner();
    let mut symbols = payload
        .symbols
        .clone()
        .into_iter()
        .map(|fact| fact.into_fact(&interner))
        .collect::<Vec<_>>();
    let mut definitions = payload
        .definitions
        .clone()
        .into_iter()
        .map(|fact| fact.into_fact(&interner))
        .collect::<Vec<_>>();
    let mut references = payload
        .references
        .clone()
        .into_iter()
        .map(|fact| fact.into_fact(&interner))
        .collect::<Vec<_>>();
    sort_symbol_facts(&interner, &mut symbols);
    sort_definition_facts(&interner, &mut definitions);
    sort_reference_facts(&interner, &mut references);
    db.replace_symbol_graph_facts(symbols, definitions, references);
    let semantic = payload.semantic_index.clone().into_output(&interner);
    db.replace_semantic_index_facts(
        semantic.scopes,
        semantic.semantic_imports,
        semantic.exports,
        semantic.aliases,
        semantic.resolutions,
        semantic.generated_symbols,
        semantic.stable_exports,
    );
}

fn validate_symbol_graph_layer_payload(
    interner: &crate::core::StableKeyInterner,
    payload: &SymbolGraphLayerPayload,
    manifest: &LayerCacheManifest,
) -> bool {
    let semantic = payload.semantic_index.clone().into_output(interner);
    payload.schema == SYMBOL_GRAPH_LAYER_SCHEMA
        && semantic_payload_rows_are_valid(interner, &semantic)
        && manifest.output_digest
            == symbol_graph_output_digest_for_payload(payload, Some(&manifest.key))
}

fn semantic_index_payload(db: &AnalysisDb) -> SemanticIndexOutput {
    let interner = db.stable_key_interner();
    SemanticIndexOutput {
        scopes: sorted_semantic_rows(db.scopes().to_vec(), |row| {
            scope_payload_sort_key(&interner, row)
        }),
        semantic_imports: sorted_semantic_rows(db.semantic_imports().to_vec(), |row| {
            semantic_import_payload_sort_key(&interner, row)
        }),
        exports: sorted_semantic_rows(db.exports().to_vec(), |row| {
            export_payload_sort_key(&interner, row)
        }),
        aliases: sorted_semantic_rows(db.aliases().to_vec(), |row| {
            alias_payload_sort_key(&interner, row)
        }),
        resolutions: sorted_semantic_rows(db.resolution_facts().to_vec(), |row| {
            resolution_payload_sort_key(&interner, row)
        }),
        generated_symbols: sorted_semantic_rows(db.generated_symbols().to_vec(), |row| {
            generated_symbol_payload_sort_key(&interner, row)
        }),
        stable_exports: sorted_semantic_rows(db.stable_exports().to_vec(), |row| {
            stable_export_payload_sort_key(&interner, row)
        }),
    }
}

fn semantic_payload_rows_are_valid(
    interner: &crate::core::StableKeyInterner,
    semantic: &SemanticIndexOutput,
) -> bool {
    semantic
        .scopes
        .iter()
        .all(|row| scope_payload_row_is_valid(interner, row))
        && semantic
            .semantic_imports
            .iter()
            .all(|row| semantic_import_payload_row_is_valid(interner, row))
        && semantic
            .exports
            .iter()
            .all(|row| export_payload_row_is_valid(interner, row))
        && semantic
            .aliases
            .iter()
            .all(|row| alias_payload_row_is_valid(interner, row))
        && semantic
            .resolutions
            .iter()
            .all(|row| resolution_payload_row_is_valid(interner, row))
        && semantic
            .generated_symbols
            .iter()
            .all(|row| generated_symbol_payload_row_is_valid(interner, row))
        && semantic
            .stable_exports
            .iter()
            .all(|row| stable_export_payload_row_is_valid(interner, row))
        && stable_export_identities_are_consistent(&semantic.stable_exports)
}

fn scope_payload_row_is_valid(interner: &crate::core::StableKeyInterner, row: &ScopeFact) -> bool {
    !interner.resolve(row.stable_key).is_empty()
        && row
            .scope_path
            .iter()
            .all(|part| !is_absolute_path_like(part))
}

fn semantic_import_payload_row_is_valid(
    interner: &crate::core::StableKeyInterner,
    row: &SemanticImportFact,
) -> bool {
    !interner.resolve(row.stable_key).is_empty() && !is_absolute_path_like(&row.import_path)
}

fn export_payload_row_is_valid(
    interner: &crate::core::StableKeyInterner,
    row: &ExportFact,
) -> bool {
    !interner.resolve(row.stable_key).is_empty()
}

fn alias_payload_row_is_valid(interner: &crate::core::StableKeyInterner, row: &AliasFact) -> bool {
    !interner.resolve(row.stable_key).is_empty()
}

fn resolution_payload_row_is_valid(
    interner: &crate::core::StableKeyInterner,
    row: &ResolutionFact,
) -> bool {
    !interner.resolve(row.stable_key).is_empty()
}

fn generated_symbol_payload_row_is_valid(
    interner: &crate::core::StableKeyInterner,
    row: &GeneratedSymbolFact,
) -> bool {
    !interner.resolve(row.stable_key).is_empty()
}

fn stable_export_payload_row_is_valid(
    interner: &crate::core::StableKeyInterner,
    row: &StableExportIdentity,
) -> bool {
    !interner.resolve(row.stable_key).is_empty()
        && row
            .package_key
            .as_deref()
            .is_none_or(|package_key| !is_absolute_path_like(package_key))
        && row
            .module_key
            .as_deref()
            .is_none_or(|module_key| !is_absolute_path_like(module_key))
}

fn stable_export_identities_are_consistent(rows: &[StableExportIdentity]) -> bool {
    let mut targets_by_stable_key = std::collections::BTreeMap::new();
    for row in rows {
        if let Some(existing) = targets_by_stable_key.get(&row.stable_key) {
            if *existing != row.symbol_stable_key {
                return false;
            }
        } else {
            targets_by_stable_key.insert(row.stable_key, row.symbol_stable_key);
        }
    }
    true
}

fn is_absolute_path_like(value: &str) -> bool {
    std::path::Path::new(value).is_absolute()
        || value.starts_with('/')
        || value.starts_with("\\\\")
        || is_windows_drive_absolute(value)
}

fn is_windows_drive_absolute(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}

fn write_symbol_graph_layer_payload(
    store: &LayerCacheStore,
    layer_key: LayerKey,
    payload: &SymbolGraphLayerPayload,
    dependencies: Vec<DependencyEdge>,
    stats: &mut CacheStats,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Digest> {
    let payload_bytes = match serde_json::to_vec(payload) {
        Ok(bytes) => bytes,
        Err(error) => {
            diagnostics.push(cache_write_diagnostic("symbol graph layer", error.into()));
            return None;
        }
    };
    let payload_digest = match LayerCacheStore::payload_digest_for_json_bytes(&payload_bytes) {
        Ok(digest) => digest,
        Err(error) => {
            diagnostics.push(cache_write_diagnostic("symbol graph layer", error));
            return None;
        }
    };
    let output_digest = symbol_graph_output_digest(&layer_key, &payload_digest);
    let manifest = LayerCacheManifest::new(
        layer_key,
        output_digest.clone(),
        payload_digest,
        dependencies,
        PrecisionTier::SetupAware,
        "native_trusted",
        Vec::new(),
    );

    match store.write_json_bytes(&manifest, payload_bytes) {
        Ok(LayerCacheWriteStatus::Written) => stats.record_write(),
        Ok(LayerCacheWriteStatus::BypassedDisabled) => stats.record_disabled_bypass(),
        Err(error) => diagnostics.push(cache_write_diagnostic("symbol graph layer", error)),
    }
    Some(output_digest)
}

fn symbol_graph_output_digest_for_payload(
    payload: &SymbolGraphLayerPayload,
    layer_key: Option<&LayerKey>,
) -> Digest {
    let payload_digest = LayerCacheStore::payload_digest_for_json(payload)
        .unwrap_or_else(|_| Digest::unsupported(DigestKind::LayerOutput, "symbol_graph", "json"));
    if let Some(layer_key) = layer_key {
        symbol_graph_output_digest(layer_key, &payload_digest)
    } else {
        Digest::from_parts(
            DigestKind::ProviderOutput,
            "symbol_graph_layer_output",
            &[&payload_digest.to_string()],
        )
    }
}

fn symbol_graph_output_digest(layer_key: &LayerKey, payload_digest: &Digest) -> Digest {
    let layer_key_json =
        serde_json::to_string(layer_key).unwrap_or_else(|_| "unserializable_layer_key".to_string());
    Digest::from_parts(
        DigestKind::ProviderOutput,
        "symbol_graph_layer_output",
        &[&payload_digest.to_string(), &layer_key_json],
    )
}

fn lifecycle_component_digest(
    kind: DigestKind,
    label: &str,
    components: &[InputComponent],
) -> Digest {
    Digest::from_unordered(
        kind,
        label,
        components
            .iter()
            .map(|component| component.digest.clone())
            .collect(),
    )
}

fn cache_write_diagnostic(path: &str, error: anyhow::Error) -> Diagnostic {
    Diagnostic::warning(
        "internal/cache",
        path,
        TextRange::point(1, 1),
        format!("cache write failed: {error}"),
    )
}

fn sorted_files(db: &AnalysisDb) -> Vec<&SourceFile> {
    let mut files = db.files().iter().collect::<Vec<_>>();
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    files
}

fn sorted_functions(db: &AnalysisDb) -> Vec<&FunctionFact> {
    let mut functions = db.functions().iter().collect::<Vec<_>>();
    functions.sort_by(|left, right| {
        (
            db.path_for(left.file),
            left.span.start_byte,
            left.span.end_byte,
            left.name.as_str(),
            left.id,
        )
            .cmp(&(
                db.path_for(right.file),
                right.span.start_byte,
                right.span.end_byte,
                right.name.as_str(),
                right.id,
            ))
    });
    functions
}

fn sorted_packages(db: &AnalysisDb) -> Vec<&PackageFact> {
    let mut packages = db.packages().iter().collect::<Vec<_>>();
    packages.sort_by(|left, right| {
        (
            db.path_for(left.file),
            left.language,
            left.name.as_str(),
            left.id,
        )
            .cmp(&(
                db.path_for(right.file),
                right.language,
                right.name.as_str(),
                right.id,
            ))
    });
    packages
}

fn sorted_imports(db: &AnalysisDb) -> Vec<&ImportFact> {
    let mut imports = db.imports().iter().collect::<Vec<_>>();
    imports.sort_by(|left, right| {
        (
            db.path_for(left.file),
            left.span.start_byte,
            left.path.as_str(),
            left.id,
        )
            .cmp(&(
                db.path_for(right.file),
                right.span.start_byte,
                right.path.as_str(),
                right.id,
            ))
    });
    imports
}

fn normalized_file_path(file: &SourceFile) -> String {
    crate::repo_fs::normalize_repo_relative(&file.relative_path)
        .unwrap_or_else(|| file.relative_path.clone())
}

fn dependency_edge(
    _from: &CacheNode,
    to: CacheNode,
    kind: DependencyKind,
    required_shape: ShapeKind,
) -> DependencyEdge {
    DependencyEdge {
        from: relative_manifest_dependency_source(),
        to,
        kind,
        required_shape,
    }
}

fn upstream_layer_key(layer_kind: LayerKind, provider_id: &str, output_digest: Digest) -> LayerKey {
    let output_dependency = dependency_layer_digest(output_digest);

    LayerKey::new(
        layer_kind,
        provider_id,
        "output-digest",
        "output-digest",
        output_dependency.clone(),
        Digest::absent(DigestKind::DependencyLayer, "upstream_lifecycle_unknown"),
        Digest::absent(DigestKind::Config, "upstream_config_unknown"),
        Digest::absent(DigestKind::ToolInvocation, "upstream_toolchain_unknown"),
        vec![output_dependency],
        Vec::new(),
        Vec::new(),
    )
}

fn language_cache_label(language: Language) -> &'static str {
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

fn sort_symbol_facts(interner: &crate::core::StableKeyInterner, symbols: &mut [SymbolFact]) {
    symbols.sort_by(|left, right| {
        let left_stable_key = interner.resolve(left.stable_key);
        let right_stable_key = interner.resolve(right.stable_key);
        fact_order_key(
            &left_stable_key,
            left.file,
            left.primary_span.as_ref(),
            &left.name,
        )
        .cmp(&fact_order_key(
            &right_stable_key,
            right.file,
            right.primary_span.as_ref(),
            &right.name,
        ))
    });
}

fn sort_definition_facts(
    interner: &crate::core::StableKeyInterner,
    definitions: &mut [DefinitionFact],
) {
    definitions.sort_by(|left, right| {
        let left_stable_key = interner.resolve(left.stable_key);
        let right_stable_key = interner.resolve(right.stable_key);
        fact_order_key(
            &left_stable_key,
            left.file,
            left.primary_span.as_ref(),
            &left.name,
        )
        .cmp(&fact_order_key(
            &right_stable_key,
            right.file,
            right.primary_span.as_ref(),
            &right.name,
        ))
    });
}

fn sort_reference_facts(
    interner: &crate::core::StableKeyInterner,
    references: &mut [ReferenceFact],
) {
    references.sort_by(|left, right| {
        let left_stable_key = interner.resolve(left.stable_key);
        let right_stable_key = interner.resolve(right.stable_key);
        fact_order_key(
            &left_stable_key,
            left.file,
            left.primary_span.as_ref(),
            &left.name,
        )
        .cmp(&fact_order_key(
            &right_stable_key,
            right.file,
            right.primary_span.as_ref(),
            &right.name,
        ))
    });
}

fn sorted_semantic_rows<T, K: Ord>(mut rows: Vec<T>, key: impl Fn(&T) -> K) -> Vec<T> {
    rows.sort_by_key(key);
    rows
}

fn scope_payload_sort_key(
    interner: &crate::core::StableKeyInterner,
    row: &ScopeFact,
) -> (String, Option<crate::core::FileId>) {
    (interner.resolve(row.stable_key).to_string(), row.file)
}

fn semantic_import_payload_sort_key(
    interner: &crate::core::StableKeyInterner,
    row: &SemanticImportFact,
) -> (String, Option<crate::core::FileId>, String) {
    (
        interner.resolve(row.stable_key).to_string(),
        row.file,
        row.import_path.clone(),
    )
}

fn export_payload_sort_key(
    interner: &crate::core::StableKeyInterner,
    row: &ExportFact,
) -> (String, Option<crate::core::FileId>, String) {
    (
        interner.resolve(row.stable_key).to_string(),
        row.file,
        row.export_name.clone(),
    )
}

fn alias_payload_sort_key(
    interner: &crate::core::StableKeyInterner,
    row: &AliasFact,
) -> (String, Option<crate::core::FileId>) {
    (interner.resolve(row.stable_key).to_string(), row.file)
}

fn resolution_payload_sort_key(
    interner: &crate::core::StableKeyInterner,
    row: &ResolutionFact,
) -> (String, Option<crate::core::FileId>) {
    (interner.resolve(row.stable_key).to_string(), row.file)
}

fn generated_symbol_payload_sort_key(
    interner: &crate::core::StableKeyInterner,
    row: &GeneratedSymbolFact,
) -> (String, Option<crate::core::FileId>) {
    (interner.resolve(row.stable_key).to_string(), row.file)
}

fn stable_export_payload_sort_key(
    interner: &crate::core::StableKeyInterner,
    row: &StableExportIdentity,
) -> (String, String) {
    (
        interner.resolve(row.stable_key).to_string(),
        interner.resolve(row.symbol_stable_key).to_string(),
    )
}

fn fact_order_key<'a>(
    stable_key: &'a str,
    file: Option<crate::core::FileId>,
    span: Option<&Span>,
    name: &'a str,
) -> (&'a str, Option<crate::core::FileId>, u32, u32, &'a str) {
    let (start_byte, end_byte) = span
        .map(|span| (span.start_byte, span.end_byte))
        .unwrap_or((u32::MAX, u32::MAX));
    (stable_key, file, start_byte, end_byte, name)
}

fn merge_language_output(
    derivation: &mut SymbolGraphDerivation,
    semantic: &mut SemanticIndexOutput,
    output: crate::analysis_neutral::symbol_graph::LanguageSymbolOutput,
    db: &AnalysisDb,
    plan: &AnalysisPlan,
) {
    derivation.diagnostics.extend(output.diagnostics);
    for support in output.capability_support {
        let files = db
            .files()
            .iter()
            .filter(|file| {
                file.language == support.language
                    || (support.language.is_ts_family() && file.language.is_ts_family())
            })
            .collect::<Vec<_>>();
        let rules = plan.rules_for_capability_matching_files(&support.capability, &files);
        if rules.is_empty() {
            continue;
        }
        let status = match support.status {
            crate::analysis_neutral::symbol_graph::SymbolCapabilityStatus::Supported => {
                CapabilitySupportStatus::Supported
            }
            crate::analysis_neutral::symbol_graph::SymbolCapabilityStatus::SetupMissing => {
                CapabilitySupportStatus::SetupMissing
            }
            crate::analysis_neutral::symbol_graph::SymbolCapabilityStatus::Unsupported => {
                CapabilitySupportStatus::Unsupported
            }
        };
        derivation.capability_support.push(CapabilitySupport {
            capability: support.capability,
            language: Some(support.language),
            status,
            rules,
            reason: support.reason,
            hint: support.hint,
            docs_path: Some(SYMBOL_FACTS_DOCS_PATH.to_string()),
        });
    }
    semantic.extend(output.semantic);
}

fn capability_diagnostics(support: &[CapabilitySupport]) -> Vec<Diagnostic> {
    support
        .iter()
        .filter(|entry| entry.status != CapabilitySupportStatus::Supported)
        .flat_map(|entry| {
            entry
                .rules
                .iter()
                .map(|rule_id| capability_diagnostic(entry, rule_id))
        })
        .collect()
}

fn capability_diagnostic(entry: &CapabilitySupport, rule_id: &str) -> Diagnostic {
    let language = entry.language.map(language_name).unwrap_or("workspace");
    let status = capability_status_json(&entry.status);
    let docs_path = entry.docs_path.as_deref().unwrap_or(SYMBOL_FACTS_DOCS_PATH);
    let reason = entry
        .reason
        .as_deref()
        .unwrap_or("Symbol/reference provider support is unavailable.");
    let mut diagnostic = Diagnostic::error(
        "polint/capability",
        "<workspace>",
        TextRange::point(1, 1),
        format!(
            "Rule `{rule_id}` requested capability `{}` for {language}, but symbol graph provider support is {status}.",
            entry.capability
        ),
    )
    .with_evidence("rule", rule_id.to_string())
    .with_evidence("capability", entry.capability.clone())
    .with_evidence("language", language.to_string())
    .with_evidence("status", status.to_string())
    .with_evidence("reason", reason.to_string())
    .with_evidence("docs_path", docs_path.to_string())
    .with_help(format!(
        "Capability `{}` is recognized but the {language} symbol/reference provider is not available yet; see {docs_path}.",
        entry.capability
    ));
    if let Some(hint) = &entry.hint {
        diagnostic = diagnostic.with_evidence("hint", hint.clone());
    }
    diagnostic
}

fn sort_symbol_derivation(derivation: &mut SymbolGraphDerivation) {
    derivation.capability_support.sort_by(|left, right| {
        (
            left.capability.as_str(),
            left.language,
            left.rules.as_slice(),
            left.reason.as_deref(),
            left.hint.as_deref(),
            left.docs_path.as_deref(),
        )
            .cmp(&(
                right.capability.as_str(),
                right.language,
                right.rules.as_slice(),
                right.reason.as_deref(),
                right.hint.as_deref(),
                right.docs_path.as_deref(),
            ))
    });
    derivation.diagnostics.sort_by(|left, right| {
        (
            left.rule_id.as_str(),
            left.file.as_str(),
            left.range.start_line,
            left.range.start_col,
            left.message.as_str(),
            left.stable_fingerprint.as_str(),
        )
            .cmp(&(
                right.rule_id.as_str(),
                right.file.as_str(),
                right.range.start_line,
                right.range.start_col,
                right.message.as_str(),
                right.stable_fingerprint.as_str(),
            ))
    });
}

pub(crate) fn language_name(language: Language) -> &'static str {
    match language {
        Language::Go => "Go",
        Language::TypeScript => "TypeScript",
        Language::Tsx => "TSX",
        Language::JavaScript => "JavaScript",
        Language::Jsx => "JSX",
        Language::Unknown => "unknown",
        _ => "unknown",
    }
}

fn capability_status_json(status: &CapabilitySupportStatus) -> &'static str {
    match status {
        CapabilitySupportStatus::Supported => "supported",
        CapabilitySupportStatus::Unsupported => "unsupported",
        CapabilitySupportStatus::SetupMissing => "setup_missing",
    }
}

#[cfg(test)]
mod symbol_graph_derivation {
    use super::{SymbolGraphDerivation, derive_requested_symbols};
    use crate::analysis_kernel::incremental::{Digest, DigestKind, InputSnapshot};
    use crate::analysis_kernel::{
        FactConfidence, FactFamily, FactPrecision, FactRef, ValidationStatus,
    };
    use crate::analysis_plan::AnalysisPlan;
    use crate::cache::Cache;
    use crate::config::load_config;
    use crate::core::{
        AnalysisDb, CapabilitySupport, CapabilitySupportStatus, CapabilitySupportView,
        DefinitionFact, DefinitionId, DefinitionKind, FileId, ImportFact, ImportId, Language,
        ModuleNode, ModuleNodeId, ModuleNodeKind, ReferenceFact, ReferenceId, ReferenceKind,
        ResolutionPrecision, ResolutionStatus, ResolvedImportFact, ResolvedImportId, Span,
        SymbolFact, SymbolId, SymbolKind, SymbolNamespace, SymbolPrecision, SymbolResolutionStatus,
        span_from_byte_range,
    };
    use std::fs;
    use std::path::Path;

    #[cfg(feature = "lang-typescript")]
    type SymbolRows = Vec<(Language, String, SymbolPrecision)>;
    #[cfg(feature = "lang-typescript")]
    type ReferenceRows = Vec<(Language, String, SymbolResolutionStatus, SymbolPrecision)>;
    #[cfg(feature = "lang-typescript")]
    type SupportRows = Vec<(String, Option<Language>, CapabilitySupportStatus)>;
    #[cfg(feature = "lang-typescript")]
    type DeriveSnapshot = (SymbolRows, ReferenceRows, SupportRows, Vec<String>);

    fn loaded_config_for(root: &Path) -> crate::config::LoadedConfig {
        load_config(root).expect("default config loads")
    }

    fn add_file(db: &mut AnalysisDb, root: &Path, relative_path: &str, source: &str) -> FileId {
        let path = root.join(relative_path);
        std::fs::create_dir_all(path.parent().expect("test file has parent")).expect("mkdirs");
        std::fs::write(&path, source).expect("write fixture file");
        db.add_file(path, relative_path.to_string(), source.to_string())
    }

    #[cfg(feature = "lang-typescript")]
    fn stale_symbol_fact(interner: &crate::core::StableKeyInterner, file: FileId) -> SymbolFact {
        SymbolFact::new(
            SymbolId::from_raw(999),
            Language::TypeScript,
            "stale".to_string(),
            "stale".to_string(),
            SymbolKind::Unknown,
            SymbolNamespace::Unknown,
            Some(file),
            None,
            None,
            None,
            Some(Span::point(file, 1, 1)),
            false,
            interner.intern("stale:symbol".to_string()),
            SymbolPrecision::Unsupported,
        )
    }

    #[cfg(feature = "lang-typescript")]
    fn stale_definition_fact(
        interner: &crate::core::StableKeyInterner,
        file: FileId,
    ) -> DefinitionFact {
        DefinitionFact::new(
            DefinitionId::from_raw(999),
            SymbolId::from_raw(999),
            Language::TypeScript,
            "stale".to_string(),
            "stale".to_string(),
            DefinitionKind::Unknown,
            SymbolNamespace::Unknown,
            Some(file),
            None,
            None,
            None,
            Some(Span::point(file, 1, 1)),
            false,
            false,
            interner.intern("stale:definition".to_string()),
            SymbolPrecision::Unsupported,
        )
    }

    #[cfg(feature = "lang-typescript")]
    fn stale_reference_fact(
        interner: &crate::core::StableKeyInterner,
        file: FileId,
    ) -> ReferenceFact {
        ReferenceFact::new(
            ReferenceId::from_raw(999),
            Language::TypeScript,
            "stale".to_string(),
            "stale".to_string(),
            ReferenceKind::Unknown,
            SymbolNamespace::Unknown,
            Some(file),
            None,
            None,
            None,
            Some(Span::point(file, 1, 1)),
            None,
            Vec::new(),
            interner.intern("stale:reference".to_string()),
            SymbolResolutionStatus::Unsupported,
            SymbolPrecision::Unsupported,
        )
    }

    fn symbol_graph_manifest() -> &'static crate::analysis_kernel::ProviderManifest {
        crate::analysis_kernel::AnalysisKernel::provider_manifests()
            .iter()
            .find(|manifest| manifest.id == "polint.symbol_graph")
            .expect("symbol graph provider manifest exists")
    }

    fn requested_symbol_plan() -> AnalysisPlan {
        AnalysisPlan::from_capability_names_for_test(&["symbols", "references"])
    }

    fn upstream_digests(label: &str) -> (Digest, Vec<Digest>) {
        (
            Digest::from_parts(DigestKind::ProviderOutput, "module_graph", &[label]),
            vec![
                Digest::from_parts(DigestKind::ProviderOutput, "go_syntax", &[label]),
                Digest::from_parts(DigestKind::ProviderOutput, "ts_syntax", &[label]),
            ],
        )
    }

    fn symbol_input_snapshot(
        loaded: &crate::config::LoadedConfig,
        db: &AnalysisDb,
        plan: &AnalysisPlan,
        config_digest: &str,
    ) -> InputSnapshot {
        crate::analysis_kernel::incremental::input_snapshot_from_run_inputs(
            loaded,
            db,
            config_digest,
            "rule-digest",
            plan.digest(),
            crate::analysis_kernel::AnalysisKernel::provider_manifests(),
        )
    }

    fn derive_symbols_with_cache(
        db: &mut AnalysisDb,
        loaded: &crate::config::LoadedConfig,
        cache: &Cache,
        plan: &AnalysisPlan,
        config_digest: &str,
        upstream_label: &str,
    ) -> SymbolGraphDerivation {
        let snapshot = symbol_input_snapshot(loaded, db, plan, config_digest);
        let (module_graph_output_digest, syntax_output_digests) = upstream_digests(upstream_label);
        super::derive_requested_symbols_with_cache_stats(
            db,
            loaded,
            plan,
            cache,
            &snapshot,
            symbol_graph_manifest(),
            module_graph_output_digest,
            syntax_output_digests,
        )
    }

    fn collect_files(root: &Path) -> Vec<std::path::PathBuf> {
        let mut files = Vec::new();
        collect_files_into(root, &mut files);
        files.sort();
        files
    }

    fn collect_files_into(root: &Path, files: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = fs::read_dir(root) else {
            return;
        };
        for entry in entries {
            let path = entry.expect("read cache entry").path();
            if path.is_dir() {
                collect_files_into(&path, files);
            } else {
                files.push(path);
            }
        }
    }

    fn first_layer_file(cache_root: &Path, category: &str) -> std::path::PathBuf {
        collect_files(&cache_root.join("layers").join(category))
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("expected layer cache {category} file"))
    }

    fn add_ts_file(db: &mut AnalysisDb, root: &Path, relative_path: &str, source: &str) -> FileId {
        add_file(db, root, relative_path, source)
    }

    fn span_for(file: FileId, source: &str, needle: &str) -> Span {
        let start = source
            .find(needle)
            .unwrap_or_else(|| panic!("missing fixture text {needle:?}"));
        span_from_byte_range(file, source, start, start + needle.len())
    }

    fn push_import(db: &mut AnalysisDb, file: FileId, source: &str, path: &str) -> ImportId {
        db.push_import(ImportFact::new(
            ImportId::from_raw(0),
            file,
            None,
            path.to_string(),
            span_for(file, source, &format!("{path:?}")),
            Language::TypeScript,
        ))
    }

    fn fixture_db(root: &Path, import_path: &str) -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let app_source = format!(
            r#"
import {{ token as importedToken }} from "{import_path}";

export function answer() {{
    return importedToken;
}}
"#
        );
        let target_source = "export const token = 42;\n";
        let app = add_ts_file(&mut db, root, "src/app.ts", &app_source);
        let target = add_ts_file(&mut db, root, "src/target.ts", target_source);
        let import = push_import(&mut db, app, &app_source, import_path);
        db.replace_module_graph_facts(
            vec![ResolvedImportFact::new(
                ResolvedImportId::from_raw(0),
                import,
                app,
                Some(ModuleNodeId::from_raw(1)),
                ResolutionStatus::Resolved,
                ResolutionPrecision::ExactFile,
                None,
            )],
            vec![
                ModuleNode::new(
                    ModuleNodeId::from_raw(0),
                    ModuleNodeKind::File,
                    "src/app.ts".to_string(),
                    Some(app),
                    None,
                    Some(Language::TypeScript),
                ),
                ModuleNode::new(
                    ModuleNodeId::from_raw(1),
                    ModuleNodeKind::File,
                    "src/target.ts".to_string(),
                    Some(target),
                    None,
                    Some(Language::TypeScript),
                ),
            ],
            Vec::new(),
        );
        db
    }

    #[test]
    fn symbol_graph_metadata_records_provider_and_reuses_existing_stable_keys() {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            Path::new("src/app.ts").to_path_buf(),
            "src/app.ts".to_string(),
            "export const value = 1;\n".to_string(),
        );
        let interner = db.stable_key_interner();
        let symbol = SymbolFact::new(
            SymbolId::from_raw(7),
            Language::TypeScript,
            "value".to_string(),
            "value".to_string(),
            SymbolKind::Variable,
            SymbolNamespace::Value,
            Some(file),
            None,
            None,
            None,
            Some(Span::point(file, 1, 14)),
            true,
            interner.intern("symbol:key:value".to_string()),
            SymbolPrecision::ExactSemantic,
        );
        let definition = DefinitionFact::new(
            DefinitionId::from_raw(11),
            symbol.id,
            Language::TypeScript,
            "value".to_string(),
            "value".to_string(),
            DefinitionKind::Declaration,
            SymbolNamespace::Value,
            Some(file),
            None,
            None,
            None,
            Some(Span::point(file, 1, 14)),
            true,
            true,
            interner.intern("definition:key:value".to_string()),
            SymbolPrecision::ExactSemantic,
        );
        let reference = ReferenceFact::new(
            ReferenceId::from_raw(13),
            Language::TypeScript,
            "value".to_string(),
            "value".to_string(),
            ReferenceKind::Read,
            SymbolNamespace::Value,
            Some(file),
            None,
            None,
            None,
            Some(Span::point(file, 1, 14)),
            Some(symbol.id),
            Vec::new(),
            interner.intern("reference:key:value".to_string()),
            SymbolResolutionStatus::Resolved,
            SymbolPrecision::ExactSemantic,
        );

        db.replace_symbol_graph_facts(vec![symbol], vec![definition], vec![reference]);

        let symbol_meta = db
            .metadata_for(FactRef::new(FactFamily::Symbol, 7))
            .expect("symbol metadata should be recorded");
        let definition_meta = db
            .metadata_for(FactRef::new(FactFamily::Definition, 11))
            .expect("definition metadata should be recorded");
        let reference_meta = db
            .metadata_for(FactRef::new(FactFamily::Reference, 13))
            .expect("reference metadata should be recorded");

        assert_eq!(symbol_meta.producer_id, "polint.symbol_graph");
        assert_eq!(symbol_meta.layer_id, "polint.symbol_graph");
        assert_eq!(symbol_meta.precision, FactPrecision::SetupAware);
        assert_eq!(symbol_meta.confidence, FactConfidence::High);
        assert_eq!(symbol_meta.validation, ValidationStatus::NativeTrusted);
        assert_eq!(
            db.resolve_stable_key(symbol_meta.stable_key).as_ref(),
            "symbol:key:value"
        );
        assert_eq!(
            db.resolve_stable_key(definition_meta.stable_key).as_ref(),
            "definition:key:value"
        );
        assert_eq!(
            db.resolve_stable_key(reference_meta.stable_key).as_ref(),
            "reference:key:value"
        );
    }

    #[test]
    fn symbol_graph_metadata_maps_low_confidence_precisions_without_mutating_status_fields() {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            Path::new("src/app.ts").to_path_buf(),
            "src/app.ts".to_string(),
            "missing;\n".to_string(),
        );
        let interner = db.stable_key_interner();
        let reference = ReferenceFact::new(
            ReferenceId::from_raw(23),
            Language::TypeScript,
            "missing".to_string(),
            "missing".to_string(),
            ReferenceKind::Read,
            SymbolNamespace::Value,
            Some(file),
            None,
            None,
            None,
            Some(Span::point(file, 1, 1)),
            None,
            Vec::new(),
            interner.intern("reference:key:missing".to_string()),
            SymbolResolutionStatus::SetupMissing,
            SymbolPrecision::SetupMissing,
        );

        db.replace_symbol_graph_facts(Vec::new(), Vec::new(), vec![reference]);

        let metadata = db
            .metadata_for(FactRef::new(FactFamily::Reference, 23))
            .expect("reference metadata should be recorded");

        assert_eq!(metadata.precision, FactPrecision::SetupMissing);
        assert_eq!(metadata.confidence, FactConfidence::Low);
        assert_eq!(
            db.references()[0].status,
            SymbolResolutionStatus::SetupMissing
        );
        assert_eq!(db.references()[0].precision, SymbolPrecision::SetupMissing);
    }

    #[test]
    fn provider_defaults_when_symbol_capabilities_are_not_requested() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut db = AnalysisDb::new();
        add_file(
            &mut db,
            temp.path(),
            "src/app.ts",
            "export const value = 1;\n",
        );

        let derivation = derive_requested_symbols(
            &mut db,
            &loaded_config_for(temp.path()),
            &AnalysisPlan::empty(),
        );

        assert!(derivation.diagnostics.is_empty());
        assert!(derivation.capability_support.is_empty());
        assert!(db.symbols().is_empty());
        assert!(db.definitions().is_empty());
        assert!(db.references().is_empty());
    }

    #[test]
    fn support_view_merges_symbol_provider_rows_in_order() {
        let derivation = SymbolGraphDerivation {
            diagnostics: Vec::new(),
            capability_support: vec![
                CapabilitySupport {
                    capability: "symbols".to_string(),
                    language: Some(Language::TypeScript),
                    status: CapabilitySupportStatus::Unsupported,
                    rules: vec!["local/symbols".to_string()],
                    reason: Some("symbol extraction pending".to_string()),
                    hint: None,
                    docs_path: Some("docs/facts/symbols-and-references.md".to_string()),
                },
                CapabilitySupport {
                    capability: "references".to_string(),
                    language: Some(Language::Go),
                    status: CapabilitySupportStatus::Unsupported,
                    rules: vec!["local/references".to_string()],
                    reason: Some("reference extraction pending".to_string()),
                    hint: None,
                    docs_path: Some("docs/facts/symbols-and-references.md".to_string()),
                },
            ],
            ..SymbolGraphDerivation::default()
        };
        let base = CapabilitySupportView::new(vec![CapabilitySupport {
            capability: "symbols".to_string(),
            language: Some(Language::TypeScript),
            status: CapabilitySupportStatus::Supported,
            rules: vec!["local/symbols".to_string()],
            reason: None,
            hint: None,
            docs_path: None,
        }]);

        let support = derivation.support_view(&base);

        assert_eq!(
            support
                .entries()
                .iter()
                .map(|entry| (
                    entry.capability.as_str(),
                    entry.language,
                    entry.status.clone(),
                ))
                .collect::<Vec<_>>(),
            vec![
                (
                    "symbols",
                    Some(Language::TypeScript),
                    CapabilitySupportStatus::Unsupported
                ),
                (
                    "references",
                    Some(Language::Go),
                    CapabilitySupportStatus::Unsupported
                ),
            ]
        );
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn requested_symbol_derivation_replaces_facts_deterministically() {
        fn derive_once() -> DeriveSnapshot {
            let temp = tempfile::tempdir().expect("tempdir");
            let mut db = AnalysisDb::new();
            let app = add_file(
                &mut db,
                temp.path(),
                "src/app.ts",
                "export const value = 1;\n",
            );
            add_file(&mut db, temp.path(), "lib/main.go", "package lib\n");
            let interner = db.stable_key_interner();
            db.replace_symbol_graph_facts(
                vec![stale_symbol_fact(&interner, app)],
                vec![stale_definition_fact(&interner, app)],
                vec![stale_reference_fact(&interner, app)],
            );

            let derivation = derive_requested_symbols(
                &mut db,
                &loaded_config_for(temp.path()),
                &AnalysisPlan::from_capability_names_for_test(&["symbols", "references"]),
            );

            (
                db.symbols()
                    .iter()
                    .map(|symbol| (symbol.language, symbol.name.clone(), symbol.precision))
                    .collect(),
                db.references()
                    .iter()
                    .map(|reference| {
                        (
                            reference.language,
                            reference.name.clone(),
                            reference.status,
                            reference.precision,
                        )
                    })
                    .collect(),
                derivation
                    .capability_support
                    .iter()
                    .map(|entry| {
                        (
                            entry.capability.clone(),
                            entry.language,
                            entry.status.clone(),
                        )
                    })
                    .collect(),
                derivation
                    .diagnostics
                    .iter()
                    .map(|diagnostic| diagnostic.stable_fingerprint.clone())
                    .collect(),
            )
        }

        let first = derive_once();
        let second = derive_once();

        assert_eq!(first, second);
        assert!(first.0.iter().any(|(_, name, _)| name == "value"));
        assert!(first.0.iter().all(|(_, name, _)| name != "stale"));
        assert!(first.1.iter().all(|(_, name, _, _)| name != "stale"));
    }

    mod symbol_graph_layer_cache {
        use super::*;

        /// A setup-missing graph records a failure of the environment, which the
        /// layer key does not cover. Caching it would replay the failure on every
        /// run after the environment is fixed.
        #[cfg(feature = "lang-go")]
        #[test]
        fn setup_missing_symbol_graph_is_recomputed_instead_of_cached() {
            let temp = tempfile::tempdir().expect("tempdir");
            let loaded = loaded_config_for(temp.path());
            let cache = Cache::new(temp.path().join("cache").join("analysis"), true);
            let plan = requested_symbol_plan();
            // No go.mod anywhere above the file, so the Go symbol facts cannot load.
            let orphan = "package main\n\nfunc main() {}\n";
            let mut first_db = AnalysisDb::new();
            add_file(&mut first_db, temp.path(), "orphan/main.go", orphan);
            let mut second_db = AnalysisDb::new();
            add_file(&mut second_db, temp.path(), "orphan/main.go", orphan);

            let first = derive_symbols_with_cache(
                &mut first_db,
                &loaded,
                &cache,
                &plan,
                "config",
                "stable",
            );
            let second = derive_symbols_with_cache(
                &mut second_db,
                &loaded,
                &cache,
                &plan,
                "config",
                "stable",
            );

            assert!(
                first
                    .capability_support
                    .iter()
                    .any(|support| support.status == CapabilitySupportStatus::SetupMissing),
                "the fixture must produce a setup-missing symbol graph"
            );
            assert_eq!(first.cache_stats.recomputes, 1);
            assert_eq!(first.cache_stats.writes, 0);
            assert_eq!(second.cache_stats.hits, 0);
            assert_eq!(second.cache_stats.recomputes, 1);
            assert_eq!(second.cache_stats.writes, 0);
            assert_eq!(first.output_digest, second.output_digest);
            assert!(first.output_digest.is_some());
        }

        #[test]
        fn symbol_graph_layer_reuses_warm_cache() {
            let temp = tempfile::tempdir().expect("tempdir");
            let loaded = loaded_config_for(temp.path());
            let cache = Cache::new(temp.path().join("cache").join("analysis"), true);
            let plan = requested_symbol_plan();
            let mut first_db = fixture_db(temp.path(), "./target");
            let mut second_db = fixture_db(temp.path(), "./target");

            let first = derive_symbols_with_cache(
                &mut first_db,
                &loaded,
                &cache,
                &plan,
                "config",
                "stable",
            );
            let second = derive_symbols_with_cache(
                &mut second_db,
                &loaded,
                &cache,
                &plan,
                "config",
                "stable",
            );

            assert_eq!(first.cache_stats.misses, 1);
            assert_eq!(first.cache_stats.recomputes, 1);
            assert_eq!(first.cache_stats.writes, 1);
            assert_eq!(second.cache_stats.hits, 1);
            assert_eq!(second.cache_stats.verified_reuse, 1);
            assert_eq!(second.cache_stats.recomputes, 0);
            assert_eq!(first.output_digest, second.output_digest);
            assert_eq!(symbol_rows(&first_db), symbol_rows(&second_db));
            assert_eq!(reference_rows(&first_db), reference_rows(&second_db));
        }

        #[test]
        fn symbol_graph_layer_invalidates_on_import_or_module_digest_change() {
            let temp = tempfile::tempdir().expect("tempdir");
            let loaded = loaded_config_for(temp.path());
            let cache = Cache::new(temp.path().join("cache").join("analysis"), true);
            let plan = requested_symbol_plan();
            let mut base_db = fixture_db(temp.path(), "./target");
            derive_symbols_with_cache(&mut base_db, &loaded, &cache, &plan, "config", "stable");

            let mut import_changed_db = fixture_db(temp.path(), "./other");
            let import_changed = derive_symbols_with_cache(
                &mut import_changed_db,
                &loaded,
                &cache,
                &plan,
                "config",
                "stable",
            );
            let mut module_changed_db = fixture_db(temp.path(), "./target");
            let module_changed = derive_symbols_with_cache(
                &mut module_changed_db,
                &loaded,
                &cache,
                &plan,
                "config",
                "changed",
            );

            assert_eq!(import_changed.cache_stats.misses, 1);
            assert_eq!(import_changed.cache_stats.recomputes, 1);
            assert_eq!(module_changed.cache_stats.misses, 1);
            assert_eq!(module_changed.cache_stats.recomputes, 1);
        }

        #[test]
        fn symbol_graph_layer_corrupt_cache_recomputes() {
            let temp = tempfile::tempdir().expect("tempdir");
            let loaded = loaded_config_for(temp.path());
            let cache = Cache::new(temp.path().join("cache").join("analysis"), true);
            let plan = requested_symbol_plan();
            let mut first_db = fixture_db(temp.path(), "./target");
            derive_symbols_with_cache(&mut first_db, &loaded, &cache, &plan, "config", "stable");
            let manifest = first_layer_file(temp.path().join("cache").as_path(), "manifests");
            fs::write(manifest, "{broken").expect("corrupt symbol graph manifest");
            let mut second_db = fixture_db(temp.path(), "./target");

            let second = derive_symbols_with_cache(
                &mut second_db,
                &loaded,
                &cache,
                &plan,
                "config",
                "stable",
            );

            assert_eq!(second.cache_stats.invalid_evicted_reads, 1);
            assert_eq!(second.cache_stats.recomputes, 1);
        }

        #[cfg(feature = "lang-typescript")]
        #[test]
        fn symbol_graph_layer_disabled_cache_records_bypass_without_layer_files() {
            let temp = tempfile::tempdir().expect("tempdir");
            let loaded = loaded_config_for(temp.path());
            let cache_root = temp.path().join("cache").join("analysis");
            let cache = Cache::new(&cache_root, false);
            let plan = requested_symbol_plan();
            let mut db = fixture_db(temp.path(), "./target");

            let derivation =
                derive_symbols_with_cache(&mut db, &loaded, &cache, &plan, "config", "stable");

            assert_eq!(derivation.cache_stats.bypasses_disabled, 1);
            assert_eq!(derivation.cache_stats.recomputes, 1);
            assert!(!temp.path().join("cache").join("layers").exists());
            assert!(!db.symbols().is_empty());
        }
    }

    fn symbol_rows(db: &AnalysisDb) -> Vec<(String, String, SymbolKind, SymbolPrecision)> {
        db.symbols()
            .iter()
            .map(|symbol| {
                (
                    db.resolve_stable_key(symbol.stable_key).to_string(),
                    symbol.name.clone(),
                    symbol.kind,
                    symbol.precision,
                )
            })
            .collect()
    }

    fn reference_rows(
        db: &AnalysisDb,
    ) -> Vec<(
        String,
        String,
        ReferenceKind,
        SymbolResolutionStatus,
        SymbolPrecision,
    )> {
        db.references()
            .iter()
            .map(|reference| {
                (
                    db.resolve_stable_key(reference.stable_key).to_string(),
                    reference.name.clone(),
                    reference.kind,
                    reference.status,
                    reference.precision,
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod semantic_layer_payload {
    use super::*;
    use crate::analysis_kernel::incremental::LayerCacheManifest;
    use crate::analysis_kernel::{FactFamily, FactRef};
    #[cfg(feature = "lang-typescript")]
    use crate::analysis_plan::AnalysisPlan;
    #[cfg(feature = "lang-typescript")]
    use crate::cache::Cache;
    #[cfg(feature = "lang-typescript")]
    use crate::config::load_config;
    use crate::core::{AnalysisDb, FileId, Language, SymbolNamespace};
    #[cfg(feature = "lang-typescript")]
    use crate::core::{
        ImportFact, ImportId, ModuleNode, ModuleNodeId, ModuleNodeKind, ResolutionPrecision,
        ResolutionStatus, ResolvedImportFact, ResolvedImportId, Span, span_from_byte_range,
    };
    use crate::symbol_graph::semantic::{
        ExportFact, ExportId, ExportKind, ScopeFact, ScopeId, ScopeKind, SemanticIndexOutput,
        SemanticStatus, StableExportId, StableExportIdentity,
    };
    use std::path::Path;

    #[cfg(feature = "lang-typescript")]
    fn add_file(db: &mut AnalysisDb, root: &Path, relative_path: &str, source: &str) -> FileId {
        let path = root.join(relative_path);
        std::fs::create_dir_all(path.parent().expect("test file has parent")).expect("mkdirs");
        std::fs::write(&path, source).expect("write fixture file");
        db.add_file(path, relative_path.to_string(), source.to_string())
    }

    #[cfg(feature = "lang-typescript")]
    fn span_for(file: FileId, source: &str, needle: &str) -> Span {
        let start = source
            .find(needle)
            .unwrap_or_else(|| panic!("missing fixture text {needle:?}"));
        span_from_byte_range(file, source, start, start + needle.len())
    }

    #[cfg(feature = "lang-typescript")]
    fn push_import(db: &mut AnalysisDb, file: FileId, source: &str, path: &str) -> ImportId {
        db.push_import(ImportFact::new(
            ImportId::from_raw(0),
            file,
            None,
            path.to_string(),
            span_for(file, source, &format!("{path:?}")),
            Language::TypeScript,
        ))
    }

    #[cfg(feature = "lang-typescript")]
    fn fixture_db(root: &Path) -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let app_source = r#"
import { token as importedToken } from "./target";

export function answer() {
    return importedToken;
}
"#;
        let target_source = "export const token = 42;\n";
        let app = add_file(&mut db, root, "src/app.ts", app_source);
        let target = add_file(&mut db, root, "src/target.ts", target_source);
        let import = push_import(&mut db, app, app_source, "./target");
        db.replace_module_graph_facts(
            vec![ResolvedImportFact::new(
                ResolvedImportId::from_raw(0),
                import,
                app,
                Some(ModuleNodeId::from_raw(1)),
                ResolutionStatus::Resolved,
                ResolutionPrecision::ExactFile,
                None,
            )],
            vec![
                ModuleNode::new(
                    ModuleNodeId::from_raw(0),
                    ModuleNodeKind::File,
                    "src/app.ts".to_string(),
                    Some(app),
                    None,
                    Some(Language::TypeScript),
                ),
                ModuleNode::new(
                    ModuleNodeId::from_raw(1),
                    ModuleNodeKind::File,
                    "src/target.ts".to_string(),
                    Some(target),
                    None,
                    Some(Language::TypeScript),
                ),
            ],
            Vec::new(),
        );
        db
    }

    fn manifest() -> &'static crate::analysis_kernel::ProviderManifest {
        crate::analysis_kernel::AnalysisKernel::provider_manifests()
            .iter()
            .find(|manifest| manifest.id == "polint.symbol_graph")
            .expect("symbol graph provider manifest exists")
    }

    fn layer_key() -> LayerKey {
        LayerKey::symbol_graph_layer_key(
            manifest(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
            Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "module_graph", &["base"]),
            vec![Digest::from_parts(
                DigestKind::ProviderOutput,
                "ts_syntax",
                &["base"],
            )],
            semantic_provider_parameter_digest(),
        )
    }

    fn cache_manifest_for(payload: &SymbolGraphLayerPayload) -> LayerCacheManifest {
        let key = layer_key();
        let payload_digest =
            LayerCacheStore::payload_digest_for_json(payload).expect("payload digest");
        let output_digest = symbol_graph_output_digest_for_payload(payload, Some(&key));
        LayerCacheManifest::new(
            key,
            output_digest,
            payload_digest,
            Vec::new(),
            PrecisionTier::SetupAware,
            "native_trusted",
            Vec::new(),
        )
    }

    fn scope(
        interner: &crate::core::StableKeyInterner,
        file: FileId,
        stable_key: &str,
    ) -> ScopeFact {
        ScopeFact {
            id: ScopeId(0),
            language: Language::TypeScript,
            file: Some(file),
            package: None,
            module: None,
            parent: None,
            scope_path: vec!["module".to_string()],
            kind: ScopeKind::Module,
            stable_key: interner.intern(stable_key),
            status: SemanticStatus::Resolved,
        }
    }

    fn stable_export(
        interner: &crate::core::StableKeyInterner,
        file: FileId,
        export_id: ExportId,
        stable_key: &str,
        symbol_key: &str,
    ) -> StableExportIdentity {
        StableExportIdentity {
            id: StableExportId(0),
            export: export_id,
            language: Language::TypeScript,
            package_key: None,
            module_key: Some(format!("file:{}", file.0)),
            export_name: "answer".to_string(),
            namespace: SymbolNamespace::Value,
            symbol_stable_key: interner.intern(symbol_key),
            generated_discriminator: None,
            stable_key: interner.intern(stable_key),
            status: SemanticStatus::Resolved,
        }
    }

    fn payload_with_semantic_index(
        interner: &crate::core::StableKeyInterner,
        semantic_index: SemanticIndexOutput,
    ) -> SymbolGraphLayerPayload {
        SymbolGraphLayerPayload {
            schema: SYMBOL_GRAPH_LAYER_SCHEMA.to_string(),
            diagnostics: Vec::new(),
            capability_support: Vec::new(),
            symbols: Vec::new(),
            definitions: Vec::new(),
            references: Vec::new(),
            semantic_index: CachedSemanticIndexOutput::from_output(interner, &semantic_index),
        }
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn cold_payload_writes_semantic_rows() {
        let temp = tempfile::tempdir().expect("tempdir");
        let loaded = load_config(temp.path()).expect("config");
        let cache = Cache::new(temp.path().join("cache").join("analysis"), true);
        let plan = AnalysisPlan::from_capability_names_for_test(&["symbols", "references"]);
        let mut db = fixture_db(temp.path());
        let snapshot = crate::analysis_kernel::incremental::input_snapshot_from_run_inputs(
            &loaded,
            &db,
            "config",
            "rule-digest",
            plan.digest(),
            crate::analysis_kernel::AnalysisKernel::provider_manifests(),
        );
        let derivation = derive_requested_symbols_with_cache_stats(
            &mut db,
            &loaded,
            &plan,
            &cache,
            &snapshot,
            manifest(),
            Digest::from_parts(DigestKind::ProviderOutput, "module_graph", &["base"]),
            vec![Digest::from_parts(
                DigestKind::ProviderOutput,
                "ts_syntax",
                &["base"],
            )],
        );

        let payload = symbol_graph_layer_payload(&db, &derivation);

        assert!(!payload.semantic_index.scopes.is_empty());
        assert!(!payload.semantic_index.stable_exports.is_empty());
    }

    #[test]
    fn restore_replaces_semantic_rows_and_metadata() {
        let mut db = AnalysisDb::new();
        let interner = db.stable_key_interner();
        let file = db.add_file(
            Path::new("src/app.ts").to_path_buf(),
            "src/app.ts".to_string(),
            "export const answer = 42;\n".to_string(),
        );
        let export = ExportFact {
            id: ExportId(0),
            language: Language::TypeScript,
            file: Some(file),
            package: None,
            module: None,
            scope: None,
            symbol: None,
            export_name: "answer".to_string(),
            namespace: SymbolNamespace::Value,
            kind: ExportKind::Named,
            stable_key: interner.intern("semantic:export:answer"),
            status: SemanticStatus::Resolved,
        };
        let payload = payload_with_semantic_index(
            &interner,
            SemanticIndexOutput {
                scopes: vec![scope(&interner, file, "semantic:scope:module")],
                exports: vec![export],
                stable_exports: vec![stable_export(
                    &interner,
                    file,
                    ExportId(0),
                    "semantic:stable-export:answer",
                    "symbol:answer",
                )],
                ..SemanticIndexOutput::default()
            },
        );

        restore_symbol_graph_layer_payload(&mut db, &payload);

        assert_eq!(db.scopes().len(), 1);
        assert_eq!(db.exports().len(), 1);
        assert_eq!(db.stable_exports().len(), 1);
        assert!(
            db.metadata_for(FactRef::new(FactFamily::StableExport, 0))
                .is_some()
        );
    }

    #[test]
    fn validation_rejects_missing_keys_absolute_paths_and_stable_export_conflicts() {
        let mut db = AnalysisDb::new();
        let interner = db.stable_key_interner();
        let file = db.add_file(
            Path::new("src/app.ts").to_path_buf(),
            "src/app.ts".to_string(),
            "export const answer = 42;\n".to_string(),
        );
        let valid_payload = payload_with_semantic_index(
            &interner,
            SemanticIndexOutput {
                scopes: vec![scope(&interner, file, "semantic:scope:module")],
                stable_exports: vec![stable_export(
                    &interner,
                    file,
                    ExportId(0),
                    "semantic:stable-export:answer",
                    "symbol:answer",
                )],
                ..SemanticIndexOutput::default()
            },
        );
        let valid_manifest = cache_manifest_for(&valid_payload);
        assert!(validate_symbol_graph_layer_payload(
            &interner,
            &valid_payload,
            &valid_manifest
        ));

        let mut empty_key_payload = valid_payload.clone();
        empty_key_payload.semantic_index.scopes[0]
            .stable_key_text
            .clear();
        assert!(!validate_symbol_graph_layer_payload(
            &interner,
            &empty_key_payload,
            &cache_manifest_for(&empty_key_payload)
        ));

        let mut absolute_path_payload = valid_payload.clone();
        absolute_path_payload.semantic_index.stable_exports[0].module_key =
            Some("/tmp/repo/src/app.ts".to_string());
        assert!(!validate_symbol_graph_layer_payload(
            &interner,
            &absolute_path_payload,
            &cache_manifest_for(&absolute_path_payload)
        ));

        let mut conflict_payload = valid_payload;
        let mut conflicting_row = conflict_payload.semantic_index.stable_exports[0].clone();
        conflicting_row.id = StableExportId(1);
        conflicting_row.export = ExportId(1);
        conflicting_row.symbol_stable_key_text = "symbol:other".to_string();
        conflict_payload
            .semantic_index
            .stable_exports
            .push(conflicting_row);
        assert!(!validate_symbol_graph_layer_payload(
            &interner,
            &conflict_payload,
            &cache_manifest_for(&conflict_payload)
        ));
    }
}

#[cfg(test)]
mod semantic_cache_restore {
    #[cfg(any(feature = "lang-go", feature = "lang-typescript"))]
    use crate::analysis_kernel::{AnalysisKernel, KernelInput, KernelOutput};
    #[cfg(any(feature = "lang-go", feature = "lang-typescript"))]
    use crate::analysis_plan::AnalysisPlan;
    #[cfg(any(feature = "lang-go", feature = "lang-typescript"))]
    use crate::cache::Cache;
    #[cfg(any(feature = "lang-go", feature = "lang-typescript"))]
    use crate::config::load_config;
    #[cfg(feature = "lang-go")]
    use crate::symbol_graph::semantic::SemanticStatus;
    #[cfg(any(feature = "lang-go", feature = "lang-typescript"))]
    use std::path::Path;

    #[cfg(any(feature = "lang-go", feature = "lang-typescript"))]
    fn requested_symbol_plan() -> AnalysisPlan {
        AnalysisPlan::from_capability_names_for_test(&["symbols", "references"])
    }

    #[cfg(any(feature = "lang-go", feature = "lang-typescript"))]
    fn run_kernel(root: &Path, cache: &Cache, plan: &AnalysisPlan) -> KernelOutput {
        let loaded = load_config(root).expect("default config loads");
        AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache,
            config_digest: "config",
            rule_digest: "rules",
            plan,
            parallel: false,
        })
        .expect("kernel run should succeed")
    }

    #[cfg(any(feature = "lang-go", feature = "lang-typescript"))]
    fn symbol_graph_outcome(output: &KernelOutput) -> &crate::analysis_kernel::ProviderOutcome {
        output
            .run_report
            .provider_outcomes
            .iter()
            .find(|row| row.provider_id == "polint.symbol_graph")
            .expect("symbol graph provider outcome exists")
    }

    #[cfg(any(feature = "lang-go", feature = "lang-typescript"))]
    fn symbol_graph_telemetry(
        output: &KernelOutput,
    ) -> &crate::analysis_kernel::incremental::ProviderTelemetry {
        output
            .run_report
            .provider_telemetry
            .iter()
            .find(|row| row.provider_id == "polint.symbol_graph")
            .expect("symbol graph provider telemetry exists")
    }

    #[cfg(any(feature = "lang-go", feature = "lang-typescript"))]
    fn stable_export_keys(output: &KernelOutput) -> Vec<String> {
        let has_empty_key = output
            .db
            .stable_exports()
            .iter()
            .any(|row| output.db.resolve_stable_key(row.stable_key).is_empty());
        assert!(!has_empty_key);
        let mut keys = output
            .db
            .stable_exports()
            .iter()
            .map(|row| output.db.resolve_stable_key(row.stable_key).to_string())
            .collect::<Vec<_>>();
        keys.sort();
        keys
    }

    #[cfg(any(feature = "lang-go", feature = "lang-typescript"))]
    fn assert_warm_symbol_graph_reuse(output: &KernelOutput) {
        let outcome = symbol_graph_outcome(output);
        assert_eq!(
            outcome.status,
            crate::analysis_kernel::ProviderOutcomeStatus::Succeeded
        );
        assert!(outcome.output_identity.is_some());
        let telemetry = symbol_graph_telemetry(output);
        assert_eq!(telemetry.cache_stats.hits, 1);
        assert_eq!(telemetry.cache_stats.verified_reuse, 1);
        assert_eq!(telemetry.cache_stats.recomputes, 0);
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn ts_stable_exports_survive_warm_cache_restore() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(temp.path().join("src")).expect("create src");
        std::fs::write(
            temp.path().join("src/app.ts"),
            "import { token } from './tokens';\nexport function answer() { return token; }\n",
        )
        .expect("write app");
        std::fs::write(
            temp.path().join("src/tokens.ts"),
            "export const token = 42;\n",
        )
        .expect("write tokens");
        let cache = Cache::new(temp.path().join("cache").join("analysis"), true);
        let plan = requested_symbol_plan();

        let cold = run_kernel(temp.path(), &cache, &plan);
        let warm = run_kernel(temp.path(), &cache, &plan);

        assert_warm_symbol_graph_reuse(&warm);
        let cold_keys = stable_export_keys(&cold);
        let warm_keys = stable_export_keys(&warm);
        assert!(!cold_keys.is_empty());
        assert_eq!(cold_keys, warm_keys);
    }

    #[cfg(feature = "lang-go")]
    #[test]
    fn go_stable_exports_survive_warm_cache_restore_when_setup_succeeds() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join("go.mod"),
            "module example.com/cache\n\ngo 1.24\n",
        )
        .expect("write go.mod");
        std::fs::create_dir_all(temp.path().join("pkg")).expect("create pkg");
        std::fs::write(
            temp.path().join("pkg/cache.go"),
            "package pkg\n\nfunc Answer() int { return 42 }\n",
        )
        .expect("write go file");
        let cache = Cache::new(temp.path().join("cache").join("analysis"), true);
        let plan = requested_symbol_plan();

        let cold = run_kernel(temp.path(), &cache, &plan);
        let warm = run_kernel(temp.path(), &cache, &plan);

        assert_warm_symbol_graph_reuse(&warm);
        let cold_keys = stable_export_keys(&cold);
        let warm_keys = stable_export_keys(&warm);
        if cold_keys.is_empty() {
            assert!(cold.db.resolution_facts().iter().any(|row| {
                matches!(
                    row.status,
                    SemanticStatus::SetupMissing | SemanticStatus::Unsupported
                )
            }));
            assert!(cold.diagnostics.iter().any(|diagnostic| {
                diagnostic.rule_id == "polint/capability"
                    && diagnostic
                        .message
                        .contains("symbol graph provider support is setup_missing")
            }));
        } else {
            assert_eq!(cold_keys, warm_keys);
        }
    }
}

#[cfg(test)]
mod symbol_graph_go_facade_tests {
    use super::*;
    use crate::analysis_plan::AnalysisPlan;
    use crate::config::{LoadedConfig, load_config};
    use crate::core::{
        AnalysisDb, Capabilities, Rule, RuleKind, RuleMeta, RuleOptions, SymbolResolutionStatus,
    };
    use crate::diagnostics::Severity;
    use std::collections::BTreeMap;
    use std::path::Path;

    fn add_file(db: &mut AnalysisDb, root: &Path, relative_path: &str, source: &str) {
        let path = root.join(relative_path);
        std::fs::create_dir_all(path.parent().expect("fixture has parent")).expect("mkdirs");
        std::fs::write(&path, source).expect("write fixture");
        db.add_file(path, relative_path.to_string(), source.to_string());
    }

    fn add_go_file(db: &mut AnalysisDb, root: &Path, relative_path: &str, source: &str) {
        add_file(db, root, relative_path, source);
    }

    fn loaded_config_for(root: &Path) -> LoadedConfig {
        load_config(root).expect("config loads")
    }

    fn requested_plan_with_files(files: &[&str]) -> AnalysisPlan {
        let rule = Rule::from_parts(
            || RuleMeta {
                id: "local/needs-symbols".to_string(),
                description: "Needs symbols".to_string(),
                severity: Severity::Warn,
                kind: RuleKind::Check,
            },
            || Capabilities::new().references(),
            |_db, _ctx| Ok(()),
        );
        let mut options = BTreeMap::new();
        options.insert(
            "local/needs-symbols".to_string(),
            RuleOptions {
                files: files.iter().map(|file| (*file).to_string()).collect(),
                ..RuleOptions::default()
            },
        );
        AnalysisPlan::from_rules(&[rule], None, &options)
    }

    #[test]
    fn missing_go_mod_does_not_block_ts_only_symbol_rules() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut db = AnalysisDb::new();
        add_go_file(&mut db, temp.path(), "main.go", "package main\n");
        add_file(
            &mut db,
            temp.path(),
            "src/app.ts",
            "export function run() { return 1; }\n",
        );
        let derivation = derive_requested_symbols(
            &mut db,
            &loaded_config_for(temp.path()),
            &requested_plan_with_files(&["src/**/*.ts"]),
        );

        assert!(
            derivation
                .capability_support
                .iter()
                .all(|entry| entry.language != Some(crate::core::Language::Go)),
            "{:#?}",
            derivation.capability_support
        );
        assert!(
            db.references()
                .iter()
                .filter(|reference| reference.language == crate::core::Language::Go)
                .all(|reference| reference.status != SymbolResolutionStatus::SetupMissing),
            "TS-only rules should not receive Go setup-missing reference placeholders: {:#?}",
            db.references()
        );
    }

    #[test]
    fn go_setup_missing_derivation_emits_capability_diagnostics() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut db = AnalysisDb::new();
        add_go_file(&mut db, temp.path(), "main.go", "package main\n");

        let derivation = derive_requested_symbols(
            &mut db,
            &loaded_config_for(temp.path()),
            &AnalysisPlan::from_capability_names_for_test(&["symbols", "references"]),
        );

        assert!(
            derivation.diagnostics.iter().any(|diagnostic| {
                diagnostic.rule_id == "polint/capability"
                    && diagnostic.evidence.iter().any(|evidence| {
                        evidence.label == "status" && evidence.value == "setup_missing"
                    })
            }),
            "{:#?}",
            derivation.diagnostics
        );
    }
}

#[cfg(all(test, feature = "lang-typescript"))]
mod symbol_graph_ts_import_links {
    use super::ts;
    use crate::core::{
        AnalysisDb, FileId, ImportFact, ImportId, Language, ModuleNode, ModuleNodeId,
        ModuleNodeKind, ReferenceFact, ReferenceKind, ResolutionPrecision, ResolutionStatus,
        ResolvedImportFact, SymbolFact, SymbolPrecision, SymbolResolutionStatus, UnresolvedReason,
        span_from_byte_range,
    };
    use crate::symbol_graph::model::SymbolGraphBuilder;
    use std::fs;
    use std::path::Path;

    fn add_file(db: &mut AnalysisDb, root: &Path, relative_path: &str, source: &str) -> FileId {
        let path = root.join(relative_path);
        fs::create_dir_all(path.parent().expect("test file has parent")).expect("create fixture");
        fs::write(&path, source).expect("write fixture");
        db.add_file(path, relative_path.to_string(), source.to_string())
    }

    fn span_for(file: FileId, source: &str, needle: &str, occurrence: usize) -> crate::core::Span {
        let start = source
            .match_indices(needle)
            .nth(occurrence)
            .map(|(index, _)| index)
            .unwrap_or_else(|| panic!("missing occurrence {occurrence} of {needle:?}"));
        span_from_byte_range(file, source, start, start + needle.len())
    }

    fn push_import(
        db: &mut AnalysisDb,
        file: FileId,
        source: &str,
        path: &str,
        occurrence: usize,
    ) -> ImportId {
        db.push_import(ImportFact::new(
            ImportId::from_raw(0),
            file,
            None,
            path.to_string(),
            span_for(file, source, &format!("{path:?}"), occurrence),
            Language::TypeScript,
        ))
    }

    fn file_node(id: ModuleNodeId, label: &str, file: FileId) -> ModuleNode {
        ModuleNode::new(
            id,
            ModuleNodeKind::File,
            label.to_string(),
            Some(file),
            None,
            Some(Language::TypeScript),
        )
    }

    fn derive_symbol_reference_facts(
        db: &AnalysisDb,
        _root: &Path,
    ) -> (Vec<SymbolFact>, Vec<ReferenceFact>) {
        let mut builder = SymbolGraphBuilder::new(crate::core::StableKeyInterner::default());

        ts::derive_ts_symbols(
            &mut builder,
            db,
            crate::ts::symbol_graph::TsSymbolOptions {
                request: crate::analysis_neutral::symbol_graph::SymbolGraphRequest::new(true, true),
                resolved_imports: db.resolved_imports().to_vec(),
                module_nodes: db.module_nodes().to_vec(),
            },
        );
        let output = builder.finish();
        (output.symbols, output.references)
    }

    fn exported_symbol<'a>(symbols: &'a [SymbolFact], name: &str) -> &'a SymbolFact {
        symbols
            .iter()
            .find(|symbol| symbol.name == name && symbol.is_exported)
            .unwrap_or_else(|| panic!("missing exported symbol `{name}` in {symbols:#?}"))
    }

    #[test]
    fn links_named_default_and_namespace_imports_through_module_graph_targets() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut db = AnalysisDb::new();
        let source = r#"
import defaultThing, { exportedValue as localValue } from "./target";
import * as targetModule from "./target";

export const used = localValue + defaultThing() + targetModule.exportedValue;
"#;
        let target = r#"
export const exportedValue = 1;

const defaultThing = function() {
    return exportedValue;
};
export default defaultThing;
"#;
        let source_file = add_file(&mut db, temp.path(), "src/source.ts", source);
        let target_file = add_file(&mut db, temp.path(), "src/target.ts", target);
        let first_import = push_import(&mut db, source_file, source, "./target", 0);
        let second_import = push_import(&mut db, source_file, source, "./target", 1);
        let source_node = ModuleNodeId::from_raw(0);
        let target_node = ModuleNodeId::from_raw(1);
        db.replace_module_graph_facts(
            vec![
                ResolvedImportFact::new(
                    crate::core::ResolvedImportId::from_raw(0),
                    first_import,
                    source_file,
                    Some(target_node),
                    ResolutionStatus::Resolved,
                    ResolutionPrecision::ExactFile,
                    None,
                ),
                ResolvedImportFact::new(
                    crate::core::ResolvedImportId::from_raw(0),
                    second_import,
                    source_file,
                    Some(target_node),
                    ResolutionStatus::Resolved,
                    ResolutionPrecision::ExactFile,
                    None,
                ),
            ],
            vec![
                file_node(source_node, "src/source.ts", source_file),
                file_node(target_node, "src/target.ts", target_file),
            ],
            vec![],
        );

        let (symbols, references) = derive_symbol_reference_facts(&db, temp.path());
        let exported_value = exported_symbol(&symbols, "exportedValue");
        let default_thing = exported_symbol(&symbols, "defaultThing");

        assert!(references.iter().any(|reference| {
            reference.name == "localValue"
                && reference.kind == ReferenceKind::Import
                && reference.target == Some(exported_value.id)
                && reference.status == SymbolResolutionStatus::Resolved
                && reference.precision == SymbolPrecision::ModuleLinked
        }));
        assert!(references.iter().any(|reference| {
            reference.name == "defaultThing"
                && reference.kind == ReferenceKind::Import
                && reference.target == Some(default_thing.id)
                && reference.status == SymbolResolutionStatus::Resolved
                && reference.precision == SymbolPrecision::ModuleLinked
        }));
        assert!(references.iter().any(|reference| {
            reference.name == "targetModule"
                && reference.kind == ReferenceKind::Import
                && reference.status == SymbolResolutionStatus::Ambiguous
                && reference.candidates.contains(&exported_value.id)
                && reference.candidates.contains(&default_thing.id)
        }));
    }

    #[test]
    fn emits_visible_unresolved_import_alias_references() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut db = AnalysisDb::new();
        let source = r#"
import { missing } from "./missing";

export const used = missing;
"#;
        let source_file = add_file(&mut db, temp.path(), "src/source.ts", source);
        let import = push_import(&mut db, source_file, source, "./missing", 0);
        let source_node = ModuleNodeId::from_raw(0);
        db.replace_module_graph_facts(
            vec![ResolvedImportFact::new(
                crate::core::ResolvedImportId::from_raw(0),
                import,
                source_file,
                None,
                ResolutionStatus::Unresolved,
                ResolutionPrecision::None,
                Some(UnresolvedReason::NotFound),
            )],
            vec![file_node(source_node, "src/source.ts", source_file)],
            vec![],
        );

        let (_symbols, references) = derive_symbol_reference_facts(&db, temp.path());

        assert!(references.iter().any(|reference| {
            reference.name == "missing"
                && reference.kind == ReferenceKind::Import
                && reference.target.is_none()
                && reference.status == SymbolResolutionStatus::Unresolved
                && reference.precision == SymbolPrecision::Unresolved
        }));
    }
}
