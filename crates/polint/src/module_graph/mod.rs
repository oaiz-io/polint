pub(crate) mod go;
pub(crate) mod model;
use crate::repo_fs as paths;
pub(crate) mod topology;
pub(crate) mod ts;

use crate::analysis_kernel::incremental::{
    CacheNode, CacheStats, DependencyEdge, DependencyKind, Digest, DigestKind, InputComponent,
    InputSnapshot, LayerCacheManifest, LayerCacheReadStatus, LayerCacheStore,
    LayerCacheWriteStatus, LayerKey, LayerKind, PrecisionTier, ShapeKind, dependency_layer_digest,
    module_graph_topology_input_digest_rows, module_graph_topology_input_digests,
    relative_manifest_dependency_source, semantic_provider_parameter_digest,
};
use crate::analysis_kernel::{FactFamily, ProviderManifest, stable_key_text_from_parts};
use crate::analysis_plan::AnalysisPlan;
use crate::cache::Cache;
use crate::config::LoadedConfig;
use crate::core::{
    AnalysisDb, CapabilitySupport, CapabilitySupportStatus, CapabilitySupportView, FileId,
    ImportFact, Language, ModuleNodeId, ModuleNodeKind, ResolutionStatus, ResolvedImportId,
    SourceFile, UnresolvedReason,
};
use crate::diagnostics::{Diagnostic, TextRange};
use crate::symbol_graph::semantic::{SemanticImportFact, SemanticImportKind, SemanticStatus};
use model::{
    MODULE_GRAPH_LAYER_SCHEMA, MODULE_TOPOLOGY_LAYER_SCHEMA, ModuleGraphBuilder,
    ModuleGraphLayerPayload, ModuleTopologyLayerPayload, sort_packages,
};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use topology::{
    DependencyRequirementFact, ImportContextKind, ImportToPackageFact, ImportToPackageId,
    ImportToPackageStatus, RepoTopologyOverlayFact, RepoTopologyOverlayId, RepoTopologyOverlayKind,
    ResolvedDependencyEdgeFact, SourceSetFact, SourceSetKind, TopologyOutput, TopologyPackageFact,
    TopologyPackageKind, TopologyPrecision, TopologyStatus, WorkspaceRootFact,
};

const MODULE_GRAPH_TRIGGER_CAPABILITIES: &[&str] =
    &["resolved_imports", "module_graph", "symbols", "references"];

thread_local! {
    static MODULE_GRAPH_STABLE_KEYS: RefCell<Option<crate::core::StableKeyInterner>> =
        const { RefCell::new(None) };
}

pub(crate) fn with_module_graph_stable_keys<T>(
    interner: &crate::core::StableKeyInterner,
    f: impl FnOnce() -> T,
) -> T {
    MODULE_GRAPH_STABLE_KEYS.with(|slot| {
        let previous = slot.replace(Some(interner.clone()));
        let output = f();
        slot.replace(previous);
        output
    })
}

pub(crate) fn intern_module_graph_stable_key(
    key: impl AsRef<str> + Into<String>,
) -> crate::core::StableKeyId {
    MODULE_GRAPH_STABLE_KEYS.with(|slot| {
        slot.borrow()
            .as_ref()
            .expect("module graph stable-key interner is installed during topology derivation")
            .intern(key)
    })
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ModuleGraphDerivation {
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) capability_support: Vec<CapabilitySupport>,
    pub(crate) cache_stats: CacheStats,
    pub(crate) output_digest: Option<Digest>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ModuleTopologyDerivation {
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) capability_support: Vec<CapabilitySupport>,
    pub(crate) cache_stats: CacheStats,
    pub(crate) output_digest: Option<Digest>,
}

impl ModuleGraphDerivation {
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

pub(crate) fn module_graph_layer_key(
    root: &Path,
    db: &AnalysisDb,
    manifest: &ProviderManifest,
    config_digest: Digest,
    go_lifecycle_digest: Digest,
    ts_js_lifecycle_digest: Digest,
    upstream_syntax_output_digests: Vec<Digest>,
) -> LayerKey {
    let mut source_package_digests = module_graph_source_package_digests(db);
    source_package_digests.extend(module_graph_topology_input_digests(root, db));
    LayerKey::module_graph_layer_key(
        manifest,
        module_graph_import_shape_digests(db),
        source_package_digests,
        config_digest,
        go_lifecycle_digest,
        ts_js_lifecycle_digest,
        upstream_syntax_output_digests,
        module_graph_parameter_digest(),
    )
}

pub(crate) fn module_topology_layer_key(
    db: &AnalysisDb,
    manifest: &ProviderManifest,
    config_digest: Digest,
    go_lifecycle_digest: Digest,
    ts_js_lifecycle_digest: Digest,
    module_graph_output_digest: Digest,
    symbol_graph_output_digest: Digest,
) -> LayerKey {
    LayerKey::module_topology_layer_key(
        manifest,
        module_graph_import_shape_digests(db),
        module_topology_base_topology_digests(db),
        config_digest,
        go_lifecycle_digest,
        ts_js_lifecycle_digest,
        module_graph_output_digest,
        symbol_graph_output_digest,
        semantic_provider_parameter_digest(),
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "dependency-index rows are built from explicit cache-key inputs to avoid bundling unrelated state"
)]
pub(crate) fn module_graph_layer_dependency_edges(
    root: &Path,
    db: &AnalysisDb,
    key: &LayerKey,
    manifest: &ProviderManifest,
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

    for package in sort_packages(db.packages(), db) {
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

    for (relative_path, digest) in module_graph_topology_input_digest_rows(root, db) {
        edges.push(dependency_edge(
            &from,
            CacheNode::Input(format!("topology_input:{relative_path}:{digest}")),
            DependencyKind::Input,
            ShapeKind::ModuleTopology,
        ));
    }

    for file in sorted_files(db) {
        edges.push(dependency_edge(
            &from,
            CacheNode::Input(format!(
                "topology_source_set:{}:{}",
                normalized_file_path(file),
                language_cache_label(file.language)
            )),
            DependencyKind::Input,
            ShapeKind::ModuleTopology,
        ));
        edges.push(dependency_edge(
            &from,
            CacheNode::Input(format!(
                "topology_overlay_path:{}",
                normalized_file_path(file)
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
        CacheNode::Input("toolchain:module_graph:absent".to_string()),
        DependencyKind::Toolchain,
        ShapeKind::Toolchain,
    ));

    for (index, output_digest) in upstream_syntax_output_digests.iter().cloned().enumerate() {
        edges.push(dependency_edge(
            &from,
            CacheNode::Layer(upstream_syntax_layer_key(index, output_digest)),
            DependencyKind::UpstreamLayer,
            ShapeKind::Output,
        ));
    }

    edges.sort();
    edges.dedup();
    edges
}

#[expect(
    clippy::too_many_arguments,
    reason = "module topology cache dependencies mirror the explicit layer-key inputs"
)]
pub(crate) fn module_topology_layer_dependency_edges(
    db: &AnalysisDb,
    key: &LayerKey,
    manifest: &ProviderManifest,
    module_graph_output_digest: Digest,
    symbol_graph_output_digest: Digest,
    config_digest: Digest,
    go_lifecycle_digest: Digest,
    ts_js_lifecycle_digest: Digest,
) -> Vec<DependencyEdge> {
    let from = CacheNode::Layer(key.clone());
    let mut edges = Vec::new();

    for import in sorted_imports(db) {
        edges.push(dependency_edge(
            &from,
            CacheNode::Input(format!(
                "module_topology_import:{}:{}:{}",
                db.path_for(import.file),
                import.path,
                import.span.start_byte
            )),
            DependencyKind::ImportShape,
            ShapeKind::Import,
        ));
    }

    for digest in module_topology_base_topology_digests(db) {
        edges.push(dependency_edge(
            &from,
            CacheNode::Input(format!("module_topology_base:{digest}")),
            DependencyKind::Input,
            ShapeKind::ModuleTopology,
        ));
    }

    for semantic_import in db.semantic_imports() {
        edges.push(dependency_edge(
            &from,
            CacheNode::Input(format!(
                "semantic_import:{}:{}",
                db.resolve_stable_key(semantic_import.stable_key),
                semantic_import.import_path
            )),
            DependencyKind::Input,
            ShapeKind::ModuleTopology,
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
        CacheNode::Input(format!(
            "semantic_parameters:{}",
            semantic_provider_parameter_digest()
        )),
        DependencyKind::Provider,
        ShapeKind::ProviderVersion,
    ));
    edges.push(dependency_edge(
        &from,
        CacheNode::Input("toolchain:module_topology:absent".to_string()),
        DependencyKind::Toolchain,
        ShapeKind::Toolchain,
    ));
    edges.push(dependency_edge(
        &from,
        CacheNode::Layer(upstream_layer_key(
            LayerKind::ModuleGraph,
            "polint.module_graph",
            module_graph_output_digest,
        )),
        DependencyKind::UpstreamLayer,
        ShapeKind::Output,
    ));
    edges.push(dependency_edge(
        &from,
        CacheNode::Layer(upstream_layer_key(
            LayerKind::SymbolGraph,
            "polint.symbol_graph",
            symbol_graph_output_digest,
        )),
        DependencyKind::UpstreamLayer,
        ShapeKind::Output,
    ));

    edges.sort();
    edges.dedup();
    edges
}

pub(crate) fn module_graph_import_shape_digests(db: &AnalysisDb) -> Vec<Digest> {
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

pub(crate) fn module_graph_source_package_digests(db: &AnalysisDb) -> Vec<Digest> {
    let mut digests = sorted_files(db)
        .into_iter()
        .map(|file| {
            let parts = [
                normalized_file_path(file),
                file.content_hash.clone(),
                language_cache_label(file.language).to_string(),
            ];
            let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
            Digest::from_parts(DigestKind::SourceText, "source_package", &refs)
        })
        .collect::<Vec<_>>();
    digests.extend(sort_packages(db.packages(), db).into_iter().map(|package| {
        let parts = [
            db.path_for(package.file),
            package.name.clone(),
            language_cache_label(package.language).to_string(),
        ];
        let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
        Digest::from_parts(DigestKind::ProviderParameters, "package_context", &refs)
    }));
    digests.sort();
    digests
}

pub(crate) fn module_topology_base_topology_digests(db: &AnalysisDb) -> Vec<Digest> {
    let interner = db.stable_key_interner();
    let mut digests = Vec::new();
    digests.extend(db.workspace_roots().iter().map(|row| {
        Digest::from_parts(
            DigestKind::ProviderParameters,
            "topology_workspace_root",
            &[
                interner.resolve(row.stable_key).as_ref(),
                row.root_path.as_str(),
            ],
        )
    }));
    digests.extend(db.topology_packages().iter().map(|row| {
        Digest::from_parts(
            DigestKind::ProviderParameters,
            "topology_package",
            &[
                interner.resolve(row.stable_key).as_ref(),
                row.name.as_str(),
                row.path.as_str(),
            ],
        )
    }));
    digests.extend(db.source_sets().iter().map(|row| {
        Digest::from_parts(
            DigestKind::ProviderParameters,
            "topology_source_set",
            &[interner.resolve(row.stable_key).as_ref(), row.path.as_str()],
        )
    }));
    digests.extend(db.dependency_requirements().iter().map(|row| {
        Digest::from_parts(
            DigestKind::ProviderParameters,
            "topology_dependency_requirement",
            &[
                interner.resolve(row.stable_key).as_ref(),
                row.target_name.as_str(),
            ],
        )
    }));
    digests.extend(db.resolved_dependency_edges().iter().map(|row| {
        Digest::from_parts(
            DigestKind::ProviderParameters,
            "topology_resolved_dependency",
            &[
                interner.resolve(row.stable_key).as_ref(),
                row.package_name.as_str(),
            ],
        )
    }));
    digests.sort();
    digests
}

pub(crate) fn module_graph_parameter_digest() -> Digest {
    Digest::from_parts(
        DigestKind::ProviderParameters,
        "module_graph_parameters",
        &[
            "resolver=go+ts",
            "output=resolved_imports",
            "output=module_nodes",
            "output=module_edges",
        ],
    )
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Compatibility wrapper remains for direct in-crate module graph derivation callers while the kernel uses the stats-returning cache path."
    )
)]
pub(crate) fn derive_requested_module_graph(
    db: &mut AnalysisDb,
    loaded: &LoadedConfig,
    plan: &AnalysisPlan,
) -> ModuleGraphDerivation {
    derive_requested_module_graph_uncached(db, loaded, plan)
}

pub(crate) fn derive_import_to_package_edges(db: &AnalysisDb) -> Vec<ImportToPackageFact> {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let resolved_by_import = db
        .resolved_imports()
        .iter()
        .map(|fact| (fact.import, fact))
        .collect::<BTreeMap<_, _>>();
    let semantic_by_file_path =
        semantic_imports_by_file_path(&db.stable_key_interner(), db.semantic_imports());
    let source_sets_by_file = source_sets_by_file(db.source_sets());

    sorted_imports(db)
        .into_iter()
        .enumerate()
        .map(|(index, import)| {
            let resolved = resolved_by_import.get(&import.id).copied();
            let semantic_candidates = semantic_by_file_path
                .get(&(import.file, import.path.clone()))
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let semantic = semantic_import_match(semantic_candidates);
            let source_set = source_sets_by_file.get(&import.file).and_then(|sets| {
                sets.iter()
                    .filter_map(|id| source_set_by_id(db.source_sets(), *id))
                    .min_by_key(|set| interner.resolve(set.stable_key))
            });
            let from_package = source_set
                .and_then(|set| set.package)
                .and_then(|id| topology_package_by_id(db.topology_packages(), id));
            let target_node = resolved.and_then(|fact| fact.target_node);
            let mut candidates = target_node
                .map(|node| package_candidates_for_node(db, node))
                .unwrap_or_default();
            if candidates.is_empty()
                && let Some(node) = target_node.and_then(|id| module_node_by_id(db, id))
                && let Some(file) = node.file
            {
                candidates.extend(package_candidates_for_file(db, file));
            }
            candidates.sort_by(|package, other| {
                interner.compare_canonical(package.stable_key, other.stable_key)
            });
            candidates.dedup_by_key(|package| package.id);

            let status = import_to_package_status(
                import,
                resolved,
                semantic,
                target_node.and_then(|id| module_node_by_id(db, id)),
                &candidates,
                from_package,
                RequirementScope {
                    requirements: db.dependency_requirements(),
                    packages: db.topology_packages(),
                },
            );
            let to_package = if status == ImportToPackageStatus::Resolved && candidates.len() == 1 {
                candidates.first().copied()
            } else {
                None
            };
            let import_path = semantic
                .unique
                .map(|fact| fact.import_path.clone())
                .unwrap_or_else(|| import.path.clone());
            let from_package_stable_key = from_package.map(|package| package.stable_key);
            let to_package_stable_key = to_package.map(|package| package.stable_key);
            let source_set_stable_key = source_set.map(|set| set.stable_key);
            let stable_key = interner.intern(stable_key_text_from_parts(
                FactFamily::ImportToPackage,
                &[
                    ("import_id", import.id.0.to_string()),
                    ("path", import_path.clone()),
                    ("from_file", db.path_for(import.file)),
                    ("status", import_to_package_status_label(status).to_string()),
                ],
            ));

            ImportToPackageFact {
                id: ImportToPackageId(index as u64),
                syntax_import: Some(import.id),
                resolved_import: resolved.map(|fact| fact.id),
                semantic_import_stable_key: semantic.unique.map(|fact| fact.stable_key),
                from_file: Some(import.file),
                from_package: from_package.map(|package| package.id),
                to_package: to_package.map(|package| package.id),
                target_node,
                from_package_stable_key,
                to_package_stable_key,
                source_set_stable_key,
                import_path,
                context: import_context_for_source_set(source_set, db.file(import.file)),
                stable_key,
                producer_id: "polint.module_topology",
                precision: import_to_package_precision(status),
                status,
            }
        })
        .collect()
}

pub(crate) fn derive_module_topology_with_cache_stats(
    db: &mut AnalysisDb,
    cache: &Cache,
    input_snapshot: &InputSnapshot,
    manifest: &ProviderManifest,
    module_graph_output_digest: Digest,
    symbol_graph_output_digest: Digest,
) -> ModuleTopologyDerivation {
    if db.imports().is_empty() {
        db.replace_import_to_package_facts(Vec::new());
        return ModuleTopologyDerivation {
            output_digest: Some(module_topology_output_digest_for_payload(
                &ModuleTopologyLayerPayload {
                    schema: MODULE_TOPOLOGY_LAYER_SCHEMA.to_string(),
                    stable_key_texts: Vec::new(),
                    diagnostics: Vec::new(),
                    capability_support: Vec::new(),
                    import_to_package_edges: Vec::new(),
                },
                None,
            )),
            ..ModuleTopologyDerivation::default()
        };
    }

    let config_digest = input_snapshot.config.digest.clone();
    let go_lifecycle_digest = lifecycle_component_digest(
        DigestKind::GoLifecycle,
        "module_topology_go_lifecycle",
        &input_snapshot.go_lifecycle.components,
    );
    let ts_js_lifecycle_digest = lifecycle_component_digest(
        DigestKind::TsJsLifecycle,
        "module_topology_ts_js_lifecycle",
        &input_snapshot.ts_js_lifecycle.components,
    );
    let layer_key = module_topology_layer_key(
        db,
        manifest,
        config_digest.clone(),
        go_lifecycle_digest.clone(),
        ts_js_lifecycle_digest.clone(),
        module_graph_output_digest.clone(),
        symbol_graph_output_digest.clone(),
    );
    let store = cache.layer_cache_store();
    let mut cache_stats = CacheStats::default();
    let read = store.read_json_validated::<ModuleTopologyLayerPayload, _>(
        &layer_key,
        validate_module_topology_layer_payload,
    );

    match read.status {
        LayerCacheReadStatus::Hit => {
            cache_stats.record_hit();
            cache_stats.record_verified_reuse();
            let payload = read
                .value
                .expect("layer cache hit should include module topology payload");
            restore_module_topology_layer_payload(db, &payload);
            ModuleTopologyDerivation {
                diagnostics: payload.diagnostics,
                capability_support: payload.capability_support,
                cache_stats,
                output_digest: read.output_digest,
            }
        }
        LayerCacheReadStatus::BypassedDisabled => {
            cache_stats.record_disabled_bypass();
            cache_stats.record_recompute();
            let mut derivation = derive_module_topology_uncached(db);
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
            let mut derivation = derive_module_topology_uncached(db);
            let payload = module_topology_layer_payload(db, &derivation);
            let dependencies = module_topology_layer_dependency_edges(
                db,
                &layer_key,
                manifest,
                module_graph_output_digest,
                symbol_graph_output_digest,
                config_digest,
                go_lifecycle_digest,
                ts_js_lifecycle_digest,
            );
            derivation.output_digest = write_module_topology_layer_payload(
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

fn derive_module_topology_uncached(db: &mut AnalysisDb) -> ModuleTopologyDerivation {
    let import_to_package_edges = derive_import_to_package_edges(db);
    db.replace_import_to_package_facts(import_to_package_edges);
    let payload = module_topology_layer_payload(db, &ModuleTopologyDerivation::default());
    ModuleTopologyDerivation {
        diagnostics: Vec::new(),
        capability_support: Vec::new(),
        cache_stats: CacheStats::default(),
        output_digest: Some(module_topology_output_digest_for_payload(&payload, None)),
    }
}

fn semantic_imports_by_file_path<'a>(
    interner: &crate::core::StableKeyInterner,
    semantic_imports: &'a [SemanticImportFact],
) -> BTreeMap<(FileId, String), Vec<&'a SemanticImportFact>> {
    let mut imports = semantic_imports.iter().collect::<Vec<_>>();
    imports
        .sort_by(|import, other| interner.compare_canonical(import.stable_key, other.stable_key));
    let mut by_file_path = BTreeMap::new();
    for import in imports {
        if let Some(file) = import.file {
            by_file_path
                .entry((file, import.import_path.clone()))
                .or_insert_with(Vec::new)
                .push(import);
        }
    }
    by_file_path
}

#[derive(Clone, Copy)]
struct SemanticImportMatch<'a> {
    unique: Option<&'a SemanticImportFact>,
    duplicate_status: Option<ImportToPackageStatus>,
}

fn semantic_import_match<'a>(
    semantic_imports: &[&'a SemanticImportFact],
) -> SemanticImportMatch<'a> {
    SemanticImportMatch {
        unique: semantic_imports
            .first()
            .copied()
            .filter(|_| semantic_imports.len() == 1),
        duplicate_status: duplicate_semantic_import_status(semantic_imports),
    }
}

fn duplicate_semantic_import_status(
    semantic_imports: &[&SemanticImportFact],
) -> Option<ImportToPackageStatus> {
    if semantic_imports.len() <= 1 {
        return None;
    }
    let any_dynamic = semantic_imports
        .iter()
        .any(|fact| semantic_import_is_dynamic(fact));
    if any_dynamic {
        return if semantic_imports
            .iter()
            .all(|fact| semantic_import_is_dynamic(fact))
        {
            Some(ImportToPackageStatus::Dynamic)
        } else {
            Some(ImportToPackageStatus::Ambiguous)
        };
    }

    let first = semantic_imports[0];
    if semantic_imports
        .iter()
        .any(|fact| fact.status != first.status || fact.kind != first.kind)
    {
        return Some(ImportToPackageStatus::Ambiguous);
    }

    match first.status {
        SemanticStatus::Ambiguous | SemanticStatus::Cycle => Some(ImportToPackageStatus::Ambiguous),
        SemanticStatus::SetupMissing => Some(ImportToPackageStatus::SetupMissing),
        SemanticStatus::Unsupported => Some(ImportToPackageStatus::Unsupported),
        SemanticStatus::Dynamic
        | SemanticStatus::Resolved
        | SemanticStatus::Unresolved
        | SemanticStatus::Generated
        | SemanticStatus::External => None,
    }
}

fn semantic_import_is_dynamic(fact: &SemanticImportFact) -> bool {
    fact.status == SemanticStatus::Dynamic
        || matches!(
            fact.kind,
            SemanticImportKind::DynamicImport | SemanticImportKind::Dynamic
        )
}

fn source_sets_by_file(
    source_sets: &[SourceSetFact],
) -> BTreeMap<FileId, Vec<topology::SourceSetId>> {
    let mut by_file = BTreeMap::<FileId, Vec<_>>::new();
    for source_set in source_sets {
        for file in &source_set.files {
            by_file.entry(*file).or_default().push(source_set.id);
        }
    }
    by_file
}

fn source_set_by_id(
    source_sets: &[SourceSetFact],
    id: topology::SourceSetId,
) -> Option<&SourceSetFact> {
    source_sets.iter().find(|source_set| source_set.id == id)
}

fn topology_package_by_id(
    packages: &[TopologyPackageFact],
    id: topology::TopologyPackageId,
) -> Option<&TopologyPackageFact> {
    packages.iter().find(|package| package.id == id)
}

fn module_node_by_id(db: &AnalysisDb, id: ModuleNodeId) -> Option<&crate::core::ModuleNode> {
    db.module_nodes().iter().find(|node| node.id == id)
}

fn package_candidates_for_node(db: &AnalysisDb, node: ModuleNodeId) -> Vec<&TopologyPackageFact> {
    let mut candidates = db
        .topology_packages()
        .iter()
        .filter(|package| package.module_node == Some(node))
        .collect::<Vec<_>>();
    if candidates.is_empty()
        && let Some(node) = module_node_by_id(db, node)
        && node.language == Some(Language::Go)
        && node.kind == ModuleNodeKind::Package
    {
        candidates.extend(db.topology_packages().iter().filter(|package| {
            package.language == Some(Language::Go)
                && package.kind == TopologyPackageKind::Package
                && package.name == node.label
        }));
    }
    candidates
}

fn package_candidates_for_file(db: &AnalysisDb, file: FileId) -> Vec<&TopologyPackageFact> {
    db.source_sets()
        .iter()
        .filter(|source_set| source_set.files.contains(&file))
        .filter_map(|source_set| source_set.package)
        .filter_map(|package| topology_package_by_id(db.topology_packages(), package))
        .collect()
}

fn import_to_package_status(
    import: &ImportFact,
    resolved: Option<&crate::core::ResolvedImportFact>,
    semantic: SemanticImportMatch<'_>,
    target_node: Option<&crate::core::ModuleNode>,
    candidates: &[&TopologyPackageFact],
    from_package: Option<&TopologyPackageFact>,
    requirement_scope: RequirementScope<'_>,
) -> ImportToPackageStatus {
    if resolved.is_some_and(|fact| fact.reason == Some(UnresolvedReason::OutsideWorkspace)) {
        return ImportToPackageStatus::OutsideWorkspace;
    }
    if let Some(status) = semantic.duplicate_status {
        return status;
    }
    if semantic.unique.is_some_and(semantic_import_is_dynamic)
        || resolved.is_some_and(|fact| fact.status == ResolutionStatus::Dynamic)
    {
        return ImportToPackageStatus::Dynamic;
    }
    if semantic
        .unique
        .is_some_and(|fact| fact.status == SemanticStatus::SetupMissing)
        || resolved.is_some_and(|fact| fact.status == ResolutionStatus::SetupMissing)
    {
        return ImportToPackageStatus::SetupMissing;
    }
    if semantic
        .unique
        .is_some_and(|fact| fact.status == SemanticStatus::Unsupported)
        || resolved.is_some_and(|fact| fact.status == ResolutionStatus::Unsupported)
    {
        return ImportToPackageStatus::Unsupported;
    }
    if semantic
        .unique
        .is_some_and(|fact| fact.status == SemanticStatus::Ambiguous)
        || candidates.len() > 1
    {
        return ImportToPackageStatus::Ambiguous;
    }
    if target_node.is_none() || resolved.is_none_or(|fact| fact.target_node.is_none()) {
        return ImportToPackageStatus::Unresolved;
    }
    if target_node.is_some_and(|node| node.kind == ModuleNodeKind::External)
        || resolved.is_some_and(|fact| fact.status == ResolutionStatus::External)
        || semantic
            .unique
            .is_some_and(|fact| fact.status == SemanticStatus::External)
    {
        if declared_requirement_exists(import, from_package, requirement_scope) {
            ImportToPackageStatus::External
        } else {
            ImportToPackageStatus::Undeclared
        }
    } else if candidates.len() == 1 {
        ImportToPackageStatus::Resolved
    } else {
        ImportToPackageStatus::Unresolved
    }
}

#[derive(Clone, Copy)]
struct RequirementScope<'a> {
    requirements: &'a [DependencyRequirementFact],
    packages: &'a [TopologyPackageFact],
}

fn declared_requirement_exists(
    import: &ImportFact,
    from_package: Option<&TopologyPackageFact>,
    scope: RequirementScope<'_>,
) -> bool {
    scope.requirements.iter().any(|requirement| {
        requirement_matches_import(requirement, import)
            && from_package.is_none_or(|package| {
                requirement_applies_to_package(requirement, package, scope.packages)
            })
    })
}

fn requirement_matches_import(
    requirement: &DependencyRequirementFact,
    import: &ImportFact,
) -> bool {
    if import.language == Language::Go {
        import.path == requirement.target_name
            || import
                .path
                .strip_prefix(requirement.target_name.as_str())
                .is_some_and(|suffix| suffix.starts_with('/'))
    } else {
        requirement.target_name == external_package_name(&import.path)
    }
}

fn requirement_applies_to_package(
    requirement: &DependencyRequirementFact,
    package: &TopologyPackageFact,
    packages: &[TopologyPackageFact],
) -> bool {
    if requirement.from_package == Some(package.id) {
        return true;
    }
    if package.language != Some(Language::Go) {
        return false;
    }
    requirement
        .from_package
        .and_then(|id| topology_package_by_id(packages, id))
        .is_some_and(|requirement_package| {
            requirement_package.language == Some(Language::Go)
                && requirement_package.kind == TopologyPackageKind::Workspace
                && requirement_package.workspace_root == package.workspace_root
        })
}

fn external_package_name(path: &str) -> &str {
    if let Some(stripped) = path.strip_prefix('@') {
        let mut parts = stripped.split('/');
        let Some(scope_name) = parts.next() else {
            return path;
        };
        let Some(package_name) = parts.next() else {
            return path;
        };
        let end = 1 + scope_name.len() + 1 + package_name.len();
        &path[..end]
    } else {
        path.split('/').next().unwrap_or(path)
    }
}

fn import_context_for_source_set(
    source_set: Option<&SourceSetFact>,
    file: Option<&SourceFile>,
) -> ImportContextKind {
    match source_set.map(|set| set.kind) {
        Some(SourceSetKind::Test) => ImportContextKind::Test,
        Some(SourceSetKind::Generated) => ImportContextKind::Generated,
        Some(SourceSetKind::Vendor) => ImportContextKind::Vendor,
        Some(SourceSetKind::External) => ImportContextKind::External,
        Some(SourceSetKind::Source) => ImportContextKind::Source,
        Some(SourceSetKind::Unknown) | None => file
            .map(|file| path_context(&file.relative_path))
            .unwrap_or(ImportContextKind::Unknown),
    }
}

fn path_context(relative_path: &str) -> ImportContextKind {
    let normalized = relative_path.replace('\\', "/");
    if normalized.contains("/vendor/") || normalized.starts_with("vendor/") {
        ImportContextKind::Vendor
    } else if normalized.contains(".generated.") || normalized.contains("/generated/") {
        ImportContextKind::Generated
    } else if normalized.ends_with("_test.go")
        || normalized.contains(".test.")
        || normalized.contains(".spec.")
    {
        ImportContextKind::Test
    } else {
        ImportContextKind::Source
    }
}

fn import_to_package_precision(status: ImportToPackageStatus) -> TopologyPrecision {
    match status {
        ImportToPackageStatus::Resolved | ImportToPackageStatus::External => {
            TopologyPrecision::ExactStatic
        }
        ImportToPackageStatus::Unsupported => TopologyPrecision::Unsupported,
        ImportToPackageStatus::Ambiguous => TopologyPrecision::Heuristic,
        ImportToPackageStatus::Unresolved
        | ImportToPackageStatus::SetupMissing
        | ImportToPackageStatus::Dynamic
        | ImportToPackageStatus::Undeclared
        | ImportToPackageStatus::OutsideWorkspace => TopologyPrecision::Unknown,
    }
}

fn import_to_package_status_label(status: ImportToPackageStatus) -> &'static str {
    match status {
        ImportToPackageStatus::Resolved => "resolved",
        ImportToPackageStatus::External => "external",
        ImportToPackageStatus::Unresolved => "unresolved",
        ImportToPackageStatus::SetupMissing => "setup-missing",
        ImportToPackageStatus::Unsupported => "unsupported",
        ImportToPackageStatus::Dynamic => "dynamic",
        ImportToPackageStatus::Ambiguous => "ambiguous",
        ImportToPackageStatus::Undeclared => "undeclared",
        ImportToPackageStatus::OutsideWorkspace => "outside-workspace",
    }
}

pub(crate) fn derive_requested_module_graph_with_cache_stats(
    db: &mut AnalysisDb,
    loaded: &LoadedConfig,
    plan: &AnalysisPlan,
    cache: &Cache,
    input_snapshot: &InputSnapshot,
    manifest: &ProviderManifest,
    upstream_syntax_output_digests: Vec<Digest>,
) -> ModuleGraphDerivation {
    if !plan.requests_any_capability(MODULE_GRAPH_TRIGGER_CAPABILITIES) {
        return ModuleGraphDerivation::default();
    }

    let config_digest = input_snapshot.config.digest.clone();
    let go_lifecycle_digest = lifecycle_component_digest(
        DigestKind::GoLifecycle,
        "module_graph_go_lifecycle",
        &input_snapshot.go_lifecycle.components,
    );
    let ts_js_lifecycle_digest = lifecycle_component_digest(
        DigestKind::TsJsLifecycle,
        "module_graph_ts_js_lifecycle",
        &input_snapshot.ts_js_lifecycle.components,
    );
    let layer_key = module_graph_layer_key(
        loaded.root.as_path(),
        db,
        manifest,
        config_digest.clone(),
        go_lifecycle_digest.clone(),
        ts_js_lifecycle_digest.clone(),
        upstream_syntax_output_digests.clone(),
    );
    let store = cache.layer_cache_store();
    let mut cache_stats = CacheStats::default();
    let read = store
        .read_json_validated::<ModuleGraphLayerPayload, _>(&layer_key, |payload, manifest| {
            validate_module_graph_layer_payload(payload, manifest)
        });

    match read.status {
        LayerCacheReadStatus::Hit => {
            cache_stats.record_hit();
            cache_stats.record_verified_reuse();
            let payload = read
                .value
                .expect("layer cache hit should include module graph payload");
            restore_module_graph_layer_payload(db, &payload);
            ModuleGraphDerivation {
                diagnostics: payload.diagnostics,
                capability_support: payload.capability_support,
                cache_stats,
                output_digest: read.output_digest,
            }
        }
        LayerCacheReadStatus::BypassedDisabled => {
            cache_stats.record_disabled_bypass();
            cache_stats.record_recompute();
            let mut derivation = derive_requested_module_graph_uncached(db, loaded, plan);
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
            let mut derivation = derive_requested_module_graph_uncached(db, loaded, plan);
            let payload = module_graph_layer_payload(db, &derivation);
            let dependencies = module_graph_layer_dependency_edges(
                loaded.root.as_path(),
                db,
                &layer_key,
                manifest,
                &upstream_syntax_output_digests,
                config_digest,
                go_lifecycle_digest,
                ts_js_lifecycle_digest,
            );
            derivation.output_digest = write_module_graph_layer_payload(
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

fn derive_requested_module_graph_uncached(
    db: &mut AnalysisDb,
    loaded: &LoadedConfig,
    plan: &AnalysisPlan,
) -> ModuleGraphDerivation {
    if !plan.requests_any_capability(MODULE_GRAPH_TRIGGER_CAPABILITIES) {
        return ModuleGraphDerivation::default();
    }

    let mut builder = ModuleGraphBuilder::new(db);
    let has_graph_inputs =
        !db.files().is_empty() || !db.packages().is_empty() || !db.imports().is_empty();
    let root_module = has_graph_inputs.then(|| builder.ensure_module_node("."));
    let mut file_nodes = BTreeMap::new();
    let mut files = db.files().iter().collect::<Vec<_>>();
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

    for file in &files {
        file_nodes.insert(file.id, builder.ensure_file_node(file.id));
    }
    let mut file_owner_modules =
        ts::seed_ts_project_module_nodes(&mut builder, loaded.root.as_path(), &files);
    let has_go_inputs = files
        .iter()
        .any(|file| file.language == crate::core::Language::Go)
        || db
            .imports()
            .iter()
            .any(|import| import.language == crate::core::Language::Go);
    let go_metadata = if has_go_inputs {
        go::GoPackageIndex::load(loaded.root.as_path(), &loaded.config.languages.go, db)
    } else {
        go::GoPackageIndex::default()
    };
    let go_ownership = go::seed_go_module_nodes(&mut builder, &go_metadata);
    for (file, module) in go_ownership.file_owner_modules() {
        file_owner_modules.insert(file, module);
    }

    let mut package_nodes_by_file = BTreeMap::new();
    for package in sort_packages(db.packages(), db) {
        let package_node = if package.language == crate::core::Language::Go {
            go_ownership
                .package_node_for_file(package.file)
                .unwrap_or_else(|| builder.ensure_package_node(package))
        } else {
            builder.ensure_package_node(package)
        };
        let file_node = builder.ensure_file_node(package.file);
        if let Some(owner_module) = file_owner_modules
            .get(&package.file)
            .copied()
            .or(root_module)
        {
            builder.link_module_contains(owner_module, package_node);
        }
        builder.link_contains(package_node, file_node);
        package_nodes_by_file.insert(package.file, package_node);
    }
    for (file, package_node) in go_ownership.package_nodes_by_file() {
        package_nodes_by_file.entry(file).or_insert(package_node);
    }

    for file in &files {
        if !package_nodes_by_file.contains_key(&file.id) {
            let file_node = builder.ensure_file_node(file.id);
            if let Some(owner_module) = file_owner_modules.get(&file.id).copied().or(root_module) {
                builder.link_module_contains(owner_module, file_node);
            }
        }
    }

    let mut imports = db.imports().iter().collect::<Vec<_>>();
    imports.sort_by(|left, right| import_order(left, right, db));
    let ts_resolver_context = imports
        .iter()
        .any(|import| import.language.is_ts_family())
        .then(|| ts::TsResolverContext::new(loaded.root.as_path(), db, root_module));

    let mut resolved_imports = Vec::with_capacity(imports.len());
    let mut saw_setup_missing = false;
    let mut setup_missing_reason = None;
    for import in imports {
        let owner_module = file_owner_modules
            .get(&import.file)
            .copied()
            .or(root_module);
        let default_owner = package_nodes_by_file
            .get(&import.file)
            .copied()
            .or_else(|| file_nodes.get(&import.file).copied())
            .or(owner_module)
            .unwrap_or_else(|| builder.ensure_module_node("."));
        let index = resolved_imports.len();
        let input = crate::analysis_neutral::module_graph::model::ResolverInput {
            root: loaded.root.as_path(),
            db,
            import,
            owner_module,
            owner_package: package_nodes_by_file.get(&import.file).copied(),
        };
        let draft = if import.language.is_ts_family() {
            ts::resolve_ts_import(input, ts_resolver_context.as_ref())
        } else if matches!(import.language, crate::core::Language::Go) {
            go::resolve_go_import(input, &go_metadata)
        } else {
            model::ResolvedImportDraft::unsupported_language()
        };
        if draft.status == ResolutionStatus::SetupMissing && setup_missing_reason.is_none() {
            setup_missing_reason = Some(setup_missing_reason_for_import(
                import.language,
                &go_metadata,
                has_go_inputs,
            ));
        }
        let owner = if import.language.is_ts_family() && draft.status == ResolutionStatus::External
        {
            owner_module.unwrap_or(default_owner)
        } else {
            default_owner
        };
        let fact = builder.apply_resolved_import_draft_with_id(
            import,
            owner,
            draft,
            ResolvedImportId::from_raw(index as u64),
        );
        saw_setup_missing |= fact.status == ResolutionStatus::SetupMissing;
        resolved_imports.push(fact);
    }

    let output = builder.finish();
    db.replace_module_graph_facts(resolved_imports, output.nodes, output.edges);
    let base_topology = derive_base_topology(loaded, db, &go_metadata);
    db.replace_topology_facts(base_topology);

    let capability_support =
        setup_missing_support(plan, saw_setup_missing, setup_missing_reason.as_deref());
    let diagnostics = setup_missing_diagnostics(&capability_support);
    let output_digest = Some(module_graph_output_digest_for_payload(
        &module_graph_layer_payload_parts(
            &db.stable_key_interner(),
            &diagnostics,
            &capability_support,
            db.resolved_imports(),
            db.module_nodes(),
            db.module_edges(),
            db.workspace_roots(),
            db.topology_packages(),
            db.source_sets(),
            db.dependency_requirements(),
            db.resolved_dependency_edges(),
            db.repo_topology_overlays(),
        ),
        None,
    ));
    ModuleGraphDerivation {
        diagnostics,
        capability_support,
        cache_stats: CacheStats::default(),
        output_digest,
    }
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

fn module_graph_layer_payload(
    db: &AnalysisDb,
    derivation: &ModuleGraphDerivation,
) -> ModuleGraphLayerPayload {
    let (topology, stable_key_texts) = TopologyOutput {
        workspace_roots: db.workspace_roots().to_vec(),
        packages: db.topology_packages().to_vec(),
        source_sets: db.source_sets().to_vec(),
        dependency_requirements: db.dependency_requirements().to_vec(),
        resolved_dependency_edges: db.resolved_dependency_edges().to_vec(),
        import_to_package_edges: Vec::new(),
        overlays: db.repo_topology_overlays().to_vec(),
    }
    .canonicalized_for_cache(&db.stable_key_interner());
    ModuleGraphLayerPayload {
        schema: MODULE_GRAPH_LAYER_SCHEMA.to_string(),
        stable_key_texts,
        diagnostics: derivation.diagnostics.clone(),
        capability_support: derivation.capability_support.clone(),
        resolved_imports: db.resolved_imports().to_vec(),
        nodes: db.module_nodes().to_vec(),
        edges: db.module_edges().to_vec(),
        workspace_roots: topology.workspace_roots,
        topology_packages: topology.packages,
        source_sets: topology.source_sets,
        dependency_requirements: topology.dependency_requirements,
        resolved_dependency_edges: topology.resolved_dependency_edges,
        repo_topology_overlays: topology.overlays,
    }
}

fn module_topology_layer_payload(
    db: &AnalysisDb,
    derivation: &ModuleTopologyDerivation,
) -> ModuleTopologyLayerPayload {
    let (topology, stable_key_texts) = TopologyOutput {
        import_to_package_edges: db.import_to_package_edges().to_vec(),
        ..TopologyOutput::default()
    }
    .canonicalized_for_cache(&db.stable_key_interner());
    ModuleTopologyLayerPayload {
        schema: MODULE_TOPOLOGY_LAYER_SCHEMA.to_string(),
        stable_key_texts,
        diagnostics: derivation.diagnostics.clone(),
        capability_support: derivation.capability_support.clone(),
        import_to_package_edges: topology.import_to_package_edges,
    }
}

fn restore_module_graph_layer_payload(db: &mut AnalysisDb, payload: &ModuleGraphLayerPayload) {
    db.replace_module_graph_facts(
        payload.resolved_imports.clone(),
        payload.nodes.clone(),
        payload.edges.clone(),
    );
    let topology = TopologyOutput {
        workspace_roots: payload.workspace_roots.clone(),
        packages: payload.topology_packages.clone(),
        source_sets: payload.source_sets.clone(),
        dependency_requirements: payload.dependency_requirements.clone(),
        resolved_dependency_edges: payload.resolved_dependency_edges.clone(),
        import_to_package_edges: Vec::new(),
        overlays: payload.repo_topology_overlays.clone(),
    }
    .reintern_cached(&payload.stable_key_texts, &db.stable_key_interner())
    .expect("validated module graph cache contains stable-key text for every topology identity");
    db.replace_topology_facts(topology);
}

fn restore_module_topology_layer_payload(
    db: &mut AnalysisDb,
    payload: &ModuleTopologyLayerPayload,
) {
    let topology = TopologyOutput {
        import_to_package_edges: payload.import_to_package_edges.clone(),
        ..TopologyOutput::default()
    }
    .reintern_cached(&payload.stable_key_texts, &db.stable_key_interner())
    .expect("validated module topology cache contains stable-key text for every identity");
    db.replace_import_to_package_facts(topology.import_to_package_edges);
}

fn validate_module_graph_layer_payload(
    payload: &ModuleGraphLayerPayload,
    manifest: &LayerCacheManifest,
) -> bool {
    payload.schema == MODULE_GRAPH_LAYER_SCHEMA
        && topology_payload_stable_keys_are_unique(payload)
        && cached_module_graph_topology(payload).is_some()
        && manifest.output_digest
            == module_graph_output_digest_for_payload(payload, Some(&manifest.key))
}

pub(crate) fn validate_module_topology_layer_payload(
    payload: &ModuleTopologyLayerPayload,
    manifest: &LayerCacheManifest,
) -> bool {
    payload.schema == MODULE_TOPOLOGY_LAYER_SCHEMA
        && import_to_package_payload_stable_keys_are_unique(&payload.import_to_package_edges)
        && cached_module_topology(payload).is_some()
        && manifest.output_digest
            == module_topology_output_digest_for_payload(payload, Some(&manifest.key))
}

fn cached_module_graph_topology(payload: &ModuleGraphLayerPayload) -> Option<TopologyOutput> {
    TopologyOutput {
        workspace_roots: payload.workspace_roots.clone(),
        packages: payload.topology_packages.clone(),
        source_sets: payload.source_sets.clone(),
        dependency_requirements: payload.dependency_requirements.clone(),
        resolved_dependency_edges: payload.resolved_dependency_edges.clone(),
        import_to_package_edges: Vec::new(),
        overlays: payload.repo_topology_overlays.clone(),
    }
    .reintern_cached(
        &payload.stable_key_texts,
        &crate::core::StableKeyInterner::default(),
    )
}

fn cached_module_topology(payload: &ModuleTopologyLayerPayload) -> Option<TopologyOutput> {
    TopologyOutput {
        import_to_package_edges: payload.import_to_package_edges.clone(),
        ..TopologyOutput::default()
    }
    .reintern_cached(
        &payload.stable_key_texts,
        &crate::core::StableKeyInterner::default(),
    )
}

fn import_to_package_payload_stable_keys_are_unique(rows: &[ImportToPackageFact]) -> bool {
    let mut seen = BTreeSet::new();
    rows.iter().all(|row| seen.insert(row.stable_key))
}

fn write_module_graph_layer_payload(
    store: &LayerCacheStore,
    layer_key: LayerKey,
    payload: &ModuleGraphLayerPayload,
    dependencies: Vec<DependencyEdge>,
    stats: &mut CacheStats,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Digest> {
    let payload_digest = match LayerCacheStore::payload_digest_for_json(payload) {
        Ok(digest) => digest,
        Err(error) => {
            diagnostics.push(cache_write_diagnostic("module graph layer", error));
            return None;
        }
    };
    let output_digest = module_graph_output_digest(&layer_key, &payload_digest);
    let manifest = LayerCacheManifest::new(
        layer_key,
        output_digest.clone(),
        payload_digest,
        dependencies,
        PrecisionTier::SetupAware,
        "native_trusted",
        Vec::new(),
    );

    match store.write_json(&manifest, payload) {
        Ok(LayerCacheWriteStatus::Written) => stats.record_write(),
        Ok(LayerCacheWriteStatus::BypassedDisabled) => stats.record_disabled_bypass(),
        Err(error) => diagnostics.push(cache_write_diagnostic("module graph layer", error)),
    }
    Some(output_digest)
}

fn write_module_topology_layer_payload(
    store: &LayerCacheStore,
    layer_key: LayerKey,
    payload: &ModuleTopologyLayerPayload,
    dependencies: Vec<DependencyEdge>,
    stats: &mut CacheStats,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Digest> {
    let payload_digest = match LayerCacheStore::payload_digest_for_json(payload) {
        Ok(digest) => digest,
        Err(error) => {
            diagnostics.push(cache_write_diagnostic("module topology layer", error));
            return None;
        }
    };
    let output_digest = module_topology_output_digest(&layer_key, &payload_digest);
    let manifest = LayerCacheManifest::new(
        layer_key,
        output_digest.clone(),
        payload_digest,
        dependencies,
        PrecisionTier::SetupAware,
        "native_trusted",
        Vec::new(),
    );

    match store.write_json(&manifest, payload) {
        Ok(LayerCacheWriteStatus::Written) => stats.record_write(),
        Ok(LayerCacheWriteStatus::BypassedDisabled) => stats.record_disabled_bypass(),
        Err(error) => diagnostics.push(cache_write_diagnostic("module topology layer", error)),
    }
    Some(output_digest)
}

fn module_graph_output_digest_for_payload(
    payload: &ModuleGraphLayerPayload,
    layer_key: Option<&LayerKey>,
) -> Digest {
    let payload_digest = LayerCacheStore::payload_digest_for_json(payload)
        .unwrap_or_else(|_| Digest::unsupported(DigestKind::LayerOutput, "module_graph", "json"));
    if let Some(layer_key) = layer_key {
        module_graph_output_digest(layer_key, &payload_digest)
    } else {
        Digest::from_parts(
            DigestKind::ProviderOutput,
            "module_graph_layer_output",
            &[&payload_digest.to_string()],
        )
    }
}

fn module_topology_output_digest_for_payload(
    payload: &ModuleTopologyLayerPayload,
    layer_key: Option<&LayerKey>,
) -> Digest {
    let payload_digest = LayerCacheStore::payload_digest_for_json(payload).unwrap_or_else(|_| {
        Digest::unsupported(DigestKind::LayerOutput, "module_topology", "json")
    });
    if let Some(layer_key) = layer_key {
        module_topology_output_digest(layer_key, &payload_digest)
    } else {
        Digest::from_parts(
            DigestKind::ProviderOutput,
            "module_topology_layer_output",
            &[&payload_digest.to_string()],
        )
    }
}

fn module_graph_output_digest(layer_key: &LayerKey, payload_digest: &Digest) -> Digest {
    let layer_key_json =
        serde_json::to_string(layer_key).unwrap_or_else(|_| "unserializable_layer_key".to_string());
    Digest::from_parts(
        DigestKind::ProviderOutput,
        "module_graph_layer_output",
        &[&payload_digest.to_string(), &layer_key_json],
    )
}

fn module_topology_output_digest(layer_key: &LayerKey, payload_digest: &Digest) -> Digest {
    let layer_key_json =
        serde_json::to_string(layer_key).unwrap_or_else(|_| "unserializable_layer_key".to_string());
    Digest::from_parts(
        DigestKind::ProviderOutput,
        "module_topology_layer_output",
        &[&payload_digest.to_string(), &layer_key_json],
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "payload construction keeps persisted row families explicit and schema-aligned"
)]
fn module_graph_layer_payload_parts(
    interner: &crate::core::StableKeyInterner,
    diagnostics: &[Diagnostic],
    capability_support: &[CapabilitySupport],
    resolved_imports: &[crate::core::ResolvedImportFact],
    nodes: &[crate::core::ModuleNode],
    edges: &[crate::core::ModuleEdge],
    workspace_roots: &[WorkspaceRootFact],
    topology_packages: &[TopologyPackageFact],
    source_sets: &[SourceSetFact],
    dependency_requirements: &[DependencyRequirementFact],
    resolved_dependency_edges: &[ResolvedDependencyEdgeFact],
    repo_topology_overlays: &[RepoTopologyOverlayFact],
) -> ModuleGraphLayerPayload {
    let (topology, stable_key_texts) = TopologyOutput {
        workspace_roots: workspace_roots.to_vec(),
        packages: topology_packages.to_vec(),
        source_sets: source_sets.to_vec(),
        dependency_requirements: dependency_requirements.to_vec(),
        resolved_dependency_edges: resolved_dependency_edges.to_vec(),
        import_to_package_edges: Vec::new(),
        overlays: repo_topology_overlays.to_vec(),
    }
    .canonicalized_for_cache(interner);
    ModuleGraphLayerPayload {
        schema: MODULE_GRAPH_LAYER_SCHEMA.to_string(),
        stable_key_texts,
        diagnostics: diagnostics.to_vec(),
        capability_support: capability_support.to_vec(),
        resolved_imports: resolved_imports.to_vec(),
        nodes: nodes.to_vec(),
        edges: edges.to_vec(),
        workspace_roots: topology.workspace_roots,
        topology_packages: topology.packages,
        source_sets: topology.source_sets,
        dependency_requirements: topology.dependency_requirements,
        resolved_dependency_edges: topology.resolved_dependency_edges,
        repo_topology_overlays: topology.overlays,
    }
}

fn topology_payload_stable_keys_are_unique(payload: &ModuleGraphLayerPayload) -> bool {
    topology_stable_keys_unique(
        "WorkspaceRootFact",
        payload.workspace_roots.iter().map(|row| row.stable_key),
    ) && topology_stable_keys_unique(
        "TopologyPackageFact",
        payload.topology_packages.iter().map(|row| row.stable_key),
    ) && topology_stable_keys_unique(
        "SourceSetFact",
        payload.source_sets.iter().map(|row| row.stable_key),
    ) && topology_stable_keys_unique(
        "DependencyRequirementFact",
        payload
            .dependency_requirements
            .iter()
            .map(|row| row.stable_key),
    ) && topology_stable_keys_unique(
        "ResolvedDependencyEdgeFact",
        payload
            .resolved_dependency_edges
            .iter()
            .map(|row| row.stable_key),
    ) && topology_stable_keys_unique(
        "RepoTopologyOverlayFact",
        payload
            .repo_topology_overlays
            .iter()
            .map(|row| row.stable_key),
    )
}

fn topology_stable_keys_unique(
    family: &'static str,
    stable_keys: impl Iterator<Item = crate::core::StableKeyId>,
) -> bool {
    let mut seen = BTreeSet::new();
    for stable_key in stable_keys {
        if !seen.insert(stable_key) {
            let _stable_key_conflict = (family, stable_key);
            return false;
        }
    }
    true
}

pub(crate) fn derive_base_topology(
    loaded: &LoadedConfig,
    db: &AnalysisDb,
    go_metadata: &go::GoPackageIndex,
) -> TopologyOutput {
    let interner = db.stable_key_interner();
    let mut output = go::collect_go_topology(
        loaded.root.as_path(),
        &loaded.config.languages.go,
        db,
        go_metadata,
    );
    output.merge(ts::collect_ts_topology(loaded.root.as_path(), db));
    output.overlays.extend(collect_repo_topology_overlays(
        loaded.root.as_path(),
        db,
        &output,
    ));
    output.normalized(&interner)
}

pub(crate) fn collect_repo_topology_overlays(
    root: &Path,
    db: &AnalysisDb,
    topology: &TopologyOutput,
) -> Vec<RepoTopologyOverlayFact> {
    let interner = db.stable_key_interner();
    with_module_graph_stable_keys(&interner, || {
        collect_repo_topology_overlays_inner(root, topology)
    })
}

fn collect_repo_topology_overlays_inner(
    root: &Path,
    topology: &TopologyOutput,
) -> Vec<RepoTopologyOverlayFact> {
    let mut overlays = Vec::new();
    let mut seen_keys = BTreeSet::new();

    for path in codeowner_paths(root) {
        push_repo_overlay(
            &mut overlays,
            &mut seen_keys,
            RepoTopologyOverlayKind::OwnershipZone,
            format!("CODEOWNERS:{path}"),
            Some(path.clone()),
            format!("repo-topology:ownership-zone:{path}"),
            TopologyPrecision::ExactStatic,
            TopologyStatus::Present,
        );
    }

    for source_set in &topology.source_sets {
        match source_set.kind {
            SourceSetKind::Generated => push_repo_overlay(
                &mut overlays,
                &mut seen_keys,
                RepoTopologyOverlayKind::GeneratedZone,
                format!("generated:{}", source_set.path),
                Some(source_set.path.clone()),
                format!("repo-topology:generated-zone:{}", source_set.path),
                TopologyPrecision::ExactStatic,
                TopologyStatus::Present,
            ),
            SourceSetKind::Test => push_repo_overlay(
                &mut overlays,
                &mut seen_keys,
                RepoTopologyOverlayKind::TestOnlyVisibility,
                format!("test-only:{}", source_set.path),
                Some(source_set.path.clone()),
                format!("repo-topology:test-only-visibility:{}", source_set.path),
                TopologyPrecision::ExactStatic,
                TopologyStatus::Present,
            ),
            SourceSetKind::Source
            | SourceSetKind::Vendor
            | SourceSetKind::External
            | SourceSetKind::Unknown => {}
        }

        if let Some(layer) = conventional_architecture_layer(&source_set.path) {
            push_repo_overlay(
                &mut overlays,
                &mut seen_keys,
                RepoTopologyOverlayKind::ArchitectureLayer,
                format!("{layer}:{}", source_set.path),
                Some(source_set.path.clone()),
                format!(
                    "repo-topology:architecture-layer:{layer}:{}",
                    source_set.path
                ),
                TopologyPrecision::Heuristic,
                TopologyStatus::Present,
            );
        }
        if let Some(deploy_unit) = conventional_deploy_unit(&source_set.path) {
            push_repo_overlay(
                &mut overlays,
                &mut seen_keys,
                RepoTopologyOverlayKind::DeployUnit,
                format!("{deploy_unit}:{}", source_set.path),
                Some(source_set.path.clone()),
                format!(
                    "repo-topology:deploy-unit:{deploy_unit}:{}",
                    source_set.path
                ),
                TopologyPrecision::Heuristic,
                TopologyStatus::Present,
            );
        }
        if is_internal_path(&source_set.path) {
            push_repo_overlay(
                &mut overlays,
                &mut seen_keys,
                RepoTopologyOverlayKind::InternalPublicApiBoundary,
                format!("internal:{}", source_set.path),
                Some(source_set.path.clone()),
                format!(
                    "repo-topology:internal-public-api-boundary:internal:{}",
                    source_set.path
                ),
                TopologyPrecision::ExactStatic,
                TopologyStatus::Present,
            );
        }
    }

    for package in &topology.packages {
        push_repo_overlay(
            &mut overlays,
            &mut seen_keys,
            RepoTopologyOverlayKind::SourceOfTruthDirectory,
            format!("package:{}", package.name),
            Some(package.path.clone()),
            format!(
                "repo-topology:source-of-truth-directory:package:{}",
                package.path
            ),
            TopologyPrecision::ExactStatic,
            TopologyStatus::Present,
        );
    }
    for workspace_root in &topology.workspace_roots {
        push_repo_overlay(
            &mut overlays,
            &mut seen_keys,
            RepoTopologyOverlayKind::SourceOfTruthDirectory,
            format!("workspace-root:{}", workspace_root.root_path),
            Some(workspace_root.root_path.clone()),
            format!(
                "repo-topology:source-of-truth-directory:workspace-root:{}",
                workspace_root.root_path
            ),
            TopologyPrecision::ExactStatic,
            TopologyStatus::Present,
        );
    }

    for (kind, label, stable_key, always_emit) in [
        (
            RepoTopologyOverlayKind::OwnershipZone,
            "ownership-zone:unknown",
            "repo-topology:ownership-zone:unknown",
            false,
        ),
        (
            RepoTopologyOverlayKind::ArchitectureLayer,
            "architecture-layer:unknown",
            "repo-topology:architecture-layer:unknown",
            true,
        ),
        (
            RepoTopologyOverlayKind::DeployUnit,
            "deploy-unit:unknown",
            "repo-topology:deploy-unit:unknown",
            true,
        ),
        (
            RepoTopologyOverlayKind::GeneratedZone,
            "generated-zone:unknown",
            "repo-topology:generated-zone:unknown",
            false,
        ),
        (
            RepoTopologyOverlayKind::TestOnlyVisibility,
            "test-only-visibility:unknown",
            "repo-topology:test-only-visibility:unknown",
            false,
        ),
        (
            RepoTopologyOverlayKind::InternalPublicApiBoundary,
            "internal-public-api-boundary:unknown",
            "repo-topology:internal-public-api-boundary:unknown",
            true,
        ),
        (
            RepoTopologyOverlayKind::SourceOfTruthDirectory,
            "source-of-truth-directory:unknown",
            "repo-topology:source-of-truth-directory:unknown",
            false,
        ),
    ] {
        if always_emit || !overlays.iter().any(|overlay| overlay.kind == kind) {
            push_repo_overlay(
                &mut overlays,
                &mut seen_keys,
                kind,
                label.to_string(),
                None,
                stable_key.to_string(),
                TopologyPrecision::Unknown,
                TopologyStatus::Unknown,
            );
        }
    }

    overlays
}

#[expect(
    clippy::too_many_arguments,
    reason = "overlay construction records every topology evidence field at the call site"
)]
fn push_repo_overlay(
    overlays: &mut Vec<RepoTopologyOverlayFact>,
    seen_keys: &mut BTreeSet<String>,
    kind: RepoTopologyOverlayKind,
    label: String,
    path: Option<String>,
    stable_key_text: String,
    precision: TopologyPrecision,
    status: TopologyStatus,
) {
    if !seen_keys.insert(stable_key_text.clone()) {
        return;
    }
    overlays.push(RepoTopologyOverlayFact {
        id: RepoTopologyOverlayId(overlays.len() as u64),
        root: None,
        package: None,
        source_set: None,
        kind,
        label,
        path,
        stable_key: intern_module_graph_stable_key(stable_key_text),
        producer_id: "polint.module_graph",
        precision,
        status,
    });
}

fn codeowner_paths(root: &Path) -> Vec<String> {
    [".github/CODEOWNERS", "CODEOWNERS", "docs/CODEOWNERS"]
        .into_iter()
        .filter(|relative_path| root.join(relative_path).is_file())
        .map(str::to_string)
        .collect()
}

fn conventional_architecture_layer(path: &str) -> Option<&'static str> {
    match first_component(path) {
        Some("apps" | "cmd" | "services") => Some("entrypoint"),
        Some("internal") => Some("internal"),
        Some("pkg" | "packages" | "libs" | "lib") => Some("library"),
        Some("src") => Some("source"),
        Some("test" | "tests" | "__tests__") => Some("test"),
        Some("generated" | "gen") => Some("generated"),
        _ => None,
    }
}

fn conventional_deploy_unit(path: &str) -> Option<&'static str> {
    match first_component(path) {
        Some("apps" | "cmd" | "services") => Some("service"),
        _ => None,
    }
}

fn is_internal_path(path: &str) -> bool {
    path == "internal" || path.starts_with("internal/") || path.contains("/internal/")
}

fn first_component(path: &str) -> Option<&str> {
    path.split('/').find(|part| !part.is_empty())
}

fn cache_write_diagnostic(path: &str, error: anyhow::Error) -> Diagnostic {
    Diagnostic::warning(
        "internal/cache",
        path,
        TextRange::point(1, 1),
        format!("cache write failed: {error}"),
    )
}

fn setup_missing_reason_for_import(
    language: Language,
    go_metadata: &go::GoPackageIndex,
    has_go_inputs: bool,
) -> String {
    if matches!(language, Language::Go) && has_go_inputs {
        return go_metadata
            .setup_missing_reason()
            .unwrap_or("Go package metadata was not loaded.")
            .to_string();
    }
    if language.is_ts_family() {
        return "TypeScript/JavaScript resolver setup failed; check tsconfig.json or package.json."
            .to_string();
    }
    "Resolver setup is required to resolve requested module relationships.".to_string()
}

fn import_order(left: &ImportFact, right: &ImportFact, db: &AnalysisDb) -> std::cmp::Ordering {
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
}

fn sorted_files(db: &AnalysisDb) -> Vec<&SourceFile> {
    let mut files = db.files().iter().collect::<Vec<_>>();
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    files
}

fn sorted_imports(db: &AnalysisDb) -> Vec<&ImportFact> {
    let mut imports = db.imports().iter().collect::<Vec<_>>();
    imports.sort_by(|left, right| import_order(left, right, db));
    imports
}

fn normalized_file_path(file: &SourceFile) -> String {
    paths::normalize_repo_relative(&file.relative_path)
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

fn upstream_syntax_layer_key(index: usize, output_digest: Digest) -> LayerKey {
    let (layer_kind, provider_id) = match index {
        0 => (LayerKind::GoSyntax, "polint.go.syntax"),
        1 => (LayerKind::TsSyntax, "polint.ts.syntax"),
        _ => (LayerKind::Extension, "polint.unknown_upstream"),
    };
    upstream_layer_key(layer_kind, provider_id, output_digest)
}

fn upstream_layer_key(
    layer_kind: LayerKind,
    provider_id: &'static str,
    output_digest: Digest,
) -> LayerKey {
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

fn setup_missing_support(
    plan: &AnalysisPlan,
    saw_setup_missing: bool,
    setup_missing_reason: Option<&str>,
) -> Vec<CapabilitySupport> {
    if !saw_setup_missing {
        return Vec::new();
    }

    ["resolved_imports", "module_graph"]
        .into_iter()
        .filter(|capability| plan.requests_capability(capability))
        .filter_map(|capability| {
            let base = plan
                .support_view()
                .entries()
                .iter()
                .find(|entry| entry.capability == capability)?;
            Some(CapabilitySupport {
                capability: capability.to_string(),
                language: None,
                status: CapabilitySupportStatus::SetupMissing,
                rules: base.rules.clone(),
                reason: Some(
                    setup_missing_reason
                        .unwrap_or(
                            "Resolver setup is required to resolve requested module relationships.",
                        )
                        .to_string(),
                ),
                hint: Some("Check language resolver configuration such as tsconfig.json, package.json, or Go package metadata.".to_string()),
                docs_path: Some(
                    "docs/roadmap/12_RESOLVED_IMPORTS_MODULE_GRAPH_ARCHITECTURE.md".to_string(),
                ),
            })
        })
        .collect()
}

fn setup_missing_diagnostics(support: &[CapabilitySupport]) -> Vec<Diagnostic> {
    support
        .iter()
        .flat_map(|entry| {
            entry
                .rules
                .iter()
                .map(|rule_id| setup_missing_diagnostic(entry, rule_id))
        })
        .collect()
}

fn setup_missing_diagnostic(entry: &CapabilitySupport, rule_id: &str) -> Diagnostic {
    let docs_path = entry
        .docs_path
        .as_deref()
        .unwrap_or("docs/roadmap/12_RESOLVED_IMPORTS_MODULE_GRAPH_ARCHITECTURE.md");
    Diagnostic::error(
        "polint/capability",
        "<workspace>",
        TextRange::point(1, 1),
        format!(
            "Rule `{rule_id}` requested capability `{}`, but required setup is missing.",
            entry.capability
        ),
    )
    .with_evidence("rule", rule_id.to_string())
    .with_evidence("capability", entry.capability.clone())
    .with_evidence("status", "setup_missing")
    .with_help(format!(
        "Capability `{}` needs language resolver setup before this rule can run; see {docs_path}.",
        entry.capability
    ))
}

#[cfg(test)]
mod tests {
    use super::model::ModuleGraphBuilder;
    use super::{derive_requested_module_graph, paths};
    use crate::analysis_kernel::incremental::{
        CacheNode, DependencyKind, Digest, DigestKind, InputSnapshot, LayerKey, ShapeKind,
    };
    use crate::analysis_kernel::{
        FactConfidence, FactFamily, FactPrecision, FactRef, ValidationStatus,
    };
    use crate::analysis_plan::AnalysisPlan;
    use crate::config::load_config;
    use crate::core::{
        AnalysisDb, CapabilitySupport, CapabilitySupportStatus, CapabilitySupportView, ImportFact,
        ImportId, Language, ModuleEdge, ModuleEdgeId, ModuleEdgeKind, ModuleNode, ModuleNodeId,
        ModuleNodeKind, PackageFact, PackageId, ResolutionPrecision, ResolutionStatus, Span,
        UnresolvedReason,
    };
    use crate::module_graph::topology::WorkspaceRootId;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn span(file: crate::core::FileId, start_byte: u32) -> Span {
        Span::new(
            file,
            start_byte,
            start_byte + 1,
            1,
            start_byte + 1,
            1,
            start_byte + 2,
        )
    }

    type DeterministicSnapshot = (
        Vec<String>,
        Vec<(ModuleNodeId, ModuleNodeId, ModuleEdgeKind)>,
        Vec<(
            ImportId,
            Option<ModuleNodeId>,
            ResolutionStatus,
            ResolutionPrecision,
            Option<UnresolvedReason>,
        )>,
    );

    fn loaded_config() -> crate::config::LoadedConfig {
        let temp = tempfile::tempdir().expect("tempdir");
        load_config(temp.path()).expect("default config loads")
    }

    fn loaded_config_for(root: &Path) -> crate::config::LoadedConfig {
        load_config(root).expect("default config loads")
    }

    fn collect_files(root: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        collect_files_into(root, &mut files);
        files.sort();
        files
    }

    fn collect_files_into(root: &Path, files: &mut Vec<PathBuf>) {
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

    fn first_layer_file(cache_root: &Path, category: &str) -> PathBuf {
        collect_files(&cache_root.join("layers").join(category))
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("expected layer cache {category} file"))
    }

    fn module_graph_manifest() -> &'static crate::analysis_kernel::ProviderManifest {
        crate::analysis_kernel::AnalysisKernel::provider_manifests()
            .iter()
            .find(|manifest| manifest.id == "polint.module_graph")
            .expect("module graph provider manifest exists")
    }

    fn module_graph_key(db: &AnalysisDb) -> LayerKey {
        module_graph_key_for_root(Path::new("."), db)
    }

    fn module_graph_key_for_root(root: &Path, db: &AnalysisDb) -> LayerKey {
        super::module_graph_layer_key(
            root,
            db,
            module_graph_manifest(),
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
            Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
            vec![Digest::from_parts(
                DigestKind::ProviderOutput,
                "go_syntax",
                &["base"],
            )],
        )
    }

    fn module_graph_input_snapshot(
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

    fn derive_module_graph_with_cache(
        db: &mut AnalysisDb,
        loaded: &crate::config::LoadedConfig,
        cache: &crate::cache::Cache,
        plan: &AnalysisPlan,
        config_digest: &str,
    ) -> super::ModuleGraphDerivation {
        let snapshot = module_graph_input_snapshot(loaded, db, plan, config_digest);
        super::derive_requested_module_graph_with_cache_stats(
            db,
            loaded,
            plan,
            cache,
            &snapshot,
            module_graph_manifest(),
            vec![
                Digest::from_parts(DigestKind::ProviderOutput, "go_syntax", &["stable"]),
                Digest::from_parts(DigestKind::ProviderOutput, "ts_syntax", &["stable"]),
            ],
        )
    }

    fn topology_stable_keys(db: &AnalysisDb) -> Vec<Vec<String>> {
        vec![
            db.workspace_roots()
                .iter()
                .map(|row| db.resolve_stable_key(row.stable_key).to_string())
                .collect(),
            db.topology_packages()
                .iter()
                .map(|row| db.resolve_stable_key(row.stable_key).to_string())
                .collect(),
            db.source_sets()
                .iter()
                .map(|row| db.resolve_stable_key(row.stable_key).to_string())
                .collect(),
            db.dependency_requirements()
                .iter()
                .map(|row| db.resolve_stable_key(row.stable_key).to_string())
                .collect(),
            db.resolved_dependency_edges()
                .iter()
                .map(|row| db.resolve_stable_key(row.stable_key).to_string())
                .collect(),
            db.repo_topology_overlays()
                .iter()
                .map(|row| db.resolve_stable_key(row.stable_key).to_string())
                .collect(),
        ]
    }

    fn write_file(root: &Path, relative_path: &str, source: &str) -> PathBuf {
        let path = root.join(relative_path);
        fs::create_dir_all(path.parent().expect("test file has parent")).expect("mkdirs");
        fs::write(&path, source).expect("write fixture file");
        path
    }

    fn add_file(
        db: &mut AnalysisDb,
        root: &Path,
        relative_path: &str,
        source: &str,
    ) -> crate::core::FileId {
        let path = write_file(root, relative_path, source);
        db.add_file(path, relative_path.to_string(), source.to_string())
    }

    fn push_ts_import(
        db: &mut AnalysisDb,
        file: crate::core::FileId,
        path: &str,
        start_byte: u32,
    ) -> ImportId {
        db.push_import(ImportFact::new(
            ImportId::from_raw(99),
            file,
            None,
            path.to_string(),
            span(file, start_byte),
            Language::TypeScript,
        ))
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn module_graph_layer_cache_cold_warm() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cache_root = temp.path().join("cache");
        let cache = crate::cache::Cache::new(cache_root.join("analysis"), true);
        let plan = AnalysisPlan::from_capability_names_for_test(&["resolved_imports"]);
        let mut first = AnalysisDb::new();
        let app = add_file(
            &mut first,
            temp.path(),
            "src/app.ts",
            "import tokens from './tokens';\n",
        );
        add_file(
            &mut first,
            temp.path(),
            "src/tokens.ts",
            "export const tokens = {};\n",
        );
        push_ts_import(&mut first, app, "./tokens", 0);
        let loaded = loaded_config_for(temp.path());

        let first_result =
            derive_module_graph_with_cache(&mut first, &loaded, &cache, &plan, "config");

        assert!(first_result.diagnostics.is_empty());
        assert_eq!(first_result.cache_stats.misses, 1);
        assert_eq!(first_result.cache_stats.recomputes, 1);
        assert_eq!(first_result.cache_stats.writes, 1);
        assert!(first_result.output_digest.is_some());
        assert_eq!(
            first.resolved_imports()[0].status,
            ResolutionStatus::Resolved
        );

        let mut second = AnalysisDb::new();
        let app = add_file(
            &mut second,
            temp.path(),
            "src/app.ts",
            "import tokens from './tokens';\n",
        );
        add_file(
            &mut second,
            temp.path(),
            "src/tokens.ts",
            "export const tokens = {};\n",
        );
        push_ts_import(&mut second, app, "./tokens", 0);
        let second_result =
            derive_module_graph_with_cache(&mut second, &loaded, &cache, &plan, "config");

        assert!(second_result.diagnostics.is_empty());
        assert_eq!(second_result.cache_stats.hits, 1);
        assert_eq!(second_result.cache_stats.verified_reuse, 1);
        assert_eq!(second_result.cache_stats.recomputes, 0);
        assert_eq!(second_result.cache_stats.writes, 0);
        assert_eq!(second_result.output_digest, first_result.output_digest);
        assert_eq!(
            second.resolved_imports()[0].status,
            ResolutionStatus::Resolved
        );
    }

    #[test]
    fn module_graph_layer_cache_topology_cold_warm_restores_identical_stable_keys() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_file(
            temp.path(),
            "package.json",
            r#"{"name":"root","dependencies":{"react":"^18.0.0"}}"#,
        );
        write_file(
            temp.path(),
            "package-lock.json",
            r#"{"lockfileVersion":3,"packages":{"node_modules/react":{"version":"18.2.0"}}}"#,
        );
        let cache_root = temp.path().join("cache");
        let cache = crate::cache::Cache::new(cache_root.join("analysis"), true);
        let plan = AnalysisPlan::from_capability_names_for_test(&["module_graph"]);
        let mut first = AnalysisDb::new();
        let app = add_file(
            &mut first,
            temp.path(),
            "src/app.ts",
            "import React from 'react';\n",
        );
        push_ts_import(&mut first, app, "react", 0);
        let loaded = loaded_config_for(temp.path());

        let first_result =
            derive_module_graph_with_cache(&mut first, &loaded, &cache, &plan, "config");

        assert_eq!(first_result.cache_stats.misses, 1);
        assert!(!first.workspace_roots().is_empty());
        let first_keys = topology_stable_keys(&first);

        let mut second = AnalysisDb::new();
        let app = add_file(
            &mut second,
            temp.path(),
            "src/app.ts",
            "import React from 'react';\n",
        );
        push_ts_import(&mut second, app, "react", 0);
        let second_result =
            derive_module_graph_with_cache(&mut second, &loaded, &cache, &plan, "config");

        assert_eq!(second_result.cache_stats.hits, 1);
        assert_eq!(topology_stable_keys(&second), first_keys);
    }

    #[test]
    fn module_graph_layer_cache_invalidates_when_source_less_workspace_member_manifest_changes() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_file(
            temp.path(),
            "package.json",
            r#"{"name":"root","workspaces":["packages/*"]}"#,
        );
        let cache_root = temp.path().join("cache");
        let cache = crate::cache::Cache::new(cache_root.join("analysis"), true);
        let plan = AnalysisPlan::from_capability_names_for_test(&["module_graph"]);
        let loaded = loaded_config_for(temp.path());
        let mut first = AnalysisDb::new();
        add_file(&mut first, temp.path(), "src/app.ts", "export {};\n");

        let first_result =
            derive_module_graph_with_cache(&mut first, &loaded, &cache, &plan, "config");

        assert_eq!(first_result.cache_stats.misses, 1);
        assert!(
            first
                .topology_packages()
                .iter()
                .all(|package| package.path != "packages/ui")
        );

        write_file(
            temp.path(),
            "packages/ui/package.json",
            r#"{"name":"@acme/ui","version":"1.0.0"}"#,
        );
        let mut second = AnalysisDb::new();
        add_file(&mut second, temp.path(), "src/app.ts", "export {};\n");
        let second_result =
            derive_module_graph_with_cache(&mut second, &loaded, &cache, &plan, "config");

        assert_eq!(second_result.cache_stats.misses, 1);
        assert_eq!(second_result.cache_stats.recomputes, 1);
        assert!(
            second
                .topology_packages()
                .iter()
                .any(|package| { package.path == "packages/ui" && package.name == "@acme/ui" })
        );
    }

    #[test]
    fn module_graph_layer_cache_topology_payload_contains_schema_v3_and_normalized_rows() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_file(
            temp.path(),
            "package.json",
            r#"{"name":"root","dependencies":{"react":"^18.0.0"}}"#,
        );
        let cache_root = temp.path().join("cache");
        let cache = crate::cache::Cache::new(cache_root.join("analysis"), true);
        let plan = AnalysisPlan::from_capability_names_for_test(&["module_graph"]);
        let mut db = AnalysisDb::new();
        let app = add_file(
            &mut db,
            temp.path(),
            "src/app.ts",
            "import React from 'react';\n",
        );
        push_ts_import(&mut db, app, "react", 0);
        let loaded = loaded_config_for(temp.path());

        let result = derive_module_graph_with_cache(&mut db, &loaded, &cache, &plan, "config");

        assert_eq!(result.cache_stats.writes, 1);
        let payload_path = first_layer_file(&cache_root, "blobs");
        let payload_text = fs::read_to_string(payload_path).expect("payload JSON readable");
        let payload: serde_json::Value =
            serde_json::from_str(&payload_text).expect("payload is JSON");
        assert_eq!(payload["schema"], "module-graph-facts-v3");
        for field in [
            "workspace_roots",
            "topology_packages",
            "source_sets",
            "dependency_requirements",
            "resolved_dependency_edges",
            "repo_topology_overlays",
        ] {
            assert!(
                payload[field]
                    .as_array()
                    .is_some_and(|rows| !rows.is_empty()),
                "{field} should contain normalized topology rows"
            );
        }
        assert!(!payload_text.contains(temp.path().to_string_lossy().as_ref()));
        assert!(!payload_text.contains("import React from 'react'"));
    }

    #[test]
    fn module_graph_layer_cache_rejects_duplicate_topology_stable_keys() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_file(temp.path(), "package.json", r#"{"name":"root"}"#);
        let cache_root = temp.path().join("cache");
        let cache = crate::cache::Cache::new(cache_root.join("analysis"), true);
        let plan = AnalysisPlan::from_capability_names_for_test(&["module_graph"]);
        let mut first = AnalysisDb::new();
        add_file(&mut first, temp.path(), "src/app.ts", "export {};\n");
        let loaded = loaded_config_for(temp.path());

        let derivation =
            derive_module_graph_with_cache(&mut first, &loaded, &cache, &plan, "config");
        let mut payload = super::module_graph_layer_payload(&first, &derivation);
        let mut duplicate_root = payload.workspace_roots[0].clone();
        duplicate_root.id = WorkspaceRootId(99);
        duplicate_root.root_path = "conflicting-root".to_string();
        payload.workspace_roots.push(duplicate_root);
        let upstream = vec![
            Digest::from_parts(DigestKind::ProviderOutput, "go_syntax", &["stable"]),
            Digest::from_parts(DigestKind::ProviderOutput, "ts_syntax", &["stable"]),
        ];
        let snapshot = module_graph_input_snapshot(&loaded, &first, &plan, "config");
        let config_digest = snapshot.config.digest.clone();
        let go_lifecycle_digest = super::lifecycle_component_digest(
            DigestKind::GoLifecycle,
            "module_graph_go_lifecycle",
            &snapshot.go_lifecycle.components,
        );
        let ts_js_lifecycle_digest = super::lifecycle_component_digest(
            DigestKind::TsJsLifecycle,
            "module_graph_ts_js_lifecycle",
            &snapshot.ts_js_lifecycle.components,
        );
        let key = super::module_graph_layer_key(
            temp.path(),
            &first,
            module_graph_manifest(),
            config_digest.clone(),
            go_lifecycle_digest.clone(),
            ts_js_lifecycle_digest.clone(),
            upstream.clone(),
        );
        let dependencies = super::module_graph_layer_dependency_edges(
            temp.path(),
            &first,
            &key,
            module_graph_manifest(),
            &upstream,
            config_digest,
            go_lifecycle_digest,
            ts_js_lifecycle_digest,
        );
        let store = cache.layer_cache_store();
        let mut stats = crate::analysis_kernel::incremental::CacheStats::default();
        let mut diagnostics = Vec::new();
        super::write_module_graph_layer_payload(
            &store,
            key,
            &payload,
            dependencies,
            &mut stats,
            &mut diagnostics,
        )
        .expect("corrupt payload writes");

        let mut second = AnalysisDb::new();
        add_file(&mut second, temp.path(), "src/app.ts", "export {};\n");
        let second_result =
            derive_module_graph_with_cache(&mut second, &loaded, &cache, &plan, "config");

        assert_eq!(second_result.cache_stats.invalid_evicted_reads, 1);
        assert_eq!(second_result.cache_stats.recomputes, 1);
        assert_eq!(
            second
                .workspace_roots()
                .iter()
                .filter(|row| {
                    second.resolve_stable_key(row.stable_key).as_ref() == "repository:."
                })
                .count(),
            1
        );
    }

    #[test]
    fn module_graph_layer_cache_invalidates_on_import_or_lifecycle_change() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_file(temp.path(), "tsconfig.json", r#"{"compilerOptions":{}}"#);
        let cache_root = temp.path().join("cache");
        let cache = crate::cache::Cache::new(cache_root.join("analysis"), true);
        let plan = AnalysisPlan::from_capability_names_for_test(&["resolved_imports"]);
        let mut first = AnalysisDb::new();
        let app = add_file(
            &mut first,
            temp.path(),
            "src/app.ts",
            "import tokens from './tokens';\n",
        );
        add_file(
            &mut first,
            temp.path(),
            "src/tokens.ts",
            "export const tokens = {};\n",
        );
        push_ts_import(&mut first, app, "./tokens", 0);
        let loaded = loaded_config_for(temp.path());
        let first_result =
            derive_module_graph_with_cache(&mut first, &loaded, &cache, &plan, "config");

        let mut changed_import = AnalysisDb::new();
        let app = add_file(
            &mut changed_import,
            temp.path(),
            "src/app.ts",
            "import other from './other';\n",
        );
        add_file(
            &mut changed_import,
            temp.path(),
            "src/other.ts",
            "export const other = {};\n",
        );
        push_ts_import(&mut changed_import, app, "./other", 0);
        let changed_import_result =
            derive_module_graph_with_cache(&mut changed_import, &loaded, &cache, &plan, "config");

        assert_eq!(changed_import_result.cache_stats.misses, 1);
        assert_eq!(changed_import_result.cache_stats.recomputes, 1);
        assert_ne!(
            changed_import_result.output_digest,
            first_result.output_digest
        );

        write_file(
            temp.path(),
            "tsconfig.json",
            r#"{"compilerOptions":{"baseUrl":"."}}"#,
        );
        let reloaded = loaded_config_for(temp.path());
        let mut changed_lifecycle = AnalysisDb::new();
        let app = add_file(
            &mut changed_lifecycle,
            temp.path(),
            "src/app.ts",
            "import tokens from './tokens';\n",
        );
        add_file(
            &mut changed_lifecycle,
            temp.path(),
            "src/tokens.ts",
            "export const tokens = {};\n",
        );
        push_ts_import(&mut changed_lifecycle, app, "./tokens", 0);
        let changed_lifecycle_result = derive_module_graph_with_cache(
            &mut changed_lifecycle,
            &reloaded,
            &cache,
            &plan,
            "config",
        );

        assert_eq!(changed_lifecycle_result.cache_stats.misses, 1);
        assert_eq!(changed_lifecycle_result.cache_stats.recomputes, 1);
    }

    #[test]
    fn module_graph_layer_cache_corrupt_recomputes_without_crashing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cache_root = temp.path().join("cache");
        let cache = crate::cache::Cache::new(cache_root.join("analysis"), true);
        let plan = AnalysisPlan::from_capability_names_for_test(&["resolved_imports"]);
        let mut first = AnalysisDb::new();
        let app = add_file(
            &mut first,
            temp.path(),
            "src/app.ts",
            "import missing from './missing';\n",
        );
        push_ts_import(&mut first, app, "./missing", 0);
        let loaded = loaded_config_for(temp.path());
        let first_result =
            derive_module_graph_with_cache(&mut first, &loaded, &cache, &plan, "config");
        assert!(first_result.diagnostics.is_empty());

        let manifest = first_layer_file(&cache_root, "manifests");
        fs::write(manifest, "{not-json").expect("corrupt manifest");

        let mut second = AnalysisDb::new();
        let app = add_file(
            &mut second,
            temp.path(),
            "src/app.ts",
            "import missing from './missing';\n",
        );
        push_ts_import(&mut second, app, "./missing", 0);
        let second_result =
            derive_module_graph_with_cache(&mut second, &loaded, &cache, &plan, "config");

        assert!(second_result.diagnostics.is_empty());
        assert_eq!(second_result.cache_stats.invalid_evicted_reads, 1);
        assert_eq!(second_result.cache_stats.recomputes, 1);
        assert_eq!(second_result.cache_stats.writes, 1);
    }

    #[test]
    fn module_graph_layer_cache_disabled_bypasses_without_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cache_root = temp.path().join("cache");
        let cache = crate::cache::Cache::new(cache_root.join("analysis"), false);
        let plan = AnalysisPlan::from_capability_names_for_test(&["resolved_imports"]);
        let mut db = AnalysisDb::new();
        let app = add_file(
            &mut db,
            temp.path(),
            "src/app.ts",
            "import missing from './missing';\n",
        );
        push_ts_import(&mut db, app, "./missing", 0);
        let loaded = loaded_config_for(temp.path());

        let result = derive_module_graph_with_cache(&mut db, &loaded, &cache, &plan, "config");

        assert!(result.diagnostics.is_empty());
        assert_eq!(result.cache_stats.bypasses_disabled, 1);
        assert_eq!(result.cache_stats.recomputes, 1);
        assert_eq!(result.cache_stats.writes, 0);
        assert!(!cache_root.join("layers").exists());
    }

    #[test]
    fn module_graph_dependency_edges_include_inputs_schema_lifecycle_and_upstream_layers() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut db = AnalysisDb::new();
        let file = add_file(
            &mut db,
            temp.path(),
            "src/app.ts",
            "import React from 'react';\n",
        );
        push_ts_import(&mut db, file, "react", 0);
        db.push_package(PackageFact::new(
            PackageId::from_raw(99),
            file,
            "app".to_string(),
            span(file, 0),
            Language::TypeScript,
        ));
        let key = module_graph_key(&db);
        let go_syntax_output =
            Digest::from_parts(DigestKind::ProviderOutput, "go_syntax", &["base"]);
        let ts_syntax_output =
            Digest::from_parts(DigestKind::ProviderOutput, "ts_syntax", &["base"]);

        let edges = super::module_graph_layer_dependency_edges(
            temp.path(),
            &db,
            &key,
            module_graph_manifest(),
            &[go_syntax_output, ts_syntax_output],
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
            Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
        );

        let edge_kinds = edges.iter().map(|edge| edge.kind).collect::<Vec<_>>();
        assert!(edge_kinds.contains(&DependencyKind::SourceText));
        assert!(edge_kinds.contains(&DependencyKind::ImportShape));
        assert!(edge_kinds.contains(&DependencyKind::Config));
        assert!(edge_kinds.contains(&DependencyKind::Lifecycle));
        assert!(edge_kinds.contains(&DependencyKind::ProviderSchema));
        assert!(edge_kinds.contains(&DependencyKind::UpstreamLayer));
        let manifest_source =
            crate::analysis_kernel::incremental::relative_manifest_dependency_source();
        assert!(edges.iter().any(|edge| matches!(
            (&edge.from, &edge.to, edge.required_shape),
            (from, CacheNode::Input(input), ShapeKind::Import)
                if from == &manifest_source && input.contains("import:src/app.ts:react")
        )));
    }

    #[test]
    fn module_graph_layer_key_topology_inputs_dependency_edges_include_manifest_lock_workspace_and_overlay_inputs()
     {
        let temp = tempfile::tempdir().expect("tempdir");
        for (path, source) in [
            ("go.mod", "module example.com/repo\n"),
            ("go.sum", "github.com/acme/lib v1.0.0 h1:abc\n"),
            ("go.work", "go 1.22\nuse ./services/api\n"),
            ("package.json", r#"{"name":"root"}"#),
            ("package-lock.json", r#"{"lockfileVersion":3}"#),
            ("pnpm-workspace.yaml", "packages:\n  - packages/*\n"),
            ("tsconfig.json", r#"{"compilerOptions":{"baseUrl":"."}}"#),
        ] {
            write_file(temp.path(), path, source);
        }
        let mut db = AnalysisDb::new();
        add_file(&mut db, temp.path(), "src/app.ts", "export {};\n");
        let key = module_graph_key_for_root(temp.path(), &db);

        let edges = super::module_graph_layer_dependency_edges(
            temp.path(),
            &db,
            &key,
            module_graph_manifest(),
            &[],
            Digest::from_parts(DigestKind::Config, "config", &["base"]),
            Digest::from_parts(DigestKind::GoLifecycle, "go", &["base"]),
            Digest::from_parts(DigestKind::TsJsLifecycle, "ts", &["base"]),
        );

        for file_name in [
            "go.mod",
            "go.sum",
            "go.work",
            "package.json",
            "package-lock.json",
            "pnpm-workspace.yaml",
            "tsconfig.json",
        ] {
            let manifest_source =
                crate::analysis_kernel::incremental::relative_manifest_dependency_source();
            assert!(
                edges.iter().any(|edge| matches!(
                    (&edge.from, &edge.to, edge.kind, edge.required_shape),
                    (
                        from,
                        CacheNode::Input(input),
                        DependencyKind::Input,
                        ShapeKind::ModuleTopology
                    ) if from == &manifest_source && input.contains(file_name)
                )),
                "missing topology dependency edge for {file_name}"
            );
        }
    }

    #[cfg(feature = "lang-typescript")]
    fn node_label(db: &AnalysisDb, id: Option<ModuleNodeId>) -> Option<&str> {
        let id = id?;
        db.module_nodes()
            .iter()
            .find(|node| node.id == id)
            .map(|node| node.label.as_str())
    }

    #[test]
    fn module_graph_provider_skips_work_when_relationship_capabilities_are_not_requested() {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/app.ts"),
            "src/app.ts".to_string(),
            "import React from 'react';\n".to_string(),
        );
        db.push_import(ImportFact::new(
            ImportId::from_raw(99),
            file,
            None,
            "react".to_string(),
            span(file, 0),
            Language::TypeScript,
        ));

        let derivation =
            derive_requested_module_graph(&mut db, &loaded_config(), &AnalysisPlan::empty());

        assert!(derivation.diagnostics.is_empty());
        assert!(derivation.capability_support.is_empty());
        assert!(db.resolved_imports().is_empty());
        assert!(db.module_nodes().is_empty());
        assert!(db.module_edges().is_empty());
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn module_graph_derives_for_symbol_capabilities() {
        for capability in ["symbols", "references"] {
            let temp = tempfile::tempdir().expect("tempdir");
            let mut db = AnalysisDb::new();
            let app = add_file(
                &mut db,
                temp.path(),
                "src/app.ts",
                "import tokens from './tokens';\n",
            );
            add_file(
                &mut db,
                temp.path(),
                "src/tokens.ts",
                "export const tokens = {};\n",
            );
            push_ts_import(&mut db, app, "./tokens", 0);

            derive_requested_module_graph(
                &mut db,
                &loaded_config_for(temp.path()),
                &AnalysisPlan::from_capability_names_for_test(&[capability]),
            );

            assert_eq!(
                db.resolved_imports().len(),
                1,
                "{capability} should trigger module graph derivation"
            );
            assert_eq!(db.resolved_imports()[0].status, ResolutionStatus::Resolved);
        }
    }

    #[test]
    fn module_graph_derivation_support_view_overrides_base_rows() {
        let derivation = super::ModuleGraphDerivation {
            diagnostics: Vec::new(),
            capability_support: vec![CapabilitySupport {
                capability: "module_graph".to_string(),
                language: None,
                status: CapabilitySupportStatus::SetupMissing,
                rules: vec!["local/graph".to_string()],
                reason: Some("setup missing".to_string()),
                hint: None,
                docs_path: None,
            }],
            ..Default::default()
        };
        let base = CapabilitySupportView::new(vec![CapabilitySupport {
            capability: "module_graph".to_string(),
            language: None,
            status: CapabilitySupportStatus::Supported,
            rules: vec!["local/graph".to_string()],
            reason: None,
            hint: None,
            docs_path: None,
        }]);

        let support = derivation.support_view(&base);

        assert_eq!(
            support.status_for("module_graph"),
            Some(CapabilitySupportStatus::SetupMissing)
        );
    }

    #[test]
    fn module_graph_provider_seeds_file_package_and_module_contains_nodes() {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("internal/payments/service.go"),
            "internal/payments/service.go".to_string(),
            "package payments\n".to_string(),
        );
        db.push_package(PackageFact::new(
            PackageId::from_raw(99),
            file,
            "payments".to_string(),
            span(file, 0),
            Language::Go,
        ));

        derive_requested_module_graph(
            &mut db,
            &loaded_config(),
            &AnalysisPlan::from_capability_names_for_test(&["module_graph"]),
        );

        assert!(db.module_nodes().iter().any(|node| {
            node.kind == ModuleNodeKind::File && node.label == "internal/payments/service.go"
        }));
        assert!(db.module_nodes().iter().any(|node| {
            node.kind == ModuleNodeKind::Package && node.label == "go:internal/payments:payments"
        }));
        let root = db
            .module_nodes()
            .iter()
            .find(|node| node.kind == ModuleNodeKind::Module && node.label == ".")
            .map(|node| node.id)
            .expect("root module node exists");
        let package = db
            .module_nodes()
            .iter()
            .find(|node| {
                node.kind == ModuleNodeKind::Package
                    && node.label == "go:internal/payments:payments"
            })
            .map(|node| node.id)
            .expect("package node exists");
        let file_node = db
            .module_nodes()
            .iter()
            .find(|node| node.kind == ModuleNodeKind::File)
            .map(|node| node.id)
            .expect("file node exists");

        assert!(db.module_edges().iter().any(|edge| {
            edge.kind == ModuleEdgeKind::Contains && edge.from == root && edge.to == package
        }));
        assert!(db.module_edges().iter().any(|edge| {
            edge.kind == ModuleEdgeKind::Contains && edge.from == package && edge.to == file_node
        }));
    }

    #[test]
    fn module_graph_metadata_records_provider_for_replaced_facts() {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/app.ts"),
            "src/app.ts".to_string(),
            "import React from 'react';\n".to_string(),
        );
        let import = db.push_import(ImportFact::new(
            ImportId::from_raw(99),
            file,
            None,
            "react".to_string(),
            span(file, 0),
            Language::TypeScript,
        ));
        let import_key = db
            .resolve_stable_key(
                db.metadata_for(FactRef::new(FactFamily::Import, import.0))
                    .expect("import metadata should exist")
                    .stable_key,
            )
            .to_string();

        db.replace_module_graph_facts(
            vec![crate::core::ResolvedImportFact::new(
                crate::core::ResolvedImportId::from_raw(99),
                import,
                file,
                Some(ModuleNodeId::from_raw(1)),
                ResolutionStatus::Resolved,
                ResolutionPrecision::ExactFile,
                None,
            )],
            vec![
                ModuleNode::new(
                    ModuleNodeId::from_raw(99),
                    ModuleNodeKind::File,
                    "src/app.ts".to_string(),
                    Some(file),
                    None,
                    Some(Language::TypeScript),
                ),
                ModuleNode::new(
                    ModuleNodeId::from_raw(100),
                    ModuleNodeKind::External,
                    "react".to_string(),
                    None,
                    None,
                    Some(Language::TypeScript),
                ),
            ],
            vec![ModuleEdge::new(
                ModuleEdgeId::from_raw(99),
                ModuleNodeId::from_raw(0),
                ModuleNodeId::from_raw(1),
                Some(import),
                Some(crate::core::ResolvedImportId::from_raw(0)),
                ModuleEdgeKind::Imports,
                ResolutionStatus::Resolved,
            )],
        );

        let resolved = db
            .metadata_for(FactRef::new(FactFamily::ResolvedImport, 0))
            .expect("resolved import metadata should be recorded");
        let node = db
            .metadata_for(FactRef::new(FactFamily::ModuleNode, 0))
            .expect("module node metadata should be recorded");
        let edge = db
            .metadata_for(FactRef::new(FactFamily::ModuleEdge, 0))
            .expect("module edge metadata should be recorded");

        assert_eq!(resolved.producer_id, "polint.module_graph");
        assert_eq!(resolved.layer_id, "polint.module_graph");
        assert_eq!(resolved.precision, FactPrecision::SetupAware);
        assert_eq!(resolved.confidence, FactConfidence::High);
        assert_eq!(resolved.validation, ValidationStatus::NativeTrusted);
        assert!(
            db.resolve_stable_key(resolved.stable_key)
                .contains(&import_key)
        );
        assert_eq!(node.producer_id, "polint.module_graph");
        assert_eq!(edge.producer_id, "polint.module_graph");
    }

    #[test]
    fn module_graph_metadata_maps_status_precision_without_mutating_facts() {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/app.ts"),
            "src/app.ts".to_string(),
            "import missing from './missing';\n".to_string(),
        );
        let import = db.push_import(ImportFact::new(
            ImportId::from_raw(99),
            file,
            None,
            "./missing".to_string(),
            span(file, 0),
            Language::TypeScript,
        ));

        db.replace_module_graph_facts(
            vec![
                crate::core::ResolvedImportFact::new(
                    crate::core::ResolvedImportId::from_raw(99),
                    import,
                    file,
                    None,
                    ResolutionStatus::Unresolved,
                    ResolutionPrecision::None,
                    Some(UnresolvedReason::NotFound),
                ),
                crate::core::ResolvedImportFact::new(
                    crate::core::ResolvedImportId::from_raw(100),
                    import,
                    file,
                    None,
                    ResolutionStatus::SetupMissing,
                    ResolutionPrecision::None,
                    Some(UnresolvedReason::SetupMissing),
                ),
                crate::core::ResolvedImportFact::new(
                    crate::core::ResolvedImportId::from_raw(101),
                    import,
                    file,
                    None,
                    ResolutionStatus::Unsupported,
                    ResolutionPrecision::None,
                    Some(UnresolvedReason::UnsupportedImport),
                ),
                crate::core::ResolvedImportFact::new(
                    crate::core::ResolvedImportId::from_raw(102),
                    import,
                    file,
                    None,
                    ResolutionStatus::Dynamic,
                    ResolutionPrecision::None,
                    Some(UnresolvedReason::DynamicExpression),
                ),
            ],
            Vec::new(),
            Vec::new(),
        );

        let rows = db
            .resolved_imports()
            .iter()
            .enumerate()
            .map(|(index, fact)| {
                let metadata = db
                    .metadata_for(FactRef::new(FactFamily::ResolvedImport, index as u64))
                    .expect("resolved import metadata should be recorded");
                (
                    fact.status,
                    fact.precision,
                    metadata.precision,
                    metadata.confidence,
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            rows,
            vec![
                (
                    ResolutionStatus::Unresolved,
                    ResolutionPrecision::None,
                    FactPrecision::Unresolved,
                    FactConfidence::Low,
                ),
                (
                    ResolutionStatus::SetupMissing,
                    ResolutionPrecision::None,
                    FactPrecision::SetupMissing,
                    FactConfidence::Low,
                ),
                (
                    ResolutionStatus::Unsupported,
                    ResolutionPrecision::None,
                    FactPrecision::Unsupported,
                    FactConfidence::Low,
                ),
                (
                    ResolutionStatus::Dynamic,
                    ResolutionPrecision::None,
                    FactPrecision::Heuristic,
                    FactConfidence::Low,
                ),
            ]
        );
    }

    #[test]
    fn module_graph_provider_keeps_same_name_go_packages_in_different_directories_distinct() {
        let temp = tempfile::tempdir().expect("tempdir");
        let loaded = loaded_config_for(temp.path());
        let mut db = AnalysisDb::new();
        let api = add_file(&mut db, temp.path(), "cmd/api/main.go", "package main\n");
        let worker = add_file(&mut db, temp.path(), "cmd/worker/main.go", "package main\n");
        db.push_package(PackageFact::new(
            PackageId::from_raw(99),
            api,
            "main".to_string(),
            span(api, 0),
            Language::Go,
        ));
        db.push_package(PackageFact::new(
            PackageId::from_raw(99),
            worker,
            "main".to_string(),
            span(worker, 0),
            Language::Go,
        ));

        derive_requested_module_graph(
            &mut db,
            &loaded,
            &AnalysisPlan::from_capability_names_for_test(&["module_graph"]),
        );

        let api_package = db
            .module_nodes()
            .iter()
            .find(|node| node.kind == ModuleNodeKind::Package && node.label == "go:cmd/api:main")
            .map(|node| node.id)
            .expect("api package node exists");
        let worker_package = db
            .module_nodes()
            .iter()
            .find(|node| node.kind == ModuleNodeKind::Package && node.label == "go:cmd/worker:main")
            .map(|node| node.id)
            .expect("worker package node exists");

        assert_ne!(api_package, worker_package);
    }

    #[test]
    fn module_graph_builder_links_external_dependencies_from_owner_nodes() {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/app.ts"),
            "src/app.ts".to_string(),
            "import React from 'react';\n".to_string(),
        );
        let mut builder = ModuleGraphBuilder::new(&db);
        let root = builder.ensure_module_node(".");
        let file_node = builder.ensure_file_node(file);
        builder.link_module_contains(root, file_node);
        let external = builder.ensure_external_node("react", Some(Language::TypeScript));
        builder.link_dependency(root, external, None, ModuleEdgeKind::DependsOn);
        let output = builder.finish();

        assert!(
            output
                .nodes
                .iter()
                .any(|node| { node.kind == ModuleNodeKind::External && node.label == "react" })
        );
        assert!(output.edges.iter().any(|edge| {
            edge.kind == ModuleEdgeKind::DependsOn && edge.from == root && edge.to == external
        }));
    }

    #[test]
    fn module_graph_provider_emits_one_resolved_import_for_each_syntax_import() {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/app.ts"),
            "src/app.ts".to_string(),
            "import React from 'react';\nimport x from './missing';\n".to_string(),
        );
        let first = db.push_import(ImportFact::new(
            ImportId::from_raw(99),
            file,
            None,
            "react".to_string(),
            span(file, 0),
            Language::TypeScript,
        ));
        let second = db.push_import(ImportFact::new(
            ImportId::from_raw(99),
            file,
            None,
            "./missing".to_string(),
            span(file, 25),
            Language::TypeScript,
        ));

        derive_requested_module_graph(
            &mut db,
            &loaded_config(),
            &AnalysisPlan::from_capability_names_for_test(&["resolved_imports"]),
        );

        let import_ids = db
            .resolved_imports()
            .iter()
            .map(|fact| fact.import)
            .collect::<Vec<_>>();
        assert_eq!(import_ids, vec![first, second]);
    }

    #[test]
    fn module_graph_provider_orders_output_deterministically() {
        fn derive() -> DeterministicSnapshot {
            let mut db = AnalysisDb::new();
            let app = db.add_file(
                PathBuf::from("src/app.ts"),
                "src/app.ts".to_string(),
                "import React from 'react';\n".to_string(),
            );
            let util = db.add_file(
                PathBuf::from("src/util.ts"),
                "src/util.ts".to_string(),
                "export const util = 1;\n".to_string(),
            );
            db.push_import(ImportFact::new(
                ImportId::from_raw(99),
                app,
                None,
                "react".to_string(),
                span(app, 0),
                Language::TypeScript,
            ));
            db.push_import(ImportFact::new(
                ImportId::from_raw(99),
                util,
                None,
                "./missing".to_string(),
                span(util, 0),
                Language::TypeScript,
            ));

            derive_requested_module_graph(
                &mut db,
                &loaded_config(),
                &AnalysisPlan::from_capability_names_for_test(&[
                    "resolved_imports",
                    "module_graph",
                ]),
            );

            (
                db.module_nodes()
                    .iter()
                    .map(|node| format!("{:?}:{}", node.kind, node.label))
                    .collect(),
                db.module_edges()
                    .iter()
                    .map(|edge| (edge.from, edge.to, edge.kind))
                    .collect(),
                db.resolved_imports()
                    .iter()
                    .map(|fact| {
                        (
                            fact.import,
                            fact.target_node,
                            fact.status,
                            fact.precision,
                            fact.reason,
                        )
                    })
                    .collect(),
            )
        }

        assert_eq!(derive(), derive());
    }

    #[test]
    fn module_graph_paths_normalize_lexically_without_escaping_repo_root() {
        assert_eq!(
            paths::normalize_repo_relative("src/./app/../app/main.ts"),
            Some("src/app/main.ts".to_string())
        );
        assert_eq!(paths::normalize_repo_relative("../escape.ts"), None);
    }

    #[test]
    fn module_graph_query_reachable_from_uses_sorted_bfs_order() {
        let tuple_edges = vec![
            (ModuleNodeId::from_raw(0), ModuleNodeId::from_raw(2)),
            (ModuleNodeId::from_raw(0), ModuleNodeId::from_raw(1)),
            (ModuleNodeId::from_raw(1), ModuleNodeId::from_raw(3)),
            (ModuleNodeId::from_raw(2), ModuleNodeId::from_raw(3)),
        ];
        let module_edges = vec![
            ModuleEdge::new(
                ModuleEdgeId::from_raw(0),
                ModuleNodeId::from_raw(0),
                ModuleNodeId::from_raw(2),
                None,
                None,
                ModuleEdgeKind::Imports,
                ResolutionStatus::Resolved,
            ),
            ModuleEdge::new(
                ModuleEdgeId::from_raw(1),
                ModuleNodeId::from_raw(1),
                ModuleNodeId::from_raw(0),
                None,
                None,
                ModuleEdgeKind::Imports,
                ResolutionStatus::Resolved,
            ),
        ];

        assert_eq!(
            crate::analysis_neutral::module_graph::query::reachable_from(
                ModuleNodeId::from_raw(0),
                tuple_edges
            ),
            vec![
                ModuleNodeId::from_raw(1),
                ModuleNodeId::from_raw(2),
                ModuleNodeId::from_raw(3)
            ]
        );
        assert_eq!(
            crate::analysis_neutral::module_graph::query::outgoing(
                &module_edges,
                ModuleNodeId::from_raw(0)
            )
            .map(|edge| edge.to)
            .collect::<Vec<_>>(),
            vec![ModuleNodeId::from_raw(2)]
        );
        assert_eq!(
            crate::analysis_neutral::module_graph::query::incoming(
                &module_edges,
                ModuleNodeId::from_raw(0)
            )
            .map(|edge| edge.from)
            .collect::<Vec<_>>(),
            vec![ModuleNodeId::from_raw(1)]
        );
    }

    #[test]
    fn module_graph_provider_keeps_unsupported_imports_visible() {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("README.md"),
            "README.md".to_string(),
            "not source\n".to_string(),
        );
        db.push_import(ImportFact::new(
            ImportId::from_raw(99),
            file,
            None,
            "unknown".to_string(),
            span(file, 0),
            Language::Unknown,
        ));

        derive_requested_module_graph(
            &mut db,
            &loaded_config(),
            &AnalysisPlan::from_capability_names_for_test(&["resolved_imports"]),
        );

        assert_eq!(db.resolved_imports().len(), 1);
        assert_eq!(db.resolved_imports()[0].target_node, None);
        assert_eq!(
            db.resolved_imports()[0].status,
            ResolutionStatus::Unsupported
        );
        assert_eq!(
            db.resolved_imports()[0].precision,
            ResolutionPrecision::None
        );
        assert_eq!(
            db.resolved_imports()[0].reason,
            Some(UnresolvedReason::UnsupportedLanguage)
        );
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn module_graph_ts_resolution_resolves_relative_import_to_local_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut db = AnalysisDb::new();
        let component = add_file(
            &mut db,
            temp.path(),
            "src/component.ts",
            "import tokens from './tokens';\n",
        );
        add_file(
            &mut db,
            temp.path(),
            "src/tokens.ts",
            "export const tokens = {};\n",
        );
        push_ts_import(&mut db, component, "./tokens", 0);

        derive_requested_module_graph(
            &mut db,
            &loaded_config_for(temp.path()),
            &AnalysisPlan::from_capability_names_for_test(&["resolved_imports", "module_graph"]),
        );

        let fact = &db.resolved_imports()[0];
        assert_eq!(fact.status, ResolutionStatus::Resolved);
        assert_eq!(fact.precision, ResolutionPrecision::ExactFile);
        assert_eq!(fact.reason, None);
        assert_eq!(node_label(&db, fact.target_node), Some("src/tokens.ts"));
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn module_graph_ts_resolution_resolves_tsconfig_path_alias_to_local_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_file(
            temp.path(),
            "tsconfig.json",
            r#"{"compilerOptions":{"baseUrl":".","paths":{"@/*":["src/*"]}}}"#,
        );
        let mut db = AnalysisDb::new();
        let component = add_file(
            &mut db,
            temp.path(),
            "src/component.ts",
            "import tokens from '@/tokens';\n",
        );
        add_file(
            &mut db,
            temp.path(),
            "src/tokens.ts",
            "export const tokens = {};\n",
        );
        push_ts_import(&mut db, component, "@/tokens", 0);

        derive_requested_module_graph(
            &mut db,
            &loaded_config_for(temp.path()),
            &AnalysisPlan::from_capability_names_for_test(&["resolved_imports", "module_graph"]),
        );

        let fact = &db.resolved_imports()[0];
        assert_eq!(fact.status, ResolutionStatus::Resolved);
        assert_eq!(fact.precision, ResolutionPrecision::ExactFile);
        assert_eq!(node_label(&db, fact.target_node), Some("src/tokens.ts"));
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn module_graph_ts_resolution_classifies_package_imports_as_external_dependencies() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_file(
            temp.path(),
            "package.json",
            r#"{"name":"frontend","dependencies":{"react":"latest","@scope/lib":"latest"}}"#,
        );
        let mut db = AnalysisDb::new();
        let app = add_file(
            &mut db,
            temp.path(),
            "src/app.ts",
            "import React from 'react';\nconst lib = require('@scope/lib');\n",
        );
        push_ts_import(&mut db, app, "react", 0);
        push_ts_import(&mut db, app, "@scope/lib", 28);

        derive_requested_module_graph(
            &mut db,
            &loaded_config_for(temp.path()),
            &AnalysisPlan::from_capability_names_for_test(&["resolved_imports", "module_graph"]),
        );

        let facts = db.resolved_imports();
        assert!(facts.iter().all(|fact| {
            fact.status == ResolutionStatus::External
                && fact.precision == ResolutionPrecision::ExternalPackage
                && fact.reason.is_none()
        }));
        assert!(
            db.module_nodes()
                .iter()
                .any(|node| node.kind == ModuleNodeKind::External && node.label == "react")
        );
        assert!(
            db.module_nodes()
                .iter()
                .any(|node| node.kind == ModuleNodeKind::External && node.label == "@scope/lib")
        );
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn module_graph_ts_resolution_keeps_missing_relative_import_unresolved_not_found() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut db = AnalysisDb::new();
        let app = add_file(
            &mut db,
            temp.path(),
            "src/app.ts",
            "import missing from './missing';\n",
        );
        push_ts_import(&mut db, app, "./missing", 0);

        derive_requested_module_graph(
            &mut db,
            &loaded_config_for(temp.path()),
            &AnalysisPlan::from_capability_names_for_test(&["resolved_imports"]),
        );

        let fact = &db.resolved_imports()[0];
        assert_eq!(fact.status, ResolutionStatus::Unresolved);
        assert_eq!(fact.precision, ResolutionPrecision::None);
        assert_eq!(fact.reason, Some(UnresolvedReason::NotFound));
        assert_eq!(fact.target_node, None);
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn module_graph_ts_resolution_keeps_missing_matched_alias_unresolved_not_found() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_file(
            temp.path(),
            "tsconfig.json",
            r#"{"compilerOptions":{"baseUrl":".","paths":{"@domain/*":["src/domain/*"]}}}"#,
        );
        let mut db = AnalysisDb::new();
        let app = add_file(
            &mut db,
            temp.path(),
            "src/app.ts",
            "import missing from '@domain/missing';\n",
        );
        push_ts_import(&mut db, app, "@domain/missing", 0);

        derive_requested_module_graph(
            &mut db,
            &loaded_config_for(temp.path()),
            &AnalysisPlan::from_capability_names_for_test(&["resolved_imports"]),
        );

        let fact = &db.resolved_imports()[0];
        assert_eq!(fact.status, ResolutionStatus::Unresolved);
        assert_eq!(fact.precision, ResolutionPrecision::None);
        assert_eq!(fact.reason, Some(UnresolvedReason::NotFound));
        assert_eq!(fact.target_node, None);
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn module_graph_ts_resolution_keeps_missing_alias_from_commented_tsconfig_unresolved_not_found()
    {
        let temp = tempfile::tempdir().expect("tempdir");
        write_file(
            temp.path(),
            "tsconfig.json",
            r#"{
  // Repo-local aliases are often documented inline.
  "compilerOptions": {
    "baseUrl": ".",
    "paths": {"@domain/*": ["src/domain/*"]}
  }
}"#,
        );
        let mut db = AnalysisDb::new();
        let app = add_file(
            &mut db,
            temp.path(),
            "src/app.ts",
            "import missing from '@domain/missing';\n",
        );
        push_ts_import(&mut db, app, "@domain/missing", 0);

        derive_requested_module_graph(
            &mut db,
            &loaded_config_for(temp.path()),
            &AnalysisPlan::from_capability_names_for_test(&["resolved_imports"]),
        );

        let fact = &db.resolved_imports()[0];
        assert_eq!(fact.status, ResolutionStatus::Unresolved);
        assert_eq!(fact.precision, ResolutionPrecision::None);
        assert_eq!(fact.reason, Some(UnresolvedReason::NotFound));
        assert_eq!(fact.target_node, None);
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn module_graph_ts_resolution_keeps_missing_alias_from_extended_tsconfig_unresolved_not_found()
    {
        let temp = tempfile::tempdir().expect("tempdir");
        write_file(
            temp.path(),
            "tsconfig.base.json",
            r#"{"compilerOptions":{"baseUrl":".","paths":{"@domain/*":["src/domain/*"]}}}"#,
        );
        write_file(
            temp.path(),
            "tsconfig.json",
            r#"{"extends":"./tsconfig.base.json","compilerOptions":{"strict":true}}"#,
        );
        let mut db = AnalysisDb::new();
        let app = add_file(
            &mut db,
            temp.path(),
            "src/app.ts",
            "import missing from '@domain/missing';\n",
        );
        push_ts_import(&mut db, app, "@domain/missing", 0);

        derive_requested_module_graph(
            &mut db,
            &loaded_config_for(temp.path()),
            &AnalysisPlan::from_capability_names_for_test(&["resolved_imports"]),
        );

        let fact = &db.resolved_imports()[0];
        assert_eq!(fact.status, ResolutionStatus::Unresolved);
        assert_eq!(fact.precision, ResolutionPrecision::None);
        assert_eq!(fact.reason, Some(UnresolvedReason::NotFound));
        assert_eq!(fact.target_node, None);
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn module_graph_ts_setup_missing_reports_ts_reason_without_go_inputs() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_file(temp.path(), "tsconfig.json", "{ invalid json");
        let mut db = AnalysisDb::new();
        let app = add_file(
            &mut db,
            temp.path(),
            "src/app.ts",
            "import dependency from './dependency';\n",
        );
        push_ts_import(&mut db, app, "./dependency", 0);

        let derivation = derive_requested_module_graph(
            &mut db,
            &loaded_config_for(temp.path()),
            &AnalysisPlan::from_capability_names_for_test(&["resolved_imports"]),
        );

        assert_eq!(
            db.resolved_imports()[0].status,
            ResolutionStatus::SetupMissing
        );
        let reason = derivation.capability_support[0]
            .reason
            .as_deref()
            .expect("setup missing reason");
        assert!(reason.contains("TypeScript/JavaScript"), "{reason}");
        assert!(!reason.contains("Go package metadata"), "{reason}");
    }

    #[test]
    fn module_graph_ts_resolution_resolves_multiple_imports_in_one_run() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut db = AnalysisDb::new();
        let app = add_file(
            &mut db,
            temp.path(),
            "src/app.ts",
            "import one from './one';\nimport two from './two';\n",
        );
        add_file(
            &mut db,
            temp.path(),
            "src/one.ts",
            "export const one = 1;\n",
        );
        add_file(
            &mut db,
            temp.path(),
            "src/two.ts",
            "export const two = 2;\n",
        );
        push_ts_import(&mut db, app, "./one", 0);
        push_ts_import(&mut db, app, "./two", 25);
        derive_requested_module_graph(
            &mut db,
            &loaded_config_for(temp.path()),
            &AnalysisPlan::from_capability_names_for_test(&["resolved_imports", "module_graph"]),
        );
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn module_graph_ts_resolution_creates_project_module_with_contains_and_dependency_edges() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_file(
            temp.path(),
            "package.json",
            r#"{"name":"frontend","dependencies":{"react":"latest"}}"#,
        );
        write_file(
            temp.path(),
            "tsconfig.json",
            r#"{"compilerOptions":{"baseUrl":".","paths":{"@/*":["src/*"]}}}"#,
        );
        let mut db = AnalysisDb::new();
        let app = add_file(
            &mut db,
            temp.path(),
            "src/app.ts",
            "import tokens from '@/tokens';\nimport React from 'react';\n",
        );
        add_file(
            &mut db,
            temp.path(),
            "src/tokens.ts",
            "export const tokens = {};\n",
        );
        push_ts_import(&mut db, app, "@/tokens", 0);
        push_ts_import(&mut db, app, "react", 32);

        derive_requested_module_graph(
            &mut db,
            &loaded_config_for(temp.path()),
            &AnalysisPlan::from_capability_names_for_test(&["resolved_imports", "module_graph"]),
        );

        let module = db
            .module_nodes()
            .iter()
            .find(|node| node.kind == ModuleNodeKind::Module && node.label == "frontend")
            .map(|node| node.id)
            .expect("package-named module node exists");
        let app_file = db
            .module_nodes()
            .iter()
            .find(|node| node.kind == ModuleNodeKind::File && node.label == "src/app.ts")
            .map(|node| node.id)
            .expect("file node exists");
        let external = db
            .module_nodes()
            .iter()
            .find(|node| node.kind == ModuleNodeKind::External && node.label == "react")
            .map(|node| node.id)
            .expect("external node exists");

        assert!(db.module_edges().iter().any(|edge| {
            edge.kind == ModuleEdgeKind::Contains && edge.from == module && edge.to == app_file
        }));
        assert!(db.module_edges().iter().any(|edge| {
            edge.kind == ModuleEdgeKind::DependsOn && edge.from == module && edge.to == external
        }));
    }
}

#[cfg(test)]
mod base_topology {
    use super::derive_requested_module_graph;
    use crate::analysis_plan::AnalysisPlan;
    use crate::config::load_config;
    use crate::core::{AnalysisDb, ImportFact, ImportId, Language, Span};
    use crate::module_graph::topology::{RepoTopologyOverlayKind, TopologyStatus};
    use std::fs;
    use std::path::{Path, PathBuf};

    fn span(file: crate::core::FileId, start_byte: u32) -> Span {
        Span::new(
            file,
            start_byte,
            start_byte + 1,
            1,
            start_byte + 1,
            1,
            start_byte + 2,
        )
    }

    fn write_file(root: &Path, relative_path: &str, source: &str) -> PathBuf {
        let path = root.join(relative_path);
        fs::create_dir_all(path.parent().expect("test fixture path has parent")).expect("mkdirs");
        fs::write(&path, source).expect("write fixture");
        path
    }

    fn add_file(
        db: &mut AnalysisDb,
        root: &Path,
        relative_path: &str,
        source: &str,
    ) -> crate::core::FileId {
        let path = write_file(root, relative_path, source);
        db.add_file(path, relative_path.to_string(), source.to_string())
    }

    fn push_ts_import(db: &mut AnalysisDb, file: crate::core::FileId, path: &str, start_byte: u32) {
        db.push_import(ImportFact::new(
            ImportId::from_raw(99),
            file,
            None,
            path.to_string(),
            span(file, start_byte),
            Language::TypeScript,
        ));
    }

    #[test]
    fn module_graph_derivation_stores_mixed_go_ts_base_topology() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_file(
            temp.path(),
            "go.mod",
            "module example.com/repo\n\ngo 1.22\n\nrequire github.com/acme/lib v1.2.3\n",
        );
        write_file(
            temp.path(),
            "package.json",
            r#"{"name":"root","dependencies":{"react":"^18.0.0"}}"#,
        );
        write_file(
            temp.path(),
            "package-lock.json",
            r#"{"lockfileVersion":3,"packages":{"node_modules/react":{"version":"18.2.0"}}}"#,
        );
        let mut db = AnalysisDb::new();
        add_file(&mut db, temp.path(), "cmd/api/main.go", "package main\n");
        let app = add_file(
            &mut db,
            temp.path(),
            "src/app.ts",
            "import React from 'react';\n",
        );
        push_ts_import(&mut db, app, "react", 0);
        let loaded = load_config(temp.path()).expect("config loads");

        derive_requested_module_graph(
            &mut db,
            &loaded,
            &AnalysisPlan::from_capability_names_for_test(&["module_graph"]),
        );

        assert!(!db.workspace_roots().is_empty());
        assert!(!db.topology_packages().is_empty());
        assert!(!db.source_sets().is_empty());
        assert!(!db.dependency_requirements().is_empty());
        assert!(!db.resolved_dependency_edges().is_empty());
        assert!(!db.repo_topology_overlays().is_empty());
    }

    #[test]
    fn base_topology_stable_keys_are_byte_identical_across_derivations() {
        let first = derive_stable_keys_for_fixture();
        let second = derive_stable_keys_for_fixture();

        assert_eq!(first, second);
    }

    #[test]
    fn declared_dependencies_and_actual_imports_remain_separate_rows() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_file(
            temp.path(),
            "package.json",
            r#"{"name":"root","dependencies":{"react":"^18.0.0"}}"#,
        );
        let mut db = AnalysisDb::new();
        let app = add_file(
            &mut db,
            temp.path(),
            "src/app.ts",
            "import React from 'react';\n",
        );
        push_ts_import(&mut db, app, "react", 0);
        let loaded = load_config(temp.path()).expect("config loads");

        derive_requested_module_graph(
            &mut db,
            &loaded,
            &AnalysisPlan::from_capability_names_for_test(&["module_graph"]),
        );

        assert!(
            db.dependency_requirements()
                .iter()
                .any(|row| row.target_name == "react")
        );
        assert!(
            db.resolved_imports()
                .iter()
                .any(|row| row.import == ImportId::from_raw(0))
        );
        assert!(db.import_to_package_edges().is_empty());
    }

    #[test]
    fn base_topology_emits_d21_overlay_rows_or_unknowns() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_file(temp.path(), "package.json", r#"{"name":"root"}"#);
        let mut db = AnalysisDb::new();
        add_file(
            &mut db,
            temp.path(),
            "src/app.generated.ts",
            "export const app = true;\n",
        );
        add_file(
            &mut db,
            temp.path(),
            "src/app.test.ts",
            "export const app = true;\n",
        );
        let loaded = load_config(temp.path()).expect("config loads");

        derive_requested_module_graph(
            &mut db,
            &loaded,
            &AnalysisPlan::from_capability_names_for_test(&["module_graph"]),
        );

        let overlays = db.repo_topology_overlays();
        for kind in [
            RepoTopologyOverlayKind::OwnershipZone,
            RepoTopologyOverlayKind::ArchitectureLayer,
            RepoTopologyOverlayKind::DeployUnit,
            RepoTopologyOverlayKind::GeneratedZone,
            RepoTopologyOverlayKind::TestOnlyVisibility,
            RepoTopologyOverlayKind::InternalPublicApiBoundary,
            RepoTopologyOverlayKind::SourceOfTruthDirectory,
        ] {
            assert!(
                overlays.iter().any(|overlay| overlay.kind == kind),
                "missing overlay kind {kind:?}"
            );
        }
        assert!(overlays.iter().any(|overlay| {
            db.resolve_stable_key(overlay.stable_key).as_ref()
                == "repo-topology:ownership-zone:unknown"
                && overlay.status == TopologyStatus::Unknown
        }));
        assert!(overlays.iter().any(|overlay| {
            db.resolve_stable_key(overlay.stable_key).as_ref()
                == "repo-topology:architecture-layer:unknown"
                && overlay.status == TopologyStatus::Unknown
        }));
        assert!(overlays.iter().any(|overlay| {
            db.resolve_stable_key(overlay.stable_key).as_ref()
                == "repo-topology:deploy-unit:unknown"
                && overlay.status == TopologyStatus::Unknown
        }));
        assert!(overlays.iter().any(|overlay| {
            db.resolve_stable_key(overlay.stable_key).as_ref()
                == "repo-topology:internal-public-api-boundary:unknown"
                && overlay.status == TopologyStatus::Unknown
        }));
    }

    fn derive_stable_keys_for_fixture() -> Vec<Vec<String>> {
        let temp = tempfile::tempdir().expect("tempdir");
        write_file(
            temp.path(),
            "package.json",
            r#"{"name":"root","dependencies":{"react":"^18.0.0"}}"#,
        );
        write_file(
            temp.path(),
            "package-lock.json",
            r#"{"lockfileVersion":3,"packages":{"node_modules/react":{"version":"18.2.0"}}}"#,
        );
        let mut db = AnalysisDb::new();
        let app = add_file(
            &mut db,
            temp.path(),
            "src/app.ts",
            "import React from 'react';\n",
        );
        push_ts_import(&mut db, app, "react", 0);
        let loaded = load_config(temp.path()).expect("config loads");

        derive_requested_module_graph(
            &mut db,
            &loaded,
            &AnalysisPlan::from_capability_names_for_test(&["module_graph"]),
        );

        vec![
            db.workspace_roots()
                .iter()
                .map(|row| db.resolve_stable_key(row.stable_key).to_string())
                .collect(),
            db.topology_packages()
                .iter()
                .map(|row| db.resolve_stable_key(row.stable_key).to_string())
                .collect(),
            db.source_sets()
                .iter()
                .map(|row| db.resolve_stable_key(row.stable_key).to_string())
                .collect(),
            db.dependency_requirements()
                .iter()
                .map(|row| db.resolve_stable_key(row.stable_key).to_string())
                .collect(),
            db.resolved_dependency_edges()
                .iter()
                .map(|row| db.resolve_stable_key(row.stable_key).to_string())
                .collect(),
            db.repo_topology_overlays()
                .iter()
                .map(|row| db.resolve_stable_key(row.stable_key).to_string())
                .collect(),
        ]
    }
}

#[cfg(test)]
mod topology_overlay_categories {
    use crate::module_graph::topology::RepoTopologyOverlayKind;

    #[test]
    fn overlay_kind_contains_d21_categories() {
        let kinds = [
            RepoTopologyOverlayKind::OwnershipZone,
            RepoTopologyOverlayKind::ArchitectureLayer,
            RepoTopologyOverlayKind::DeployUnit,
            RepoTopologyOverlayKind::GeneratedZone,
            RepoTopologyOverlayKind::TestOnlyVisibility,
            RepoTopologyOverlayKind::InternalPublicApiBoundary,
            RepoTopologyOverlayKind::SourceOfTruthDirectory,
        ];

        assert_eq!(kinds.len(), 7);
    }
}

#[cfg(test)]
mod import_to_package {
    use super::derive_import_to_package_edges;
    use crate::core::{
        AnalysisDb, FileId, ImportFact, ImportId, Language, ModuleNode, ModuleNodeId,
        ModuleNodeKind, ResolutionPrecision, ResolutionStatus, ResolvedImportFact,
        ResolvedImportId, Span, UnresolvedReason,
    };
    use crate::module_graph::topology::{
        DependencyRequirementFact, DependencyRequirementId, ImportContextKind,
        ImportToPackageStatus, RequirementKind, SourceSetFact, SourceSetId, SourceSetKind,
        TopologyOutput, TopologyPackageFact, TopologyPackageId, TopologyPackageKind,
        TopologyPrecision, TopologyStatus, WorkspaceRootFact, WorkspaceRootId, WorkspaceRootKind,
    };
    use crate::symbol_graph::semantic::{
        SemanticImportFact, SemanticImportId, SemanticImportKind, SemanticStatus,
    };
    use std::path::PathBuf;
    use std::sync::Arc;

    fn span(file: FileId, start_byte: u32) -> Span {
        Span::new(
            file,
            start_byte,
            start_byte + 1,
            1,
            start_byte + 1,
            1,
            start_byte + 2,
        )
    }

    fn add_file(db: &mut AnalysisDb, relative_path: &str, language: Language) -> FileId {
        db.add_source_file(
            PathBuf::from(relative_path),
            relative_path.to_string(),
            language,
            Arc::from(""),
            format!("hash:{relative_path}"),
        )
    }

    fn push_import(
        db: &mut AnalysisDb,
        file: FileId,
        path: &str,
        language: Language,
        status: SemanticStatus,
        kind: SemanticImportKind,
    ) -> ImportId {
        let import = db.push_import(ImportFact::new(
            ImportId::from_raw(99),
            file,
            None,
            path.to_string(),
            span(file, 0),
            language,
        ));
        db.replace_semantic_index_facts(
            Vec::new(),
            vec![SemanticImportFact {
                id: SemanticImportId(0),
                language,
                file: Some(file),
                package: None,
                module: None,
                scope: None,
                import_path: path.to_string(),
                local_name: None,
                imported_name: None,
                namespace: crate::core::SymbolNamespace::Value,
                kind,
                stable_key: db
                    .stable_key_interner()
                    .intern(format!("semantic-import:{path}")),
                status,
            }],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        import
    }

    fn replace_graph(
        db: &mut AnalysisDb,
        import: ImportId,
        from_file: FileId,
        target_node: Option<ModuleNodeId>,
        status: ResolutionStatus,
        reason: Option<UnresolvedReason>,
        nodes: Vec<ModuleNode>,
    ) {
        db.replace_module_graph_facts(
            vec![ResolvedImportFact::new(
                ResolvedImportId::from_raw(0),
                import,
                from_file,
                target_node,
                status,
                ResolutionPrecision::ExactFile,
                reason,
            )],
            nodes,
            Vec::new(),
        );
    }

    fn replace_topology(
        db: &mut AnalysisDb,
        from_file: FileId,
        target_file: Option<FileId>,
        context: SourceSetKind,
        requirements: Vec<DependencyRequirementFact>,
        extra_packages: Vec<TopologyPackageFact>,
    ) {
        let mut packages = vec![TopologyPackageFact {
            id: TopologyPackageId(0),
            workspace_root: Some(WorkspaceRootId(0)),
            package: None,
            module_node: Some(ModuleNodeId::from_raw(0)),
            kind: TopologyPackageKind::Workspace,
            name: "app".to_string(),
            version: None,
            path: ".".to_string(),
            language: Some(Language::TypeScript),
            stable_key: crate::core::stable_key_for_test("package:app"),
            producer_id: "test",
            precision: TopologyPrecision::ExactStatic,
            status: TopologyStatus::Present,
        }];
        packages.push(TopologyPackageFact {
            id: TopologyPackageId(1),
            workspace_root: Some(WorkspaceRootId(0)),
            package: None,
            module_node: Some(ModuleNodeId::from_raw(1)),
            kind: TopologyPackageKind::Workspace,
            name: "target".to_string(),
            version: None,
            path: "src/target.ts".to_string(),
            language: Some(Language::TypeScript),
            stable_key: crate::core::stable_key_for_test("package:target"),
            producer_id: "test",
            precision: TopologyPrecision::ExactStatic,
            status: TopologyStatus::Present,
        });
        packages.extend(extra_packages);
        let mut source_files = vec![from_file];
        if let Some(target_file) = target_file {
            source_files.push(target_file);
        }
        db.replace_topology_facts(TopologyOutput {
            workspace_roots: vec![WorkspaceRootFact {
                id: WorkspaceRootId(0),
                kind: WorkspaceRootKind::Repository,
                root_path: ".".to_string(),
                manifest_path: None,
                language: None,
                stable_key: crate::core::stable_key_for_test("root:repo"),
                producer_id: "test",
                precision: TopologyPrecision::ExactStatic,
                status: TopologyStatus::Present,
            }],
            packages,
            source_sets: vec![SourceSetFact {
                id: SourceSetId(0),
                package: Some(TopologyPackageId(0)),
                root: Some(WorkspaceRootId(0)),
                kind: context,
                path: ".".to_string(),
                language: Some(Language::TypeScript),
                files: source_files,
                stable_key: crate::core::stable_key_for_test(&format!("source-set:{context:?}")),
                producer_id: "test",
                precision: TopologyPrecision::ExactStatic,
                status: TopologyStatus::Present,
            }],
            dependency_requirements: requirements,
            ..TopologyOutput::default()
        });
    }

    fn external_requirement(target: &str) -> DependencyRequirementFact {
        DependencyRequirementFact {
            id: DependencyRequirementId(0),
            from_package: Some(TopologyPackageId(0)),
            target_package: None,
            target_name: target.to_string(),
            version_requirement: Some("^1.0.0".to_string()),
            kind: RequirementKind::Runtime,
            manifest_path: Some("package.json".to_string()),
            stable_key: crate::core::stable_key_for_test(&format!("requirement:{target}")),
            producer_id: "test",
            precision: TopologyPrecision::ExactStatic,
            status: TopologyStatus::Present,
        }
    }

    #[test]
    fn static_ts_import_to_workspace_package_emits_resolved_source_edge() {
        let mut db = AnalysisDb::new();
        let from = add_file(&mut db, "src/app.ts", Language::TypeScript);
        let target = add_file(&mut db, "src/target.ts", Language::TypeScript);
        let import = push_import(
            &mut db,
            from,
            "./target",
            Language::TypeScript,
            SemanticStatus::Resolved,
            SemanticImportKind::StaticDefault,
        );
        replace_graph(
            &mut db,
            import,
            from,
            Some(ModuleNodeId::from_raw(1)),
            ResolutionStatus::Resolved,
            None,
            vec![
                ModuleNode::new(
                    ModuleNodeId::from_raw(0),
                    ModuleNodeKind::Package,
                    "app".to_string(),
                    None,
                    None,
                    Some(Language::TypeScript),
                ),
                ModuleNode::new(
                    ModuleNodeId::from_raw(1),
                    ModuleNodeKind::Package,
                    "target".to_string(),
                    Some(target),
                    None,
                    Some(Language::TypeScript),
                ),
            ],
        );
        replace_topology(
            &mut db,
            from,
            Some(target),
            SourceSetKind::Source,
            Vec::new(),
            Vec::new(),
        );

        let edges = derive_import_to_package_edges(&db);

        assert_eq!(edges.len(), 1);
        let edge = &edges[0];
        assert_eq!(edge.syntax_import, Some(import));
        assert_eq!(edge.resolved_import, Some(ResolvedImportId::from_raw(0)));
        assert_eq!(
            edge.semantic_import_stable_key
                .map(|key| db.resolve_stable_key(key))
                .as_deref(),
            Some("semantic-import:./target")
        );
        assert_eq!(
            edge.from_package_stable_key
                .map(|key| db.resolve_stable_key(key))
                .as_deref(),
            Some("package:app")
        );
        assert_eq!(
            edge.to_package_stable_key
                .map(|key| db.resolve_stable_key(key))
                .as_deref(),
            Some("package:target")
        );
        assert_eq!(
            edge.source_set_stable_key
                .map(|key| db.resolve_stable_key(key))
                .as_deref(),
            Some("source-set:Source")
        );
        assert_eq!(edge.context, ImportContextKind::Source);
        assert_eq!(edge.status, ImportToPackageStatus::Resolved);
    }

    #[test]
    fn mixed_duplicate_semantic_import_paths_are_ambiguous_not_exact() {
        let mut db = AnalysisDb::new();
        let from = add_file(&mut db, "src/app.ts", Language::TypeScript);
        let target = add_file(&mut db, "src/target.ts", Language::TypeScript);
        let first_import = db.push_import(ImportFact::new(
            ImportId::from_raw(99),
            from,
            None,
            "./target".to_string(),
            span(from, 0),
            Language::TypeScript,
        ));
        let second_import = db.push_import(ImportFact::new(
            ImportId::from_raw(99),
            from,
            None,
            "./target".to_string(),
            span(from, 10),
            Language::TypeScript,
        ));
        db.replace_semantic_index_facts(
            Vec::new(),
            vec![
                SemanticImportFact {
                    id: SemanticImportId(0),
                    language: Language::TypeScript,
                    file: Some(from),
                    package: None,
                    module: None,
                    scope: None,
                    import_path: "./target".to_string(),
                    local_name: None,
                    imported_name: None,
                    namespace: crate::core::SymbolNamespace::Value,
                    kind: SemanticImportKind::DynamicImport,
                    stable_key: db.stable_key_interner().intern("semantic-import:dynamic"),
                    status: SemanticStatus::Dynamic,
                },
                SemanticImportFact {
                    id: SemanticImportId(1),
                    language: Language::TypeScript,
                    file: Some(from),
                    package: None,
                    module: None,
                    scope: None,
                    import_path: "./target".to_string(),
                    local_name: None,
                    imported_name: None,
                    namespace: crate::core::SymbolNamespace::Value,
                    kind: SemanticImportKind::StaticDefault,
                    stable_key: db.stable_key_interner().intern("semantic-import:static"),
                    status: SemanticStatus::Resolved,
                },
            ],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        db.replace_module_graph_facts(
            vec![
                ResolvedImportFact::new(
                    ResolvedImportId::from_raw(0),
                    first_import,
                    from,
                    Some(ModuleNodeId::from_raw(1)),
                    ResolutionStatus::Resolved,
                    ResolutionPrecision::ExactFile,
                    None,
                ),
                ResolvedImportFact::new(
                    ResolvedImportId::from_raw(1),
                    second_import,
                    from,
                    Some(ModuleNodeId::from_raw(1)),
                    ResolutionStatus::Resolved,
                    ResolutionPrecision::ExactFile,
                    None,
                ),
            ],
            vec![
                ModuleNode::new(
                    ModuleNodeId::from_raw(0),
                    ModuleNodeKind::Package,
                    "app".to_string(),
                    None,
                    None,
                    Some(Language::TypeScript),
                ),
                ModuleNode::new(
                    ModuleNodeId::from_raw(1),
                    ModuleNodeKind::Package,
                    "target".to_string(),
                    Some(target),
                    None,
                    Some(Language::TypeScript),
                ),
            ],
            Vec::new(),
        );
        replace_topology(
            &mut db,
            from,
            Some(target),
            SourceSetKind::Source,
            Vec::new(),
            Vec::new(),
        );

        let edges = derive_import_to_package_edges(&db);

        assert_eq!(edges.len(), 2);
        assert!(
            edges
                .iter()
                .all(|edge| edge.status == ImportToPackageStatus::Ambiguous)
        );
        assert!(
            edges
                .iter()
                .all(|edge| edge.semantic_import_stable_key.is_none())
        );
        assert!(
            edges
                .iter()
                .all(|edge| edge.to_package_stable_key.is_none())
        );
    }

    #[test]
    fn duplicate_dynamic_import_paths_remain_dynamic_without_unique_semantic_link() {
        let mut db = AnalysisDb::new();
        let from = add_file(&mut db, "src/app.ts", Language::TypeScript);
        let target = add_file(&mut db, "src/target.ts", Language::TypeScript);
        let first_import = db.push_import(ImportFact::new(
            ImportId::from_raw(99),
            from,
            None,
            "./target".to_string(),
            span(from, 0),
            Language::TypeScript,
        ));
        let second_import = db.push_import(ImportFact::new(
            ImportId::from_raw(99),
            from,
            None,
            "./target".to_string(),
            span(from, 10),
            Language::TypeScript,
        ));
        db.replace_semantic_index_facts(
            Vec::new(),
            vec![
                SemanticImportFact {
                    id: SemanticImportId(0),
                    language: Language::TypeScript,
                    file: Some(from),
                    package: None,
                    module: None,
                    scope: None,
                    import_path: "./target".to_string(),
                    local_name: None,
                    imported_name: None,
                    namespace: crate::core::SymbolNamespace::Value,
                    kind: SemanticImportKind::DynamicImport,
                    stable_key: db.stable_key_interner().intern("semantic-import:dynamic-a"),
                    status: SemanticStatus::Dynamic,
                },
                SemanticImportFact {
                    id: SemanticImportId(1),
                    language: Language::TypeScript,
                    file: Some(from),
                    package: None,
                    module: None,
                    scope: None,
                    import_path: "./target".to_string(),
                    local_name: None,
                    imported_name: None,
                    namespace: crate::core::SymbolNamespace::Value,
                    kind: SemanticImportKind::DynamicImport,
                    stable_key: db.stable_key_interner().intern("semantic-import:dynamic-b"),
                    status: SemanticStatus::Dynamic,
                },
            ],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        db.replace_module_graph_facts(
            vec![
                ResolvedImportFact::new(
                    ResolvedImportId::from_raw(0),
                    first_import,
                    from,
                    Some(ModuleNodeId::from_raw(1)),
                    ResolutionStatus::Resolved,
                    ResolutionPrecision::ExactFile,
                    None,
                ),
                ResolvedImportFact::new(
                    ResolvedImportId::from_raw(1),
                    second_import,
                    from,
                    Some(ModuleNodeId::from_raw(1)),
                    ResolutionStatus::Resolved,
                    ResolutionPrecision::ExactFile,
                    None,
                ),
            ],
            vec![
                ModuleNode::new(
                    ModuleNodeId::from_raw(0),
                    ModuleNodeKind::Package,
                    "app".to_string(),
                    None,
                    None,
                    Some(Language::TypeScript),
                ),
                ModuleNode::new(
                    ModuleNodeId::from_raw(1),
                    ModuleNodeKind::Package,
                    "target".to_string(),
                    Some(target),
                    None,
                    Some(Language::TypeScript),
                ),
            ],
            Vec::new(),
        );
        replace_topology(
            &mut db,
            from,
            Some(target),
            SourceSetKind::Source,
            Vec::new(),
            Vec::new(),
        );

        let edges = derive_import_to_package_edges(&db);

        assert_eq!(edges.len(), 2);
        assert!(
            edges
                .iter()
                .all(|edge| edge.status == ImportToPackageStatus::Dynamic)
        );
        assert!(
            edges
                .iter()
                .all(|edge| edge.semantic_import_stable_key.is_none())
        );
        assert!(
            edges
                .iter()
                .all(|edge| edge.to_package_stable_key.is_none())
        );
    }

    #[test]
    fn go_local_package_import_resolves_by_import_path_package_label() {
        let mut db = AnalysisDb::new();
        let from = add_file(&mut db, "cmd/app/main.go", Language::Go);
        let target = add_file(&mut db, "internal/foo/foo.go", Language::Go);
        let import_path = "example.com/app/internal/foo";
        let import = push_import(
            &mut db,
            from,
            import_path,
            Language::Go,
            SemanticStatus::Resolved,
            SemanticImportKind::StaticNamed,
        );
        replace_graph(
            &mut db,
            import,
            from,
            Some(ModuleNodeId::from_raw(1)),
            ResolutionStatus::Resolved,
            None,
            vec![
                ModuleNode::new(
                    ModuleNodeId::from_raw(0),
                    ModuleNodeKind::Package,
                    "example.com/app/cmd/app".to_string(),
                    None,
                    None,
                    Some(Language::Go),
                ),
                ModuleNode::new(
                    ModuleNodeId::from_raw(1),
                    ModuleNodeKind::Package,
                    import_path.to_string(),
                    None,
                    None,
                    Some(Language::Go),
                ),
            ],
        );
        db.replace_topology_facts(TopologyOutput {
            workspace_roots: vec![WorkspaceRootFact {
                id: WorkspaceRootId(0),
                kind: WorkspaceRootKind::GoModule,
                root_path: ".".to_string(),
                manifest_path: Some("go.mod".to_string()),
                language: Some(Language::Go),
                stable_key: crate::core::stable_key_for_test("go-root:."),
                producer_id: "test",
                precision: TopologyPrecision::ExactStatic,
                status: TopologyStatus::Present,
            }],
            packages: vec![
                TopologyPackageFact {
                    id: TopologyPackageId(0),
                    workspace_root: Some(WorkspaceRootId(0)),
                    package: None,
                    module_node: None,
                    kind: TopologyPackageKind::Package,
                    name: "example.com/app/cmd/app".to_string(),
                    version: None,
                    path: "cmd/app".to_string(),
                    language: Some(Language::Go),
                    stable_key: crate::core::stable_key_for_test(
                        "go-package:example.com/app/cmd/app",
                    ),
                    producer_id: "test",
                    precision: TopologyPrecision::ExactStatic,
                    status: TopologyStatus::Present,
                },
                TopologyPackageFact {
                    id: TopologyPackageId(1),
                    workspace_root: Some(WorkspaceRootId(0)),
                    package: None,
                    module_node: None,
                    kind: TopologyPackageKind::Package,
                    name: import_path.to_string(),
                    version: None,
                    path: "internal/foo".to_string(),
                    language: Some(Language::Go),
                    stable_key: crate::core::stable_key_for_test(&format!(
                        "go-package:{import_path}"
                    )),
                    producer_id: "test",
                    precision: TopologyPrecision::ExactStatic,
                    status: TopologyStatus::Present,
                },
            ],
            source_sets: vec![
                SourceSetFact {
                    id: SourceSetId(0),
                    package: Some(TopologyPackageId(0)),
                    root: Some(WorkspaceRootId(0)),
                    kind: SourceSetKind::Source,
                    path: "cmd/app/main.go".to_string(),
                    language: Some(Language::Go),
                    files: vec![from],
                    stable_key: crate::core::stable_key_for_test(
                        "go-source-set:source:cmd/app/main.go",
                    ),
                    producer_id: "test",
                    precision: TopologyPrecision::ExactStatic,
                    status: TopologyStatus::Present,
                },
                SourceSetFact {
                    id: SourceSetId(1),
                    package: Some(TopologyPackageId(1)),
                    root: Some(WorkspaceRootId(0)),
                    kind: SourceSetKind::Source,
                    path: "internal/foo/foo.go".to_string(),
                    language: Some(Language::Go),
                    files: vec![target],
                    stable_key: crate::core::stable_key_for_test(
                        "go-source-set:source:internal/foo/foo.go",
                    ),
                    producer_id: "test",
                    precision: TopologyPrecision::ExactStatic,
                    status: TopologyStatus::Present,
                },
            ],
            ..TopologyOutput::default()
        });

        let edges = derive_import_to_package_edges(&db);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].status, ImportToPackageStatus::Resolved);
        assert_eq!(edges[0].to_package, Some(TopologyPackageId(1)));
        assert_eq!(
            edges[0]
                .to_package_stable_key
                .map(|key| db.resolve_stable_key(key))
                .as_deref(),
            Some("go-package:example.com/app/internal/foo")
        );
    }

    #[test]
    fn go_external_import_uses_module_requirement_for_local_package() {
        let mut db = AnalysisDb::new();
        let from = add_file(&mut db, "cmd/app/main.go", Language::Go);
        let import_path = "github.com/acme/lib/subpkg";
        let import = push_import(
            &mut db,
            from,
            import_path,
            Language::Go,
            SemanticStatus::External,
            SemanticImportKind::StaticNamed,
        );
        replace_graph(
            &mut db,
            import,
            from,
            Some(ModuleNodeId::from_raw(1)),
            ResolutionStatus::External,
            None,
            vec![
                ModuleNode::new(
                    ModuleNodeId::from_raw(0),
                    ModuleNodeKind::Package,
                    "example.com/app/cmd/app".to_string(),
                    None,
                    None,
                    Some(Language::Go),
                ),
                ModuleNode::new(
                    ModuleNodeId::from_raw(1),
                    ModuleNodeKind::External,
                    import_path.to_string(),
                    None,
                    None,
                    Some(Language::Go),
                ),
            ],
        );
        db.replace_topology_facts(TopologyOutput {
            workspace_roots: vec![WorkspaceRootFact {
                id: WorkspaceRootId(0),
                kind: WorkspaceRootKind::GoModule,
                root_path: ".".to_string(),
                manifest_path: Some("go.mod".to_string()),
                language: Some(Language::Go),
                stable_key: crate::core::stable_key_for_test("go-root:."),
                producer_id: "test",
                precision: TopologyPrecision::ExactStatic,
                status: TopologyStatus::Present,
            }],
            packages: vec![
                TopologyPackageFact {
                    id: TopologyPackageId(0),
                    workspace_root: Some(WorkspaceRootId(0)),
                    package: None,
                    module_node: None,
                    kind: TopologyPackageKind::Workspace,
                    name: "example.com/app".to_string(),
                    version: None,
                    path: ".".to_string(),
                    language: Some(Language::Go),
                    stable_key: crate::core::stable_key_for_test("go-module:.:example.com/app"),
                    producer_id: "test",
                    precision: TopologyPrecision::ExactStatic,
                    status: TopologyStatus::Present,
                },
                TopologyPackageFact {
                    id: TopologyPackageId(1),
                    workspace_root: Some(WorkspaceRootId(0)),
                    package: None,
                    module_node: None,
                    kind: TopologyPackageKind::Package,
                    name: "example.com/app/cmd/app".to_string(),
                    version: None,
                    path: "cmd/app".to_string(),
                    language: Some(Language::Go),
                    stable_key: crate::core::stable_key_for_test(
                        "go-package:example.com/app/cmd/app",
                    ),
                    producer_id: "test",
                    precision: TopologyPrecision::ExactStatic,
                    status: TopologyStatus::Present,
                },
            ],
            source_sets: vec![SourceSetFact {
                id: SourceSetId(0),
                package: Some(TopologyPackageId(1)),
                root: Some(WorkspaceRootId(0)),
                kind: SourceSetKind::Source,
                path: "cmd/app/main.go".to_string(),
                language: Some(Language::Go),
                files: vec![from],
                stable_key: crate::core::stable_key_for_test(
                    "go-source-set:source:cmd/app/main.go",
                ),
                producer_id: "test",
                precision: TopologyPrecision::ExactStatic,
                status: TopologyStatus::Present,
            }],
            dependency_requirements: vec![DependencyRequirementFact {
                id: DependencyRequirementId(0),
                from_package: Some(TopologyPackageId(0)),
                target_package: None,
                target_name: "github.com/acme/lib".to_string(),
                version_requirement: Some("v1.2.3".to_string()),
                kind: RequirementKind::Direct,
                manifest_path: Some("go.mod".to_string()),
                stable_key: crate::core::stable_key_for_test(
                    "go-require:go.mod:github.com/acme/lib:v1.2.3",
                ),
                producer_id: "test",
                precision: TopologyPrecision::ExactStatic,
                status: TopologyStatus::Present,
            }],
            ..TopologyOutput::default()
        });

        let edges = derive_import_to_package_edges(&db);

        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].status, ImportToPackageStatus::External);
        assert_eq!(edges[0].from_package, Some(TopologyPackageId(1)));
    }

    #[test]
    fn source_set_ownership_classifies_test_generated_and_vendor_contexts() {
        for (path, source_set, expected) in [
            (
                "pkg/service_test.go",
                SourceSetKind::Test,
                ImportContextKind::Test,
            ),
            (
                "src/app.generated.ts",
                SourceSetKind::Generated,
                ImportContextKind::Generated,
            ),
            (
                "vendor/lib/index.ts",
                SourceSetKind::Vendor,
                ImportContextKind::Vendor,
            ),
        ] {
            let mut db = AnalysisDb::new();
            let from = add_file(&mut db, path, Language::TypeScript);
            let import = push_import(
                &mut db,
                from,
                "./missing",
                Language::TypeScript,
                SemanticStatus::Unresolved,
                SemanticImportKind::StaticDefault,
            );
            replace_graph(
                &mut db,
                import,
                from,
                None,
                ResolutionStatus::Unresolved,
                Some(UnresolvedReason::NotFound),
                vec![ModuleNode::new(
                    ModuleNodeId::from_raw(0),
                    ModuleNodeKind::Package,
                    "app".to_string(),
                    None,
                    None,
                    Some(Language::TypeScript),
                )],
            );
            replace_topology(&mut db, from, None, source_set, Vec::new(), Vec::new());

            let edges = derive_import_to_package_edges(&db);

            assert_eq!(edges[0].context, expected);
        }
    }

    #[test]
    fn uncertainty_statuses_cover_dynamic_unresolved_undeclared_outside_and_ambiguous() {
        let cases = [
            (
                ResolutionStatus::Dynamic,
                SemanticStatus::Dynamic,
                None,
                None,
                Vec::new(),
                ImportToPackageStatus::Dynamic,
            ),
            (
                ResolutionStatus::Unresolved,
                SemanticStatus::Unresolved,
                None,
                Some(UnresolvedReason::NotFound),
                Vec::new(),
                ImportToPackageStatus::Unresolved,
            ),
            (
                ResolutionStatus::External,
                SemanticStatus::External,
                Some(ModuleNodeId::from_raw(1)),
                None,
                Vec::new(),
                ImportToPackageStatus::Undeclared,
            ),
            (
                ResolutionStatus::Resolved,
                SemanticStatus::Resolved,
                Some(ModuleNodeId::from_raw(1)),
                Some(UnresolvedReason::OutsideWorkspace),
                Vec::new(),
                ImportToPackageStatus::OutsideWorkspace,
            ),
            (
                ResolutionStatus::Resolved,
                SemanticStatus::Resolved,
                Some(ModuleNodeId::from_raw(1)),
                None,
                vec![TopologyPackageFact {
                    id: TopologyPackageId(2),
                    workspace_root: Some(WorkspaceRootId(0)),
                    package: None,
                    module_node: Some(ModuleNodeId::from_raw(1)),
                    kind: TopologyPackageKind::Workspace,
                    name: "second-target".to_string(),
                    version: None,
                    path: "src/target.ts".to_string(),
                    language: Some(Language::TypeScript),
                    stable_key: crate::core::stable_key_for_test("package:second-target"),
                    producer_id: "test",
                    precision: TopologyPrecision::ExactStatic,
                    status: TopologyStatus::Present,
                }],
                ImportToPackageStatus::Ambiguous,
            ),
        ];

        for (index, (resolution, semantic, target_node, reason, extra_packages, expected)) in
            cases.into_iter().enumerate()
        {
            let mut db = AnalysisDb::new();
            let from = add_file(&mut db, "src/app.ts", Language::TypeScript);
            let target = add_file(&mut db, "src/target.ts", Language::TypeScript);
            let path = if resolution == ResolutionStatus::External {
                "react"
            } else {
                "./target"
            };
            let import = push_import(
                &mut db,
                from,
                path,
                Language::TypeScript,
                semantic,
                if resolution == ResolutionStatus::Dynamic {
                    SemanticImportKind::DynamicImport
                } else {
                    SemanticImportKind::StaticDefault
                },
            );
            replace_graph(
                &mut db,
                import,
                from,
                target_node,
                resolution,
                reason,
                vec![
                    ModuleNode::new(
                        ModuleNodeId::from_raw(0),
                        ModuleNodeKind::Package,
                        "app".to_string(),
                        None,
                        None,
                        Some(Language::TypeScript),
                    ),
                    ModuleNode::new(
                        ModuleNodeId::from_raw(1),
                        if resolution == ResolutionStatus::External {
                            ModuleNodeKind::External
                        } else {
                            ModuleNodeKind::Package
                        },
                        path.to_string(),
                        Some(target),
                        None,
                        Some(Language::TypeScript),
                    ),
                ],
            );
            let requirements = if expected == ImportToPackageStatus::Undeclared {
                Vec::new()
            } else if resolution == ResolutionStatus::External {
                vec![external_requirement(path)]
            } else {
                Vec::new()
            };
            replace_topology(
                &mut db,
                from,
                Some(target),
                SourceSetKind::Source,
                requirements,
                extra_packages,
            );

            let edges = derive_import_to_package_edges(&db);

            assert_eq!(edges[0].status, expected, "case {index}");
        }
    }
}

#[cfg(test)]
mod module_topology_layer_cache {
    use super::{
        derive_module_topology_with_cache_stats, lifecycle_component_digest,
        module_topology_layer_dependency_edges, module_topology_layer_key,
        module_topology_layer_payload, write_module_topology_layer_payload,
    };
    use crate::analysis_kernel::incremental::{CacheStats, Digest, DigestKind};
    use crate::analysis_plan::AnalysisPlan;
    use crate::cache::Cache;
    use crate::config::load_config;
    use crate::core::{
        AnalysisDb, FileId, ImportFact, ImportId, Language, ModuleNode, ModuleNodeId,
        ModuleNodeKind, ResolutionPrecision, ResolutionStatus, ResolvedImportFact,
        ResolvedImportId, Span,
    };
    use crate::module_graph::topology::{
        ImportContextKind, ImportToPackageFact, ImportToPackageId, ImportToPackageStatus,
        SourceSetFact, SourceSetId, SourceSetKind, TopologyOutput, TopologyPackageFact,
        TopologyPackageId, TopologyPackageKind, TopologyPrecision, TopologyStatus,
        WorkspaceRootFact, WorkspaceRootId, WorkspaceRootKind,
    };
    use crate::symbol_graph::semantic::{
        SemanticImportFact, SemanticImportId, SemanticImportKind, SemanticStatus,
    };
    use std::path::PathBuf;
    use std::sync::Arc;

    fn module_topology_manifest() -> &'static crate::analysis_kernel::ProviderManifest {
        crate::analysis_kernel::AnalysisKernel::provider_manifests()
            .iter()
            .find(|manifest| manifest.id == "polint.module_topology")
            .expect("module topology provider manifest exists")
    }

    fn span(file: FileId) -> Span {
        Span::new(file, 0, 1, 1, 1, 1, 2)
    }

    fn add_file(db: &mut AnalysisDb, relative_path: &str) -> FileId {
        db.add_source_file(
            PathBuf::from(relative_path),
            relative_path.to_string(),
            Language::TypeScript,
            Arc::from(""),
            format!("hash:{relative_path}"),
        )
    }

    fn db_with_import_to_package_inputs() -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let from = add_file(&mut db, "src/app.ts");
        let target = add_file(&mut db, "src/target.ts");
        let import = db.push_import(ImportFact::new(
            ImportId::from_raw(99),
            from,
            None,
            "./target".to_string(),
            span(from),
            Language::TypeScript,
        ));
        db.replace_module_graph_facts(
            vec![ResolvedImportFact::new(
                ResolvedImportId::from_raw(0),
                import,
                from,
                Some(ModuleNodeId::from_raw(1)),
                ResolutionStatus::Resolved,
                ResolutionPrecision::ExactFile,
                None,
            )],
            vec![
                ModuleNode::new(
                    ModuleNodeId::from_raw(0),
                    ModuleNodeKind::Package,
                    "app".to_string(),
                    None,
                    None,
                    Some(Language::TypeScript),
                ),
                ModuleNode::new(
                    ModuleNodeId::from_raw(1),
                    ModuleNodeKind::Package,
                    "target".to_string(),
                    Some(target),
                    None,
                    Some(Language::TypeScript),
                ),
            ],
            Vec::new(),
        );
        db.replace_topology_facts(TopologyOutput {
            workspace_roots: vec![WorkspaceRootFact {
                id: WorkspaceRootId(0),
                kind: WorkspaceRootKind::Repository,
                root_path: ".".to_string(),
                manifest_path: None,
                language: None,
                stable_key: crate::core::stable_key_for_test("root:repo"),
                producer_id: "test",
                precision: TopologyPrecision::ExactStatic,
                status: TopologyStatus::Present,
            }],
            packages: vec![
                TopologyPackageFact {
                    id: TopologyPackageId(0),
                    workspace_root: Some(WorkspaceRootId(0)),
                    package: None,
                    module_node: Some(ModuleNodeId::from_raw(0)),
                    kind: TopologyPackageKind::Workspace,
                    name: "app".to_string(),
                    version: None,
                    path: ".".to_string(),
                    language: Some(Language::TypeScript),
                    stable_key: crate::core::stable_key_for_test("package:app"),
                    producer_id: "test",
                    precision: TopologyPrecision::ExactStatic,
                    status: TopologyStatus::Present,
                },
                TopologyPackageFact {
                    id: TopologyPackageId(1),
                    workspace_root: Some(WorkspaceRootId(0)),
                    package: None,
                    module_node: Some(ModuleNodeId::from_raw(1)),
                    kind: TopologyPackageKind::Workspace,
                    name: "target".to_string(),
                    version: None,
                    path: "src/target.ts".to_string(),
                    language: Some(Language::TypeScript),
                    stable_key: crate::core::stable_key_for_test("package:target"),
                    producer_id: "test",
                    precision: TopologyPrecision::ExactStatic,
                    status: TopologyStatus::Present,
                },
            ],
            source_sets: vec![SourceSetFact {
                id: SourceSetId(0),
                package: Some(TopologyPackageId(0)),
                root: Some(WorkspaceRootId(0)),
                kind: SourceSetKind::Source,
                path: ".".to_string(),
                language: Some(Language::TypeScript),
                files: vec![from, target],
                stable_key: crate::core::stable_key_for_test("source-set:source"),
                producer_id: "test",
                precision: TopologyPrecision::ExactStatic,
                status: TopologyStatus::Present,
            }],
            ..TopologyOutput::default()
        });
        db.replace_semantic_index_facts(
            Vec::new(),
            vec![SemanticImportFact {
                id: SemanticImportId(0),
                language: Language::TypeScript,
                file: Some(from),
                package: None,
                module: None,
                scope: None,
                import_path: "./target".to_string(),
                local_name: None,
                imported_name: None,
                namespace: crate::core::SymbolNamespace::Value,
                kind: SemanticImportKind::StaticDefault,
                stable_key: db.stable_key_interner().intern("semantic-import:target"),
                status: SemanticStatus::Resolved,
            }],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        db
    }

    #[test]
    fn empty_imports_clear_stale_import_to_package_edges_before_cache_return() {
        let temp = tempfile::tempdir().expect("tempdir");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new(temp.path().join("cache").join("analysis"), true);
        let plan = AnalysisPlan::from_capability_names_for_test(&["symbols", "references"]);
        let mut db = AnalysisDb::new();
        db.replace_import_to_package_facts(vec![ImportToPackageFact {
            id: ImportToPackageId(0),
            syntax_import: None,
            resolved_import: None,
            semantic_import_stable_key: None,
            from_file: None,
            from_package: None,
            to_package: None,
            target_node: None,
            from_package_stable_key: None,
            to_package_stable_key: None,
            source_set_stable_key: None,
            import_path: "stale".to_string(),
            context: ImportContextKind::Unknown,
            stable_key: crate::core::stable_key_for_test("import-to-package:stale"),
            producer_id: "polint.module_topology",
            precision: TopologyPrecision::Unknown,
            status: ImportToPackageStatus::Unresolved,
        }]);
        let snapshot = crate::analysis_kernel::incremental::input_snapshot_from_run_inputs(
            &loaded,
            &db,
            "config",
            "rules",
            plan.digest(),
            crate::analysis_kernel::AnalysisKernel::provider_manifests(),
        );

        let result = derive_module_topology_with_cache_stats(
            &mut db,
            &cache,
            &snapshot,
            module_topology_manifest(),
            Digest::from_parts(DigestKind::ProviderOutput, "module_graph", &["empty"]),
            Digest::from_parts(DigestKind::ProviderOutput, "symbol_graph", &["empty"]),
        );

        assert_eq!(db.import_to_package_edges(), &[]);
        assert!(result.output_digest.is_some());
    }

    #[test]
    fn warm_cache_restore_preserves_import_to_package_stable_keys() {
        let temp = tempfile::tempdir().expect("tempdir");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new(temp.path().join("cache").join("analysis"), true);
        let plan = AnalysisPlan::from_capability_names_for_test(&["symbols", "references"]);
        let mut first = db_with_import_to_package_inputs();
        let first_snapshot = crate::analysis_kernel::incremental::input_snapshot_from_run_inputs(
            &loaded,
            &first,
            "config",
            "rules",
            plan.digest(),
            crate::analysis_kernel::AnalysisKernel::provider_manifests(),
        );

        let first_result = derive_module_topology_with_cache_stats(
            &mut first,
            &cache,
            &first_snapshot,
            module_topology_manifest(),
            Digest::from_parts(DigestKind::ProviderOutput, "module_graph", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "symbol_graph", &["base"]),
        );
        let first_keys = first
            .import_to_package_edges()
            .iter()
            .map(|row| first.resolve_stable_key(row.stable_key).to_string())
            .collect::<Vec<_>>();

        let mut second = db_with_import_to_package_inputs();
        let second_snapshot = crate::analysis_kernel::incremental::input_snapshot_from_run_inputs(
            &loaded,
            &second,
            "config",
            "rules",
            plan.digest(),
            crate::analysis_kernel::AnalysisKernel::provider_manifests(),
        );
        let second_result = derive_module_topology_with_cache_stats(
            &mut second,
            &cache,
            &second_snapshot,
            module_topology_manifest(),
            Digest::from_parts(DigestKind::ProviderOutput, "module_graph", &["base"]),
            Digest::from_parts(DigestKind::ProviderOutput, "symbol_graph", &["base"]),
        );

        assert_eq!(first_result.cache_stats.misses, 1);
        assert_eq!(first_result.cache_stats.writes, 1);
        assert_eq!(second_result.cache_stats.hits, 1);
        assert_eq!(
            second
                .import_to_package_edges()
                .iter()
                .map(|row| second.resolve_stable_key(row.stable_key).to_string())
                .collect::<Vec<_>>(),
            first_keys
        );
    }

    #[test]
    fn module_topology_layer_cache_rejects_duplicate_stable_keys() {
        let temp = tempfile::tempdir().expect("tempdir");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new(temp.path().join("cache").join("analysis"), true);
        let plan = AnalysisPlan::from_capability_names_for_test(&["symbols", "references"]);
        let mut first = db_with_import_to_package_inputs();
        let snapshot = crate::analysis_kernel::incremental::input_snapshot_from_run_inputs(
            &loaded,
            &first,
            "config",
            "rules",
            plan.digest(),
            crate::analysis_kernel::AnalysisKernel::provider_manifests(),
        );
        let module_digest =
            Digest::from_parts(DigestKind::ProviderOutput, "module_graph", &["base"]);
        let symbol_digest =
            Digest::from_parts(DigestKind::ProviderOutput, "symbol_graph", &["base"]);
        let derivation = derive_module_topology_with_cache_stats(
            &mut first,
            &cache,
            &snapshot,
            module_topology_manifest(),
            module_digest.clone(),
            symbol_digest.clone(),
        );
        let mut payload = module_topology_layer_payload(&first, &derivation);
        let mut duplicate = payload.import_to_package_edges[0].clone();
        duplicate.id = crate::module_graph::topology::ImportToPackageId(99);
        duplicate.import_path = "conflicting".to_string();
        payload.import_to_package_edges.push(duplicate);

        let config_digest = snapshot.config.digest.clone();
        let go_lifecycle_digest = lifecycle_component_digest(
            DigestKind::GoLifecycle,
            "module_topology_go_lifecycle",
            &snapshot.go_lifecycle.components,
        );
        let ts_js_lifecycle_digest = lifecycle_component_digest(
            DigestKind::TsJsLifecycle,
            "module_topology_ts_js_lifecycle",
            &snapshot.ts_js_lifecycle.components,
        );
        let key = module_topology_layer_key(
            &first,
            module_topology_manifest(),
            config_digest.clone(),
            go_lifecycle_digest.clone(),
            ts_js_lifecycle_digest.clone(),
            module_digest.clone(),
            symbol_digest.clone(),
        );
        let dependencies = module_topology_layer_dependency_edges(
            &first,
            &key,
            module_topology_manifest(),
            module_digest.clone(),
            symbol_digest.clone(),
            config_digest,
            go_lifecycle_digest,
            ts_js_lifecycle_digest,
        );
        let store = crate::analysis_kernel::incremental::LayerCacheStore::new(
            cache.layer_cache_dir(),
            true,
        );
        let mut stats = CacheStats::default();
        let mut diagnostics = Vec::new();
        write_module_topology_layer_payload(
            &store,
            key,
            &payload,
            dependencies,
            &mut stats,
            &mut diagnostics,
        )
        .expect("corrupt module topology payload writes");

        let mut second = db_with_import_to_package_inputs();
        let second_snapshot = crate::analysis_kernel::incremental::input_snapshot_from_run_inputs(
            &loaded,
            &second,
            "config",
            "rules",
            plan.digest(),
            crate::analysis_kernel::AnalysisKernel::provider_manifests(),
        );
        let second_result = derive_module_topology_with_cache_stats(
            &mut second,
            &cache,
            &second_snapshot,
            module_topology_manifest(),
            module_digest,
            symbol_digest,
        );

        assert_eq!(second_result.cache_stats.invalid_evicted_reads, 1);
        assert_eq!(second_result.cache_stats.recomputes, 1);
        assert_eq!(second.import_to_package_edges().len(), 1);
    }
}
