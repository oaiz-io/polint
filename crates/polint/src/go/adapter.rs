use crate::analysis_api::{
    AnalysisCache, BranchObligation, CacheStats, CachedFileAnalysis, CachedFileFacts, Digest,
    DigestKind, DisabledAnalysisCache, FactDatabase, FileCacheKeyParts, FileCacheReadStatus,
    FunctionFact, GO_PARSER_BACKEND, GO_PARSER_GRAMMAR, GoTypeDeclFact, GoTypeDeclKind, ImportFact,
    LayerCacheEntryDigests, LayerCacheKeyParts, LayerCacheKind, LayerCachePrecision,
    LayerCacheReadStatus, LayerCacheWriteStatus, PackageFact, ProviderRunResult, SourceFile,
    StringLiteralFact, TestFact,
};
use crate::internal_core::{
    BranchId, Diagnostic, DiagnosticRange as TextRange, FileId, FunctionId, Language, Span,
    fingerprint,
};
use anyhow::{Context, Result};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use tree_sitter::{Node, Parser};

use crate::go::local_db::LocalFactDb;

const GO_CACHE_SCHEMA: &str = "go-facts-v3";
const GO_PROVIDER_ID: &str = "polint.go.syntax";
const GO_SYNTAX_LAYER_SCHEMA: &str = "go-syntax-layer-v2";

thread_local! {
    static GO_PARSER: RefCell<Option<Parser>> = const { RefCell::new(None) };
}

/// Convenience wrapper used by Go-adapter unit tests (no cache, sequential).
#[cfg(test)]
pub(crate) fn analyze(db: &mut dyn FactDatabase) -> Vec<Diagnostic> {
    let cache = DisabledAnalysisCache;
    analyze_with_options(db, &cache, "", "", false)
}

/// Sequential, cache-aware analysis used by Go-adapter unit tests only.
#[cfg(test)]
pub(crate) fn analyze_with_cache(
    db: &mut dyn FactDatabase,
    cache: &dyn AnalysisCache,
    config_hash: &str,
    rule_hash: &str,
) -> Vec<Diagnostic> {
    analyze_with_options(db, cache, config_hash, rule_hash, false)
}

/// Used by `polint::_bench::go` / `polint-bench`.
#[allow(dead_code, unreachable_pub)]
pub fn analyze_with_options(
    db: &mut dyn FactDatabase,
    cache: &dyn AnalysisCache,
    config_hash: &str,
    rule_hash: &str,
    parallel: bool,
) -> Vec<Diagnostic> {
    analyze_with_plan_options(db, cache, config_hash, rule_hash, "", parallel)
}

pub(crate) fn analyze_with_plan_options(
    db: &mut dyn FactDatabase,
    cache: &dyn AnalysisCache,
    config_hash: &str,
    rule_hash: &str,
    plan_digest: &str,
    parallel: bool,
) -> Vec<Diagnostic> {
    analyze_with_plan_options_and_cache_stats(
        db,
        cache,
        config_hash,
        rule_hash,
        plan_digest,
        parallel,
    )
    .diagnostics
}

pub(crate) fn analyze_with_plan_options_and_cache_stats(
    db: &mut dyn FactDatabase,
    cache: &dyn AnalysisCache,
    config_hash: &str,
    rule_hash: &str,
    plan_digest: &str,
    parallel: bool,
) -> ProviderRunResult {
    let owned: Vec<SourceFile> = db
        .files()
        .iter()
        .filter(|file| file.language == Language::Go)
        .cloned()
        .collect();
    let files: Vec<&SourceFile> = owned.iter().collect();
    analyze_files_with_plan_options_and_cache_stats(
        db,
        &files,
        cache,
        config_hash,
        rule_hash,
        plan_digest,
        parallel,
    )
}

pub(crate) fn analyze_files_with_plan_options_and_cache_stats(
    db: &mut dyn FactDatabase,
    files: &[&SourceFile],
    cache: &dyn AnalysisCache,
    config_hash: &str,
    rule_hash: &str,
    plan_digest: &str,
    parallel: bool,
) -> ProviderRunResult {
    let mut cache_stats = CacheStats::default();
    if files.is_empty() {
        return ProviderRunResult {
            counts: Default::default(),
            diagnostics: Vec::new(),
            cache_stats,
            output_digest: None,
            execution: Default::default(),
        };
    }

    let layer_key = go_syntax_layer_key(files, config_hash);
    // The validator parses the blob to check it; keep that payload so a hit does
    // not deserialize the same bytes a second time.
    let mut validated_payload: Option<SyntaxLayerPayload> = None;
    let read = {
        let mut validate = |bytes: &[u8], digests: LayerCacheEntryDigests<'_>| -> bool {
            let Ok(payload) = serde_json::from_slice::<SyntaxLayerPayload>(bytes) else {
                return false;
            };
            if !validate_syntax_layer_payload(&payload, digests, GO_SYNTAX_LAYER_SCHEMA, files) {
                return false;
            }
            validated_payload = Some(payload);
            true
        };
        cache.read_layer_json(&layer_key, &mut validate)
    };

    match read.status {
        LayerCacheReadStatus::Hit => {
            cache_stats.record_hit();
            cache_stats.record_verified_reuse();
            let payload = validated_payload
                .take()
                .expect("layer cache hit ran the validator, which keeps the parsed payload");
            ProviderRunResult {
                counts: Default::default(),
                diagnostics: restore_syntax_layer_payload(db, payload),
                cache_stats,
                output_digest: read.output_digest,
                execution: Default::default(),
            }
        }
        LayerCacheReadStatus::BypassedDisabled => {
            cache_stats.record_disabled_bypass();
            cache_stats.record_recompute();
            let payload = parse_go_syntax_layer_payload(
                files,
                cache,
                config_hash,
                rule_hash,
                plan_digest,
                parallel,
            );
            let output_digest = payload.output_digest.clone();
            ProviderRunResult {
                counts: Default::default(),
                diagnostics: restore_syntax_layer_payload(db, payload),
                cache_stats,
                output_digest,
                execution: Default::default(),
            }
        }
        LayerCacheReadStatus::Miss | LayerCacheReadStatus::InvalidEvicted => {
            if read.status == LayerCacheReadStatus::Miss {
                cache_stats.record_miss();
            } else {
                cache_stats.record_invalid_evicted_read();
            }
            cache_stats.record_recompute();
            let payload = parse_go_syntax_layer_payload(
                files,
                cache,
                config_hash,
                rule_hash,
                plan_digest,
                parallel,
            );
            let mut write_diagnostics = Vec::new();
            let output_digest = write_syntax_layer_payload(
                cache,
                &layer_key,
                &payload,
                &mut cache_stats,
                &mut write_diagnostics,
            );
            let mut diagnostics = restore_syntax_layer_payload(db, payload);
            diagnostics.extend(write_diagnostics);
            ProviderRunResult {
                counts: Default::default(),
                diagnostics,
                cache_stats,
                output_digest,
                execution: Default::default(),
            }
        }
    }
}

struct GoFileAnalysis {
    relative_path: String,
    content_hash: String,
    diagnostics: Vec<Diagnostic>,
    facts: Option<crate::analysis_api::CachedFileFacts>,
    output_digest: Option<Digest>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SyntaxLayerPayload {
    schema: String,
    #[serde(default)]
    output_digest: Option<Digest>,
    files: Vec<SyntaxLayerFile>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SyntaxLayerFile {
    relative_path: String,
    content_hash: String,
    diagnostics: Vec<Diagnostic>,
    facts: Option<crate::analysis_api::CachedFileFacts>,
}

fn go_syntax_layer_key(files: &[&SourceFile], config_hash: &str) -> LayerCacheKeyParts {
    LayerCacheKeyParts {
        layer_kind: LayerCacheKind::GoSyntax,
        provider_id: GO_PROVIDER_ID.to_string(),
        provider_version: env!("CARGO_PKG_VERSION").to_string(),
        schema_version: GO_CACHE_SCHEMA.to_string(),
        parameter_digest: parser_parameter_digest(files),
        lifecycle_digest: Digest::absent(DigestKind::GoLifecycle, "go_syntax_lifecycle_absent"),
        config_digest: Digest::from_parts(DigestKind::Config, "config_hash", &[config_hash]),
        toolchain_digest: go_parser_toolchain_digest(GO_PARSER_BACKEND, GO_PARSER_GRAMMAR),
        input_digests: files.iter().map(|file| source_text_digest(file)).collect(),
    }
}

/// Identity of the toolchain that produced a Go syntax layer: this engine plus
/// the parser it links. Facts are only reusable when both still match, so both
/// belong in the key rather than the engine version alone.
fn go_parser_toolchain_digest(backend: &str, grammar: &str) -> Digest {
    Digest::from_parts(
        DigestKind::ToolInvocation,
        "go_syntax_parser_toolchain",
        &[env!("CARGO_PKG_VERSION"), backend, grammar],
    )
}

/// Parser identity carried by every per-file Go cache entry, for the same reason
/// the layer key carries it.
fn go_parser_identity() -> String {
    format!("{GO_PARSER_BACKEND}+{GO_PARSER_GRAMMAR}")
}

fn go_file_cache_key(
    file: &SourceFile,
    config_hash: &str,
    rule_hash: &str,
    plan_hash: &str,
) -> FileCacheKeyParts {
    FileCacheKeyParts {
        relative_path: file.relative_path.clone(),
        content_hash: file.content_hash.clone(),
        config_hash: config_hash.to_string(),
        rule_hash: rule_hash.to_string(),
        plan_hash: plan_hash.to_string(),
        schema: GO_CACHE_SCHEMA.to_string(),
        parser_identity: go_parser_identity(),
    }
}

fn source_text_digest(file: &SourceFile) -> Digest {
    Digest::from_parts(
        DigestKind::SourceText,
        "source_file",
        &[file.relative_path.as_str(), file.content_hash.as_str()],
    )
}

fn parser_parameter_digest(files: &[&SourceFile]) -> Digest {
    let mut parts = files
        .iter()
        .map(|file| format!("{}={:?}", file.relative_path, file.language))
        .collect::<Vec<_>>();
    parts.sort();
    let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    Digest::from_parts(
        DigestKind::ProviderParameters,
        "go_parser_parameters",
        &refs,
    )
}

/// Accepts a cached payload when it describes exactly `files` and its embedded
/// native output digest matches the layer manifest.
///
/// The cache verifies the digest of the complete blob before calling this
/// validator, so the embedded identity is already covered by the payload's
/// integrity check. This avoids reserializing every fact on warm reads.
fn validate_syntax_layer_payload(
    payload: &SyntaxLayerPayload,
    digests: LayerCacheEntryDigests<'_>,
    schema: &str,
    files: &[&SourceFile],
) -> bool {
    if payload.schema != schema || payload.files.len() != files.len() {
        return false;
    }
    let mut expected = files
        .iter()
        .map(|file| (file.relative_path.as_str(), file.content_hash.as_str()))
        .collect::<Vec<_>>();
    expected.sort();
    let actual = payload
        .files
        .iter()
        .map(|file| (file.relative_path.as_str(), file.content_hash.as_str()))
        .collect::<Vec<_>>();
    actual == expected
        && digests.payload.is_some()
        && payload
            .output_digest
            .as_ref()
            .is_some_and(|expected| digests.output == Some(expected))
}

fn parse_go_syntax_layer_payload(
    files: &[&SourceFile],
    cache: &dyn AnalysisCache,
    config_hash: &str,
    rule_hash: &str,
    plan_digest: &str,
    parallel: bool,
) -> SyntaxLayerPayload {
    let mut results = if parallel {
        files
            .par_iter()
            .map(|file| analyze_go_source_file(file, cache, config_hash, rule_hash, plan_digest))
            .collect::<Vec<_>>()
    } else {
        files
            .iter()
            .map(|file| analyze_go_source_file(file, cache, config_hash, rule_hash, plan_digest))
            .collect::<Vec<_>>()
    };
    results.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

    let output_digest = results
        .iter()
        .map(|result| result.output_digest.clone())
        .collect::<Option<Vec<_>>>()
        .map(|file_digests| go_syntax_output_digest(&file_digests));

    SyntaxLayerPayload {
        schema: GO_SYNTAX_LAYER_SCHEMA.to_string(),
        output_digest,
        files: results
            .into_iter()
            .map(|result| SyntaxLayerFile {
                relative_path: result.relative_path,
                content_hash: result.content_hash,
                diagnostics: result.diagnostics,
                facts: result.facts,
            })
            .collect(),
    }
}

fn restore_syntax_layer_payload(
    db: &mut dyn FactDatabase,
    payload: SyntaxLayerPayload,
) -> Vec<Diagnostic> {
    // FileId is dense; build a path→id index once so restore is O(F), not O(F²).
    let restores: Vec<(FileId, Option<_>, Vec<Diagnostic>)> = {
        let file_id_by_path: HashMap<&str, FileId> = db
            .files()
            .iter()
            .map(|source| (source.relative_path.as_str(), source.id))
            .collect();
        payload
            .files
            .into_iter()
            .filter_map(|file| {
                let &file_id = file_id_by_path.get(file.relative_path.as_str())?;
                Some((file_id, file.facts, file.diagnostics))
            })
            .collect()
    };
    let mut diagnostics = Vec::new();
    for (file_id, facts, file_diagnostics) in restores {
        if let Some(facts) = facts {
            db.restore_file_facts(file_id, facts);
        }
        diagnostics.extend(file_diagnostics);
    }
    diagnostics
}

fn write_syntax_layer_payload(
    cache: &dyn AnalysisCache,
    layer_key: &LayerCacheKeyParts,
    payload: &SyntaxLayerPayload,
    stats: &mut CacheStats,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Digest> {
    let output_digest = payload.output_digest.clone()?;
    let payload_bytes = match serde_json::to_vec(payload) {
        Ok(bytes) => bytes,
        Err(error) => {
            diagnostics.push(cache_write_diagnostic(
                "go syntax layer",
                anyhow::anyhow!(error),
            ));
            return None;
        }
    };
    let payload_digest = match cache.payload_digest_for_json_bytes(&payload_bytes) {
        Ok(digest) => digest,
        Err(error) => {
            diagnostics.push(cache_write_diagnostic(
                "go syntax layer",
                anyhow::anyhow!(error),
            ));
            return None;
        }
    };
    match cache.write_layer_json(
        layer_key,
        &output_digest,
        &payload_digest,
        LayerCachePrecision::Syntax,
        "native_trusted",
        &payload_bytes,
    ) {
        Ok(LayerCacheWriteStatus::Written) => {
            stats.record_write();
            Some(output_digest)
        }
        Ok(LayerCacheWriteStatus::BypassedDisabled) => {
            stats.record_disabled_bypass();
            Some(output_digest)
        }
        Err(error) => {
            diagnostics.push(cache_write_diagnostic(
                "go syntax layer",
                anyhow::anyhow!(error),
            ));
            None
        }
    }
}

#[derive(Serialize)]
struct CachedFileOutput<'a> {
    schema: &'a str,
    diagnostics: &'a [Diagnostic],
    facts: Option<&'a CachedFileFacts>,
}

fn go_file_output_digest_from_bytes(
    relative_path: &str,
    content_hash: &str,
    cached_file_bytes: &[u8],
) -> Digest {
    let mut digest = Digest::builder(DigestKind::ProviderOutput, "go-syntax-file-output-v1");
    digest.field("path", relative_path);
    digest.field("content-hash", content_hash);
    digest.bytes_field("cached-file", cached_file_bytes);
    digest.finish()
}

fn go_file_output_digest(
    relative_path: &str,
    content_hash: &str,
    diagnostics: &[Diagnostic],
    facts: Option<&CachedFileFacts>,
) -> Option<Digest> {
    let bytes = serde_json::to_vec(&CachedFileOutput {
        schema: GO_CACHE_SCHEMA,
        diagnostics,
        facts,
    })
    .ok()?;
    Some(go_file_output_digest_from_bytes(
        relative_path,
        content_hash,
        &bytes,
    ))
}

fn go_syntax_output_digest(file_digests: &[Digest]) -> Digest {
    let mut digest = Digest::builder(DigestKind::ProviderOutput, "go-syntax-output-v1");
    digest.field("schema", GO_SYNTAX_LAYER_SCHEMA);
    digest.u64_field("file-count", file_digests.len() as u64);
    for file_digest in file_digests {
        digest.field("file", &file_digest.to_string());
    }
    digest.finish()
}

#[cfg(test)]
fn go_syntax_output_digest_for_payload(payload: &SyntaxLayerPayload) -> Option<Digest> {
    let file_digests = payload
        .files
        .iter()
        .map(|file| {
            go_file_output_digest(
                &file.relative_path,
                &file.content_hash,
                &file.diagnostics,
                file.facts.as_ref(),
            )
        })
        .collect::<Option<Vec<_>>>()?;
    Some(go_syntax_output_digest(&file_digests))
}

fn analyze_go_source_file(
    file: &SourceFile,
    cache: &dyn AnalysisCache,
    config_hash: &str,
    rule_hash: &str,
    plan_hash: &str,
) -> GoFileAnalysis {
    let key = go_file_cache_key(file, config_hash, rule_hash, plan_hash);
    let read = cache.read_file_json(&key);
    if read.status == FileCacheReadStatus::Hit
        && let Some(bytes) = read.value
        && let Ok(cached) = serde_json::from_slice::<CachedFileAnalysis>(&bytes)
        && cached.schema == GO_CACHE_SCHEMA
    {
        let output_digest = Some(go_file_output_digest_from_bytes(
            &file.relative_path,
            &file.content_hash,
            &bytes,
        ));
        return GoFileAnalysis {
            relative_path: file.relative_path.clone(),
            content_hash: file.content_hash.clone(),
            diagnostics: cached.diagnostics,
            facts: Some(cached.facts),
            output_digest,
        };
    }

    let mut local_db = LocalFactDb::new();
    let local_file = local_db.add_source_file_from(file);
    let mut diagnostics = match parse_go_file(&mut local_db, local_file) {
        Ok(file_diagnostics) => file_diagnostics,
        Err(error) => {
            let diagnostics = vec![Diagnostic::error(
                "parser/go",
                file.relative_path.clone(),
                TextRange::point(1, 1),
                format!("Failed to parse Go file: {error}"),
            )];
            let output_digest =
                go_file_output_digest(&file.relative_path, &file.content_hash, &diagnostics, None);
            return GoFileAnalysis {
                relative_path: file.relative_path.clone(),
                content_hash: file.content_hash.clone(),
                diagnostics,
                facts: None,
                output_digest,
            };
        }
    };
    let facts = local_db.facts_for_file(local_file);
    let cached = CachedFileAnalysis {
        schema: GO_CACHE_SCHEMA.to_string(),
        diagnostics: diagnostics.clone(),
        facts: facts.clone(),
    };
    let mut output_digest = None;
    if let Ok(bytes) = serde_json::to_vec(&cached) {
        output_digest = Some(go_file_output_digest_from_bytes(
            &file.relative_path,
            &file.content_hash,
            &bytes,
        ));
        if let Err(error) = cache.write_file_json(&key, &bytes) {
            diagnostics.push(cache_write_diagnostic(
                file.relative_path.as_str(),
                anyhow::anyhow!(error),
            ));
            output_digest = None;
        }
    };
    GoFileAnalysis {
        relative_path: file.relative_path.clone(),
        content_hash: file.content_hash.clone(),
        diagnostics,
        facts: Some(facts),
        output_digest,
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

fn parse_go_file(db: &mut dyn FactDatabase, file_id: FileId) -> Result<Vec<Diagnostic>> {
    let source_owned = {
        let file = db.file(file_id).context("missing source file")?;
        Arc::clone(&file.source)
    };
    let source = source_owned.as_ref();

    let tree = GO_PARSER.with(|cell| -> Result<_> {
        let mut slot = cell.borrow_mut();
        let parser = match slot.as_mut() {
            Some(parser) => parser,
            None => {
                let mut parser = Parser::new();
                parser.set_language(&super::grammar::language())?;
                slot.insert(parser)
            }
        };
        parser
            .parse(source, None)
            .context("tree-sitter returned no parse tree")
    })?;
    let root = tree.root_node();
    let mut diagnostics = Vec::new();

    if root.has_error() {
        diagnostics.push(parser_error_diagnostic(db, file_id, root));
    }

    extract_package(db, file_id, source, root);
    extract_imports(db, file_id, source, root);
    extract_string_literals(db, file_id, source, root);
    extract_functions(db, file_id, source, root);
    extract_go_types(db, file_id, source, root);
    Ok(diagnostics)
}

fn parser_error_diagnostic(db: &dyn FactDatabase, file_id: FileId, root: Node<'_>) -> Diagnostic {
    let range = first_error_node(root)
        .map(|node| node_span(db, file_id, node).diagnostic_range())
        .unwrap_or_else(|| TextRange::point(1, 1));

    Diagnostic::error(
        "parser/go",
        db.path_for(file_id),
        range,
        "Go parser reported a syntax error.",
    )
}

fn extract_package(db: &mut dyn FactDatabase, file: FileId, source: &str, root: Node<'_>) {
    let Some(package_clause) =
        first_named_descendant(root, &|node| node.kind() == "package_clause")
    else {
        return;
    };

    let Some(identifier) =
        first_named_descendant(package_clause, &|node| node.kind() == "package_identifier")
    else {
        return;
    };

    let Some(name) = node_text(source, identifier) else {
        return;
    };

    db.push_package(PackageFact::new(
        crate::internal_core::PackageId::from_raw(0),
        file,
        name.to_string(),
        node_span(db, file, identifier),
        Language::Go,
    ));
}

fn indexed_span(db: &dyn FactDatabase, file: FileId, start_byte: usize, end_byte: usize) -> Span {
    db.file(file)
        .map(|source_file| source_file.span_from_byte_range(start_byte, end_byte))
        .unwrap_or_else(|| Span::point(file, 1, 1))
}

fn node_span(db: &dyn FactDatabase, file: FileId, node: Node<'_>) -> Span {
    indexed_span(db, file, node.start_byte(), node.end_byte())
}

fn node_text<'source>(source: &'source str, node: Node<'_>) -> Option<&'source str> {
    source.get(node.start_byte()..node.end_byte())
}

fn first_error_node(node: Node<'_>) -> Option<Node<'_>> {
    if node.is_error() || node.is_missing() {
        return Some(node);
    }

    for index in 0..node.named_child_count() as u32 {
        let Some(child) = node.named_child(index) else {
            continue;
        };
        if let Some(error) = first_error_node(child) {
            return Some(error);
        }
    }
    None
}

fn first_named_descendant<'tree, F>(node: Node<'tree>, predicate: &F) -> Option<Node<'tree>>
where
    F: Fn(Node<'tree>) -> bool,
{
    if predicate(node) {
        return Some(node);
    }

    walk_named_children(node, |child| {
        if let Some(descendant) = first_named_descendant(child, predicate) {
            return Some(descendant);
        }
        None
    })
}

fn walk_named_children<'tree, F>(node: Node<'tree>, mut visit: F) -> Option<Node<'tree>>
where
    F: FnMut(Node<'tree>) -> Option<Node<'tree>>,
{
    for index in 0..node.named_child_count() as u32 {
        let Some(child) = node.named_child(index) else {
            continue;
        };
        if let Some(found) = visit(child) {
            return Some(found);
        }
    }
    None
}

fn extract_imports(db: &mut dyn FactDatabase, file: FileId, source: &str, root: Node<'_>) {
    visit_named_descendants(root, &mut |node| {
        if node.kind() != "import_declaration" {
            return;
        }

        let mut specs = Vec::new();
        visit_named_descendants(node, &mut |descendant| {
            if descendant.kind() == "import_spec" {
                specs.push(descendant);
            }
        });

        if specs.is_empty() {
            push_import_from_node(db, file, source, node);
            return;
        }

        for spec in specs {
            push_import_from_node(db, file, source, spec);
        }
    });
}

fn push_import_from_node(db: &mut dyn FactDatabase, file: FileId, source: &str, node: Node<'_>) {
    let Some(path_node) = first_named_descendant(node, &is_go_string_literal) else {
        return;
    };
    let Some(path) = unquote_go_string_literal(source, path_node) else {
        return;
    };
    let package = import_alias(source, node, path_node);

    db.push_import(ImportFact::new(
        crate::internal_core::ImportId::from_raw(0),
        file,
        package,
        path,
        node_span(db, file, node),
        Language::Go,
    ));
}

fn import_alias(source: &str, spec: Node<'_>, path: Node<'_>) -> Option<String> {
    let raw = source.get(spec.start_byte()..path.start_byte())?.trim();
    let alias = raw
        .strip_prefix("import")
        .unwrap_or(raw)
        .trim()
        .trim_matches('(')
        .trim();
    if alias.is_empty() {
        None
    } else {
        Some(alias.to_string())
    }
}

fn unquote_go_string_literal(source: &str, node: Node<'_>) -> Option<String> {
    let text = node_text(source, node)?.trim();
    if text.len() < 2 {
        return None;
    }

    let first = text.as_bytes()[0] as char;
    let last = text.as_bytes()[text.len() - 1] as char;
    if matches!((first, last), ('"', '"') | ('`', '`')) {
        Some(text[1..text.len() - 1].to_string())
    } else {
        None
    }
}

fn is_go_string_literal(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "interpreted_string_literal" | "raw_string_literal"
    )
}

fn extract_string_literals(db: &mut dyn FactDatabase, file: FileId, source: &str, root: Node<'_>) {
    visit_named_descendants(root, &mut |node| {
        if !is_go_string_literal(node) || is_inside_go_import(node) {
            return;
        }

        let Some(value) = unquote_go_string_literal(source, node) else {
            return;
        };

        db.push_string_literal(StringLiteralFact::new(
            file,
            value,
            node_span(db, file, node),
            Language::Go,
        ));
    });
}

fn is_inside_go_import(node: Node<'_>) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if matches!(parent.kind(), "import_spec" | "import_declaration") {
            return true;
        }
        current = parent.parent();
    }
    false
}

fn extract_functions(db: &mut dyn FactDatabase, file: FileId, source: &str, root: Node<'_>) {
    let file_path = db.path_for(file);
    visit_named_descendants(root, &mut |node| {
        if !matches!(node.kind(), "function_declaration" | "method_declaration") {
            return;
        }

        let Some(simple_name) = declaration_name(source, node) else {
            return;
        };
        let body_node = node.child_by_field_name("body");
        let name = if node.kind() == "method_declaration" {
            receiver_type_name(source, node)
                .map(|receiver| format!("{receiver}.{simple_name}"))
                .unwrap_or_else(|| simple_name.clone())
        } else {
            simple_name.clone()
        };
        let span = node_span(db, file, node);
        let is_test = is_go_test_entry(&file_path, &simple_name, node, source);
        let fact = FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            name.clone(),
            span.clone(),
            Language::Go,
            is_test,
            simple_name.chars().next().is_some_and(char::is_uppercase),
            body_node
                .map(|body| go_cyclomatic_complexity(source, body))
                .unwrap_or(1),
            body_node
                .map(|body| extract_calls(source, body))
                .unwrap_or_default(),
        );
        let function_id = db.push_function(fact);

        if let Some(body) = body_node {
            extract_branches(
                db,
                file,
                function_id,
                &name,
                function_result_contains_error(source, node),
                source,
                body,
            );
        }
        if is_test && let Some(body) = body_node {
            db.push_test(go_test_fact(file, function_id, name, span, source, body));
        }
    });
}

/// Extracts typed Go structural facts: one row per named `type_spec`/`type_alias`
/// plus one row per anonymous `struct_type` occurrence that is not the body of a
/// top-level, non-grouped, non-alias declaration (those are fully described by their
/// named row, mirroring the line-anchored `^type NAME struct {` shape consumers
/// historically matched).
fn extract_go_types(db: &mut dyn FactDatabase, file: FileId, source: &str, root: Node<'_>) {
    visit_named_descendants(root, &mut |node| match node.kind() {
        "type_spec" | "type_alias" => push_named_go_type(db, file, source, node),
        "struct_type" if !is_suppressed_named_struct(node) => {
            push_anonymous_go_struct(db, file, node);
        }
        _ => {}
    });
}

fn push_named_go_type(db: &mut dyn FactDatabase, file: FileId, source: &str, spec: Node<'_>) {
    let Some(name_node) = spec.child_by_field_name("name") else {
        return;
    };
    let Some(type_node) = spec.child_by_field_name("type") else {
        return;
    };
    let Some(declaration) = spec.parent() else {
        return;
    };
    let Some(name) = node_text(source, name_node).map(str::to_string) else {
        return;
    };

    let (kind, direct_name, body_range) = go_type_shape(source, type_node);
    let fact = GoTypeDeclFact::new(
        file,
        node_span(db, file, name_node),
        Some(name),
        kind,
        spec.kind() == "type_alias",
        is_grouped_go_declaration(declaration),
        declaration
            .parent()
            .is_some_and(|grand| grand.kind() == "source_file"),
        spec.child_by_field_name("type_parameters").is_some(),
        direct_name,
        body_range,
        declaration.start_byte() as u32,
    );
    db.push_go_type(fact);
}

fn push_anonymous_go_struct(db: &mut dyn FactDatabase, file: FileId, node: Node<'_>) {
    let Some(open) = go_body_open_byte(node) else {
        return;
    };
    let end = node.end_byte() as u32;
    let fact = GoTypeDeclFact::new(
        file,
        node_span(db, file, node),
        None,
        GoTypeDeclKind::Struct,
        false,
        false,
        false,
        false,
        None,
        Some((open + 1, end.saturating_sub(1))),
        node.start_byte() as u32,
    );
    db.push_go_type(fact);
}

/// A `struct_type` directly under a top-level, non-grouped, non-alias declaration is
/// already represented by that declaration's named row, so it gets no anonymous row.
fn is_suppressed_named_struct(node: Node<'_>) -> bool {
    let Some(spec) = node.parent() else {
        return false;
    };
    if !matches!(spec.kind(), "type_spec" | "type_alias") || spec.kind() == "type_alias" {
        return false;
    }
    let Some(declaration) = spec.parent() else {
        return false;
    };
    declaration
        .parent()
        .is_some_and(|grand| grand.kind() == "source_file")
        && !is_grouped_go_declaration(declaration)
}

fn go_type_shape(
    source: &str,
    type_node: Node<'_>,
) -> (GoTypeDeclKind, Option<String>, Option<(u32, u32)>) {
    match type_node.kind() {
        "struct_type" => (GoTypeDeclKind::Struct, None, go_body_range(type_node)),
        "interface_type" => (GoTypeDeclKind::Interface, None, go_body_range(type_node)),
        _ => (
            GoTypeDeclKind::Named,
            go_direct_type_name(source, type_node),
            None,
        ),
    }
}

/// Final identifier of a named underlying type expression: `Y`, `Y.Z` -> `Z`,
/// `Y[T]` -> `Y`. Pointer, map, slice, function, and channel heads yield `None`.
fn go_direct_type_name(source: &str, type_node: Node<'_>) -> Option<String> {
    match type_node.kind() {
        "type_identifier" => node_text(source, type_node).map(str::to_string),
        "qualified_type" => type_node
            .child_by_field_name("name")
            .and_then(|name| node_text(source, name))
            .map(str::to_string),
        "parameterized_type" | "generic_type" => type_node
            .child_by_field_name("type")
            .and_then(|inner| go_direct_type_name(source, inner)),
        _ => None,
    }
}

/// Byte position of the `{` that opens a struct or interface body.
fn go_body_open_byte(node: Node<'_>) -> Option<u32> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "{" || child.kind() == "field_declaration_list" {
            return Some(child.start_byte() as u32);
        }
    }
    None
}

fn go_body_range(node: Node<'_>) -> Option<(u32, u32)> {
    let open = go_body_open_byte(node)?;
    let end = node.end_byte() as u32;
    Some((open + 1, end.saturating_sub(1)))
}

/// True when the `type_declaration` groups its specs: the first meaningful child
/// after the `type` keyword is an opening parenthesis.
fn is_grouped_go_declaration(declaration: Node<'_>) -> bool {
    let mut cursor = declaration.walk();
    let mut seen_type_keyword = false;
    for child in declaration.children(&mut cursor) {
        match child.kind() {
            "type" if !seen_type_keyword => seen_type_keyword = true,
            "comment" => {}
            "(" => return true,
            _ => return false,
        }
    }
    false
}

fn declaration_name(source: &str, node: Node<'_>) -> Option<String> {
    node.child_by_field_name("name")
        .and_then(|name| node_text(source, name))
        .map(str::to_string)
        .or_else(|| {
            first_named_descendant(node, &|child| {
                matches!(child.kind(), "identifier" | "field_identifier")
            })
            .and_then(|name| node_text(source, name))
            .map(str::to_string)
        })
}

fn receiver_type_name(source: &str, node: Node<'_>) -> Option<String> {
    let receiver = node.child_by_field_name("receiver")?;
    let receiver_text = node_text(source, receiver)?;
    let inner = receiver_text
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .trim();
    let raw_type = inner
        .split_whitespace()
        .last()
        .unwrap_or(inner)
        .trim_start_matches('*')
        .trim_start_matches('&');
    let without_package = raw_type.rsplit('.').next().unwrap_or(raw_type);
    let without_generics = without_package
        .split_once('[')
        .map(|(name, _)| name)
        .unwrap_or(without_package);

    (!without_generics.is_empty()).then(|| without_generics.to_string())
}

fn is_go_test_entry(path: &str, name: &str, node: Node<'_>, source: &str) -> bool {
    if !path.ends_with("_test.go") || node.kind() != "function_declaration" {
        return false;
    }

    let expected_parameter = if name.starts_with("Test") {
        "*testing.T"
    } else if name.starts_with("Benchmark") {
        "*testing.B"
    } else if name.starts_with("Fuzz") {
        "*testing.F"
    } else {
        return false;
    };

    let Some(parameters) = node
        .child_by_field_name("parameters")
        .or_else(|| first_named_child(node, "parameter_list"))
    else {
        return false;
    };
    let normalized = node_text(source, parameters)
        .unwrap_or_default()
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    let Some(parameters) = normalized
        .strip_prefix('(')
        .and_then(|parameters| parameters.strip_suffix(')'))
    else {
        return false;
    };

    if parameters.contains(',') {
        return false;
    }

    parameters == expected_parameter
        || parameters
            .strip_suffix(expected_parameter)
            .is_some_and(|name| {
                !name.is_empty()
                    && name
                        .chars()
                        .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
            })
}

fn extract_calls(source: &str, body: Node<'_>) -> Vec<String> {
    let mut calls = Vec::new();
    visit_named_descendants(body, &mut |node| {
        if node.kind() != "call_expression" {
            return;
        }
        let Some(call) = call_callee(source, node) else {
            return;
        };
        calls.push(call);
    });
    calls.sort();
    calls.dedup();
    calls
}

fn call_callee(source: &str, node: Node<'_>) -> Option<String> {
    let function_node = node
        .child_by_field_name("function")
        .or_else(|| node.named_child(0))?;
    stable_call_name(source, function_node)
}

fn stable_call_name(source: &str, node: Node<'_>) -> Option<String> {
    let text = node_text(source, node)?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("");
    if text.is_empty() || text.starts_with("func") {
        None
    } else {
        Some(text)
    }
}

fn go_test_fact(
    file: FileId,
    function: FunctionId,
    name: String,
    span: Span,
    source: &str,
    body: Node<'_>,
) -> TestFact {
    TestFact::new(
        file,
        Some(function),
        name,
        span,
        go_test_evidence_terms(source, body),
        go_assertion_count(source, body),
        go_subtest_count(source, body),
        go_subtest_literal_names(source, body),
        go_table_rows(source, body),
    )
}

fn go_subtest_count(source: &str, body: Node<'_>) -> u32 {
    let mut count = 0;
    visit_named_descendants(body, &mut |node| {
        if node.kind() == "call_expression" && call_callee(source, node).as_deref() == Some("t.Run")
        {
            count += 1;
        }
    });
    count
}

fn go_subtest_literal_names(source: &str, body: Node<'_>) -> Vec<String> {
    let mut names = Vec::new();
    visit_named_descendants(body, &mut |node| {
        if node.kind() != "call_expression" {
            return;
        }
        if call_callee(source, node).as_deref() != Some("t.Run") {
            return;
        }
        let Some(args) = node.child_by_field_name("arguments") else {
            return;
        };
        let Some(first) = args.named_child(0) else {
            return;
        };
        if let Some(name) = unquote_go_string_literal(source, first) {
            names.push(name);
        }
    });
    names
}

fn go_assertion_count(source: &str, body: Node<'_>) -> u32 {
    let mut count = 0;
    visit_named_descendants(body, &mut |node| match node.kind() {
        "call_expression"
            if call_callee(source, node).is_some_and(|callee| is_go_assertion_callee(&callee)) =>
        {
            count += 1;
        }
        "if_statement" if is_simple_assertion_condition(source, node) => {
            count += 1;
        }
        _ => {}
    });
    count
}

fn is_go_assertion_callee(callee: &str) -> bool {
    matches!(callee, "t.Fatal" | "t.Fatalf" | "t.Error" | "t.Errorf")
        || callee.starts_with("require.")
        || callee.starts_with("assert.")
}

fn is_simple_assertion_condition(source: &str, node: Node<'_>) -> bool {
    let Some(condition) = if_condition_text(source, node) else {
        return false;
    };
    let normalized = condition
        .trim()
        .trim_matches('(')
        .trim_matches(')')
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();

    matches!(normalized.as_str(), "err==nil" | "err!=nil" | "got!=want")
}

fn if_condition_text<'source>(source: &'source str, node: Node<'_>) -> Option<&'source str> {
    if let Some(condition) = node.child_by_field_name("condition") {
        return node_text(source, condition);
    }

    let text = node_text(source, node)?.trim();
    let condition = text.strip_prefix("if")?.split('{').next()?.trim();
    condition.rsplit(';').next().map(str::trim)
}

fn go_table_rows(source: &str, body: Node<'_>) -> u32 {
    let mut rows = std::collections::BTreeSet::new();
    visit_named_descendants(body, &mut |node| {
        if node.kind() != "literal_value" || !is_anonymous_struct_table(source, node) {
            return;
        }

        for index in 0..node.named_child_count() as u32 {
            let Some(row) = node.named_child(index) else {
                continue;
            };
            let Some(text) = node_text(source, row).map(str::trim) else {
                continue;
            };
            if matches!(
                row.kind(),
                "literal_element" | "keyed_element" | "literal_value"
            ) && text.starts_with('{')
                && text.ends_with('}')
                && text.contains(':')
            {
                rows.insert((row.start_byte(), row.end_byte()));
            }
        }
    });
    rows.len() as u32
}

fn is_anonymous_struct_table(source: &str, literal_value: Node<'_>) -> bool {
    let Some(parent) = literal_value
        .parent()
        .filter(|parent| parent.kind() == "composite_literal")
    else {
        return false;
    };
    let Some(type_text) = source.get(parent.start_byte()..literal_value.start_byte()) else {
        return false;
    };
    let compact = type_text
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    compact.contains("[]struct") || compact.contains("[...]struct")
}

fn go_test_evidence_terms(source: &str, body: Node<'_>) -> Vec<String> {
    let mut terms = std::collections::BTreeSet::new();
    visit_named_descendants(body, &mut |node| match node.kind() {
        "identifier" | "field_identifier" | "package_identifier" => {
            if let Some(text) = node_text(source, node) {
                add_identifier_evidence(&mut terms, text);
            }
        }
        "interpreted_string_literal" | "raw_string_literal" => {
            if let Some(text) = unquote_go_string_literal(source, node) {
                add_string_evidence(&mut terms, source, node, &text);
            }
        }
        "nil" => {
            terms.insert("nil".to_string());
        }
        _ => {}
    });

    visit_named_descendants(body, &mut |node| {
        if node.kind() == "if_statement"
            && if_condition_text(source, node).is_some_and(|condition| condition.contains("nil"))
        {
            terms.insert("nil".to_string());
        }
    });

    terms.into_iter().collect()
}

fn add_identifier_evidence(terms: &mut std::collections::BTreeSet<String>, text: &str) {
    let lower = text.to_ascii_lowercase();
    if is_go_evidence_word(&lower) {
        terms.insert(lower);
    }
}

fn add_string_evidence(
    terms: &mut std::collections::BTreeSet<String>,
    source: &str,
    node: Node<'_>,
    text: &str,
) {
    let words = evidence_words(text);
    let is_case_or_subtest_name = string_literal_looks_like_test_case_name(source, node);
    let has_marker = words.iter().any(|word| is_go_evidence_word(word));
    for word in words {
        if is_go_evidence_word(&word)
            || (is_case_or_subtest_name && !has_marker && !word.contains('_'))
        {
            terms.insert(word);
        }
    }
}

fn evidence_words(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .map(str::trim)
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn is_go_evidence_word(word: &str) -> bool {
    matches!(
        word,
        "allowed" | "denied" | "err" | "error" | "fail" | "invalid" | "nil"
    )
}

fn string_literal_looks_like_test_case_name(source: &str, node: Node<'_>) -> bool {
    let line_start = source[..node.start_byte()]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let prefix = &source[line_start..node.start_byte()];
    prefix.contains("name:") || prefix.contains("t.Run(")
}

fn go_cyclomatic_complexity(_source: &str, body: Node<'_>) -> u32 {
    let mut complexity = 1;
    visit_named_descendants(body, &mut |node| match node.kind() {
        "if_statement" | "for_statement" => complexity += 1,
        "expression_case" | "type_case" | "communication_case" | "default_case" => {
            complexity += 1;
        }
        "binary_expression" => complexity += boolean_operator_count(node),
        _ => {}
    });
    complexity
}

fn boolean_operator_count(node: Node<'_>) -> u32 {
    let mut count = 0;
    for index in 0..node.child_count() as u32 {
        let Some(child) = node.child(index) else {
            continue;
        };
        if matches!(child.kind(), "&&" | "||") {
            count += 1;
        }
    }
    count
}

fn visit_named_descendants<'tree, F>(node: Node<'tree>, visit: &mut F)
where
    F: FnMut(Node<'tree>),
{
    visit(node);
    for index in 0..node.named_child_count() as u32 {
        let Some(child) = node.named_child(index) else {
            continue;
        };
        visit_named_descendants(child, visit);
    }
}

fn first_named_child<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    for index in 0..node.named_child_count() as u32 {
        let Some(child) = node.named_child(index) else {
            continue;
        };
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}

fn extract_branches(
    db: &mut dyn FactDatabase,
    file: FileId,
    function: FunctionId,
    function_name: &str,
    function_returns_error: bool,
    source: &str,
    body: Node<'_>,
) {
    visit_named_descendants(body, &mut |node| match node.kind() {
        "if_statement" => push_if_branches(
            db,
            file,
            function,
            function_name,
            function_returns_error,
            source,
            node,
        ),
        "expression_switch_statement" | "type_switch_statement" => {
            push_switch_branch(db, file, function, function_name, source, node);
        }
        "expression_case" | "type_case" | "communication_case" => push_case_branch(
            db,
            BranchTarget {
                file,
                function,
                function_name,
            },
            function_returns_error,
            source,
            node,
            "case",
        ),
        "default_case" => push_case_branch(
            db,
            BranchTarget {
                file,
                function,
                function_name,
            },
            function_returns_error,
            source,
            node,
            "default",
        ),
        "for_statement" => push_for_branch(
            db,
            file,
            function,
            function_name,
            function_returns_error,
            source,
            node,
        ),
        "select_statement" => push_select_branch(db, file, function, function_name, node),
        _ => {}
    });
}

fn push_if_branches(
    db: &mut dyn FactDatabase,
    file: FileId,
    function: FunctionId,
    function_name: &str,
    function_returns_error: bool,
    source: &str,
    node: Node<'_>,
) {
    let decision = node.child_by_field_name("condition").unwrap_or(node);
    let condition = if_condition_text(source, node)
        .or_else(|| node_text(source, decision))
        .unwrap_or("if")
        .trim()
        .to_string();
    let span = node_span(db, file, decision);
    let true_body = node
        .child_by_field_name("consequence")
        .or_else(|| node.child_by_field_name("body"));
    let false_body = node.child_by_field_name("alternative");
    let true_is_error_path = branch_body_returns_error(source, true_body, function_returns_error)
        || condition_implies_error_edge(&condition, "true");
    let false_is_error_path = branch_body_returns_error(source, false_body, function_returns_error)
        || condition_implies_error_edge(&condition, "false");
    push_branch(
        db,
        BranchTarget {
            file,
            function,
            function_name,
        },
        span.clone(),
        condition.clone(),
        "true",
        true_is_error_path,
    );
    push_branch(
        db,
        BranchTarget {
            file,
            function,
            function_name,
        },
        span,
        condition,
        "false",
        false_is_error_path,
    );
}

fn push_switch_branch(
    db: &mut dyn FactDatabase,
    file: FileId,
    function: FunctionId,
    function_name: &str,
    source: &str,
    node: Node<'_>,
) {
    let Some((start, end)) = switch_decision_range(source, node) else {
        return;
    };
    let condition = trimmed_text(source, start, end)
        .unwrap_or("switch")
        .to_string();
    let decision_span = trimmed_span(db, file, source, start, end);
    push_branch(
        db,
        BranchTarget {
            file,
            function,
            function_name,
        },
        decision_span,
        condition.clone(),
        "switch",
        is_go_error_path_heuristic(source, &condition, Some(node), false),
    );
}

fn push_case_branch(
    db: &mut dyn FactDatabase,
    target: BranchTarget<'_>,
    function_returns_error: bool,
    source: &str,
    node: Node<'_>,
    edge_label: &str,
) {
    let (start, end) = if edge_label == "default" {
        (node.start_byte(), node.end_byte())
    } else {
        case_header_range(source, node)
    };
    let condition = if edge_label == "default" {
        "default".to_string()
    } else {
        trimmed_text(source, start, end)
            .unwrap_or("case")
            .to_string()
    };
    let decision_span = trimmed_span(db, target.file, source, start, end);
    push_branch(
        db,
        target,
        decision_span,
        condition.clone(),
        edge_label,
        is_go_error_path_heuristic(source, &condition, Some(node), function_returns_error),
    );
}

fn push_for_branch(
    db: &mut dyn FactDatabase,
    file: FileId,
    function: FunctionId,
    function_name: &str,
    function_returns_error: bool,
    source: &str,
    node: Node<'_>,
) {
    if let Some(range) = direct_range_clause(node) {
        let condition = node_text(source, range)
            .unwrap_or("range")
            .trim()
            .to_string();
        let decision_span = node_span(db, file, range);
        push_branch(
            db,
            BranchTarget {
                file,
                function,
                function_name,
            },
            decision_span,
            condition.clone(),
            "range",
            is_go_error_path_heuristic(source, &condition, Some(node), function_returns_error),
        );
        return;
    }

    let (start, end) = statement_header_range(source, node);
    let condition = trimmed_text(source, start, end)
        .unwrap_or("for")
        .to_string();
    let decision_span = trimmed_span(db, file, source, start, end);
    push_branch(
        db,
        BranchTarget {
            file,
            function,
            function_name,
        },
        decision_span,
        condition.clone(),
        "loop",
        is_go_error_path_heuristic(source, &condition, Some(node), function_returns_error),
    );
}

fn direct_range_clause(node: Node<'_>) -> Option<Node<'_>> {
    node.child_by_field_name("clause")
        .filter(|child| child.kind() == "range_clause")
        .or_else(|| first_named_child(node, "range_clause"))
}

fn push_select_branch(
    db: &mut dyn FactDatabase,
    file: FileId,
    function: FunctionId,
    function_name: &str,
    node: Node<'_>,
) {
    let end = node.start_byte().saturating_add("select".len());
    let decision_span = indexed_span(db, file, node.start_byte(), end);
    push_branch(
        db,
        BranchTarget {
            file,
            function,
            function_name,
        },
        decision_span,
        "select".to_string(),
        "select",
        false,
    );
}

fn switch_decision_range(source: &str, node: Node<'_>) -> Option<(usize, usize)> {
    if node.kind() == "expression_switch_statement"
        && let Some(value) = node.child_by_field_name("value")
    {
        return Some((value.start_byte(), value.end_byte()));
    }

    let (start, end) = statement_header_range(source, node);
    let header = source.get(start..end)?;
    let switch_offset = header.find("switch")? + "switch".len();
    let after_switch = start + switch_offset;
    let rest = source.get(after_switch..end)?;
    let leading = rest.len() - rest.trim_start().len();
    let decision_start = after_switch + leading;
    let trimmed_rest = source.get(decision_start..end)?.trim_end();
    if trimmed_rest.is_empty() {
        return Some((start, end));
    }
    Some((decision_start, decision_start + trimmed_rest.len()))
}

fn case_header_range(source: &str, node: Node<'_>) -> (usize, usize) {
    if let Some(case_text) = source.get(node.start_byte()..node.end_byte())
        && let Some(colon_offset) = case_delimiter_colon(case_text)
    {
        return (node.start_byte(), node.start_byte() + colon_offset + 1);
    }

    let Some(after_node) = source.get(node.end_byte()..node.end_byte().saturating_add(16)) else {
        return (node.start_byte(), node.end_byte());
    };
    let colon_offset = after_node.find(':').filter(|offset| {
        after_node[..*offset]
            .chars()
            .all(|character| character.is_whitespace())
    });

    let end = colon_offset
        .map(|offset| node.end_byte() + offset + 1)
        .unwrap_or_else(|| node.end_byte());
    (node.start_byte(), end)
}

fn case_delimiter_colon(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut paren_depth = 0_u32;
    let mut bracket_depth = 0_u32;
    let mut brace_depth = 0_u32;

    while index < bytes.len() {
        match bytes[index] {
            b'"' | b'\'' => index = skip_quoted_literal(bytes, index)?,
            b'`' => index = skip_raw_string_literal(bytes, index)?,
            b'(' => paren_depth += 1,
            b')' => paren_depth = paren_depth.saturating_sub(1),
            b'[' => bracket_depth += 1,
            b']' => bracket_depth = bracket_depth.saturating_sub(1),
            b'{' => brace_depth += 1,
            b'}' => brace_depth = brace_depth.saturating_sub(1),
            b':' if paren_depth == 0
                && bracket_depth == 0
                && brace_depth == 0
                && bytes.get(index + 1) != Some(&b'=') =>
            {
                return Some(index);
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn skip_quoted_literal(bytes: &[u8], start: usize) -> Option<usize> {
    let quote = *bytes.get(start)?;
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            byte if byte == quote => return Some(index),
            _ => index += 1,
        }
    }
    None
}

fn skip_raw_string_literal(bytes: &[u8], start: usize) -> Option<usize> {
    let mut index = start + 1;
    while index < bytes.len() {
        if bytes[index] == b'`' {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn statement_header_range(source: &str, node: Node<'_>) -> (usize, usize) {
    let body_start = node
        .child_by_field_name("body")
        .map(|body| body.start_byte())
        .or_else(|| {
            source
                .get(node.start_byte()..node.end_byte())
                .and_then(|text| text.find('{'))
                .map(|offset| node.start_byte() + offset)
        })
        .unwrap_or_else(|| node.end_byte());

    (node.start_byte(), body_start)
}

fn trimmed_text(source: &str, start: usize, end: usize) -> Option<&str> {
    source.get(start..end).map(str::trim)
}

fn trimmed_span(
    db: &dyn FactDatabase,
    file: FileId,
    source: &str,
    start: usize,
    end: usize,
) -> Span {
    let Some(text) = source.get(start..end) else {
        return indexed_span(db, file, start, end);
    };
    let leading = text.len() - text.trim_start().len();
    let trailing = text.len() - text.trim_end().len();
    indexed_span(db, file, start + leading, end - trailing)
}

#[derive(Clone, Copy)]
struct BranchTarget<'name> {
    file: FileId,
    function: FunctionId,
    function_name: &'name str,
}

fn push_branch(
    db: &mut dyn FactDatabase,
    target: BranchTarget<'_>,
    decision_span: Span,
    condition_text: String,
    edge_label: &str,
    is_error_path: bool,
) -> BranchId {
    let start_line = decision_span.start_line.to_string();
    let start_col = decision_span.start_col.to_string();
    let normalized_condition = normalize_branch_condition(&condition_text);
    let stable_fingerprint = fingerprint(&[
        &db.path_for(target.file),
        target.function_name,
        &start_line,
        &start_col,
        &normalized_condition,
        edge_label,
    ]);
    db.push_branch(BranchObligation::new(
        BranchId::from_raw(0),
        Some(target.function),
        target.file,
        decision_span,
        condition_text,
        edge_label.to_string(),
        is_error_path,
        stable_fingerprint,
    ))
}

fn normalize_branch_condition(condition_text: &str) -> String {
    condition_text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn function_result_contains_error(source: &str, node: Node<'_>) -> bool {
    if node
        .child_by_field_name("result")
        .and_then(|result| node_text(source, result))
        .is_some_and(contains_error_word)
    {
        return true;
    }

    let (start, end) = statement_header_range(source, node);
    let Some(header) = trimmed_text(source, start, end) else {
        return false;
    };
    header
        .rsplit_once(')')
        .is_some_and(|(_, result)| contains_error_word(result))
}

// Syntax-only heuristic: this flags obvious Go error branches, not exact path coverage.
fn is_go_error_path_heuristic(
    source: &str,
    condition_text: &str,
    branch_node: Option<Node<'_>>,
    function_returns_error: bool,
) -> bool {
    condition_implies_error_edge(condition_text, "true")
        || branch_body_returns_error(source, branch_node, function_returns_error)
}

fn condition_implies_error_edge(condition_text: &str, edge_label: &str) -> bool {
    let compact = condition_text
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    match edge_label {
        "true" => {
            compact.contains("err!=nil")
                || compact.contains("errors.is(")
                || compact.contains("errors.as(")
                || condition_text.to_ascii_lowercase().contains("error")
        }
        "false" => compact.contains("err==nil"),
        _ => false,
    }
}

fn branch_body_returns_error(
    source: &str,
    branch_node: Option<Node<'_>>,
    function_returns_error: bool,
) -> bool {
    function_returns_error
        && branch_node.is_some_and(|node| subtree_returns_error_looking_value(source, node))
}

fn subtree_returns_error_looking_value(source: &str, node: Node<'_>) -> bool {
    let mut returns_error = false;
    visit_named_descendants(node, &mut |descendant| {
        if returns_error || descendant.kind() != "return_statement" {
            return;
        }
        returns_error = return_statement_looks_error(source, descendant);
    });
    returns_error
}

fn return_statement_looks_error(source: &str, node: Node<'_>) -> bool {
    let Some(returned) = node_text(source, node)
        .map(str::trim)
        .and_then(|text| text.strip_prefix("return"))
        .map(str::trim)
    else {
        return false;
    };

    returned
        .split(',')
        .map(str::trim)
        .any(error_value_looks_error)
}

fn error_value_looks_error(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let trimmed = lower.trim_matches(|character: char| !character.is_ascii_alphanumeric());
    !matches!(trimmed, "" | "nil" | "true" | "false")
        && (trimmed == "err"
            || trimmed.starts_with("err")
            || trimmed.contains("error")
            || trimmed.contains("errors.")
            || trimmed.contains("fmt.errorf"))
}

fn contains_error_word(text: &str) -> bool {
    text.split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .any(|word| word == "error")
}

#[cfg(test)]
mod subtest_name_tests {
    use super::*;

    #[test]
    fn harvests_literal_t_run_names() {
        let src = r#"
package foo

import "testing"

func TestFoo(t *testing.T) {
    t.Run("case_a", func(t *testing.T) {})
    t.Run(`case_b`, func(t *testing.T) {})
}
"#;
        let mut db = LocalFactDb::new();
        let fid = db.add_file(
            std::path::PathBuf::from("x_test.go"),
            "x_test.go".to_string(),
            src.to_string(),
        );
        let _ = parse_go_file(&mut db, fid).unwrap();
        let names: Vec<_> = db.tests()[0].subtest_names.clone();
        assert_eq!(names, vec!["case_a".to_string(), "case_b".to_string()]);
        assert_eq!(db.tests()[0].subtest_count, 2);
    }
}

#[cfg(test)]
mod parser_identity_tests {
    use super::*;

    fn source() -> SourceFile {
        SourceFile::new(
            FileId::from_raw(0),
            std::path::PathBuf::from("payment.go"),
            "payment.go".to_string(),
            Language::Go,
            "package payment\n".into(),
            "0123456789abcdef".to_string(),
        )
    }

    /// The parser labels are compile-time constants, so the guard is on the
    /// derivation: the layer key's toolchain component must be exactly the one
    /// built from the live labels, and must move when either label moves.
    #[test]
    fn a_changed_parser_label_changes_the_go_syntax_layer_key() {
        let file = source();
        let key = go_syntax_layer_key(&[&file], "config");

        assert_eq!(
            key.toolchain_digest,
            go_parser_toolchain_digest(GO_PARSER_BACKEND, GO_PARSER_GRAMMAR)
        );
        for (backend, grammar) in [
            ("tree-sitter-0.0.0", GO_PARSER_GRAMMAR),
            (GO_PARSER_BACKEND, "tree-sitter-go-0.0.0"),
        ] {
            let moved = LayerCacheKeyParts {
                toolchain_digest: go_parser_toolchain_digest(backend, grammar),
                ..key.clone()
            };
            assert_ne!(moved, key);
        }
    }

    #[test]
    fn the_per_file_cache_key_carries_the_parser_identity() {
        let key = go_file_cache_key(&source(), "config", "rule", "plan");
        assert_eq!(
            key.parser_identity,
            format!("{GO_PARSER_BACKEND}+{GO_PARSER_GRAMMAR}")
        );
    }
}

#[cfg(test)]
mod layer_digest_tests {
    use super::*;
    use crate::go::local_db::LocalFactDb;
    use crate::go::test_cache::FsAnalysisCache;
    use std::sync::atomic::{AtomicU64, Ordering};

    static CACHE_ROOT_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique_cache_root() -> std::path::PathBuf {
        let sequence = CACHE_ROOT_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "polint-go-layer-digest-{}-{sequence}",
            std::process::id()
        ))
    }

    fn payload_for(source: &str) -> (LocalFactDb, SyntaxLayerPayload) {
        let mut db = LocalFactDb::new();
        FactDatabase::add_file(
            &mut db,
            std::path::PathBuf::from("main.go"),
            "main.go".to_string(),
            source.to_string(),
        );
        let owned = db.files().to_vec();
        let files = owned.iter().collect::<Vec<_>>();
        let payload = parse_go_syntax_layer_payload(
            &files,
            &DisabledAnalysisCache,
            "config",
            "rule",
            "",
            false,
        );
        (db, payload)
    }

    #[test]
    fn native_output_digest_survives_a_validated_write_and_read() {
        let cache_root = unique_cache_root();
        let cache = FsAnalysisCache::new(cache_root.join("analysis"), true);
        let (db, payload) = payload_for("package main\nfunc main() {}\n");
        let owned = db.files().to_vec();
        let files = owned.iter().collect::<Vec<_>>();
        let layer_key = go_syntax_layer_key(&files, "config");

        let mut stats = CacheStats::default();
        let mut write_diagnostics = Vec::new();
        let written = write_syntax_layer_payload(
            &cache,
            &layer_key,
            &payload,
            &mut stats,
            &mut write_diagnostics,
        )
        .expect("layer cache write records an output digest");
        assert!(write_diagnostics.is_empty());

        let mut validated_digest = None;
        let read = {
            let mut validate = |bytes: &[u8], digests: LayerCacheEntryDigests<'_>| -> bool {
                let payload = serde_json::from_slice::<SyntaxLayerPayload>(bytes)
                    .expect("written payload parses");
                let recorded = payload
                    .output_digest
                    .clone()
                    .expect("written payload records its native output identity");
                let recomputed = go_syntax_output_digest_for_payload(&payload)
                    .expect("written payload can be independently digested");
                assert_eq!(recorded, recomputed);
                validated_digest = Some(recorded);
                validate_syntax_layer_payload(&payload, digests, GO_SYNTAX_LAYER_SCHEMA, &files)
            };
            cache.read_layer_json(&layer_key, &mut validate)
        };

        assert_eq!(read.status, LayerCacheReadStatus::Hit);
        let validated_digest = validated_digest.expect("a hit runs the validator");
        assert_eq!(read.output_digest, Some(written));
        assert_eq!(read.output_digest, Some(validated_digest));

        std::fs::remove_dir_all(&cache_root).ok();
    }

    /// A blob swapped for a different but well-formed payload must not be
    /// accepted: the payload-digest gate has to fire before the validator sees
    /// bytes the manifest does not describe.
    #[test]
    fn a_substituted_payload_is_evicted_rather_than_reused() {
        let cache_root = unique_cache_root();
        let cache = FsAnalysisCache::new(cache_root.join("analysis"), true);
        let (db, payload) = payload_for("package main\nfunc main() {}\n");
        let owned = db.files().to_vec();
        let files = owned.iter().collect::<Vec<_>>();
        let layer_key = go_syntax_layer_key(&files, "config");

        let mut stats = CacheStats::default();
        let mut write_diagnostics = Vec::new();
        write_syntax_layer_payload(
            &cache,
            &layer_key,
            &payload,
            &mut stats,
            &mut write_diagnostics,
        )
        .expect("layer cache write records an output digest");

        let (_, other) = payload_for("package main\nfunc other() {}\n");
        let payload_bytes = serde_json::to_vec(&payload).expect("payload serializes");
        let payload_digest = DisabledAnalysisCache
            .payload_digest_for_json_bytes(&payload_bytes)
            .expect("payload digest");
        let blob = cache_root
            .join("layers")
            .join("blobs")
            .join(format!("{}.json", payload_digest.value));
        std::fs::write(
            &blob,
            serde_json::to_vec(&other).expect("substitute payload serializes"),
        )
        .expect("write substituted blob");

        let read = {
            let mut validate = |bytes: &[u8], digests: LayerCacheEntryDigests<'_>| -> bool {
                let Ok(payload) = serde_json::from_slice::<SyntaxLayerPayload>(bytes) else {
                    return false;
                };
                validate_syntax_layer_payload(&payload, digests, GO_SYNTAX_LAYER_SCHEMA, &files)
            };
            cache.read_layer_json(&layer_key, &mut validate)
        };

        assert_eq!(read.status, LayerCacheReadStatus::InvalidEvicted);

        std::fs::remove_dir_all(&cache_root).ok();
    }
}
