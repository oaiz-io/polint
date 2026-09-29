use crate::analysis_api::ProviderExecution;
use crate::analysis_kernel::ProviderManifest;
use crate::analysis_kernel::incremental::{
    CacheNode, CacheStats, DependencyEdge, DependencyKind, Digest, DigestKind, LayerCacheManifest,
    LayerCacheReadStatus, LayerCacheStore, LayerCacheWriteStatus, LayerKey, PrecisionTier,
    ShapeKind,
};
use crate::analysis_kernel::metrics_projection::{
    CanonicalMetricsContext, CanonicalMetricsInputs, CanonicalMetricsOutput, MetricsProjectionError,
};
use crate::analysis_neutral::metrics::{
    METRIC_CAPABILITIES, METRICS_LAYER_SCHEMA, MetricsLayerPayload,
};
use crate::analysis_plan::AnalysisPlan;
use crate::cache::Cache;
use crate::core::AnalysisDb;
use crate::diagnostics::{Diagnostic, TextRange};

#[derive(Debug, Clone, Default)]
pub(crate) struct MetricsDerivation {
    pub(crate) cache_stats: CacheStats,
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) output_digest: Option<Digest>,
    pub(crate) execution: ProviderExecution,
}

#[cfg(test)]
pub(crate) fn derive_requested_metrics(db: &mut AnalysisDb, plan: &AnalysisPlan) {
    if !plan.requests_any_capability(METRIC_CAPABILITIES) {
        return;
    }
    let context = CanonicalMetricsContext::from_db(db).expect("valid canonical metrics context");
    let _ = derive_requested_metrics_uncached(db, plan, &context);
}

pub(crate) fn derive_requested_metrics_with_cache_stats(
    db: &mut AnalysisDb,
    plan: &AnalysisPlan,
    cache: &Cache,
    manifest: &ProviderManifest,
) -> Result<MetricsDerivation, MetricsProjectionError> {
    if !plan.requests_any_capability(METRIC_CAPABILITIES) {
        return Ok(MetricsDerivation::default());
    }

    let context = CanonicalMetricsContext::from_db(db)?;
    let inputs = context.inputs();
    let layer_key = metrics_layer_key(inputs, manifest);
    let store = cache.layer_cache_store();
    let mut cache_stats = CacheStats::default();
    let read = store
        .read_json_validated::<MetricsLayerPayload, _>(&layer_key, |payload, manifest| {
            validate_metrics_layer_payload(&context, payload, manifest)
        });

    Ok(match read.status {
        LayerCacheReadStatus::Hit => {
            cache_stats.record_hit();
            cache_stats.record_verified_reuse();
            let payload = read
                .value
                .expect("layer cache hit should include metrics payload");
            restore_metrics_layer_payload(db, &payload);
            MetricsDerivation {
                cache_stats,
                diagnostics: Vec::new(),
                output_digest: read.output_digest,
                execution: ProviderExecution::Succeeded,
            }
        }
        LayerCacheReadStatus::BypassedDisabled => {
            cache_stats.record_disabled_bypass();
            cache_stats.record_recompute();
            let mut derivation = derive_requested_metrics_uncached(db, plan, &context)?;
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
            let mut derivation = derive_requested_metrics_uncached(db, plan, &context)?;
            let payload = metrics_layer_payload(db);
            let dependencies = metrics_layer_dependency_edges(inputs, &layer_key);
            let output_digest = derivation
                .output_digest
                .clone()
                .ok_or(MetricsProjectionError::Output)?;
            write_metrics_layer_payload(
                &store,
                layer_key,
                &payload,
                dependencies,
                output_digest,
                &mut cache_stats,
                &mut derivation.diagnostics,
            );
            derivation.cache_stats = cache_stats;
            derivation
        }
    })
}

pub(crate) fn metrics_layer_key(
    inputs: &CanonicalMetricsInputs,
    manifest: &ProviderManifest,
) -> LayerKey {
    LayerKey::metrics_layer_key(
        manifest,
        inputs.source_digests(),
        inputs.function_digests(),
        metrics_parameter_digest(),
    )
}

fn derive_requested_metrics_uncached(
    db: &mut AnalysisDb,
    plan: &AnalysisPlan,
    context: &CanonicalMetricsContext,
) -> Result<MetricsDerivation, MetricsProjectionError> {
    let requested = plan.requests_any_capability(METRIC_CAPABILITIES);
    if !requested {
        return Ok(MetricsDerivation::default());
    }

    if let Some(output) = crate::analysis_neutral::metrics::derive_requested_metrics(
        db,
        requested,
        context.file_summaries(),
    ) {
        db.replace_metric_facts(
            output.file_metrics,
            output.function_metrics,
            output.complexity_metrics,
        );
    }
    Ok(MetricsDerivation {
        cache_stats: CacheStats::default(),
        diagnostics: Vec::new(),
        output_digest: Some(CanonicalMetricsOutput::from_db_with_context(context, db)?.digest()),
        execution: Default::default(),
    })
}
fn metrics_layer_dependency_edges(
    inputs: &CanonicalMetricsInputs,
    key: &LayerKey,
) -> Vec<DependencyEdge> {
    let from = CacheNode::Layer(key.clone());
    let mut edges = Vec::new();

    for (ordinal, digest) in inputs.source_digests().into_iter().enumerate() {
        edges.push(dependency_edge(
            &from,
            CacheNode::Input(format!("metrics-source:{ordinal}:{digest}")),
            DependencyKind::SourceText,
            ShapeKind::Content,
        ));
    }

    // One edge for the whole function set, not one per function. Metrics are
    // recomputed wholesale whenever any function changes, so a per-function edge
    // carries no invalidation signal the folded digest does not, while making the
    // manifest O(functions) — large enough on a real repo to blow past the
    // manifest read ceiling and make the layer miss forever.
    edges.push(dependency_edge(
        &from,
        CacheNode::Input(format!(
            "metrics-functions:{}",
            combined_function_digest(inputs)
        )),
        DependencyKind::Input,
        ShapeKind::Syntax,
    ));

    edges.sort();
    edges
}

/// Fold every function digest into one. Values are sorted so the fold depends on
/// the set of functions and not on the order the projection happened to list them.
fn combined_function_digest(inputs: &CanonicalMetricsInputs) -> Digest {
    let mut values = inputs
        .function_digests()
        .iter()
        .map(|digest| digest.value.clone())
        .collect::<Vec<_>>();
    values.sort();
    let parts = values.iter().map(String::as_str).collect::<Vec<_>>();
    Digest::from_parts(
        DigestKind::ProviderParameters,
        "metrics_function_facts_combined",
        &parts,
    )
}

fn metrics_parameter_digest() -> Digest {
    crate::analysis_neutral::metrics::metrics_parameter_digest()
}

fn metrics_layer_payload(db: &AnalysisDb) -> MetricsLayerPayload {
    crate::analysis_neutral::metrics::metrics_layer_payload(db)
}

fn restore_metrics_layer_payload(db: &mut AnalysisDb, payload: &MetricsLayerPayload) {
    let output = crate::analysis_neutral::metrics::restore_metrics_layer_payload(payload);
    db.replace_metric_facts(
        output.file_metrics,
        output.function_metrics,
        output.complexity_metrics,
    );
}

fn validate_metrics_layer_payload(
    context: &CanonicalMetricsContext,
    payload: &MetricsLayerPayload,
    manifest: &LayerCacheManifest,
) -> bool {
    payload.schema == METRICS_LAYER_SCHEMA
        && CanonicalMetricsOutput::from_metric_facts_with_context(
            context,
            &payload.file_metrics,
            &payload.function_metrics,
            &payload.complexity_metrics,
        )
        .is_ok_and(|output| manifest.output_digest == output.digest())
}

fn write_metrics_layer_payload(
    store: &LayerCacheStore,
    layer_key: LayerKey,
    payload: &MetricsLayerPayload,
    dependencies: Vec<DependencyEdge>,
    output_digest: Digest,
    stats: &mut CacheStats,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let payload_digest = match LayerCacheStore::payload_digest_for_json(payload) {
        Ok(digest) => digest,
        Err(error) => {
            diagnostics.push(cache_write_diagnostic("metrics layer", error));
            return;
        }
    };
    let manifest = LayerCacheManifest::new(
        layer_key,
        output_digest,
        payload_digest,
        dependencies,
        PrecisionTier::Syntax,
        "native_trusted",
        Vec::new(),
    );

    match store.write_json(&manifest, payload) {
        Ok(LayerCacheWriteStatus::Written) => stats.record_write(),
        Ok(LayerCacheWriteStatus::BypassedDisabled) => stats.record_disabled_bypass(),
        Err(error) => diagnostics.push(cache_write_diagnostic("metrics layer", error)),
    }
}

fn cache_write_diagnostic(path: &str, error: anyhow::Error) -> Diagnostic {
    Diagnostic::warning(
        "internal/cache",
        path,
        TextRange::point(1, 1),
        format!("cache write failed: {error}"),
    )
}

fn dependency_edge(
    from: &CacheNode,
    to: CacheNode,
    kind: DependencyKind,
    required_shape: ShapeKind,
) -> DependencyEdge {
    DependencyEdge {
        from: from.clone(),
        to,
        kind,
        required_shape,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_kernel::{
        FactConfidence, FactFamily, FactPrecision, FactRef, ValidationStatus,
    };
    use crate::analysis_plan::AnalysisPlan;
    use crate::cache::Cache;
    use crate::config::load_config;
    use crate::core::{
        FileId, FunctionFact, FunctionId, ImportFact, ImportId, Language, PackageFact, PackageId,
        Span, StringLiteralFact, TS_JS_MODULE_FUNCTION_NAME,
    };
    use std::fs;
    use std::path::{Path, PathBuf};

    fn metrics_manifest() -> &'static crate::analysis_kernel::ProviderManifest {
        crate::analysis_kernel::AnalysisKernel::provider_manifests()
            .iter()
            .find(|manifest| manifest.id == "polint.metrics")
            .expect("metrics provider manifest exists")
    }

    fn requested_metrics_plan() -> AnalysisPlan {
        AnalysisPlan::from_capability_names_for_test(&[
            "file_metrics",
            "function_metrics",
            "complexity_metrics",
        ])
    }

    fn derive_metrics_with_cache(
        db: &mut AnalysisDb,
        loaded: &crate::config::LoadedConfig,
        cache: &Cache,
        plan: &AnalysisPlan,
        config_digest: &str,
        upstream_label: &str,
    ) -> MetricsDerivation {
        let _excluded_inputs = (loaded, config_digest, upstream_label);
        derive_requested_metrics_with_cache_stats(db, plan, cache, metrics_manifest())
            .expect("canonical metrics derivation")
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

    fn fixture_db(root: &Path, function_name: &str, source: &str) -> AnalysisDb {
        let mut db = AnalysisDb::new();
        let path = root.join("src/app.ts");
        fs::create_dir_all(path.parent().expect("test file has parent")).expect("mkdirs");
        fs::write(&path, source).expect("write fixture file");
        let file = db.add_file(path, "src/app.ts".to_string(), source.to_string());
        push_function(&mut db, file, function_name, source);
        db
    }

    fn push_function(
        db: &mut AnalysisDb,
        file: FileId,
        function_name: &str,
        source: &str,
    ) -> FunctionId {
        let start = source.find(function_name).unwrap_or(0);
        db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            function_name.to_string(),
            Span::new(
                file,
                start as u32,
                source.len() as u32,
                1,
                (start + 1) as u32,
                source.lines().count() as u32,
                1,
            ),
            Language::TypeScript,
            false,
            true,
            2,
            Vec::new(),
        ))
    }

    fn metrics_inputs_for(sources: usize, functions_per_source: usize) -> CanonicalMetricsInputs {
        let mut db = AnalysisDb::new();
        for source_index in 0..sources {
            let mut text = String::new();
            for function_index in 0..functions_per_source {
                text.push_str(&format!("export function f{function_index}() {{}}\n"));
            }
            let relative_path = format!("src/module{source_index}.ts");
            let file = db.add_file(
                PathBuf::from(&relative_path),
                relative_path.clone(),
                text.clone(),
            );
            for function_index in 0..functions_per_source {
                let line = u32::try_from(function_index + 1).expect("line fits in u32");
                db.push_function(FunctionFact::new(
                    FunctionId::from_raw(0),
                    file,
                    format!("f{function_index}"),
                    Span::new(file, 0, 0, line, 1, line, 2),
                    Language::TypeScript,
                    false,
                    true,
                    1,
                    Vec::new(),
                ));
            }
        }
        CanonicalMetricsInputs::from_db(&db).expect("canonical metrics inputs")
    }

    /// The manifest carries one edge per source plus one for the whole function
    /// set. Fanning out per function made it grow past the read ceiling on real
    /// repos, which turned every subsequent read into a miss.
    #[test]
    fn metrics_dependency_edges_are_counted_by_source_not_by_function() {
        let few = metrics_inputs_for(3, 1);
        let many = metrics_inputs_for(3, 400);

        for (inputs, label) in [(&few, "few"), (&many, "many")] {
            let edges = metrics_layer_dependency_edges(
                inputs,
                &metrics_layer_key(inputs, metrics_manifest()),
            );
            assert_eq!(
                edges.len(),
                inputs.source_digests().len() + 1,
                "{label} functions produced {} edges for {} sources",
                edges.len(),
                inputs.source_digests().len()
            );
        }

        // Collapsing the fan-out must not collapse the invalidation signal.
        assert_ne!(
            combined_function_digest(&few),
            combined_function_digest(&many)
        );
    }

    #[test]
    fn derive_requested_metrics_skips_when_plan_does_not_request_metrics() {
        let mut db = AnalysisDb::new();
        db.add_file(
            PathBuf::from("src/app.ts"),
            "src/app.ts".to_string(),
            "x\n".to_string(),
        );

        derive_requested_metrics(&mut db, &AnalysisPlan::empty());

        assert!(db.file_metrics().is_empty());
        assert!(db.function_metrics().is_empty());
        assert!(db.complexity_metrics().is_empty());
    }

    #[test]
    fn derive_requested_metrics_populates_shared_file_function_and_complexity_facts() {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/app.ts"),
            "src/app.ts".to_string(),
            "export function handler() {\n  if (ok) return 1;\n  return 0;\n}\n".to_string(),
        );
        let span = Span::new(file, 0, 62, 1, 1, 4, 2);
        db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            TS_JS_MODULE_FUNCTION_NAME.to_string(),
            Span::new(file, 0, 63, 1, 1, 5, 1),
            Language::TypeScript,
            false,
            false,
            1,
            Vec::new(),
        ));
        let function = db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "handler".to_string(),
            span,
            Language::TypeScript,
            false,
            true,
            2,
            Vec::new(),
        ));
        let plan = AnalysisPlan::from_capability_names_for_test(&["file_metrics"]);

        derive_requested_metrics(&mut db, &plan);

        assert_eq!(db.file_metrics().len(), 1);
        assert_eq!(db.file_metrics()[0].line_count, 4);
        assert_eq!(db.file_metrics()[0].non_empty_line_count, 4);
        assert_eq!(db.file_metrics()[0].function_count, 1);
        assert_eq!(db.function_metrics().len(), 1);
        assert_eq!(db.function_metrics()[0].function, function);
        assert_eq!(db.function_metrics()[0].line_count, 4);
        assert_eq!(db.function_metrics()[0].byte_count, 62);
        assert_eq!(db.complexity_metrics().len(), 1);
        assert_eq!(db.complexity_metrics()[0].function, function);
        assert_eq!(db.complexity_metrics()[0].cyclomatic_complexity, 2);
        assert_eq!(
            CanonicalMetricsInputs::from_db(&db)
                .unwrap()
                .functions
                .len(),
            1
        );
    }

    #[test]
    fn metrics_metadata_is_recorded_only_when_metrics_are_requested() {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/app.ts"),
            "src/app.ts".to_string(),
            "export function handler() { return 1; }\n".to_string(),
        );
        let span = Span::new(file, 0, 37, 1, 1, 1, 38);
        db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "handler".to_string(),
            span,
            Language::TypeScript,
            false,
            true,
            1,
            Vec::new(),
        ));

        derive_requested_metrics(&mut db, &AnalysisPlan::empty());

        assert!(
            db.metadata_for(FactRef::new(FactFamily::FileMetric, 0))
                .is_none()
        );
        assert!(
            db.metadata_for(FactRef::new(FactFamily::FunctionMetric, 0))
                .is_none()
        );
        assert!(
            db.metadata_for(FactRef::new(FactFamily::ComplexityMetric, 0))
                .is_none()
        );

        derive_requested_metrics(
            &mut db,
            &AnalysisPlan::from_capability_names_for_test(&["complexity_metrics"]),
        );

        assert!(
            db.metadata_for(FactRef::new(FactFamily::FileMetric, 0))
                .is_some()
        );
        assert!(
            db.metadata_for(FactRef::new(FactFamily::FunctionMetric, 0))
                .is_some()
        );
        assert!(
            db.metadata_for(FactRef::new(FactFamily::ComplexityMetric, 0))
                .is_some()
        );
    }

    #[test]
    fn metrics_metadata_uses_provider_defaults_and_source_stable_keys() {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            PathBuf::from("src/app.ts"),
            "src/app.ts".to_string(),
            "export function handler() {\n  return 1;\n}\n".to_string(),
        );
        let span = Span::new(file, 0, 40, 1, 1, 3, 2);
        let function = db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "handler".to_string(),
            span,
            Language::TypeScript,
            false,
            true,
            1,
            Vec::new(),
        ));
        let file_key = db
            .resolve_stable_key(
                db.metadata_for(FactRef::new(FactFamily::SourceFile, u64::from(file.0)))
                    .expect("source metadata should exist")
                    .stable_key,
            )
            .to_string();
        let function_key = db
            .resolve_stable_key(
                db.metadata_for(FactRef::new(FactFamily::Function, function.0))
                    .expect("function metadata should exist")
                    .stable_key,
            )
            .to_string();

        derive_requested_metrics(
            &mut db,
            &AnalysisPlan::from_capability_names_for_test(&["function_metrics"]),
        );

        let file_metric = db
            .metadata_for(FactRef::new(FactFamily::FileMetric, 0))
            .expect("file metric metadata should be recorded");
        let function_metric = db
            .metadata_for(FactRef::new(FactFamily::FunctionMetric, 0))
            .expect("function metric metadata should be recorded");
        let complexity_metric = db
            .metadata_for(FactRef::new(FactFamily::ComplexityMetric, 0))
            .expect("complexity metric metadata should be recorded");

        assert_eq!(file_metric.producer_id, "polint.metrics");
        assert_eq!(file_metric.layer_id, "polint.metrics");
        assert_eq!(file_metric.precision, FactPrecision::Syntax);
        assert_eq!(file_metric.confidence, FactConfidence::High);
        assert_eq!(file_metric.validation, ValidationStatus::NativeTrusted);
        assert!(
            db.resolve_stable_key(file_metric.stable_key)
                .contains(&file_key)
        );
        assert!(
            db.resolve_stable_key(function_metric.stable_key)
                .contains(&function_key)
        );
        assert!(
            db.resolve_stable_key(function_metric.stable_key)
                .contains("metric_name")
        );
        assert!(
            db.resolve_stable_key(function_metric.stable_key)
                .contains("function_size")
        );
        assert!(
            db.resolve_stable_key(complexity_metric.stable_key)
                .contains(&function_key)
        );
        assert!(
            db.resolve_stable_key(complexity_metric.stable_key)
                .contains("metric_name")
        );
        assert!(
            db.resolve_stable_key(complexity_metric.stable_key)
                .contains("cyclomatic_complexity")
        );
    }

    /// A database with a stable-key interner of its own. Test databases share a
    /// process-wide one, which hands every database the same id for the same
    /// key whatever order each interned in.
    fn isolated_db() -> AnalysisDb {
        let mut db = AnalysisDb::new();
        db.stable_keys = crate::core::StableKeyInterner::default();
        db
    }

    fn metadata_rows(db: &AnalysisDb) -> Vec<(FactRef, String, crate::analysis_kernel::FactMeta)> {
        db.fact_meta()
            .rows()
            .map(|(reference, metadata)| {
                let key = db.resolve_stable_key(metadata.stable_key).to_string();
                (reference, key, metadata.clone())
            })
            .collect()
    }

    /// Loads a Go and a TypeScript file, restores their syntax facts, and runs
    /// the metrics provider over them: the kernel's order.
    fn restore_then_derive_metrics(root: &Path, cache: &Cache, defer: bool) -> AnalysisDb {
        // (path, source, function, the function's first and last line, import)
        let sources = [
            (
                "main.go",
                "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tif true {\n\t\tfmt.Println()\n\t}\n}\n",
                "main",
                (5, 9),
                "fmt",
            ),
            (
                "src/app.ts",
                "import { x } from './x';\nexport function handler() {\n  if (x) return 1;\n  return 0;\n}\n",
                "handler",
                (2, 5),
                "./x",
            ),
        ];
        let mut db = isolated_db();
        let files = sources.map(|(path, source, ..)| {
            db.add_file(root.join(path), path.to_string(), source.to_string())
        });
        if defer {
            db.defer_syntax_fact_metadata();
        }
        for (file, (path, source, name, (first, last), import)) in files.into_iter().zip(sources) {
            let language = Language::from_path(Path::new(path));
            let start = source.find(&format!("{name}(")).expect("function name") as u32;
            let end = source.len() as u32 - 1;
            db.restore_file_facts(
                file,
                crate::analysis_api::CachedFileFacts {
                    packages: (language == Language::Go)
                        .then(|| {
                            PackageFact::new(
                                PackageId::from_raw(0),
                                file,
                                "main".to_string(),
                                Span::new(file, 0, 12, 1, 1, 1, 13),
                                language,
                            )
                        })
                        .into_iter()
                        .collect(),
                    functions: vec![FunctionFact::new(
                        FunctionId::from_raw(7),
                        file,
                        name.to_string(),
                        Span::new(file, start, end, first, 1, last, 2),
                        language,
                        false,
                        true,
                        2,
                        vec!["Println".to_string()],
                    )],
                    imports: vec![ImportFact::new(
                        ImportId::from_raw(0),
                        file,
                        None,
                        import.to_string(),
                        Span::new(file, 0, 10, 1, 1, 1, 11),
                        language,
                    )],
                    string_literals: vec![StringLiteralFact::new(
                        file,
                        import.to_string(),
                        Span::new(file, 1, 4, 1, 2, 1, 5),
                        language,
                    )],
                    ..Default::default()
                },
            );
        }
        derive_requested_metrics_with_cache_stats(
            &mut db,
            &requested_metrics_plan(),
            cache,
            metrics_manifest(),
        )
        .expect("canonical metrics derivation");
        db
    }

    /// Deferral must be invisible once the metadata is recorded: the same rows
    /// in the same order, and so the same stable-key ids, as an eager run —
    /// whether the metrics provider derived its facts or restored its layer.
    #[test]
    fn deferred_metric_metadata_records_what_an_eager_run_records() {
        let temp = tempfile::tempdir().expect("tempdir");
        let disabled = Cache::new(temp.path().join("disabled"), false);
        let warm = Cache::new(temp.path().join("cache").join("analysis"), true);
        let primed = restore_then_derive_metrics(temp.path(), &warm, false);
        assert!(!primed.file_metrics().is_empty());

        fn json<T: serde::Serialize>(facts: &[T]) -> String {
            serde_json::to_string(facts).expect("facts serialize")
        }
        for (label, cache) in [("derived", &disabled), ("restored", &warm)] {
            let eager = restore_then_derive_metrics(temp.path(), cache, false);
            let mut deferred = restore_then_derive_metrics(temp.path(), cache, true);

            for (family, deferred_facts, eager_facts) in [
                (
                    "functions",
                    json(deferred.functions()),
                    json(eager.functions()),
                ),
                (
                    "file metrics",
                    json(deferred.file_metrics()),
                    json(eager.file_metrics()),
                ),
                (
                    "function metrics",
                    json(deferred.function_metrics()),
                    json(eager.function_metrics()),
                ),
                (
                    "complexity metrics",
                    json(deferred.complexity_metrics()),
                    json(eager.complexity_metrics()),
                ),
            ] {
                assert_eq!(deferred_facts, eager_facts, "{label}: {family}");
            }
            assert_eq!(
                deferred
                    .fact_meta()
                    .family_rows(FactFamily::FunctionMetric)
                    .count(),
                0,
                "{label}: metric metadata was recorded while syntax metadata was deferred"
            );
            let eager_rows = metadata_rows(&eager);
            assert_eq!(
                metadata_rows(&deferred).len() + deferred.deferred_syntax_metadata_len(),
                eager_rows.len(),
                "{label}"
            );

            deferred.record_deferred_syntax_metadata();
            assert_eq!(deferred.deferred_syntax_metadata_len(), 0, "{label}");
            assert_eq!(metadata_rows(&deferred), eager_rows, "{label}");
            assert_eq!(
                deferred.stable_key_interner().len(),
                eager.stable_key_interner().len(),
                "{label}"
            );
        }
    }

    mod metrics_layer_cache {
        use super::*;

        #[test]
        fn metrics_layer_reuses_warm_cache() {
            let temp = tempfile::tempdir().expect("tempdir");
            let loaded = load_config(temp.path()).expect("default config loads");
            let cache = Cache::new(temp.path().join("cache").join("analysis"), true);
            let plan = requested_metrics_plan();
            let source = "export function handler() {\n  if (ok) return 1;\n  return 0;\n}\n";
            let mut first_db = fixture_db(temp.path(), "handler", source);
            let mut second_db = fixture_db(temp.path(), "handler", source);

            let first = derive_metrics_with_cache(
                &mut first_db,
                &loaded,
                &cache,
                &plan,
                "config",
                "stable",
            );
            let second = derive_metrics_with_cache(
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
            assert_eq!(metric_rows(&first_db), metric_rows(&second_db));
        }

        #[test]
        fn metrics_layer_invalidates_on_function_input_change() {
            let temp = tempfile::tempdir().expect("tempdir");
            let loaded = load_config(temp.path()).expect("default config loads");
            let cache = Cache::new(temp.path().join("cache").join("analysis"), true);
            let plan = requested_metrics_plan();
            let source = "export function handler() {\n  return 1;\n}\n";
            let changed = "export function renamed() {\n  return 1;\n}\n";
            let mut base_db = fixture_db(temp.path(), "handler", source);
            derive_metrics_with_cache(&mut base_db, &loaded, &cache, &plan, "config", "stable");
            let mut changed_db = fixture_db(temp.path(), "renamed", changed);

            let changed = derive_metrics_with_cache(
                &mut changed_db,
                &loaded,
                &cache,
                &plan,
                "config",
                "stable",
            );

            assert_eq!(changed.cache_stats.misses, 1);
            assert_eq!(changed.cache_stats.recomputes, 1);
        }

        #[test]
        fn metrics_layer_corrupt_cache_recomputes() {
            let temp = tempfile::tempdir().expect("tempdir");
            let loaded = load_config(temp.path()).expect("default config loads");
            let cache = Cache::new(temp.path().join("cache").join("analysis"), true);
            let plan = requested_metrics_plan();
            let source = "export function handler() {\n  return 1;\n}\n";
            let mut first_db = fixture_db(temp.path(), "handler", source);
            derive_metrics_with_cache(&mut first_db, &loaded, &cache, &plan, "config", "stable");
            let manifest = first_layer_file(temp.path().join("cache").as_path(), "manifests");
            fs::write(manifest, "{broken").expect("corrupt metrics manifest");
            let mut second_db = fixture_db(temp.path(), "handler", source);

            let second = derive_metrics_with_cache(
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

        #[test]
        fn metrics_layer_cache_write_failure_is_reported() {
            let temp = tempfile::tempdir().expect("tempdir");
            let loaded = load_config(temp.path()).expect("default config loads");
            let cache_root = temp.path().join("cache");
            fs::create_dir_all(&cache_root).expect("cache root");
            fs::write(cache_root.join("layers"), "not a directory").expect("layer root file");
            let cache = Cache::new(cache_root.join("analysis"), true);
            let plan = requested_metrics_plan();
            let source = "export function handler() {\n  return 1;\n}\n";
            let mut db = fixture_db(temp.path(), "handler", source);

            let derivation =
                derive_metrics_with_cache(&mut db, &loaded, &cache, &plan, "config", "stable");

            assert_eq!(derivation.cache_stats.misses, 0);
            assert_eq!(derivation.cache_stats.invalid_evicted_reads, 1);
            assert_eq!(derivation.cache_stats.recomputes, 1);
            assert_eq!(derivation.cache_stats.writes, 0);
            assert!(derivation.diagnostics.iter().any(|diagnostic| {
                diagnostic.rule_id == "internal/cache"
                    && diagnostic.file == "metrics layer"
                    && diagnostic.message.contains("cache write failed")
            }));
            assert!(!db.file_metrics().is_empty());
        }

        #[test]
        fn metrics_layer_disabled_cache_records_bypass_without_layer_files() {
            let temp = tempfile::tempdir().expect("tempdir");
            let loaded = load_config(temp.path()).expect("default config loads");
            let cache_root = temp.path().join("cache").join("analysis");
            let cache = Cache::new(&cache_root, false);
            let plan = requested_metrics_plan();
            let source = "export function handler() {\n  return 1;\n}\n";
            let mut db = fixture_db(temp.path(), "handler", source);

            let derivation =
                derive_metrics_with_cache(&mut db, &loaded, &cache, &plan, "config", "stable");

            assert_eq!(derivation.cache_stats.bypasses_disabled, 1);
            assert_eq!(derivation.cache_stats.recomputes, 1);
            assert!(!temp.path().join("cache").join("layers").exists());
            assert!(!db.file_metrics().is_empty());
        }
    }

    fn metric_rows(db: &AnalysisDb) -> Vec<(String, u32, u32, u32)> {
        let mut rows = db
            .file_metrics()
            .iter()
            .map(|metric| {
                (
                    format!("file:{}", db.path_for(metric.file)),
                    metric.line_count,
                    metric.byte_count,
                    metric.function_count,
                )
            })
            .chain(db.function_metrics().iter().map(|metric| {
                (
                    format!("function:{}:{}", db.path_for(metric.file), metric.name),
                    metric.line_count,
                    metric.byte_count,
                    0,
                )
            }))
            .chain(db.complexity_metrics().iter().map(|metric| {
                (
                    format!("complexity:{}:{}", db.path_for(metric.file), metric.name),
                    metric.cyclomatic_complexity,
                    0,
                    0,
                )
            }))
            .collect::<Vec<_>>();
        rows.sort();
        rows
    }
}
