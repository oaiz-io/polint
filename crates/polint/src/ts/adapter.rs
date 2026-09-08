use crate::analysis_api::{
    AnalysisCache, CacheStats, CachedFileAnalysis, CachedFileFacts, Digest, DigestKind,
    DisabledAnalysisCache, FactDatabase, FileCacheKeyParts, FileCacheReadStatus, FunctionFact,
    ImportFact, JsxAttributeFact, LayerCacheEntryDigests, LayerCacheKeyParts, LayerCacheKind,
    LayerCachePrecision, LayerCacheReadStatus, LayerCacheWriteStatus, ProviderRunResult,
    SourceFile, StringLiteralFact, TS_JS_MODULE_FUNCTION_NAME, TS_PARSER_BACKEND, TsClassFact,
    TsComponentFact,
};
use crate::internal_core::{
    Diagnostic, DiagnosticRange as TextRange, FileId, FunctionId, Language, Span,
};
use anyhow::{Context, Result};
use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, ArrayExpressionElement, ArrowFunctionExpression, AssignmentTarget, BinaryOperator,
    BindingPattern, Class, ClassElement, Declaration, ExportDefaultDeclarationKind, Expression,
    ForStatementInit, ForStatementLeft, Function, FunctionBody, ImportOrExportKind,
    JSXAttributeItem, JSXAttributeName, JSXAttributeValue, JSXChild, JSXElement, JSXExpression,
    JSXFragment, MethodDefinition, MethodDefinitionKind, ModuleExportName, ObjectProperty,
    ObjectPropertyKind, Program, PropertyKey, RegExpLiteral, Statement, TemplateLiteral,
    VariableDeclarator,
};
use oxc_span::GetSpan;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use crate::ts::local_db::LocalFactDb;
use crate::ts::parse::{parse_ts_source, source_type};

const TS_CACHE_SCHEMA: &str = "ts-facts-v16";
const TS_PROVIDER_ID: &str = "polint.ts.syntax";
const TS_SYNTAX_LAYER_SCHEMA: &str = "ts-syntax-layer-v12";

// Relationship resolution converts this non-string import expression sentinel to Dynamic.
pub const DYNAMIC_IMPORT_SPECIFIER: &str = "<dynamic>";

pub(crate) use crate::analysis_api::anonymous_callable_name;

/// The callable name for a class: its own identifier if named, otherwise the
/// anonymous-callable name derived from its span. Shared by the frontend
/// (FunctionFact emission) and MIR lowering so their names always agree — if they
/// diverge, class-expression method bodies stop matching their facts.
pub fn class_callable_name(class: &Class<'_>) -> String {
    class
        .id
        .as_ref()
        .map(|id| id.name.to_string())
        .unwrap_or_else(|| anonymous_callable_name(class.span.start, class.span.end))
}

/// Convenience wrapper used by TS-adapter unit tests (no cache, sequential).
#[cfg(test)]
pub(crate) fn analyze(db: &mut dyn FactDatabase) -> Vec<Diagnostic> {
    let cache = DisabledAnalysisCache;
    analyze_with_options(db, &cache, "", "", false)
}

/// Sequential, cache-aware analysis used by TS-adapter unit tests only.
#[cfg(test)]
pub(crate) fn analyze_with_cache(
    db: &mut dyn FactDatabase,
    cache: &dyn AnalysisCache,
    config_hash: &str,
    rule_hash: &str,
) -> Vec<Diagnostic> {
    analyze_with_options(db, cache, config_hash, rule_hash, false)
}

/// Used by `polint::_bench::ts` / `polint-bench`.
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

pub fn analyze_with_plan_options(
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

pub fn analyze_with_plan_options_and_cache_stats(
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
        .filter(|file| file.language.is_ts_family())
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

pub fn analyze_files_with_plan_options_and_cache_stats(
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
            diagnostics: Vec::new(),
            cache_stats,
            output_digest: None,
            execution: Default::default(),
        };
    }

    let layer_key = ts_syntax_layer_key(files, config_hash);
    // The validator parses the blob to check it; keep that payload so a hit does
    // not deserialize the same bytes a second time.
    let mut validated_payload: Option<SyntaxLayerPayload> = None;
    let read = {
        let mut validate = |bytes: &[u8], digests: LayerCacheEntryDigests<'_>| -> bool {
            let Ok(payload) = serde_json::from_slice::<SyntaxLayerPayload>(bytes) else {
                return false;
            };
            if !validate_syntax_layer_payload(&payload, digests, TS_SYNTAX_LAYER_SCHEMA, files) {
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
                diagnostics: restore_syntax_layer_payload(db, payload),
                cache_stats,
                output_digest: read.output_digest,
                execution: Default::default(),
            }
        }
        LayerCacheReadStatus::BypassedDisabled => {
            cache_stats.record_disabled_bypass();
            cache_stats.record_recompute();
            let payload = parse_ts_syntax_layer_payload(
                files,
                cache,
                config_hash,
                rule_hash,
                plan_digest,
                parallel,
            );
            let output_digest = payload.output_digest.clone();
            ProviderRunResult {
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
            let payload = parse_ts_syntax_layer_payload(
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
                diagnostics,
                cache_stats,
                output_digest,
                execution: Default::default(),
            }
        }
    }
}

struct TsFileAnalysis {
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

fn ts_syntax_layer_key(files: &[&SourceFile], config_hash: &str) -> LayerCacheKeyParts {
    LayerCacheKeyParts {
        layer_kind: LayerCacheKind::TsSyntax,
        provider_id: TS_PROVIDER_ID.to_string(),
        provider_version: env!("CARGO_PKG_VERSION").to_string(),
        schema_version: TS_CACHE_SCHEMA.to_string(),
        parameter_digest: parser_parameter_digest(files),
        lifecycle_digest: Digest::absent(DigestKind::TsJsLifecycle, "ts_syntax_lifecycle_absent"),
        config_digest: Digest::from_parts(DigestKind::Config, "config_hash", &[config_hash]),
        toolchain_digest: ts_parser_toolchain_digest(TS_PARSER_BACKEND),
        input_digests: files.iter().map(|file| source_text_digest(file)).collect(),
    }
}

/// Identity of the toolchain that produced a TypeScript syntax layer: this engine
/// plus the parser it links. Facts are only reusable when both still match, so
/// both belong in the key rather than the engine version alone.
fn ts_parser_toolchain_digest(backend: &str) -> Digest {
    Digest::from_parts(
        DigestKind::ToolInvocation,
        "ts_syntax_parser_toolchain",
        &[env!("CARGO_PKG_VERSION"), backend],
    )
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
        .map(|file| {
            format!(
                "{}={:?}:{:?}",
                file.relative_path,
                file.language,
                source_type(&file.path)
            )
        })
        .collect::<Vec<_>>();
    parts.sort();
    let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    Digest::from_parts(
        DigestKind::ProviderParameters,
        "ts_parser_parameters",
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

fn parse_ts_syntax_layer_payload(
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
            .map(|file| analyze_ts_source_file(file, cache, config_hash, rule_hash, plan_digest))
            .collect::<Vec<_>>()
    } else {
        files
            .iter()
            .map(|file| analyze_ts_source_file(file, cache, config_hash, rule_hash, plan_digest))
            .collect::<Vec<_>>()
    };
    results.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

    let output_digest = results
        .iter()
        .map(|result| result.output_digest.clone())
        .collect::<Option<Vec<_>>>()
        .map(|file_digests| ts_syntax_output_digest(&file_digests));

    SyntaxLayerPayload {
        schema: TS_SYNTAX_LAYER_SCHEMA.to_string(),
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
                "ts syntax layer",
                anyhow::anyhow!(error),
            ));
            return None;
        }
    };
    let payload_digest = match cache.payload_digest_for_json_bytes(&payload_bytes) {
        Ok(digest) => digest,
        Err(error) => {
            diagnostics.push(cache_write_diagnostic(
                "ts syntax layer",
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
                "ts syntax layer",
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

fn ts_file_output_digest_from_bytes(
    relative_path: &str,
    content_hash: &str,
    cached_file_bytes: &[u8],
) -> Digest {
    let mut digest = Digest::builder(DigestKind::ProviderOutput, "ts-syntax-file-output-v1");
    digest.field("path", relative_path);
    digest.field("content-hash", content_hash);
    digest.bytes_field("cached-file", cached_file_bytes);
    digest.finish()
}

fn ts_file_output_digest(
    relative_path: &str,
    content_hash: &str,
    diagnostics: &[Diagnostic],
    facts: Option<&CachedFileFacts>,
) -> Option<Digest> {
    let bytes = serde_json::to_vec(&CachedFileOutput {
        schema: TS_CACHE_SCHEMA,
        diagnostics,
        facts,
    })
    .ok()?;
    Some(ts_file_output_digest_from_bytes(
        relative_path,
        content_hash,
        &bytes,
    ))
}

fn ts_syntax_output_digest(file_digests: &[Digest]) -> Digest {
    let mut digest = Digest::builder(DigestKind::ProviderOutput, "ts-syntax-output-v1");
    digest.field("schema", TS_SYNTAX_LAYER_SCHEMA);
    digest.u64_field("file-count", file_digests.len() as u64);
    for file_digest in file_digests {
        digest.field("file", &file_digest.to_string());
    }
    digest.finish()
}

#[cfg(test)]
fn ts_syntax_output_digest_for_payload(payload: &SyntaxLayerPayload) -> Option<Digest> {
    let file_digests = payload
        .files
        .iter()
        .map(|file| {
            ts_file_output_digest(
                &file.relative_path,
                &file.content_hash,
                &file.diagnostics,
                file.facts.as_ref(),
            )
        })
        .collect::<Option<Vec<_>>>()?;
    Some(ts_syntax_output_digest(&file_digests))
}

fn ts_file_cache_key(
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
        schema: TS_CACHE_SCHEMA.to_string(),
        parser_identity: TS_PARSER_BACKEND.to_string(),
    }
}

fn analyze_ts_source_file(
    file: &SourceFile,
    cache: &dyn AnalysisCache,
    config_hash: &str,
    rule_hash: &str,
    plan_hash: &str,
) -> TsFileAnalysis {
    let key = ts_file_cache_key(file, config_hash, rule_hash, plan_hash);
    let read = cache.read_file_json(&key);
    if read.status == FileCacheReadStatus::Hit
        && let Some(bytes) = read.value
        && let Ok(cached) = serde_json::from_slice::<CachedFileAnalysis>(&bytes)
        && cached.schema == TS_CACHE_SCHEMA
    {
        let output_digest = Some(ts_file_output_digest_from_bytes(
            &file.relative_path,
            &file.content_hash,
            &bytes,
        ));
        return TsFileAnalysis {
            relative_path: file.relative_path.clone(),
            content_hash: file.content_hash.clone(),
            diagnostics: cached.diagnostics,
            facts: Some(cached.facts),
            output_digest,
        };
    }

    let mut local_db = LocalFactDb::new();
    let local_file = local_db.add_source_file_from(file);
    let mut diagnostics = match extract_ts_file_facts(&mut local_db, local_file) {
        Ok(file_diagnostics) => file_diagnostics,
        Err(error) => {
            let diagnostics = vec![Diagnostic::error(
                "parser/ts",
                file.relative_path.clone(),
                TextRange::point(1, 1),
                format!("Failed to parse TS/JS file: {error}"),
            )];
            let output_digest =
                ts_file_output_digest(&file.relative_path, &file.content_hash, &diagnostics, None);
            return TsFileAnalysis {
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
        schema: TS_CACHE_SCHEMA.to_string(),
        diagnostics: diagnostics.clone(),
        facts: facts.clone(),
    };
    let mut output_digest = None;
    if let Ok(bytes) = serde_json::to_vec(&cached) {
        output_digest = Some(ts_file_output_digest_from_bytes(
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
    }
    TsFileAnalysis {
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

fn extract_ts_file_facts(db: &mut dyn FactDatabase, file_id: FileId) -> Result<Vec<Diagnostic>> {
    let (source, language, path) = {
        let file = db.file(file_id).context("missing source file")?;
        (Arc::clone(&file.source), file.language, file.path.clone())
    };
    let source = source.as_ref();
    let allocator = Allocator::default();
    let parsed = parse_ts_source(&allocator, &path, source);
    let mut diagnostics = Vec::new();

    for error in &parsed.errors {
        let range = match (error.start_byte, error.end_byte) {
            (Some(start), Some(end)) => indexed_span(db, file_id, start, end).diagnostic_range(),
            _ => TextRange::point(1, 1),
        };

        diagnostics.push(Diagnostic::error(
            "parser/ts",
            db.path_for(file_id),
            range,
            format!("TS/JS parser reported a syntax error: {}", error.message),
        ));
    }

    if parsed.is_catastrophic() && diagnostics.is_empty() {
        diagnostics.push(Diagnostic::error(
            "parser/ts",
            db.path_for(file_id),
            TextRange::point(1, 1),
            "TS/JS parser reported a syntax error",
        ));
    }

    let import_count = db.imports().len();
    extract_from_program(db, file_id, source, language, parsed.program());
    if db.imports().len() == import_count {
        let mut module_requests = parsed
            .raw
            .module_record
            .requested_modules
            .iter()
            .flat_map(|(path, requests)| {
                requests
                    .iter()
                    .map(move |request| (request.span.start, request.statement_span, path.as_str()))
            })
            .collect::<Vec<_>>();
        module_requests.sort_by_key(|(start, _, _)| *start);

        for (_, statement_span, path) in module_requests {
            let span = span_from_oxc(db, file_id, statement_span);
            push_module_import(db, file_id, path, span, language);
        }
    }
    Ok(diagnostics)
}

fn extract_from_program(
    db: &mut dyn FactDatabase,
    file_id: FileId,
    source: &str,
    language: Language,
    program: &Program<'_>,
) {
    extract_imports_and_exports(db, file_id, source, language, program);
    extract_declarations(db, file_id, source, language, program);
    extract_literals_and_jsx(db, file_id, source, language, program);
}

fn indexed_span(db: &dyn FactDatabase, file: FileId, start_byte: usize, end_byte: usize) -> Span {
    db.file(file)
        .map(|source_file| source_file.span_from_byte_range(start_byte, end_byte))
        .unwrap_or_else(|| Span::point(file, 1, 1))
}

fn span_from_oxc(db: &dyn FactDatabase, file: FileId, span: oxc_span::Span) -> Span {
    indexed_span(db, file, span.start as usize, span.end as usize)
}

fn extract_imports_and_exports(
    db: &mut dyn FactDatabase,
    file: FileId,
    source: &str,
    language: Language,
    program: &Program<'_>,
) {
    for statement in &program.body {
        match statement {
            Statement::ImportDeclaration(declaration) => {
                let span = span_from_oxc(db, file, declaration.source.span);
                push_module_import(db, file, declaration.source.value.as_str(), span, language);
            }
            Statement::ExportAllDeclaration(declaration) => {
                let span = span_from_oxc(db, file, declaration.source.span);
                push_module_import(db, file, declaration.source.value.as_str(), span, language);
            }
            Statement::ExportNamedDeclaration(declaration) => {
                if let Some(module_source) = &declaration.source {
                    let span = span_from_oxc(db, file, module_source.span);
                    push_module_import(db, file, module_source.value.as_str(), span, language);
                }
            }
            _ => {}
        }
    }
    extract_commonjs_require_imports(db, file, source, language, program);
}

fn extract_commonjs_require_imports(
    db: &mut dyn FactDatabase,
    file: FileId,
    source: &str,
    language: Language,
    program: &Program<'_>,
) {
    let ctx = TsAstCtx {
        file,
        source,
        language,
    };
    for statement in &program.body {
        collect_require_imports_from_statement(db, ctx, statement);
    }
}

fn collect_require_imports_from_statement(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    statement: &Statement<'_>,
) {
    match statement {
        Statement::BlockStatement(block) => {
            for statement in &block.body {
                collect_require_imports_from_statement(db, ctx, statement);
            }
        }
        Statement::ExpressionStatement(statement) => {
            collect_require_imports_from_expression(db, ctx, &statement.expression);
        }
        Statement::ReturnStatement(statement) => {
            if let Some(argument) = &statement.argument {
                collect_require_imports_from_expression(db, ctx, argument);
            }
        }
        Statement::IfStatement(statement) => {
            collect_require_imports_from_expression(db, ctx, &statement.test);
            collect_require_imports_from_statement(db, ctx, &statement.consequent);
            if let Some(alternate) = &statement.alternate {
                collect_require_imports_from_statement(db, ctx, alternate);
            }
        }
        Statement::DoWhileStatement(statement) => {
            collect_require_imports_from_statement(db, ctx, &statement.body);
            collect_require_imports_from_expression(db, ctx, &statement.test);
        }
        Statement::WhileStatement(statement) => {
            collect_require_imports_from_expression(db, ctx, &statement.test);
            collect_require_imports_from_statement(db, ctx, &statement.body);
        }
        Statement::ForStatement(statement) => {
            if let Some(init) = &statement.init {
                collect_require_imports_from_for_init(db, ctx, init);
            }
            if let Some(test) = &statement.test {
                collect_require_imports_from_expression(db, ctx, test);
            }
            if let Some(update) = &statement.update {
                collect_require_imports_from_expression(db, ctx, update);
            }
            collect_require_imports_from_statement(db, ctx, &statement.body);
        }
        Statement::ForInStatement(statement) => {
            collect_require_imports_from_expression(db, ctx, &statement.right);
            collect_require_imports_from_statement(db, ctx, &statement.body);
        }
        Statement::ForOfStatement(statement) => {
            collect_require_imports_from_expression(db, ctx, &statement.right);
            collect_require_imports_from_statement(db, ctx, &statement.body);
        }
        Statement::SwitchStatement(statement) => {
            collect_require_imports_from_expression(db, ctx, &statement.discriminant);
            for case in &statement.cases {
                if let Some(test) = &case.test {
                    collect_require_imports_from_expression(db, ctx, test);
                }
                for statement in &case.consequent {
                    collect_require_imports_from_statement(db, ctx, statement);
                }
            }
        }
        Statement::ThrowStatement(statement) => {
            collect_require_imports_from_expression(db, ctx, &statement.argument);
        }
        Statement::TryStatement(statement) => {
            for statement in &statement.block.body {
                collect_require_imports_from_statement(db, ctx, statement);
            }
            if let Some(handler) = &statement.handler {
                for statement in &handler.body.body {
                    collect_require_imports_from_statement(db, ctx, statement);
                }
            }
            if let Some(finalizer) = &statement.finalizer {
                for statement in &finalizer.body {
                    collect_require_imports_from_statement(db, ctx, statement);
                }
            }
        }
        Statement::VariableDeclaration(variable) => {
            for declarator in &variable.declarations {
                if let Some(init) = &declarator.init {
                    collect_require_imports_from_expression(db, ctx, init);
                }
            }
        }
        Statement::FunctionDeclaration(function) => {
            collect_require_imports_from_function(db, ctx, function);
        }
        Statement::ClassDeclaration(class) => {
            collect_require_imports_from_class(db, ctx, class);
        }
        Statement::ExportNamedDeclaration(declaration) => {
            if let Some(declaration) = &declaration.declaration {
                collect_require_imports_from_declaration(db, ctx, declaration);
            }
        }
        Statement::ExportDefaultDeclaration(declaration) => {
            collect_require_imports_from_export_default(db, ctx, &declaration.declaration);
        }
        _ => {}
    }
}

fn collect_require_imports_from_expression(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    expression: &Expression<'_>,
) {
    match expression {
        Expression::CallExpression(call) => {
            if callee_text(&call.callee).as_deref() == Some("require")
                && let Some(Argument::StringLiteral(path)) = call.arguments.first()
            {
                let span = span_from_oxc(db, ctx.file, path.span);
                push_module_import(db, ctx.file, path.value.as_str(), span, ctx.language);
            }
            collect_require_imports_from_expression(db, ctx, &call.callee);
            for argument in &call.arguments {
                collect_require_imports_from_argument(db, ctx, argument);
            }
        }
        Expression::ArrayExpression(array) => {
            for element in &array.elements {
                collect_require_imports_from_array_element(db, ctx, element);
            }
        }
        Expression::AssignmentExpression(expression) => {
            collect_require_imports_from_expression(db, ctx, &expression.right);
        }
        Expression::AwaitExpression(expression) => {
            collect_require_imports_from_expression(db, ctx, &expression.argument);
        }
        Expression::ArrowFunctionExpression(function) => {
            collect_require_imports_from_arrow_function(db, ctx, function);
        }
        Expression::BinaryExpression(expression) => {
            collect_require_imports_from_expression(db, ctx, &expression.left);
            collect_require_imports_from_expression(db, ctx, &expression.right);
        }
        Expression::ClassExpression(class) => collect_require_imports_from_class(db, ctx, class),
        Expression::ConditionalExpression(expression) => {
            collect_require_imports_from_expression(db, ctx, &expression.test);
            collect_require_imports_from_expression(db, ctx, &expression.consequent);
            collect_require_imports_from_expression(db, ctx, &expression.alternate);
        }
        Expression::FunctionExpression(function) => {
            collect_require_imports_from_function(db, ctx, function);
        }
        Expression::ImportExpression(import_expression) => {
            push_dynamic_import_expression(db, ctx, import_expression);
        }
        Expression::LogicalExpression(expression) => {
            collect_require_imports_from_expression(db, ctx, &expression.left);
            collect_require_imports_from_expression(db, ctx, &expression.right);
        }
        Expression::NewExpression(expression) => {
            collect_require_imports_from_expression(db, ctx, &expression.callee);
            for argument in &expression.arguments {
                collect_require_imports_from_argument(db, ctx, argument);
            }
        }
        Expression::ObjectExpression(object) => {
            for property in &object.properties {
                match property {
                    ObjectPropertyKind::ObjectProperty(property) => {
                        collect_require_imports_from_expression(db, ctx, &property.value);
                    }
                    ObjectPropertyKind::SpreadProperty(spread) => {
                        collect_require_imports_from_expression(db, ctx, &spread.argument);
                    }
                }
            }
        }
        Expression::ParenthesizedExpression(expression) => {
            collect_require_imports_from_expression(db, ctx, &expression.expression);
        }
        Expression::SequenceExpression(expression) => {
            for expression in &expression.expressions {
                collect_require_imports_from_expression(db, ctx, expression);
            }
        }
        Expression::TaggedTemplateExpression(tagged) => {
            collect_require_imports_from_expression(db, ctx, &tagged.tag);
            for expression in &tagged.quasi.expressions {
                collect_require_imports_from_expression(db, ctx, expression);
            }
        }
        Expression::UnaryExpression(expression) => {
            collect_require_imports_from_expression(db, ctx, &expression.argument);
        }
        Expression::YieldExpression(expression) => {
            if let Some(argument) = &expression.argument {
                collect_require_imports_from_expression(db, ctx, argument);
            }
        }
        Expression::TSAsExpression(expression) => {
            collect_require_imports_from_expression(db, ctx, &expression.expression);
        }
        Expression::TSSatisfiesExpression(expression) => {
            collect_require_imports_from_expression(db, ctx, &expression.expression);
        }
        Expression::TSNonNullExpression(expression) => {
            collect_require_imports_from_expression(db, ctx, &expression.expression);
        }
        Expression::TSTypeAssertion(expression) => {
            collect_require_imports_from_expression(db, ctx, &expression.expression);
        }
        Expression::TSInstantiationExpression(expression) => {
            collect_require_imports_from_expression(db, ctx, &expression.expression);
        }
        Expression::ComputedMemberExpression(expression) => {
            collect_require_imports_from_expression(db, ctx, &expression.object);
            collect_require_imports_from_expression(db, ctx, &expression.expression);
        }
        Expression::StaticMemberExpression(expression) => {
            collect_require_imports_from_expression(db, ctx, &expression.object);
        }
        Expression::PrivateFieldExpression(expression) => {
            collect_require_imports_from_expression(db, ctx, &expression.object);
        }
        _ => {}
    }
}

fn collect_require_imports_from_declaration(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    declaration: &Declaration<'_>,
) {
    match declaration {
        Declaration::VariableDeclaration(variable) => {
            for declarator in &variable.declarations {
                if let Some(init) = &declarator.init {
                    collect_require_imports_from_expression(db, ctx, init);
                }
            }
        }
        Declaration::FunctionDeclaration(function) => {
            collect_require_imports_from_function(db, ctx, function);
        }
        Declaration::ClassDeclaration(class) => {
            collect_require_imports_from_class(db, ctx, class);
        }
        _ => {}
    }
}

fn collect_require_imports_from_export_default(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    declaration: &ExportDefaultDeclarationKind<'_>,
) {
    match declaration {
        ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
            collect_require_imports_from_function(db, ctx, function);
        }
        ExportDefaultDeclarationKind::ClassDeclaration(class) => {
            collect_require_imports_from_class(db, ctx, class);
        }
        ExportDefaultDeclarationKind::CallExpression(call) => {
            if callee_text(&call.callee).as_deref() == Some("require")
                && let Some(Argument::StringLiteral(path)) = call.arguments.first()
            {
                let span = span_from_oxc(db, ctx.file, path.span);
                push_module_import(db, ctx.file, path.value.as_str(), span, ctx.language);
            }
            for argument in &call.arguments {
                collect_require_imports_from_argument(db, ctx, argument);
            }
        }
        _ => {}
    }
}

fn collect_require_imports_from_function(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    function: &Function<'_>,
) {
    if let Some(body) = function.body.as_deref() {
        for statement in &body.statements {
            collect_require_imports_from_statement(db, ctx, statement);
        }
    }
}

fn collect_require_imports_from_arrow_function(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    function: &ArrowFunctionExpression<'_>,
) {
    if let Some(expression) = function.get_expression() {
        collect_require_imports_from_expression(db, ctx, expression);
    } else {
        for statement in &function.body.statements {
            collect_require_imports_from_statement(db, ctx, statement);
        }
    }
}

fn collect_require_imports_from_class(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    class: &Class<'_>,
) {
    if let Some(super_class) = &class.super_class {
        collect_require_imports_from_expression(db, ctx, super_class);
    }
    for element in &class.body.body {
        match element {
            ClassElement::StaticBlock(block) => {
                for statement in &block.body {
                    collect_require_imports_from_statement(db, ctx, statement);
                }
            }
            ClassElement::MethodDefinition(method) => {
                collect_require_imports_from_function(db, ctx, &method.value);
            }
            ClassElement::PropertyDefinition(property) => {
                if let Some(value) = &property.value {
                    collect_require_imports_from_expression(db, ctx, value);
                }
            }
            ClassElement::AccessorProperty(property) => {
                if let Some(value) = &property.value {
                    collect_require_imports_from_expression(db, ctx, value);
                }
            }
            ClassElement::TSIndexSignature(_) => {}
        }
    }
}

fn collect_require_imports_from_for_init(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    init: &ForStatementInit<'_>,
) {
    match init {
        ForStatementInit::VariableDeclaration(variable) => {
            for declarator in &variable.declarations {
                if let Some(init) = &declarator.init {
                    collect_require_imports_from_expression(db, ctx, init);
                }
            }
        }
        ForStatementInit::CallExpression(call) => {
            if callee_text(&call.callee).as_deref() == Some("require")
                && let Some(Argument::StringLiteral(path)) = call.arguments.first()
            {
                let span = span_from_oxc(db, ctx.file, path.span);
                push_module_import(db, ctx.file, path.value.as_str(), span, ctx.language);
            }
            for argument in &call.arguments {
                collect_require_imports_from_argument(db, ctx, argument);
            }
        }
        ForStatementInit::ParenthesizedExpression(expression) => {
            collect_require_imports_from_expression(db, ctx, &expression.expression);
        }
        _ => {}
    }
}

fn collect_require_imports_from_argument(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    argument: &Argument<'_>,
) {
    match argument {
        Argument::SpreadElement(spread) => {
            collect_require_imports_from_expression(db, ctx, &spread.argument);
        }
        Argument::CallExpression(call) => {
            if callee_text(&call.callee).as_deref() == Some("require")
                && let Some(Argument::StringLiteral(path)) = call.arguments.first()
            {
                let span = span_from_oxc(db, ctx.file, path.span);
                push_module_import(db, ctx.file, path.value.as_str(), span, ctx.language);
            }
            for argument in &call.arguments {
                collect_require_imports_from_argument(db, ctx, argument);
            }
        }
        Argument::ParenthesizedExpression(expression) => {
            collect_require_imports_from_expression(db, ctx, &expression.expression);
        }
        Argument::ImportExpression(import_expression) => {
            push_dynamic_import_expression(db, ctx, import_expression);
        }
        _ => {}
    }
}

fn push_dynamic_import_expression(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    import_expression: &oxc_ast::ast::ImportExpression<'_>,
) {
    match &import_expression.source {
        Expression::StringLiteral(path) => {
            let span = span_from_oxc(db, ctx.file, path.span);
            push_module_import(db, ctx.file, path.value.as_str(), span, ctx.language);
        }
        _ => {
            let span = span_from_oxc(db, ctx.file, import_expression.span);
            push_module_import(db, ctx.file, DYNAMIC_IMPORT_SPECIFIER, span, ctx.language);
        }
    }
}

fn collect_require_imports_from_array_element(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    element: &ArrayExpressionElement<'_>,
) {
    if let Some(expression) = element.as_expression() {
        collect_require_imports_from_expression(db, ctx, expression);
    } else if let ArrayExpressionElement::SpreadElement(spread) = element {
        collect_require_imports_from_expression(db, ctx, &spread.argument);
    }
}

fn push_module_import(
    db: &mut dyn FactDatabase,
    file: FileId,
    path: &str,
    span: Span,
    language: Language,
) {
    db.push_import(ImportFact::new(
        crate::internal_core::ImportId::from_raw(0),
        file,
        None,
        path.to_string(),
        span,
        language,
    ));
}

#[derive(Clone, Copy)]
struct TsAstCtx<'a> {
    file: FileId,
    source: &'a str,
    language: Language,
}

fn extract_declarations(
    db: &mut dyn FactDatabase,
    file: FileId,
    source: &str,
    language: Language,
    program: &Program<'_>,
) {
    let ctx = TsAstCtx {
        file,
        source,
        language,
    };
    push_ts_module_function(db, ctx);
    let exported_names = exported_local_names(program);
    for statement in &program.body {
        match statement {
            Statement::FunctionDeclaration(function) => {
                if let Some(name) = function.id.as_ref().map(|id| id.name.to_string()) {
                    let is_exported = exported_names.contains(name.as_str());
                    let is_component_like =
                        is_component_like_name(&name) || function_returns_jsx(function);
                    push_ts_function(
                        db,
                        ctx,
                        TsFunctionSpec {
                            name,
                            span: function.span,
                            is_exported,
                            cyclomatic_complexity: ts_cyclomatic_complexity(function),
                            calls: function_body_calls(function.body.as_deref()),
                            is_component_like,
                        },
                    );
                }
            }
            Statement::VariableDeclaration(variable) => {
                for declarator in &variable.declarations {
                    extract_variable_declarator(
                        db,
                        file,
                        source,
                        language,
                        declarator,
                        false,
                        &exported_names,
                    );
                }
            }
            Statement::ClassDeclaration(class) => {
                if let Some(name) = class.id.as_ref().map(|id| id.name.to_string()) {
                    let is_exported = exported_names.contains(name.as_str());
                    push_ts_class(db, file, source, language, name, class, is_exported);
                }
            }
            Statement::ExpressionStatement(statement) => {
                extract_commonjs_exported_function_assignment(db, ctx, &statement.expression);
            }
            Statement::ExportNamedDeclaration(export) => {
                if let Some(declaration) = &export.declaration {
                    extract_declaration(
                        db,
                        file,
                        source,
                        language,
                        declaration,
                        true,
                        &exported_names,
                    );
                }
            }
            Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                    if let Some(name) = function.id.as_ref().map(|id| id.name.to_string()) {
                        let is_component_like =
                            is_component_like_name(&name) || function_returns_jsx(function);
                        push_ts_function(
                            db,
                            ctx,
                            TsFunctionSpec {
                                name,
                                span: function.span,
                                is_exported: true,
                                cyclomatic_complexity: ts_cyclomatic_complexity(function),
                                calls: function_body_calls(function.body.as_deref()),
                                is_component_like,
                            },
                        );
                    }
                }
                ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                    if let Some(name) = class.id.as_ref().map(|id| id.name.to_string()) {
                        push_ts_class(db, file, source, language, name, class, true);
                    }
                }
                _ => {}
            },
            _ => {}
        }
        extract_anonymous_callables_from_statement(db, ctx, statement);
    }
}

fn push_ts_module_function(db: &mut dyn FactDatabase, ctx: TsAstCtx<'_>) -> FunctionId {
    db.push_function(FunctionFact::new(
        FunctionId::from_raw(0),
        ctx.file,
        TS_JS_MODULE_FUNCTION_NAME.to_string(),
        indexed_span(db, ctx.file, 0, ctx.source.len()),
        ctx.language,
        false,
        false,
        1,
        Vec::new(),
    ))
}

fn extract_commonjs_exported_function_assignment(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    expression: &Expression<'_>,
) {
    let Expression::AssignmentExpression(assignment) = expression else {
        return;
    };
    let Expression::FunctionExpression(function) = &assignment.right else {
        return;
    };
    let Some(export_name) = commonjs_export_assignment_name(&assignment.left) else {
        return;
    };
    let is_component_like = is_component_like_name(&export_name) || function_returns_jsx(function);

    push_ts_function(
        db,
        ctx,
        TsFunctionSpec {
            name: export_name,
            span: function.span,
            is_exported: true,
            cyclomatic_complexity: ts_cyclomatic_complexity(function),
            calls: function_body_calls(function.body.as_deref()),
            is_component_like,
        },
    );
}

fn commonjs_export_assignment_name(target: &AssignmentTarget<'_>) -> Option<String> {
    commonjs_export_property_name(target)
}

fn commonjs_export_property_name(target: &AssignmentTarget<'_>) -> Option<String> {
    match target {
        AssignmentTarget::StaticMemberExpression(member)
            if matches!(
                callee_text(&member.object).as_deref(),
                Some("exports" | "module.exports")
            ) =>
        {
            Some(member.property.name.to_string())
        }
        _ => None,
    }
}

fn extract_declaration(
    db: &mut dyn FactDatabase,
    file: FileId,
    source: &str,
    language: Language,
    declaration: &Declaration<'_>,
    is_exported: bool,
    exported_names: &BTreeSet<String>,
) {
    let ctx = TsAstCtx {
        file,
        source,
        language,
    };
    match declaration {
        Declaration::FunctionDeclaration(function) => {
            if let Some(name) = function.id.as_ref().map(|id| id.name.to_string()) {
                let is_component_like =
                    is_component_like_name(&name) || function_returns_jsx(function);
                push_ts_function(
                    db,
                    ctx,
                    TsFunctionSpec {
                        name,
                        span: function.span,
                        is_exported,
                        cyclomatic_complexity: ts_cyclomatic_complexity(function),
                        calls: function_body_calls(function.body.as_deref()),
                        is_component_like,
                    },
                );
            }
        }
        Declaration::VariableDeclaration(variable) => {
            for declarator in &variable.declarations {
                extract_variable_declarator(
                    db,
                    file,
                    source,
                    language,
                    declarator,
                    is_exported,
                    exported_names,
                );
            }
        }
        Declaration::ClassDeclaration(class) => {
            if let Some(name) = class.id.as_ref().map(|id| id.name.to_string()) {
                push_ts_class(db, file, source, language, name, class, is_exported);
            }
        }
        _ => {}
    }
}

fn extract_variable_declarator(
    db: &mut dyn FactDatabase,
    file: FileId,
    source: &str,
    language: Language,
    declarator: &VariableDeclarator<'_>,
    is_exported: bool,
    exported_names: &BTreeSet<String>,
) {
    let ctx = TsAstCtx {
        file,
        source,
        language,
    };
    let BindingPattern::BindingIdentifier(name) = &declarator.id else {
        return;
    };
    let Some(init) = &declarator.init else {
        return;
    };

    let name = name.name.to_string();
    let is_exported = is_exported || exported_names.contains(name.as_str());
    match init {
        Expression::ArrowFunctionExpression(function) => {
            let is_component_like = is_component_like_name(&name) || arrow_returns_jsx(function);
            push_ts_function(
                db,
                ctx,
                TsFunctionSpec {
                    name,
                    span: function.span,
                    is_exported,
                    cyclomatic_complexity: arrow_cyclomatic_complexity(function),
                    calls: function_body_calls(Some(&function.body)),
                    is_component_like,
                },
            );
        }
        Expression::FunctionExpression(function) => {
            let is_component_like = is_component_like_name(&name) || function_returns_jsx(function);
            push_ts_function(
                db,
                ctx,
                TsFunctionSpec {
                    name,
                    span: function.span,
                    is_exported,
                    cyclomatic_complexity: ts_cyclomatic_complexity(function),
                    calls: function_body_calls(function.body.as_deref()),
                    is_component_like,
                },
            );
        }
        Expression::ClassExpression(class) => {
            push_ts_class(db, file, source, language, name, class, is_exported);
        }
        _ => {}
    }
}

fn extract_anonymous_callables_from_statement(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    statement: &Statement<'_>,
) {
    match statement {
        Statement::BlockStatement(block) => {
            for statement in &block.body {
                extract_anonymous_callables_from_statement(db, ctx, statement);
            }
        }
        Statement::ExpressionStatement(statement) => {
            extract_anonymous_callables_from_expression(db, ctx, &statement.expression, true);
        }
        Statement::VariableDeclaration(variable) => {
            for declarator in &variable.declarations {
                // Default values in a destructuring pattern (`{a = () => {}} = x`)
                // are callables that flow to the bound name; emit their facts.
                extract_anonymous_callables_from_binding_pattern(db, ctx, &declarator.id);
                if let Some(init) = &declarator.init {
                    // `include_self = true`: emit the init function/arrow itself
                    // (`const x = function baz(){}` inside a body), not just its
                    // body. At top level this declarator was already emitted under
                    // its variable name by `extract_variable_declarator` (which
                    // runs first), so the (file, span) dedup guard keeps that and
                    // drops this; only NESTED var-init functions — which have no
                    // other emission path — are newly materialized as targets.
                    extract_anonymous_callables_from_expression(db, ctx, init, true);
                }
            }
        }
        Statement::FunctionDeclaration(function) => {
            // Nested function declarations (inside a function/block body) are not
            // visited by the top-level declaration path; emit their facts here so a
            // call can resolve TO them. Without a FunctionFact there is no
            // FunctionId, so neither the recognizer nor the points-to heap (which
            // maps value tokens -> FunctionId by span) can emit an edge to a nested
            // function — the dominant call-graph recall gap. Idempotent via
            // `push_ts_function`'s (file, span) guard. Mirrors the nested-class arm.
            if let Some(name) = function.id.as_ref().map(|id| id.name.to_string()) {
                let is_component_like =
                    is_component_like_name(&name) || function_returns_jsx(function);
                push_ts_function(
                    db,
                    ctx,
                    TsFunctionSpec {
                        name,
                        span: function.span,
                        is_exported: false,
                        cyclomatic_complexity: ts_cyclomatic_complexity(function),
                        calls: function_body_calls(function.body.as_deref()),
                        is_component_like,
                    },
                );
            }
            if let Some(body) = function.body.as_deref() {
                extract_anonymous_callables_from_function_body(db, ctx, body);
            }
        }
        Statement::ClassDeclaration(class) => {
            // Nested class declarations (inside a function/block) are not visited by the
            // top-level declaration path; emit their facts here. Idempotent for top-level
            // classes already pushed via `push_ts_class`.
            if let Some(name) = class.id.as_ref().map(|id| id.name.to_string()) {
                push_ts_class(db, ctx.file, ctx.source, ctx.language, name, class, false);
            }
            extract_anonymous_callables_from_class(db, ctx, class);
        }
        Statement::ReturnStatement(statement) => {
            if let Some(argument) = &statement.argument {
                extract_anonymous_callables_from_expression(db, ctx, argument, true);
            }
        }
        Statement::ThrowStatement(statement) => {
            extract_anonymous_callables_from_expression(db, ctx, &statement.argument, true);
        }
        Statement::IfStatement(statement) => {
            extract_anonymous_callables_from_expression(db, ctx, &statement.test, true);
            extract_anonymous_callables_from_statement(db, ctx, &statement.consequent);
            if let Some(alternate) = &statement.alternate {
                extract_anonymous_callables_from_statement(db, ctx, alternate);
            }
        }
        Statement::WhileStatement(statement) => {
            extract_anonymous_callables_from_expression(db, ctx, &statement.test, true);
            extract_anonymous_callables_from_statement(db, ctx, &statement.body);
        }
        Statement::DoWhileStatement(statement) => {
            extract_anonymous_callables_from_statement(db, ctx, &statement.body);
            extract_anonymous_callables_from_expression(db, ctx, &statement.test, true);
        }
        Statement::ForStatement(statement) => {
            if let Some(init) = &statement.init {
                match init {
                    ForStatementInit::VariableDeclaration(variable) => {
                        for declarator in &variable.declarations {
                            if let Some(init) = &declarator.init {
                                extract_anonymous_callables_from_expression(db, ctx, init, false);
                            }
                        }
                    }
                    _ => extract_anonymous_callables_from_expression(
                        db,
                        ctx,
                        init.to_expression(),
                        true,
                    ),
                }
            }
            if let Some(test) = &statement.test {
                extract_anonymous_callables_from_expression(db, ctx, test, true);
            }
            if let Some(update) = &statement.update {
                extract_anonymous_callables_from_expression(db, ctx, update, true);
            }
            extract_anonymous_callables_from_statement(db, ctx, &statement.body);
        }
        Statement::ForInStatement(statement) => {
            extract_anonymous_callables_from_expression(db, ctx, &statement.right, true);
            extract_anonymous_callables_from_statement(db, ctx, &statement.body);
        }
        Statement::ForOfStatement(statement) => {
            extract_anonymous_callables_from_expression(db, ctx, &statement.right, true);
            extract_anonymous_callables_from_statement(db, ctx, &statement.body);
        }
        Statement::SwitchStatement(statement) => {
            extract_anonymous_callables_from_expression(db, ctx, &statement.discriminant, true);
            for case in &statement.cases {
                if let Some(test) = &case.test {
                    extract_anonymous_callables_from_expression(db, ctx, test, true);
                }
                for statement in &case.consequent {
                    extract_anonymous_callables_from_statement(db, ctx, statement);
                }
            }
        }
        Statement::ExportNamedDeclaration(export) => {
            if let Some(declaration) = &export.declaration {
                extract_anonymous_callables_from_declaration(db, ctx, declaration);
            }
        }
        Statement::ExportDefaultDeclaration(export) => match &export.declaration {
            ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                if let Some(body) = function.body.as_deref() {
                    extract_anonymous_callables_from_function_body(db, ctx, body);
                }
            }
            ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                extract_anonymous_callables_from_class(db, ctx, class);
            }
            ExportDefaultDeclarationKind::CallExpression(call) => {
                extract_anonymous_callables_from_expression(db, ctx, &call.callee, true);
                for argument in &call.arguments {
                    extract_anonymous_callables_from_argument(db, ctx, argument);
                }
            }
            _ => {}
        },
        _ => {}
    }
}

fn extract_anonymous_callables_from_declaration(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    declaration: &Declaration<'_>,
) {
    match declaration {
        Declaration::FunctionDeclaration(function) => {
            if let Some(body) = function.body.as_deref() {
                extract_anonymous_callables_from_function_body(db, ctx, body);
            }
        }
        Declaration::ClassDeclaration(class) => {
            extract_anonymous_callables_from_class(db, ctx, class);
        }
        Declaration::VariableDeclaration(variable) => {
            for declarator in &variable.declarations {
                if let Some(init) = &declarator.init {
                    extract_anonymous_callables_from_expression(db, ctx, init, false);
                }
            }
        }
        _ => {}
    }
}

/// Walk a binding pattern for destructuring default values (`{a = expr}`,
/// `[b = expr]`) and extract any anonymous callables they contain, so a default
/// arrow/function flows to the bound name as a resolvable call target.
fn extract_anonymous_callables_from_binding_pattern(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    pattern: &BindingPattern<'_>,
) {
    match pattern {
        BindingPattern::BindingIdentifier(_) => {}
        BindingPattern::ObjectPattern(object) => {
            for property in &object.properties {
                extract_anonymous_callables_from_binding_pattern(db, ctx, &property.value);
            }
            if let Some(rest) = &object.rest {
                extract_anonymous_callables_from_binding_pattern(db, ctx, &rest.argument);
            }
        }
        BindingPattern::ArrayPattern(array) => {
            for element in array.elements.iter().flatten() {
                extract_anonymous_callables_from_binding_pattern(db, ctx, element);
            }
            if let Some(rest) = &array.rest {
                extract_anonymous_callables_from_binding_pattern(db, ctx, &rest.argument);
            }
        }
        BindingPattern::AssignmentPattern(assignment) => {
            extract_anonymous_callables_from_expression(db, ctx, &assignment.right, true);
            extract_anonymous_callables_from_binding_pattern(db, ctx, &assignment.left);
        }
    }
}

fn extract_anonymous_callables_from_class(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    class: &Class<'_>,
) {
    if let Some(super_class) = &class.super_class {
        extract_anonymous_callables_from_expression(db, ctx, super_class, true);
    }
    for element in &class.body.body {
        match element {
            ClassElement::StaticBlock(block) => {
                for statement in &block.body {
                    extract_anonymous_callables_from_statement(db, ctx, statement);
                }
            }
            ClassElement::MethodDefinition(method) => {
                if let Some(body) = method.value.body.as_deref() {
                    extract_anonymous_callables_from_function_body(db, ctx, body);
                }
            }
            ClassElement::PropertyDefinition(property) => {
                if let Some(value) = &property.value {
                    extract_anonymous_callables_from_expression(db, ctx, value, true);
                }
            }
            ClassElement::AccessorProperty(property) => {
                if let Some(value) = &property.value {
                    extract_anonymous_callables_from_expression(db, ctx, value, true);
                }
            }
            ClassElement::TSIndexSignature(_) => {}
        }
    }
}

fn extract_anonymous_callables_from_function_body(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    body: &FunctionBody<'_>,
) {
    for statement in &body.statements {
        extract_anonymous_callables_from_statement(db, ctx, statement);
    }
}

fn extract_anonymous_callables_from_expression(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    expression: &Expression<'_>,
    include_self: bool,
) {
    match expression {
        Expression::ArrowFunctionExpression(function) => {
            if include_self {
                push_ts_function(
                    db,
                    ctx,
                    TsFunctionSpec {
                        name: anonymous_callable_name(function.span.start, function.span.end),
                        span: function.span,
                        is_exported: false,
                        cyclomatic_complexity: arrow_cyclomatic_complexity(function),
                        calls: function_body_calls(Some(&function.body)),
                        is_component_like: arrow_returns_jsx(function),
                    },
                );
            }
            extract_anonymous_callables_from_function_body(db, ctx, &function.body);
        }
        Expression::FunctionExpression(function) => {
            if include_self {
                push_ts_function(
                    db,
                    ctx,
                    TsFunctionSpec {
                        name: anonymous_callable_name(function.span.start, function.span.end),
                        span: function.span,
                        is_exported: false,
                        cyclomatic_complexity: ts_cyclomatic_complexity(function),
                        calls: function_body_calls(function.body.as_deref()),
                        is_component_like: function_returns_jsx(function),
                    },
                );
            }
            if let Some(body) = function.body.as_deref() {
                extract_anonymous_callables_from_function_body(db, ctx, body);
            }
        }
        Expression::ClassExpression(class) => {
            // Emit constructor + method FunctionFacts so value-flow can resolve calls
            // inside a class expression (e.g. `return class extends A { m() { super.m() } }`).
            push_ts_class(
                db,
                ctx.file,
                ctx.source,
                ctx.language,
                class_callable_name(class),
                class,
                false,
            );
            extract_anonymous_callables_from_class(db, ctx, class);
        }
        Expression::CallExpression(call) => {
            extract_anonymous_callables_from_expression(db, ctx, &call.callee, true);
            for argument in &call.arguments {
                extract_anonymous_callables_from_argument(db, ctx, argument);
            }
        }
        Expression::NewExpression(expression) => {
            extract_anonymous_callables_from_expression(db, ctx, &expression.callee, true);
            for argument in &expression.arguments {
                extract_anonymous_callables_from_argument(db, ctx, argument);
            }
        }
        Expression::StaticMemberExpression(member) => {
            extract_anonymous_callables_from_expression(db, ctx, &member.object, true);
        }
        Expression::ComputedMemberExpression(member) => {
            extract_anonymous_callables_from_expression(db, ctx, &member.object, true);
            extract_anonymous_callables_from_expression(db, ctx, &member.expression, true);
        }
        Expression::PrivateFieldExpression(member) => {
            extract_anonymous_callables_from_expression(db, ctx, &member.object, true);
        }
        Expression::ChainExpression(chain) => {
            extract_anonymous_callables_from_chain_element(db, ctx, &chain.expression);
        }
        Expression::TaggedTemplateExpression(tagged) => {
            // The tag and each interpolation (`tag`…${() => {}}…``) need callable
            // facts so the tagged-template call and its arguments resolve.
            extract_anonymous_callables_from_expression(db, ctx, &tagged.tag, true);
            for expression in &tagged.quasi.expressions {
                extract_anonymous_callables_from_expression(db, ctx, expression, true);
            }
        }
        Expression::ParenthesizedExpression(expression) => {
            extract_anonymous_callables_from_expression(
                db,
                ctx,
                &expression.expression,
                include_self,
            );
        }
        Expression::AssignmentExpression(expression) => {
            extract_anonymous_callables_from_expression(db, ctx, &expression.right, true);
        }
        Expression::AwaitExpression(expression) => {
            extract_anonymous_callables_from_expression(db, ctx, &expression.argument, true);
        }
        Expression::YieldExpression(expression) => {
            if let Some(argument) = &expression.argument {
                extract_anonymous_callables_from_expression(db, ctx, argument, true);
            }
        }
        Expression::SequenceExpression(expression) => {
            for expression in &expression.expressions {
                extract_anonymous_callables_from_expression(db, ctx, expression, true);
            }
        }
        Expression::ConditionalExpression(expression) => {
            extract_anonymous_callables_from_expression(db, ctx, &expression.test, true);
            extract_anonymous_callables_from_expression(db, ctx, &expression.consequent, true);
            extract_anonymous_callables_from_expression(db, ctx, &expression.alternate, true);
        }
        Expression::LogicalExpression(expression) => {
            extract_anonymous_callables_from_expression(db, ctx, &expression.left, true);
            extract_anonymous_callables_from_expression(db, ctx, &expression.right, true);
        }
        Expression::ObjectExpression(object) => {
            for property in &object.properties {
                match property {
                    ObjectPropertyKind::ObjectProperty(property) => {
                        extract_anonymous_callables_from_object_property(db, ctx, property);
                    }
                    ObjectPropertyKind::SpreadProperty(spread) => {
                        extract_anonymous_callables_from_expression(
                            db,
                            ctx,
                            &spread.argument,
                            true,
                        );
                    }
                }
            }
        }
        Expression::ArrayExpression(array) => {
            for element in &array.elements {
                match element {
                    ArrayExpressionElement::SpreadElement(spread) => {
                        extract_anonymous_callables_from_expression(
                            db,
                            ctx,
                            &spread.argument,
                            true,
                        );
                    }
                    ArrayExpressionElement::Elision(_) => {}
                    _ => {
                        extract_anonymous_callables_from_expression(
                            db,
                            ctx,
                            element.to_expression(),
                            true,
                        );
                    }
                }
            }
        }
        _ => {}
    }
}

fn extract_anonymous_callables_from_chain_element(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    element: &oxc_ast::ast::ChainElement<'_>,
) {
    match element {
        oxc_ast::ast::ChainElement::CallExpression(call) => {
            extract_anonymous_callables_from_expression(db, ctx, &call.callee, true);
            for argument in &call.arguments {
                extract_anonymous_callables_from_argument(db, ctx, argument);
            }
        }
        oxc_ast::ast::ChainElement::StaticMemberExpression(member) => {
            extract_anonymous_callables_from_expression(db, ctx, &member.object, true);
        }
        oxc_ast::ast::ChainElement::ComputedMemberExpression(member) => {
            extract_anonymous_callables_from_expression(db, ctx, &member.object, true);
            extract_anonymous_callables_from_expression(db, ctx, &member.expression, true);
        }
        oxc_ast::ast::ChainElement::PrivateFieldExpression(member) => {
            extract_anonymous_callables_from_expression(db, ctx, &member.object, true);
        }
        oxc_ast::ast::ChainElement::TSNonNullExpression(expression) => {
            extract_anonymous_callables_from_expression(db, ctx, &expression.expression, true);
        }
    }
}

fn extract_anonymous_callables_from_argument(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    argument: &Argument<'_>,
) {
    match argument {
        Argument::SpreadElement(spread) => {
            extract_anonymous_callables_from_expression(db, ctx, &spread.argument, true);
        }
        Argument::ArrowFunctionExpression(function) => {
            push_ts_function(
                db,
                ctx,
                TsFunctionSpec {
                    name: anonymous_callable_name(function.span.start, function.span.end),
                    span: function.span,
                    is_exported: false,
                    cyclomatic_complexity: arrow_cyclomatic_complexity(function),
                    calls: function_body_calls(Some(&function.body)),
                    is_component_like: arrow_returns_jsx(function),
                },
            );
            extract_anonymous_callables_from_function_body(db, ctx, &function.body);
        }
        Argument::FunctionExpression(function) => {
            push_ts_function(
                db,
                ctx,
                TsFunctionSpec {
                    name: anonymous_callable_name(function.span.start, function.span.end),
                    span: function.span,
                    is_exported: false,
                    cyclomatic_complexity: ts_cyclomatic_complexity(function),
                    calls: function_body_calls(function.body.as_deref()),
                    is_component_like: function_returns_jsx(function),
                },
            );
            if let Some(body) = function.body.as_deref() {
                extract_anonymous_callables_from_function_body(db, ctx, body);
            }
        }
        _ => {
            extract_anonymous_callables_from_expression(db, ctx, argument.to_expression(), true);
        }
    }
}

fn exported_local_names(program: &Program<'_>) -> BTreeSet<String> {
    let mut names = BTreeSet::new();

    for statement in &program.body {
        match statement {
            Statement::ExportNamedDeclaration(export)
                if export.source.is_none()
                    && matches!(export.export_kind, ImportOrExportKind::Value) =>
            {
                for specifier in &export.specifiers {
                    if matches!(specifier.export_kind, ImportOrExportKind::Value)
                        && let Some(name) = module_export_name_text(&specifier.local)
                    {
                        names.insert(name);
                    }
                }
            }
            Statement::ExportDefaultDeclaration(export) => {
                if let ExportDefaultDeclarationKind::Identifier(identifier) = &export.declaration {
                    names.insert(identifier.name.to_string());
                }
            }
            _ => {}
        }
    }

    names
}

fn module_export_name_text(name: &ModuleExportName<'_>) -> Option<String> {
    match name {
        ModuleExportName::IdentifierName(identifier) => Some(identifier.name.to_string()),
        ModuleExportName::IdentifierReference(identifier) => Some(identifier.name.to_string()),
        ModuleExportName::StringLiteral(literal) => Some(literal.value.to_string()),
    }
}

struct TsFunctionSpec {
    name: String,
    span: oxc_span::Span,
    is_exported: bool,
    cyclomatic_complexity: u32,
    calls: Vec<String>,
    is_component_like: bool,
}

fn push_ts_function(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    mut spec: TsFunctionSpec,
) -> FunctionId {
    spec.calls.sort();
    spec.calls.dedup();
    let span = span_from_oxc(db, ctx.file, spec.span);
    // Idempotent by (file, span): a function reached via body recursion (nested
    // declarations) must not duplicate a fact already emitted at a top-level
    // position (call argument, array element, …). `push_function` does no dedup,
    // so duplicate same-span FunctionFacts would make span-keyed resolution
    // (`function_for_span`) order-dependent. Mirrors `push_ts_class`'s guard.
    if let Some(existing) = db.functions().iter().find(|function| {
        function.file == ctx.file
            && function.span.start_byte == span.start_byte
            && function.span.end_byte == span.end_byte
    }) {
        return existing.id;
    }
    let function_id = db.push_function(FunctionFact::new(
        FunctionId::from_raw(0),
        ctx.file,
        spec.name.clone(),
        span.clone(),
        ctx.language,
        spec.name.contains("test") || spec.name.contains("spec"),
        spec.is_exported,
        spec.cyclomatic_complexity,
        spec.calls,
    ));

    if spec.is_component_like {
        // syntax-level component heuristic: PascalCase or JSX-returning syntax only.
        db.push_ts_component(TsComponentFact::new(
            ctx.file,
            Some(function_id),
            spec.name,
            span,
        ));
    }

    function_id
}

fn push_ts_class(
    db: &mut dyn FactDatabase,
    file: FileId,
    source: &str,
    language: Language,
    name: String,
    class: &Class<'_>,
    is_exported: bool,
) {
    let is_component_like = is_component_like_name(&name);
    let span = span_from_oxc(db, file, class.span);
    // Idempotent by class span: a class expression / nested class declaration reached
    // via the anonymous-callable walk must not duplicate facts already emitted by the
    // top-level declaration or `var x = class` declarator path (duplicate FunctionFacts
    // would surface as duplicate graph nodes / false-positive edges). Check the
    // (per-file, much smaller) class-fact set rather than scanning every function.
    if db.ts_classes().iter().any(|existing| {
        existing.file == file
            && existing.span.start_byte == span.start_byte
            && existing.span.end_byte == span.end_byte
    }) {
        return;
    }
    db.push_ts_class(TsClassFact::new(
        file,
        name.clone(),
        span.clone(),
        is_exported,
        is_component_like,
    ));
    // The class function *is* the constructor callable, so it carries the
    // explicit `constructor(){}` body's facts. No separate `C.constructor`
    // function is emitted below: two owners for one body would split call
    // attribution between the class and the member span.
    let constructor = class_constructor(class);
    push_ts_function(
        db,
        TsAstCtx {
            file,
            source,
            language,
        },
        TsFunctionSpec {
            name: name.clone(),
            span: class.span,
            is_exported,
            cyclomatic_complexity: constructor
                .map(|method| ts_cyclomatic_complexity(&method.value))
                .unwrap_or(1),
            calls: constructor
                .map(|method| function_body_calls(method.value.body.as_deref()))
                .unwrap_or_default(),
            is_component_like: false,
        },
    );

    if is_component_like {
        // syntax-level component heuristic: PascalCase classes are component-like only.
        db.push_ts_component(TsComponentFact::new(file, None, name.clone(), span));
    }

    for element in &class.body.body {
        match element {
            // Folded into the class function above.
            ClassElement::MethodDefinition(method)
                if method.kind == MethodDefinitionKind::Constructor => {}
            ClassElement::MethodDefinition(method) => {
                let Some(method_name) = method_name(method) else {
                    continue;
                };
                push_ts_function(
                    db,
                    TsAstCtx {
                        file,
                        source,
                        language,
                    },
                    TsFunctionSpec {
                        name: format!("{name}.{method_name}"),
                        span: class_method_function_span(method, source),
                        is_exported,
                        cyclomatic_complexity: ts_cyclomatic_complexity(&method.value),
                        calls: function_body_calls(method.value.body.as_deref()),
                        is_component_like: false,
                    },
                );
            }
            // A class static block (`static { … }`) is its own function in Jelly's
            // model — it runs at class-definition time and its direct calls
            // (`super.f()`, top-level helpers) are attributed to it. Emit a
            // span-named FunctionFact so the MIR static-block body (named the same
            // way) links to it and the value-flow `super`/`this` resolver finds an
            // owner for the block's call sites.
            ClassElement::StaticBlock(block) => {
                push_ts_function(
                    db,
                    TsAstCtx {
                        file,
                        source,
                        language,
                    },
                    TsFunctionSpec {
                        name: anonymous_callable_name(block.span.start, block.span.end),
                        span: block.span,
                        is_exported: false,
                        cyclomatic_complexity: 1,
                        calls: static_block_calls(block),
                        is_component_like: false,
                    },
                );
            }
            // A class field initializer (`x = <expr>`) is its own function in
            // Jelly's model — calls in the initializer are attributed to it. Emit a
            // span-named FunctionFact (key→property end, the same one MIR uses) so
            // the lowered initializer's call sites have an owner and the value-flow
            // can walk it with `this`/`super` bound.
            ClassElement::PropertyDefinition(property) => {
                let Some(value) = &property.value else {
                    continue;
                };
                // Start at the key (after any `static`), end at the property's end
                // (includes the trailing `;`) to match Jelly's field-init span.
                let span = oxc_span::Span::new(property.key.span().start, property.span.end);
                push_ts_function(
                    db,
                    TsAstCtx {
                        file,
                        source,
                        language,
                    },
                    TsFunctionSpec {
                        name: anonymous_callable_name(span.start, span.end),
                        span,
                        is_exported: false,
                        cyclomatic_complexity: 1,
                        calls: function_expression_calls(value),
                        is_component_like: false,
                    },
                );
            }
            _ => {}
        }
    }
}

fn function_expression_calls(expression: &Expression<'_>) -> Vec<String> {
    let mut calls = Vec::new();
    collect_calls_from_expression(expression, &mut calls);
    calls.sort();
    calls.dedup();
    calls
}

fn static_block_calls(block: &oxc_ast::ast::StaticBlock<'_>) -> Vec<String> {
    let mut calls = Vec::new();
    for statement in &block.body {
        collect_calls_from_statement(statement, &mut calls);
    }
    calls.sort();
    calls.dedup();
    calls
}

fn method_name(method: &MethodDefinition<'_>) -> Option<String> {
    match &method.key {
        PropertyKey::StaticIdentifier(identifier) => Some(identifier.name.to_string()),
        PropertyKey::PrivateIdentifier(identifier) => Some(format!("#{}", identifier.name)),
        PropertyKey::StringLiteral(literal) => Some(literal.value.to_string()),
        PropertyKey::BinaryExpression(binary) if binary.operator == BinaryOperator::Addition => {
            Some(format!(
                "{}{}",
                constant_property_key_expression(&binary.left)?,
                constant_property_key_expression(&binary.right)?
            ))
        }
        _ => None,
    }
}

fn constant_property_key_expression(expression: &Expression<'_>) -> Option<String> {
    match expression {
        Expression::StringLiteral(literal) => Some(literal.value.to_string()),
        Expression::BinaryExpression(binary) if binary.operator == BinaryOperator::Addition => {
            Some(format!(
                "{}{}",
                constant_property_key_expression(&binary.left)?,
                constant_property_key_expression(&binary.right)?
            ))
        }
        Expression::ParenthesizedExpression(expression) => {
            constant_property_key_expression(&expression.expression)
        }
        _ => None,
    }
}

fn extract_anonymous_callables_from_object_property(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    property: &ObjectProperty<'_>,
) {
    if property.method
        && let Expression::FunctionExpression(function) = &property.value
    {
        let span = object_method_function_span(property);
        push_ts_function(
            db,
            ctx,
            TsFunctionSpec {
                name: anonymous_callable_name(span.start, span.end),
                span,
                is_exported: false,
                cyclomatic_complexity: ts_cyclomatic_complexity(function),
                calls: function_body_calls(function.body.as_deref()),
                is_component_like: function_returns_jsx(function),
            },
        );
        if let Some(body) = function.body.as_deref() {
            extract_anonymous_callables_from_function_body(db, ctx, body);
        }
        return;
    }

    extract_anonymous_callables_from_expression(db, ctx, &property.value, true);
}

/// The span Jelly assigns to an object shorthand method (`{ m() {} }`): the
/// property span, except that a string-literal key contributes its *contents*
/// — `{ "m"() {} }` starts at `m`, not at the quote. Shared by the frontend
/// FunctionFact emission, MIR lowering and the callable-flow collector so the
/// span-keyed fact, the MIR body and the model rows always agree.
pub(crate) fn object_method_function_span(property: &ObjectProperty<'_>) -> oxc_span::Span {
    match &property.key {
        PropertyKey::StringLiteral(literal) if !property.computed => {
            oxc_span::Span::new(literal.span.start.saturating_add(1), property.span.end)
        }
        _ => property.span,
    }
}

/// The class's explicit `constructor(){}` member, if it declares one.
fn class_constructor<'ast>(class: &'ast Class<'ast>) -> Option<&'ast MethodDefinition<'ast>> {
    class.body.body.iter().find_map(|element| match element {
        ClassElement::MethodDefinition(method)
            if method.kind == MethodDefinitionKind::Constructor =>
        {
            Some(&**method)
        }
        _ => None,
    })
}

fn class_method_function_span(method: &MethodDefinition<'_>, source: &str) -> oxc_span::Span {
    if method.r#static && method.kind == MethodDefinitionKind::Method {
        // Drop the `static` modifier but keep the whole key syntax. For a
        // computed key (`static ["x"]() {}`) Jelly's function span starts at the
        // `[` bracket, not the inner key expression. Scan back over any
        // whitespace to the `[` (so `static [ "x" ]() {}` is handled too), falling
        // back to the key start. A plain key (`static foo()`) already starts at
        // the identifier.
        let key_start = method.key.span().start;
        let start = if method.computed {
            computed_key_bracket_start(source, key_start).unwrap_or(key_start)
        } else {
            key_start
        };
        return oxc_span::Span::new(start, method.span.end);
    }
    method.span
}

/// Byte offset of the `[` opening a computed key whose inner expression starts at
/// `key_start`, scanning back over whitespace. `None` if the preceding
/// non-whitespace byte is not `[` (defensive — leaves the span at the key).
fn computed_key_bracket_start(source: &str, key_start: u32) -> Option<u32> {
    let bytes = source.as_bytes();
    let mut index = (key_start as usize).min(bytes.len());
    while index > 0 {
        index -= 1;
        match bytes[index] {
            b'[' => return Some(index as u32),
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return None,
        }
    }
    None
}

fn expression_calls(expression: &Expression<'_>) -> Vec<String> {
    let mut calls = Vec::new();
    collect_calls_from_expression(expression, &mut calls);
    calls.sort();
    calls.dedup();
    calls
}

fn function_body_calls(body: Option<&FunctionBody<'_>>) -> Vec<String> {
    let mut calls = Vec::new();
    if let Some(body) = body {
        for statement in &body.statements {
            collect_calls_from_statement(statement, &mut calls);
        }
    }
    calls.sort();
    calls.dedup();
    calls
}

fn collect_calls_from_statement(statement: &Statement<'_>, calls: &mut Vec<String>) {
    match statement {
        Statement::BlockStatement(block) => {
            for statement in &block.body {
                collect_calls_from_statement(statement, calls);
            }
        }
        Statement::ExpressionStatement(statement) => {
            calls.extend(expression_calls(&statement.expression));
        }
        Statement::ReturnStatement(statement) => {
            if let Some(argument) = &statement.argument {
                calls.extend(expression_calls(argument));
            }
        }
        Statement::IfStatement(statement) => {
            calls.extend(expression_calls(&statement.test));
            collect_calls_from_statement(&statement.consequent, calls);
            if let Some(alternate) = &statement.alternate {
                collect_calls_from_statement(alternate, calls);
            }
        }
        Statement::DoWhileStatement(statement) => {
            collect_calls_from_statement(&statement.body, calls);
            calls.extend(expression_calls(&statement.test));
        }
        Statement::WhileStatement(statement) => {
            calls.extend(expression_calls(&statement.test));
            collect_calls_from_statement(&statement.body, calls);
        }
        Statement::ForStatement(statement) => {
            if let Some(init) = &statement.init {
                collect_calls_from_for_init(init, calls);
            }
            if let Some(test) = &statement.test {
                calls.extend(expression_calls(test));
            }
            if let Some(update) = &statement.update {
                calls.extend(expression_calls(update));
            }
            collect_calls_from_statement(&statement.body, calls);
        }
        Statement::ForInStatement(statement) => {
            collect_calls_from_for_left(&statement.left, calls);
            calls.extend(expression_calls(&statement.right));
            collect_calls_from_statement(&statement.body, calls);
        }
        Statement::ForOfStatement(statement) => {
            collect_calls_from_for_left(&statement.left, calls);
            calls.extend(expression_calls(&statement.right));
            collect_calls_from_statement(&statement.body, calls);
        }
        Statement::SwitchStatement(statement) => {
            calls.extend(expression_calls(&statement.discriminant));
            for case in &statement.cases {
                if let Some(test) = &case.test {
                    calls.extend(expression_calls(test));
                }
                for statement in &case.consequent {
                    collect_calls_from_statement(statement, calls);
                }
            }
        }
        Statement::ThrowStatement(statement) => {
            calls.extend(expression_calls(&statement.argument));
        }
        Statement::TryStatement(statement) => {
            for statement in &statement.block.body {
                collect_calls_from_statement(statement, calls);
            }
            if let Some(handler) = &statement.handler {
                for statement in &handler.body.body {
                    collect_calls_from_statement(statement, calls);
                }
            }
            if let Some(finalizer) = &statement.finalizer {
                for statement in &finalizer.body {
                    collect_calls_from_statement(statement, calls);
                }
            }
        }
        Statement::VariableDeclaration(variable) => {
            for declarator in &variable.declarations {
                if let Some(init) = &declarator.init {
                    collect_calls_from_expression(init, calls);
                }
            }
        }
        _ => {}
    }
}

fn collect_calls_from_for_init(init: &ForStatementInit<'_>, calls: &mut Vec<String>) {
    if let ForStatementInit::VariableDeclaration(variable) = init {
        for declarator in &variable.declarations {
            if let Some(init) = &declarator.init {
                collect_calls_from_expression(init, calls);
            }
        }
    }
}

fn collect_calls_from_for_left(left: &ForStatementLeft<'_>, calls: &mut Vec<String>) {
    if let ForStatementLeft::VariableDeclaration(variable) = left {
        for declarator in &variable.declarations {
            if let Some(init) = &declarator.init {
                collect_calls_from_expression(init, calls);
            }
        }
    }
}

fn collect_calls_from_expression(expression: &Expression<'_>, calls: &mut Vec<String>) {
    match expression {
        Expression::CallExpression(call) => {
            if let Some(name) = callee_text(&call.callee) {
                calls.push(name);
            }
            collect_calls_from_expression(&call.callee, calls);
            for argument in &call.arguments {
                collect_calls_from_argument(argument, calls);
            }
        }
        Expression::StaticMemberExpression(member) => {
            collect_calls_from_expression(&member.object, calls);
        }
        Expression::ArrayExpression(array) => {
            for element in &array.elements {
                collect_calls_from_array_element(element, calls);
            }
        }
        Expression::AssignmentExpression(expression) => {
            collect_calls_from_expression(&expression.right, calls);
        }
        Expression::AwaitExpression(expression) => {
            collect_calls_from_expression(&expression.argument, calls);
        }
        Expression::BinaryExpression(expression) => {
            collect_calls_from_expression(&expression.left, calls);
            collect_calls_from_expression(&expression.right, calls);
        }
        Expression::ConditionalExpression(expression) => {
            collect_calls_from_expression(&expression.test, calls);
            collect_calls_from_expression(&expression.consequent, calls);
            collect_calls_from_expression(&expression.alternate, calls);
        }
        Expression::LogicalExpression(expression) => {
            collect_calls_from_expression(&expression.left, calls);
            collect_calls_from_expression(&expression.right, calls);
        }
        Expression::NewExpression(expression) => {
            if let Some(name) = callee_text(&expression.callee) {
                calls.push(name);
            }
            collect_calls_from_expression(&expression.callee, calls);
            for argument in &expression.arguments {
                collect_calls_from_argument(argument, calls);
            }
        }
        Expression::ObjectExpression(object) => {
            for property in &object.properties {
                match property {
                    ObjectPropertyKind::ObjectProperty(property) => {
                        collect_calls_from_property_key(&property.key, calls);
                        collect_calls_from_expression(&property.value, calls);
                    }
                    ObjectPropertyKind::SpreadProperty(spread) => {
                        collect_calls_from_expression(&spread.argument, calls);
                    }
                }
            }
        }
        Expression::ParenthesizedExpression(expression) => {
            collect_calls_from_expression(&expression.expression, calls);
        }
        Expression::SequenceExpression(expression) => {
            for expression in &expression.expressions {
                collect_calls_from_expression(expression, calls);
            }
        }
        Expression::TaggedTemplateExpression(tagged) => {
            collect_calls_from_expression(&tagged.tag, calls);
            for expression in &tagged.quasi.expressions {
                collect_calls_from_expression(expression, calls);
            }
        }
        Expression::UnaryExpression(expression) => {
            collect_calls_from_expression(&expression.argument, calls);
        }
        Expression::YieldExpression(expression) => {
            if let Some(argument) = &expression.argument {
                collect_calls_from_expression(argument, calls);
            }
        }
        Expression::JSXElement(element) => collect_calls_from_jsx_element(element, calls),
        Expression::JSXFragment(fragment) => collect_calls_from_jsx_fragment(fragment, calls),
        Expression::TSAsExpression(expression) => {
            collect_calls_from_expression(&expression.expression, calls);
        }
        Expression::TSSatisfiesExpression(expression) => {
            collect_calls_from_expression(&expression.expression, calls);
        }
        Expression::TSNonNullExpression(expression) => {
            collect_calls_from_expression(&expression.expression, calls);
        }
        Expression::TSTypeAssertion(expression) => {
            collect_calls_from_expression(&expression.expression, calls);
        }
        Expression::TSInstantiationExpression(expression) => {
            collect_calls_from_expression(&expression.expression, calls);
        }
        Expression::ComputedMemberExpression(expression) => {
            collect_calls_from_expression(&expression.object, calls);
            collect_calls_from_expression(&expression.expression, calls);
        }
        Expression::PrivateFieldExpression(expression) => {
            collect_calls_from_expression(&expression.object, calls);
        }
        _ => {}
    }
}

fn ts_cyclomatic_complexity(function: &Function<'_>) -> u32 {
    1 + function
        .body
        .as_deref()
        .map_or(0, |body| complexity_from_statements(&body.statements))
}

fn arrow_cyclomatic_complexity(function: &ArrowFunctionExpression<'_>) -> u32 {
    1 + function.get_expression().map_or_else(
        || complexity_from_statements(&function.body.statements),
        complexity_from_expression,
    )
}

fn complexity_from_statements(statements: &[Statement<'_>]) -> u32 {
    statements.iter().map(complexity_from_statement).sum()
}

fn complexity_from_statement(statement: &Statement<'_>) -> u32 {
    match statement {
        Statement::BlockStatement(block) => complexity_from_statements(&block.body),
        Statement::ExpressionStatement(statement) => {
            complexity_from_expression(&statement.expression)
        }
        Statement::ReturnStatement(statement) => statement
            .argument
            .as_ref()
            .map_or(0, complexity_from_expression),
        Statement::IfStatement(statement) => {
            1 + complexity_from_expression(&statement.test)
                + complexity_from_statement(&statement.consequent)
                + statement
                    .alternate
                    .as_ref()
                    .map_or(0, |alternate| complexity_from_statement(alternate))
        }
        Statement::DoWhileStatement(statement) => {
            1 + complexity_from_statement(&statement.body)
                + complexity_from_expression(&statement.test)
        }
        Statement::WhileStatement(statement) => {
            1 + complexity_from_expression(&statement.test)
                + complexity_from_statement(&statement.body)
        }
        Statement::ForStatement(statement) => {
            1 + statement.init.as_ref().map_or(0, complexity_from_for_init)
                + statement
                    .test
                    .as_ref()
                    .map_or(0, complexity_from_expression)
                + statement
                    .update
                    .as_ref()
                    .map_or(0, complexity_from_expression)
                + complexity_from_statement(&statement.body)
        }
        Statement::ForInStatement(statement) => {
            1 + complexity_from_for_left(&statement.left)
                + complexity_from_expression(&statement.right)
                + complexity_from_statement(&statement.body)
        }
        Statement::ForOfStatement(statement) => {
            1 + complexity_from_for_left(&statement.left)
                + complexity_from_expression(&statement.right)
                + complexity_from_statement(&statement.body)
        }
        Statement::SwitchStatement(statement) => {
            complexity_from_expression(&statement.discriminant)
                + statement
                    .cases
                    .iter()
                    .map(|case| {
                        1 + case.test.as_ref().map_or(0, complexity_from_expression)
                            + complexity_from_statements(&case.consequent)
                    })
                    .sum::<u32>()
        }
        Statement::ThrowStatement(statement) => complexity_from_expression(&statement.argument),
        Statement::TryStatement(statement) => {
            complexity_from_statements(&statement.block.body)
                + statement.handler.as_ref().map_or(0, |handler| {
                    1 + complexity_from_statements(&handler.body.body)
                })
                + statement
                    .finalizer
                    .as_ref()
                    .map_or(0, |finalizer| complexity_from_statements(&finalizer.body))
        }
        Statement::VariableDeclaration(variable) => complexity_from_variable_declaration(variable),
        Statement::WithStatement(statement) => {
            complexity_from_expression(&statement.object)
                + complexity_from_statement(&statement.body)
        }
        Statement::LabeledStatement(statement) => complexity_from_statement(&statement.body),
        _ => 0,
    }
}

fn complexity_from_expression(expression: &Expression<'_>) -> u32 {
    match expression {
        Expression::ArrayExpression(array) => array
            .elements
            .iter()
            .map(complexity_from_array_element)
            .sum(),
        Expression::AssignmentExpression(expression) => {
            complexity_from_expression(&expression.right)
        }
        Expression::AwaitExpression(expression) => complexity_from_expression(&expression.argument),
        Expression::BinaryExpression(expression) => {
            complexity_from_expression(&expression.left)
                + complexity_from_expression(&expression.right)
        }
        Expression::CallExpression(call) => {
            complexity_from_expression(&call.callee)
                + call
                    .arguments
                    .iter()
                    .map(complexity_from_argument)
                    .sum::<u32>()
        }
        Expression::ConditionalExpression(expression) => {
            1 + complexity_from_expression(&expression.test)
                + complexity_from_expression(&expression.consequent)
                + complexity_from_expression(&expression.alternate)
        }
        Expression::ImportExpression(expression) => complexity_from_expression(&expression.source),
        Expression::LogicalExpression(expression) => {
            u32::from(expression.operator.is_and() || expression.operator.is_or())
                + complexity_from_expression(&expression.left)
                + complexity_from_expression(&expression.right)
        }
        Expression::NewExpression(expression) => {
            complexity_from_expression(&expression.callee)
                + expression
                    .arguments
                    .iter()
                    .map(complexity_from_argument)
                    .sum::<u32>()
        }
        Expression::ObjectExpression(object) => object
            .properties
            .iter()
            .map(|property| match property {
                ObjectPropertyKind::ObjectProperty(property) => {
                    complexity_from_property_key(&property.key)
                        + complexity_from_expression(&property.value)
                }
                ObjectPropertyKind::SpreadProperty(spread) => {
                    complexity_from_expression(&spread.argument)
                }
            })
            .sum(),
        Expression::ParenthesizedExpression(expression) => {
            complexity_from_expression(&expression.expression)
        }
        Expression::SequenceExpression(expression) => expression
            .expressions
            .iter()
            .map(complexity_from_expression)
            .sum(),
        Expression::TaggedTemplateExpression(tagged) => {
            complexity_from_expression(&tagged.tag)
                + tagged
                    .quasi
                    .expressions
                    .iter()
                    .map(complexity_from_expression)
                    .sum::<u32>()
        }
        Expression::UnaryExpression(expression) => complexity_from_expression(&expression.argument),
        Expression::YieldExpression(expression) => expression
            .argument
            .as_ref()
            .map_or(0, complexity_from_expression),
        Expression::JSXElement(element) => complexity_from_jsx_element(element),
        Expression::JSXFragment(fragment) => complexity_from_jsx_fragment(fragment),
        Expression::TSAsExpression(expression) => {
            complexity_from_expression(&expression.expression)
        }
        Expression::TSSatisfiesExpression(expression) => {
            complexity_from_expression(&expression.expression)
        }
        Expression::TSNonNullExpression(expression) => {
            complexity_from_expression(&expression.expression)
        }
        Expression::TSTypeAssertion(expression) => {
            complexity_from_expression(&expression.expression)
        }
        Expression::TSInstantiationExpression(expression) => {
            complexity_from_expression(&expression.expression)
        }
        Expression::ComputedMemberExpression(expression) => {
            complexity_from_expression(&expression.object)
                + complexity_from_expression(&expression.expression)
        }
        Expression::StaticMemberExpression(expression) => {
            complexity_from_expression(&expression.object)
        }
        Expression::PrivateFieldExpression(expression) => {
            complexity_from_expression(&expression.object)
        }
        _ => 0,
    }
}

fn complexity_from_variable_declaration(variable: &oxc_ast::ast::VariableDeclaration<'_>) -> u32 {
    variable
        .declarations
        .iter()
        .filter_map(|declarator| declarator.init.as_ref())
        .map(complexity_from_expression)
        .sum()
}

fn complexity_from_for_init(init: &ForStatementInit<'_>) -> u32 {
    match init {
        ForStatementInit::VariableDeclaration(variable) => {
            complexity_from_variable_declaration(variable)
        }
        ForStatementInit::ArrayExpression(array) => array
            .elements
            .iter()
            .map(complexity_from_array_element)
            .sum(),
        ForStatementInit::AssignmentExpression(expression) => {
            complexity_from_expression(&expression.right)
        }
        ForStatementInit::CallExpression(call) => {
            complexity_from_expression(&call.callee)
                + call
                    .arguments
                    .iter()
                    .map(complexity_from_argument)
                    .sum::<u32>()
        }
        ForStatementInit::ConditionalExpression(expression) => {
            1 + complexity_from_expression(&expression.test)
                + complexity_from_expression(&expression.consequent)
                + complexity_from_expression(&expression.alternate)
        }
        ForStatementInit::LogicalExpression(expression) => {
            u32::from(expression.operator.is_and() || expression.operator.is_or())
                + complexity_from_expression(&expression.left)
                + complexity_from_expression(&expression.right)
        }
        ForStatementInit::ParenthesizedExpression(expression) => {
            complexity_from_expression(&expression.expression)
        }
        ForStatementInit::SequenceExpression(expression) => expression
            .expressions
            .iter()
            .map(complexity_from_expression)
            .sum(),
        ForStatementInit::TSAsExpression(expression) => {
            complexity_from_expression(&expression.expression)
        }
        ForStatementInit::TSSatisfiesExpression(expression) => {
            complexity_from_expression(&expression.expression)
        }
        ForStatementInit::TSNonNullExpression(expression) => {
            complexity_from_expression(&expression.expression)
        }
        ForStatementInit::TSTypeAssertion(expression) => {
            complexity_from_expression(&expression.expression)
        }
        _ => 0,
    }
}

fn complexity_from_for_left(left: &ForStatementLeft<'_>) -> u32 {
    if let ForStatementLeft::VariableDeclaration(variable) = left {
        complexity_from_variable_declaration(variable)
    } else {
        0
    }
}

fn complexity_from_argument(argument: &Argument<'_>) -> u32 {
    match argument {
        Argument::SpreadElement(spread) => complexity_from_expression(&spread.argument),
        Argument::ArrayExpression(array) => array
            .elements
            .iter()
            .map(complexity_from_array_element)
            .sum(),
        Argument::AssignmentExpression(expression) => complexity_from_expression(&expression.right),
        Argument::BinaryExpression(expression) => {
            complexity_from_expression(&expression.left)
                + complexity_from_expression(&expression.right)
        }
        Argument::CallExpression(call) => {
            complexity_from_expression(&call.callee)
                + call
                    .arguments
                    .iter()
                    .map(complexity_from_argument)
                    .sum::<u32>()
        }
        Argument::ConditionalExpression(expression) => {
            1 + complexity_from_expression(&expression.test)
                + complexity_from_expression(&expression.consequent)
                + complexity_from_expression(&expression.alternate)
        }
        Argument::LogicalExpression(expression) => {
            u32::from(expression.operator.is_and() || expression.operator.is_or())
                + complexity_from_expression(&expression.left)
                + complexity_from_expression(&expression.right)
        }
        Argument::ObjectExpression(object) => object
            .properties
            .iter()
            .map(|property| match property {
                ObjectPropertyKind::ObjectProperty(property) => {
                    complexity_from_property_key(&property.key)
                        + complexity_from_expression(&property.value)
                }
                ObjectPropertyKind::SpreadProperty(spread) => {
                    complexity_from_expression(&spread.argument)
                }
            })
            .sum(),
        Argument::ParenthesizedExpression(expression) => {
            complexity_from_expression(&expression.expression)
        }
        Argument::SequenceExpression(expression) => expression
            .expressions
            .iter()
            .map(complexity_from_expression)
            .sum(),
        Argument::TaggedTemplateExpression(tagged) => {
            complexity_from_expression(&tagged.tag)
                + tagged
                    .quasi
                    .expressions
                    .iter()
                    .map(complexity_from_expression)
                    .sum::<u32>()
        }
        Argument::UnaryExpression(expression) => complexity_from_expression(&expression.argument),
        Argument::JSXElement(element) => complexity_from_jsx_element(element),
        Argument::JSXFragment(fragment) => complexity_from_jsx_fragment(fragment),
        Argument::TSAsExpression(expression) => complexity_from_expression(&expression.expression),
        Argument::TSSatisfiesExpression(expression) => {
            complexity_from_expression(&expression.expression)
        }
        Argument::TSNonNullExpression(expression) => {
            complexity_from_expression(&expression.expression)
        }
        Argument::TSTypeAssertion(expression) => complexity_from_expression(&expression.expression),
        _ => 0,
    }
}

fn complexity_from_array_element(element: &ArrayExpressionElement<'_>) -> u32 {
    match element {
        ArrayExpressionElement::SpreadElement(spread) => {
            complexity_from_expression(&spread.argument)
        }
        ArrayExpressionElement::ArrayExpression(array) => array
            .elements
            .iter()
            .map(complexity_from_array_element)
            .sum(),
        ArrayExpressionElement::AssignmentExpression(expression) => {
            complexity_from_expression(&expression.right)
        }
        ArrayExpressionElement::CallExpression(call) => {
            complexity_from_expression(&call.callee)
                + call
                    .arguments
                    .iter()
                    .map(complexity_from_argument)
                    .sum::<u32>()
        }
        ArrayExpressionElement::ConditionalExpression(expression) => {
            1 + complexity_from_expression(&expression.test)
                + complexity_from_expression(&expression.consequent)
                + complexity_from_expression(&expression.alternate)
        }
        ArrayExpressionElement::LogicalExpression(expression) => {
            u32::from(expression.operator.is_and() || expression.operator.is_or())
                + complexity_from_expression(&expression.left)
                + complexity_from_expression(&expression.right)
        }
        ArrayExpressionElement::ObjectExpression(object) => object
            .properties
            .iter()
            .map(|property| match property {
                ObjectPropertyKind::ObjectProperty(property) => {
                    complexity_from_property_key(&property.key)
                        + complexity_from_expression(&property.value)
                }
                ObjectPropertyKind::SpreadProperty(spread) => {
                    complexity_from_expression(&spread.argument)
                }
            })
            .sum(),
        ArrayExpressionElement::ParenthesizedExpression(expression) => {
            complexity_from_expression(&expression.expression)
        }
        ArrayExpressionElement::SequenceExpression(expression) => expression
            .expressions
            .iter()
            .map(complexity_from_expression)
            .sum(),
        ArrayExpressionElement::TaggedTemplateExpression(tagged) => {
            complexity_from_expression(&tagged.tag)
                + tagged
                    .quasi
                    .expressions
                    .iter()
                    .map(complexity_from_expression)
                    .sum::<u32>()
        }
        ArrayExpressionElement::UnaryExpression(expression) => {
            complexity_from_expression(&expression.argument)
        }
        ArrayExpressionElement::JSXElement(element) => complexity_from_jsx_element(element),
        ArrayExpressionElement::JSXFragment(fragment) => complexity_from_jsx_fragment(fragment),
        ArrayExpressionElement::TSAsExpression(expression) => {
            complexity_from_expression(&expression.expression)
        }
        ArrayExpressionElement::TSSatisfiesExpression(expression) => {
            complexity_from_expression(&expression.expression)
        }
        ArrayExpressionElement::TSNonNullExpression(expression) => {
            complexity_from_expression(&expression.expression)
        }
        ArrayExpressionElement::TSTypeAssertion(expression) => {
            complexity_from_expression(&expression.expression)
        }
        _ => 0,
    }
}

fn complexity_from_property_key(key: &PropertyKey<'_>) -> u32 {
    match key {
        PropertyKey::ConditionalExpression(expression) => {
            1 + complexity_from_expression(&expression.test)
                + complexity_from_expression(&expression.consequent)
                + complexity_from_expression(&expression.alternate)
        }
        PropertyKey::LogicalExpression(expression) => {
            u32::from(expression.operator.is_and() || expression.operator.is_or())
                + complexity_from_expression(&expression.left)
                + complexity_from_expression(&expression.right)
        }
        PropertyKey::ParenthesizedExpression(expression) => {
            complexity_from_expression(&expression.expression)
        }
        PropertyKey::TSAsExpression(expression) => {
            complexity_from_expression(&expression.expression)
        }
        PropertyKey::TSSatisfiesExpression(expression) => {
            complexity_from_expression(&expression.expression)
        }
        PropertyKey::TSNonNullExpression(expression) => {
            complexity_from_expression(&expression.expression)
        }
        PropertyKey::TSTypeAssertion(expression) => {
            complexity_from_expression(&expression.expression)
        }
        _ => 0,
    }
}

fn complexity_from_jsx_element(element: &JSXElement<'_>) -> u32 {
    element
        .opening_element
        .attributes
        .iter()
        .map(|item| match item {
            JSXAttributeItem::Attribute(attribute) => attribute
                .value
                .as_ref()
                .map_or(0, complexity_from_jsx_attribute_value),
            JSXAttributeItem::SpreadAttribute(spread) => {
                complexity_from_expression(&spread.argument)
            }
        })
        .sum::<u32>()
        + element
            .children
            .iter()
            .map(complexity_from_jsx_child)
            .sum::<u32>()
}

fn complexity_from_jsx_attribute_value(value: &JSXAttributeValue<'_>) -> u32 {
    match value {
        JSXAttributeValue::ExpressionContainer(container) => {
            complexity_from_jsx_expression(&container.expression)
        }
        JSXAttributeValue::Element(element) => complexity_from_jsx_element(element),
        JSXAttributeValue::Fragment(fragment) => complexity_from_jsx_fragment(fragment),
        JSXAttributeValue::StringLiteral(_) => 0,
    }
}

fn complexity_from_jsx_child(child: &JSXChild<'_>) -> u32 {
    match child {
        JSXChild::Element(element) => complexity_from_jsx_element(element),
        JSXChild::Fragment(fragment) => complexity_from_jsx_fragment(fragment),
        JSXChild::ExpressionContainer(container) => {
            complexity_from_jsx_expression(&container.expression)
        }
        JSXChild::Spread(spread) => complexity_from_expression(&spread.expression),
        JSXChild::Text(_) => 0,
    }
}

fn complexity_from_jsx_fragment(fragment: &JSXFragment<'_>) -> u32 {
    fragment
        .children
        .iter()
        .map(complexity_from_jsx_child)
        .sum()
}

fn complexity_from_jsx_expression(expression: &JSXExpression<'_>) -> u32 {
    match expression {
        JSXExpression::ArrayExpression(array) => array
            .elements
            .iter()
            .map(complexity_from_array_element)
            .sum(),
        JSXExpression::AssignmentExpression(expression) => {
            complexity_from_expression(&expression.right)
        }
        JSXExpression::BinaryExpression(expression) => {
            complexity_from_expression(&expression.left)
                + complexity_from_expression(&expression.right)
        }
        JSXExpression::CallExpression(call) => {
            complexity_from_expression(&call.callee)
                + call
                    .arguments
                    .iter()
                    .map(complexity_from_argument)
                    .sum::<u32>()
        }
        JSXExpression::ConditionalExpression(expression) => {
            1 + complexity_from_expression(&expression.test)
                + complexity_from_expression(&expression.consequent)
                + complexity_from_expression(&expression.alternate)
        }
        JSXExpression::LogicalExpression(expression) => {
            u32::from(expression.operator.is_and() || expression.operator.is_or())
                + complexity_from_expression(&expression.left)
                + complexity_from_expression(&expression.right)
        }
        JSXExpression::ObjectExpression(object) => object
            .properties
            .iter()
            .map(|property| match property {
                ObjectPropertyKind::ObjectProperty(property) => {
                    complexity_from_property_key(&property.key)
                        + complexity_from_expression(&property.value)
                }
                ObjectPropertyKind::SpreadProperty(spread) => {
                    complexity_from_expression(&spread.argument)
                }
            })
            .sum(),
        JSXExpression::ParenthesizedExpression(expression) => {
            complexity_from_expression(&expression.expression)
        }
        JSXExpression::TaggedTemplateExpression(tagged) => {
            complexity_from_expression(&tagged.tag)
                + tagged
                    .quasi
                    .expressions
                    .iter()
                    .map(complexity_from_expression)
                    .sum::<u32>()
        }
        JSXExpression::UnaryExpression(expression) => {
            complexity_from_expression(&expression.argument)
        }
        JSXExpression::JSXElement(element) => complexity_from_jsx_element(element),
        JSXExpression::JSXFragment(fragment) => complexity_from_jsx_fragment(fragment),
        JSXExpression::TSAsExpression(expression) => {
            complexity_from_expression(&expression.expression)
        }
        JSXExpression::TSSatisfiesExpression(expression) => {
            complexity_from_expression(&expression.expression)
        }
        JSXExpression::TSNonNullExpression(expression) => {
            complexity_from_expression(&expression.expression)
        }
        JSXExpression::TSTypeAssertion(expression) => {
            complexity_from_expression(&expression.expression)
        }
        JSXExpression::EmptyExpression(_) => 0,
        _ => 0,
    }
}

fn collect_calls_from_argument(argument: &Argument<'_>, calls: &mut Vec<String>) {
    match argument {
        Argument::SpreadElement(spread) => collect_calls_from_expression(&spread.argument, calls),
        Argument::ArrayExpression(array) => {
            for element in &array.elements {
                collect_calls_from_array_element(element, calls);
            }
        }
        Argument::AssignmentExpression(expression) => {
            collect_calls_from_expression(&expression.right, calls);
        }
        Argument::AwaitExpression(expression) => {
            collect_calls_from_expression(&expression.argument, calls);
        }
        Argument::BinaryExpression(expression) => {
            collect_calls_from_expression(&expression.left, calls);
            collect_calls_from_expression(&expression.right, calls);
        }
        Argument::CallExpression(call) => {
            if let Some(name) = callee_text(&call.callee) {
                calls.push(name);
            }
            collect_calls_from_expression(&call.callee, calls);
            for argument in &call.arguments {
                collect_calls_from_argument(argument, calls);
            }
        }
        Argument::ConditionalExpression(expression) => {
            collect_calls_from_expression(&expression.test, calls);
            collect_calls_from_expression(&expression.consequent, calls);
            collect_calls_from_expression(&expression.alternate, calls);
        }
        Argument::ImportExpression(expression) => {
            collect_calls_from_expression(&expression.source, calls);
        }
        Argument::LogicalExpression(expression) => {
            collect_calls_from_expression(&expression.left, calls);
            collect_calls_from_expression(&expression.right, calls);
        }
        Argument::NewExpression(expression) => {
            collect_calls_from_expression(&expression.callee, calls);
            for argument in &expression.arguments {
                collect_calls_from_argument(argument, calls);
            }
        }
        Argument::ObjectExpression(object) => {
            for property in &object.properties {
                match property {
                    ObjectPropertyKind::ObjectProperty(property) => {
                        collect_calls_from_property_key(&property.key, calls);
                        collect_calls_from_expression(&property.value, calls);
                    }
                    ObjectPropertyKind::SpreadProperty(spread) => {
                        collect_calls_from_expression(&spread.argument, calls);
                    }
                }
            }
        }
        Argument::ParenthesizedExpression(expression) => {
            collect_calls_from_expression(&expression.expression, calls);
        }
        Argument::SequenceExpression(expression) => {
            for expression in &expression.expressions {
                collect_calls_from_expression(expression, calls);
            }
        }
        Argument::TaggedTemplateExpression(tagged) => {
            collect_calls_from_expression(&tagged.tag, calls);
            for expression in &tagged.quasi.expressions {
                collect_calls_from_expression(expression, calls);
            }
        }
        Argument::UnaryExpression(expression) => {
            collect_calls_from_expression(&expression.argument, calls);
        }
        Argument::YieldExpression(expression) => {
            if let Some(argument) = &expression.argument {
                collect_calls_from_expression(argument, calls);
            }
        }
        Argument::JSXElement(element) => collect_calls_from_jsx_element(element, calls),
        Argument::JSXFragment(fragment) => collect_calls_from_jsx_fragment(fragment, calls),
        Argument::TSAsExpression(expression) => {
            collect_calls_from_expression(&expression.expression, calls);
        }
        Argument::TSSatisfiesExpression(expression) => {
            collect_calls_from_expression(&expression.expression, calls);
        }
        Argument::TSNonNullExpression(expression) => {
            collect_calls_from_expression(&expression.expression, calls);
        }
        Argument::TSTypeAssertion(expression) => {
            collect_calls_from_expression(&expression.expression, calls);
        }
        Argument::TSInstantiationExpression(expression) => {
            collect_calls_from_expression(&expression.expression, calls);
        }
        Argument::ComputedMemberExpression(expression) => {
            collect_calls_from_expression(&expression.object, calls);
            collect_calls_from_expression(&expression.expression, calls);
        }
        Argument::StaticMemberExpression(expression) => {
            collect_calls_from_expression(&expression.object, calls);
        }
        Argument::PrivateFieldExpression(expression) => {
            collect_calls_from_expression(&expression.object, calls);
        }
        _ => {}
    }
}

fn collect_calls_from_array_element(element: &ArrayExpressionElement<'_>, calls: &mut Vec<String>) {
    match element {
        ArrayExpressionElement::SpreadElement(spread) => {
            collect_calls_from_expression(&spread.argument, calls);
        }
        ArrayExpressionElement::ArrayExpression(array) => {
            for element in &array.elements {
                collect_calls_from_array_element(element, calls);
            }
        }
        ArrayExpressionElement::AssignmentExpression(expression) => {
            collect_calls_from_expression(&expression.right, calls);
        }
        ArrayExpressionElement::AwaitExpression(expression) => {
            collect_calls_from_expression(&expression.argument, calls);
        }
        ArrayExpressionElement::BinaryExpression(expression) => {
            collect_calls_from_expression(&expression.left, calls);
            collect_calls_from_expression(&expression.right, calls);
        }
        ArrayExpressionElement::CallExpression(call) => {
            if let Some(name) = callee_text(&call.callee) {
                calls.push(name);
            }
            collect_calls_from_expression(&call.callee, calls);
            for argument in &call.arguments {
                collect_calls_from_argument(argument, calls);
            }
        }
        ArrayExpressionElement::ConditionalExpression(expression) => {
            collect_calls_from_expression(&expression.test, calls);
            collect_calls_from_expression(&expression.consequent, calls);
            collect_calls_from_expression(&expression.alternate, calls);
        }
        ArrayExpressionElement::LogicalExpression(expression) => {
            collect_calls_from_expression(&expression.left, calls);
            collect_calls_from_expression(&expression.right, calls);
        }
        ArrayExpressionElement::ObjectExpression(object) => {
            for property in &object.properties {
                match property {
                    ObjectPropertyKind::ObjectProperty(property) => {
                        collect_calls_from_property_key(&property.key, calls);
                        collect_calls_from_expression(&property.value, calls);
                    }
                    ObjectPropertyKind::SpreadProperty(spread) => {
                        collect_calls_from_expression(&spread.argument, calls);
                    }
                }
            }
        }
        ArrayExpressionElement::ParenthesizedExpression(expression) => {
            collect_calls_from_expression(&expression.expression, calls);
        }
        ArrayExpressionElement::SequenceExpression(expression) => {
            for expression in &expression.expressions {
                collect_calls_from_expression(expression, calls);
            }
        }
        ArrayExpressionElement::TaggedTemplateExpression(tagged) => {
            collect_calls_from_expression(&tagged.tag, calls);
            for expression in &tagged.quasi.expressions {
                collect_calls_from_expression(expression, calls);
            }
        }
        ArrayExpressionElement::UnaryExpression(expression) => {
            collect_calls_from_expression(&expression.argument, calls);
        }
        ArrayExpressionElement::JSXElement(element) => {
            collect_calls_from_jsx_element(element, calls)
        }
        ArrayExpressionElement::JSXFragment(fragment) => {
            collect_calls_from_jsx_fragment(fragment, calls);
        }
        ArrayExpressionElement::TSAsExpression(expression) => {
            collect_calls_from_expression(&expression.expression, calls);
        }
        ArrayExpressionElement::TSSatisfiesExpression(expression) => {
            collect_calls_from_expression(&expression.expression, calls);
        }
        ArrayExpressionElement::TSNonNullExpression(expression) => {
            collect_calls_from_expression(&expression.expression, calls);
        }
        ArrayExpressionElement::TSTypeAssertion(expression) => {
            collect_calls_from_expression(&expression.expression, calls);
        }
        _ => {}
    }
}

fn collect_calls_from_property_key(key: &PropertyKey<'_>, calls: &mut Vec<String>) {
    match key {
        PropertyKey::CallExpression(call) => {
            if let Some(name) = callee_text(&call.callee) {
                calls.push(name);
            }
            for argument in &call.arguments {
                collect_calls_from_argument(argument, calls);
            }
        }
        PropertyKey::ConditionalExpression(expression) => {
            collect_calls_from_expression(&expression.test, calls);
            collect_calls_from_expression(&expression.consequent, calls);
            collect_calls_from_expression(&expression.alternate, calls);
        }
        PropertyKey::LogicalExpression(expression) => {
            collect_calls_from_expression(&expression.left, calls);
            collect_calls_from_expression(&expression.right, calls);
        }
        PropertyKey::ParenthesizedExpression(expression) => {
            collect_calls_from_expression(&expression.expression, calls);
        }
        PropertyKey::TSAsExpression(expression) => {
            collect_calls_from_expression(&expression.expression, calls);
        }
        PropertyKey::TSSatisfiesExpression(expression) => {
            collect_calls_from_expression(&expression.expression, calls);
        }
        PropertyKey::TSNonNullExpression(expression) => {
            collect_calls_from_expression(&expression.expression, calls);
        }
        PropertyKey::TSTypeAssertion(expression) => {
            collect_calls_from_expression(&expression.expression, calls);
        }
        _ => {}
    }
}

fn collect_calls_from_jsx_element(element: &JSXElement<'_>, calls: &mut Vec<String>) {
    for item in &element.opening_element.attributes {
        match item {
            JSXAttributeItem::Attribute(attribute) => {
                if let Some(value) = &attribute.value {
                    collect_calls_from_jsx_attribute_value(value, calls);
                }
            }
            JSXAttributeItem::SpreadAttribute(spread) => {
                collect_calls_from_expression(&spread.argument, calls);
            }
        }
    }
    for child in &element.children {
        collect_calls_from_jsx_child(child, calls);
    }
}

fn collect_calls_from_jsx_attribute_value(value: &JSXAttributeValue<'_>, calls: &mut Vec<String>) {
    match value {
        JSXAttributeValue::ExpressionContainer(container) => {
            collect_calls_from_jsx_expression(&container.expression, calls);
        }
        JSXAttributeValue::Element(element) => collect_calls_from_jsx_element(element, calls),
        JSXAttributeValue::Fragment(fragment) => collect_calls_from_jsx_fragment(fragment, calls),
        JSXAttributeValue::StringLiteral(_) => {}
    }
}

fn collect_calls_from_jsx_child(child: &JSXChild<'_>, calls: &mut Vec<String>) {
    match child {
        JSXChild::Element(element) => collect_calls_from_jsx_element(element, calls),
        JSXChild::Fragment(fragment) => collect_calls_from_jsx_fragment(fragment, calls),
        JSXChild::ExpressionContainer(container) => {
            collect_calls_from_jsx_expression(&container.expression, calls);
        }
        JSXChild::Spread(spread) => collect_calls_from_expression(&spread.expression, calls),
        JSXChild::Text(_) => {}
    }
}

fn collect_calls_from_jsx_fragment(fragment: &JSXFragment<'_>, calls: &mut Vec<String>) {
    for child in &fragment.children {
        collect_calls_from_jsx_child(child, calls);
    }
}

fn collect_calls_from_jsx_expression(expression: &JSXExpression<'_>, calls: &mut Vec<String>) {
    match expression {
        JSXExpression::ArrayExpression(array) => {
            for element in &array.elements {
                collect_calls_from_array_element(element, calls);
            }
        }
        JSXExpression::AssignmentExpression(expression) => {
            collect_calls_from_expression(&expression.right, calls);
        }
        JSXExpression::BinaryExpression(expression) => {
            collect_calls_from_expression(&expression.left, calls);
            collect_calls_from_expression(&expression.right, calls);
        }
        JSXExpression::CallExpression(call) => {
            if let Some(name) = callee_text(&call.callee) {
                calls.push(name);
            }
            collect_calls_from_expression(&call.callee, calls);
            for argument in &call.arguments {
                collect_calls_from_argument(argument, calls);
            }
        }
        JSXExpression::ConditionalExpression(expression) => {
            collect_calls_from_expression(&expression.test, calls);
            collect_calls_from_expression(&expression.consequent, calls);
            collect_calls_from_expression(&expression.alternate, calls);
        }
        JSXExpression::LogicalExpression(expression) => {
            collect_calls_from_expression(&expression.left, calls);
            collect_calls_from_expression(&expression.right, calls);
        }
        JSXExpression::ObjectExpression(object) => {
            for property in &object.properties {
                match property {
                    ObjectPropertyKind::ObjectProperty(property) => {
                        collect_calls_from_property_key(&property.key, calls);
                        collect_calls_from_expression(&property.value, calls);
                    }
                    ObjectPropertyKind::SpreadProperty(spread) => {
                        collect_calls_from_expression(&spread.argument, calls);
                    }
                }
            }
        }
        JSXExpression::ParenthesizedExpression(expression) => {
            collect_calls_from_expression(&expression.expression, calls);
        }
        JSXExpression::TaggedTemplateExpression(tagged) => {
            collect_calls_from_expression(&tagged.tag, calls);
            for expression in &tagged.quasi.expressions {
                collect_calls_from_expression(expression, calls);
            }
        }
        JSXExpression::UnaryExpression(expression) => {
            collect_calls_from_expression(&expression.argument, calls);
        }
        JSXExpression::JSXElement(element) => collect_calls_from_jsx_element(element, calls),
        JSXExpression::JSXFragment(fragment) => collect_calls_from_jsx_fragment(fragment, calls),
        JSXExpression::TSAsExpression(expression) => {
            collect_calls_from_expression(&expression.expression, calls);
        }
        JSXExpression::TSSatisfiesExpression(expression) => {
            collect_calls_from_expression(&expression.expression, calls);
        }
        JSXExpression::TSNonNullExpression(expression) => {
            collect_calls_from_expression(&expression.expression, calls);
        }
        JSXExpression::TSTypeAssertion(expression) => {
            collect_calls_from_expression(&expression.expression, calls);
        }
        JSXExpression::EmptyExpression(_) => {}
        _ => {}
    }
}

fn callee_text(expression: &Expression<'_>) -> Option<String> {
    match expression {
        Expression::Identifier(identifier) => Some(identifier.name.to_string()),
        Expression::StaticMemberExpression(member) => {
            callee_text(&member.object).map(|object| format!("{object}.{}", member.property.name))
        }
        Expression::ArrowFunctionExpression(function) => Some(anonymous_callable_name(
            function.span.start,
            function.span.end,
        )),
        Expression::FunctionExpression(function) => Some(anonymous_callable_name(
            function.span.start,
            function.span.end,
        )),
        Expression::ParenthesizedExpression(expression) => callee_text(&expression.expression),
        Expression::TSAsExpression(expression) => callee_text(&expression.expression),
        Expression::TSSatisfiesExpression(expression) => callee_text(&expression.expression),
        Expression::TSNonNullExpression(expression) => callee_text(&expression.expression),
        Expression::TSTypeAssertion(expression) => callee_text(&expression.expression),
        _ => None,
    }
}

fn is_component_like_name(name: &str) -> bool {
    name.chars().next().is_some_and(char::is_uppercase)
}

fn function_returns_jsx(function: &Function<'_>) -> bool {
    function
        .body
        .as_deref()
        .is_some_and(function_body_returns_jsx)
}

fn arrow_returns_jsx(function: &ArrowFunctionExpression<'_>) -> bool {
    function.get_expression().is_some_and(expression_is_jsx)
        || function_body_returns_jsx(&function.body)
}

fn function_body_returns_jsx(body: &FunctionBody<'_>) -> bool {
    body.statements.iter().any(statement_returns_jsx)
}

fn statement_returns_jsx(statement: &Statement<'_>) -> bool {
    match statement {
        Statement::BlockStatement(block) => block.body.iter().any(statement_returns_jsx),
        Statement::ReturnStatement(statement) => {
            statement.argument.as_ref().is_some_and(expression_is_jsx)
        }
        Statement::IfStatement(statement) => {
            statement_returns_jsx(&statement.consequent)
                || statement
                    .alternate
                    .as_ref()
                    .is_some_and(statement_returns_jsx)
        }
        _ => false,
    }
}

fn expression_is_jsx(expression: &Expression<'_>) -> bool {
    match expression {
        Expression::JSXElement(_) | Expression::JSXFragment(_) => true,
        Expression::ParenthesizedExpression(expression) => {
            expression_is_jsx(&expression.expression)
        }
        Expression::TSAsExpression(expression) => expression_is_jsx(&expression.expression),
        Expression::TSSatisfiesExpression(expression) => expression_is_jsx(&expression.expression),
        Expression::TSNonNullExpression(expression) => expression_is_jsx(&expression.expression),
        Expression::TSTypeAssertion(expression) => expression_is_jsx(&expression.expression),
        Expression::ConditionalExpression(expression) => {
            expression_is_jsx(&expression.consequent) || expression_is_jsx(&expression.alternate)
        }
        _ => false,
    }
}

fn extract_literals_and_jsx(
    db: &mut dyn FactDatabase,
    file: FileId,
    source: &str,
    language: Language,
    program: &Program<'_>,
) {
    let ctx = TsAstCtx {
        file,
        source,
        language,
    };
    for directive in &program.directives {
        push_string_literal_from_oxc(
            db,
            ctx,
            directive.expression.value.to_string(),
            directive.span,
        );
    }
    for statement in &program.body {
        walk_statement_for_literals(db, ctx, statement);
    }
}

fn walk_statement_for_literals(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    statement: &Statement<'_>,
) {
    match statement {
        Statement::BlockStatement(block) => {
            for statement in &block.body {
                walk_statement_for_literals(db, ctx, statement);
            }
        }
        Statement::ExpressionStatement(statement) => {
            walk_expression_for_literals(db, ctx, &statement.expression);
        }
        Statement::ReturnStatement(statement) => {
            if let Some(argument) = &statement.argument {
                walk_expression_for_literals(db, ctx, argument);
            }
        }
        Statement::IfStatement(statement) => {
            walk_expression_for_literals(db, ctx, &statement.test);
            walk_statement_for_literals(db, ctx, &statement.consequent);
            if let Some(alternate) = &statement.alternate {
                walk_statement_for_literals(db, ctx, alternate);
            }
        }
        Statement::DoWhileStatement(statement) => {
            walk_statement_for_literals(db, ctx, &statement.body);
            walk_expression_for_literals(db, ctx, &statement.test);
        }
        Statement::WhileStatement(statement) => {
            walk_expression_for_literals(db, ctx, &statement.test);
            walk_statement_for_literals(db, ctx, &statement.body);
        }
        Statement::ForStatement(statement) => {
            if let Some(init) = &statement.init {
                walk_for_init_for_literals(db, ctx, init);
            }
            if let Some(test) = &statement.test {
                walk_expression_for_literals(db, ctx, test);
            }
            if let Some(update) = &statement.update {
                walk_expression_for_literals(db, ctx, update);
            }
            walk_statement_for_literals(db, ctx, &statement.body);
        }
        Statement::ForInStatement(statement) => {
            walk_for_left_for_literals(db, ctx, &statement.left);
            walk_expression_for_literals(db, ctx, &statement.right);
            walk_statement_for_literals(db, ctx, &statement.body);
        }
        Statement::ForOfStatement(statement) => {
            walk_for_left_for_literals(db, ctx, &statement.left);
            walk_expression_for_literals(db, ctx, &statement.right);
            walk_statement_for_literals(db, ctx, &statement.body);
        }
        Statement::SwitchStatement(statement) => {
            walk_expression_for_literals(db, ctx, &statement.discriminant);
            for case in &statement.cases {
                if let Some(test) = &case.test {
                    walk_expression_for_literals(db, ctx, test);
                }
                for statement in &case.consequent {
                    walk_statement_for_literals(db, ctx, statement);
                }
            }
        }
        Statement::ThrowStatement(statement) => {
            walk_expression_for_literals(db, ctx, &statement.argument);
        }
        Statement::TryStatement(statement) => {
            for statement in &statement.block.body {
                walk_statement_for_literals(db, ctx, statement);
            }
            if let Some(handler) = &statement.handler {
                for statement in &handler.body.body {
                    walk_statement_for_literals(db, ctx, statement);
                }
            }
            if let Some(finalizer) = &statement.finalizer {
                for statement in &finalizer.body {
                    walk_statement_for_literals(db, ctx, statement);
                }
            }
        }
        Statement::WithStatement(statement) => {
            walk_expression_for_literals(db, ctx, &statement.object);
            walk_statement_for_literals(db, ctx, &statement.body);
        }
        Statement::LabeledStatement(statement) => {
            walk_statement_for_literals(db, ctx, &statement.body);
        }
        Statement::VariableDeclaration(variable) => {
            walk_variable_declaration_for_literals(db, ctx, variable);
        }
        Statement::FunctionDeclaration(function) => {
            walk_function_for_literals(db, ctx, function);
        }
        Statement::ClassDeclaration(class) => {
            walk_class_for_literals(db, ctx, class);
        }
        Statement::ImportDeclaration(declaration) => {
            push_string_literal_from_oxc(
                db,
                ctx,
                declaration.source.value.to_string(),
                declaration.source.span,
            );
        }
        Statement::ExportAllDeclaration(declaration) => {
            push_string_literal_from_oxc(
                db,
                ctx,
                declaration.source.value.to_string(),
                declaration.source.span,
            );
        }
        Statement::ExportNamedDeclaration(declaration) => {
            if let Some(source) = &declaration.source {
                push_string_literal_from_oxc(db, ctx, source.value.to_string(), source.span);
            }
            if let Some(declaration) = &declaration.declaration {
                walk_declaration_for_literals(db, ctx, declaration);
            }
        }
        Statement::ExportDefaultDeclaration(declaration) => {
            walk_export_default_for_literals(db, ctx, &declaration.declaration);
        }
        _ => {}
    }
}

fn walk_expression_for_literals(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    expression: &Expression<'_>,
) {
    match expression {
        Expression::RegExpLiteral(literal) => {
            push_regex_literal_from_oxc(db, ctx, literal);
        }
        Expression::StringLiteral(literal) => {
            push_string_literal_from_oxc(db, ctx, literal.value.to_string(), literal.span);
        }
        Expression::TemplateLiteral(template) => {
            push_template_literal_from_oxc(db, ctx, template);
        }
        Expression::TaggedTemplateExpression(tagged) => {
            walk_expression_for_literals(db, ctx, &tagged.tag);
            push_template_literal_from_oxc(db, ctx, &tagged.quasi);
        }
        Expression::ArrayExpression(array) => {
            for element in &array.elements {
                walk_array_element_for_literals(db, ctx, element);
            }
        }
        Expression::ArrowFunctionExpression(function) => {
            walk_function_body_for_literals(db, ctx, &function.body);
        }
        Expression::AssignmentExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.right);
        }
        Expression::AwaitExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.argument);
        }
        Expression::BinaryExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.left);
            walk_expression_for_literals(db, ctx, &expression.right);
        }
        Expression::CallExpression(call) => {
            walk_expression_for_literals(db, ctx, &call.callee);
            for argument in &call.arguments {
                walk_argument_for_literals(db, ctx, argument);
            }
        }
        Expression::ClassExpression(class) => {
            walk_class_for_literals(db, ctx, class);
        }
        Expression::ConditionalExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.test);
            walk_expression_for_literals(db, ctx, &expression.consequent);
            walk_expression_for_literals(db, ctx, &expression.alternate);
        }
        Expression::FunctionExpression(function) => {
            walk_function_for_literals(db, ctx, function);
        }
        Expression::ImportExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.source);
        }
        Expression::LogicalExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.left);
            walk_expression_for_literals(db, ctx, &expression.right);
        }
        Expression::NewExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.callee);
            for argument in &expression.arguments {
                walk_argument_for_literals(db, ctx, argument);
            }
        }
        Expression::ObjectExpression(object) => {
            for property in &object.properties {
                match property {
                    ObjectPropertyKind::ObjectProperty(property) => {
                        walk_property_key_for_literals(db, ctx, &property.key);
                        walk_expression_for_literals(db, ctx, &property.value);
                    }
                    ObjectPropertyKind::SpreadProperty(spread) => {
                        walk_expression_for_literals(db, ctx, &spread.argument);
                    }
                }
            }
        }
        Expression::ParenthesizedExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.expression);
        }
        Expression::SequenceExpression(expression) => {
            for expression in &expression.expressions {
                walk_expression_for_literals(db, ctx, expression);
            }
        }
        Expression::UnaryExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.argument);
        }
        Expression::JSXElement(element) => {
            extract_jsx_element_attributes(db, ctx, element);
        }
        Expression::JSXFragment(fragment) => {
            walk_jsx_fragment_for_literals(db, ctx, fragment);
        }
        Expression::TSAsExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.expression);
        }
        Expression::TSSatisfiesExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.expression);
        }
        Expression::TSNonNullExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.expression);
        }
        Expression::TSTypeAssertion(expression) => {
            walk_expression_for_literals(db, ctx, &expression.expression);
        }
        Expression::TSInstantiationExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.expression);
        }
        Expression::ComputedMemberExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.object);
            walk_expression_for_literals(db, ctx, &expression.expression);
        }
        Expression::StaticMemberExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.object);
        }
        Expression::PrivateFieldExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.object);
        }
        _ => {}
    }
}

fn push_string_literal_from_oxc(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    value: String,
    span: oxc_span::Span,
) {
    db.push_string_literal(StringLiteralFact::new(
        ctx.file,
        value,
        span_from_oxc(db, ctx.file, span),
        ctx.language,
    ));
}

fn push_regex_literal_from_oxc(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    literal: &RegExpLiteral<'_>,
) {
    let value = literal
        .raw
        .as_ref()
        .map_or_else(|| literal.regex.to_string(), |raw| raw.as_str().to_string());
    push_string_literal_from_oxc(db, ctx, value, literal.span);
}

fn extract_jsx_element_attributes(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    element: &JSXElement<'_>,
) {
    for item in &element.opening_element.attributes {
        match item {
            JSXAttributeItem::Attribute(attribute) => {
                if let Some(name) = jsx_attribute_name(&attribute.name) {
                    db.push_jsx_attribute(JsxAttributeFact::new(
                        ctx.file,
                        name,
                        attribute.value.as_ref().and_then(jsx_attribute_value),
                        span_from_oxc(db, ctx.file, attribute.span),
                    ));
                }
                if let Some(value) = &attribute.value {
                    walk_jsx_attribute_value_for_literals(db, ctx, value);
                }
            }
            JSXAttributeItem::SpreadAttribute(spread) => {
                walk_expression_for_literals(db, ctx, &spread.argument);
            }
        }
    }
    for child in &element.children {
        walk_jsx_child_for_literals(db, ctx, child);
    }
}

fn jsx_attribute_name(name: &JSXAttributeName<'_>) -> Option<String> {
    match name {
        JSXAttributeName::Identifier(identifier) => Some(identifier.name.to_string()),
        JSXAttributeName::NamespacedName(name) => {
            Some(format!("{}:{}", name.namespace.name, name.name.name))
        }
    }
}

fn jsx_attribute_value(value: &JSXAttributeValue<'_>) -> Option<String> {
    match value {
        JSXAttributeValue::StringLiteral(literal) => Some(literal.value.to_string()),
        JSXAttributeValue::ExpressionContainer(container) => {
            jsx_expression_static_value(&container.expression)
        }
        JSXAttributeValue::Element(_) | JSXAttributeValue::Fragment(_) => None,
    }
}

fn push_template_literal_from_oxc(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    template: &TemplateLiteral<'_>,
) {
    if template.expressions.is_empty() {
        push_string_literal_from_oxc(db, ctx, template_literal_value(template), template.span);
    } else {
        for quasi in &template.quasis {
            let value = template_element_value(quasi);
            if !value.is_empty() {
                push_string_literal_from_oxc(db, ctx, value, quasi.span);
            }
        }
        for expression in &template.expressions {
            walk_expression_for_literals(db, ctx, expression);
        }
    }
}

fn template_literal_value(template: &TemplateLiteral<'_>) -> String {
    template
        .quasis
        .iter()
        .map(template_element_value)
        .collect::<Vec<_>>()
        .join("")
}

fn template_element_value(element: &oxc_ast::ast::TemplateElement<'_>) -> String {
    element
        .value
        .cooked
        .as_ref()
        .unwrap_or(&element.value.raw)
        .to_string()
}

fn walk_declaration_for_literals(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    declaration: &Declaration<'_>,
) {
    match declaration {
        Declaration::VariableDeclaration(variable) => {
            walk_variable_declaration_for_literals(db, ctx, variable);
        }
        Declaration::FunctionDeclaration(function) => {
            walk_function_for_literals(db, ctx, function);
        }
        Declaration::ClassDeclaration(class) => {
            walk_class_for_literals(db, ctx, class);
        }
        _ => {}
    }
}

fn walk_export_default_for_literals(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    declaration: &ExportDefaultDeclarationKind<'_>,
) {
    match declaration {
        ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
            walk_function_for_literals(db, ctx, function);
        }
        ExportDefaultDeclarationKind::ClassDeclaration(class) => {
            walk_class_for_literals(db, ctx, class);
        }
        ExportDefaultDeclarationKind::RegExpLiteral(literal) => {
            push_regex_literal_from_oxc(db, ctx, literal);
        }
        ExportDefaultDeclarationKind::StringLiteral(literal) => {
            push_string_literal_from_oxc(db, ctx, literal.value.to_string(), literal.span);
        }
        ExportDefaultDeclarationKind::TemplateLiteral(template) => {
            push_template_literal_from_oxc(db, ctx, template);
        }
        ExportDefaultDeclarationKind::JSXElement(element) => {
            extract_jsx_element_attributes(db, ctx, element);
        }
        ExportDefaultDeclarationKind::JSXFragment(fragment) => {
            walk_jsx_fragment_for_literals(db, ctx, fragment);
        }
        _ => {}
    }
}

fn walk_variable_declaration_for_literals(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    variable: &oxc_ast::ast::VariableDeclaration<'_>,
) {
    for declarator in &variable.declarations {
        if let Some(init) = &declarator.init {
            walk_expression_for_literals(db, ctx, init);
        }
    }
}

fn walk_function_for_literals(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    function: &Function<'_>,
) {
    if let Some(body) = function.body.as_deref() {
        walk_function_body_for_literals(db, ctx, body);
    }
}

fn walk_function_body_for_literals(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    body: &FunctionBody<'_>,
) {
    for directive in &body.directives {
        push_string_literal_from_oxc(
            db,
            ctx,
            directive.expression.value.to_string(),
            directive.span,
        );
    }
    for statement in &body.statements {
        walk_statement_for_literals(db, ctx, statement);
    }
}

fn walk_class_for_literals(db: &mut dyn FactDatabase, ctx: TsAstCtx<'_>, class: &Class<'_>) {
    if let Some(super_class) = &class.super_class {
        walk_expression_for_literals(db, ctx, super_class);
    }
    for element in &class.body.body {
        match element {
            ClassElement::StaticBlock(block) => {
                for statement in &block.body {
                    walk_statement_for_literals(db, ctx, statement);
                }
            }
            ClassElement::MethodDefinition(method) => {
                walk_property_key_for_literals(db, ctx, &method.key);
                walk_function_for_literals(db, ctx, &method.value);
            }
            ClassElement::PropertyDefinition(property) => {
                walk_property_key_for_literals(db, ctx, &property.key);
                if let Some(value) = &property.value {
                    walk_expression_for_literals(db, ctx, value);
                }
            }
            ClassElement::AccessorProperty(property) => {
                walk_property_key_for_literals(db, ctx, &property.key);
                if let Some(value) = &property.value {
                    walk_expression_for_literals(db, ctx, value);
                }
            }
            ClassElement::TSIndexSignature(_) => {}
        }
    }
}

fn walk_for_init_for_literals(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    init: &ForStatementInit<'_>,
) {
    match init {
        ForStatementInit::VariableDeclaration(variable) => {
            walk_variable_declaration_for_literals(db, ctx, variable);
        }
        ForStatementInit::RegExpLiteral(literal) => {
            push_regex_literal_from_oxc(db, ctx, literal);
        }
        ForStatementInit::StringLiteral(literal) => {
            push_string_literal_from_oxc(db, ctx, literal.value.to_string(), literal.span);
        }
        ForStatementInit::TemplateLiteral(template) => {
            push_template_literal_from_oxc(db, ctx, template);
        }
        _ => {}
    }
}

fn walk_for_left_for_literals(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    left: &ForStatementLeft<'_>,
) {
    if let ForStatementLeft::VariableDeclaration(variable) = left {
        walk_variable_declaration_for_literals(db, ctx, variable);
    }
}

fn walk_argument_for_literals(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    argument: &Argument<'_>,
) {
    match argument {
        Argument::SpreadElement(spread) => walk_expression_for_literals(db, ctx, &spread.argument),
        Argument::RegExpLiteral(literal) => {
            push_regex_literal_from_oxc(db, ctx, literal);
        }
        Argument::StringLiteral(literal) => {
            push_string_literal_from_oxc(db, ctx, literal.value.to_string(), literal.span);
        }
        Argument::TemplateLiteral(template) => push_template_literal_from_oxc(db, ctx, template),
        Argument::TaggedTemplateExpression(tagged) => {
            walk_expression_for_literals(db, ctx, &tagged.tag);
            push_template_literal_from_oxc(db, ctx, &tagged.quasi);
        }
        Argument::ArrayExpression(array) => {
            for element in &array.elements {
                walk_array_element_for_literals(db, ctx, element);
            }
        }
        Argument::ObjectExpression(object) => {
            for property in &object.properties {
                match property {
                    ObjectPropertyKind::ObjectProperty(property) => {
                        walk_property_key_for_literals(db, ctx, &property.key);
                        walk_expression_for_literals(db, ctx, &property.value);
                    }
                    ObjectPropertyKind::SpreadProperty(spread) => {
                        walk_expression_for_literals(db, ctx, &spread.argument);
                    }
                }
            }
        }
        Argument::CallExpression(call) => {
            walk_expression_for_literals(db, ctx, &call.callee);
            for argument in &call.arguments {
                walk_argument_for_literals(db, ctx, argument);
            }
        }
        Argument::ConditionalExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.test);
            walk_expression_for_literals(db, ctx, &expression.consequent);
            walk_expression_for_literals(db, ctx, &expression.alternate);
        }
        Argument::LogicalExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.left);
            walk_expression_for_literals(db, ctx, &expression.right);
        }
        Argument::ParenthesizedExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.expression);
        }
        Argument::JSXElement(element) => extract_jsx_element_attributes(db, ctx, element),
        Argument::JSXFragment(fragment) => walk_jsx_fragment_for_literals(db, ctx, fragment),
        Argument::TSAsExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.expression);
        }
        Argument::TSSatisfiesExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.expression);
        }
        Argument::TSNonNullExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.expression);
        }
        Argument::TSTypeAssertion(expression) => {
            walk_expression_for_literals(db, ctx, &expression.expression);
        }
        _ => {}
    }
}

fn walk_array_element_for_literals(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    element: &ArrayExpressionElement<'_>,
) {
    match element {
        ArrayExpressionElement::SpreadElement(spread) => {
            walk_expression_for_literals(db, ctx, &spread.argument);
        }
        ArrayExpressionElement::RegExpLiteral(literal) => {
            push_regex_literal_from_oxc(db, ctx, literal);
        }
        ArrayExpressionElement::StringLiteral(literal) => {
            push_string_literal_from_oxc(db, ctx, literal.value.to_string(), literal.span);
        }
        ArrayExpressionElement::TemplateLiteral(template) => {
            push_template_literal_from_oxc(db, ctx, template);
        }
        ArrayExpressionElement::ObjectExpression(object) => {
            for property in &object.properties {
                match property {
                    ObjectPropertyKind::ObjectProperty(property) => {
                        walk_property_key_for_literals(db, ctx, &property.key);
                        walk_expression_for_literals(db, ctx, &property.value);
                    }
                    ObjectPropertyKind::SpreadProperty(spread) => {
                        walk_expression_for_literals(db, ctx, &spread.argument);
                    }
                }
            }
        }
        ArrayExpressionElement::JSXElement(element) => {
            extract_jsx_element_attributes(db, ctx, element);
        }
        ArrayExpressionElement::JSXFragment(fragment) => {
            walk_jsx_fragment_for_literals(db, ctx, fragment);
        }
        _ => {}
    }
}

fn walk_property_key_for_literals(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    key: &PropertyKey<'_>,
) {
    match key {
        PropertyKey::RegExpLiteral(literal) => {
            push_regex_literal_from_oxc(db, ctx, literal);
        }
        PropertyKey::StringLiteral(literal) => {
            push_string_literal_from_oxc(db, ctx, literal.value.to_string(), literal.span);
        }
        PropertyKey::TemplateLiteral(template) => push_template_literal_from_oxc(db, ctx, template),
        _ => {}
    }
}

fn walk_jsx_attribute_value_for_literals(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    value: &JSXAttributeValue<'_>,
) {
    match value {
        JSXAttributeValue::ExpressionContainer(container) => {
            walk_jsx_expression_for_literals(db, ctx, &container.expression);
        }
        JSXAttributeValue::Element(element) => extract_jsx_element_attributes(db, ctx, element),
        JSXAttributeValue::Fragment(fragment) => walk_jsx_fragment_for_literals(db, ctx, fragment),
        JSXAttributeValue::StringLiteral(literal) => {
            push_string_literal_from_oxc(db, ctx, literal.value.to_string(), literal.span);
        }
    }
}

fn walk_jsx_child_for_literals(db: &mut dyn FactDatabase, ctx: TsAstCtx<'_>, child: &JSXChild<'_>) {
    match child {
        JSXChild::Element(element) => extract_jsx_element_attributes(db, ctx, element),
        JSXChild::Fragment(fragment) => walk_jsx_fragment_for_literals(db, ctx, fragment),
        JSXChild::ExpressionContainer(container) => {
            walk_jsx_expression_for_literals(db, ctx, &container.expression);
        }
        JSXChild::Spread(spread) => walk_expression_for_literals(db, ctx, &spread.expression),
        JSXChild::Text(_) => {}
    }
}

fn walk_jsx_fragment_for_literals(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    fragment: &JSXFragment<'_>,
) {
    for child in &fragment.children {
        walk_jsx_child_for_literals(db, ctx, child);
    }
}

fn walk_jsx_expression_for_literals(
    db: &mut dyn FactDatabase,
    ctx: TsAstCtx<'_>,
    expression: &JSXExpression<'_>,
) {
    match expression {
        JSXExpression::RegExpLiteral(literal) => {
            push_regex_literal_from_oxc(db, ctx, literal);
        }
        JSXExpression::StringLiteral(literal) => {
            push_string_literal_from_oxc(db, ctx, literal.value.to_string(), literal.span);
        }
        JSXExpression::TemplateLiteral(template) => {
            push_template_literal_from_oxc(db, ctx, template);
        }
        JSXExpression::TaggedTemplateExpression(tagged) => {
            walk_expression_for_literals(db, ctx, &tagged.tag);
            push_template_literal_from_oxc(db, ctx, &tagged.quasi);
        }
        JSXExpression::ArrayExpression(array) => {
            for element in &array.elements {
                walk_array_element_for_literals(db, ctx, element);
            }
        }
        JSXExpression::ObjectExpression(object) => {
            for property in &object.properties {
                match property {
                    ObjectPropertyKind::ObjectProperty(property) => {
                        walk_property_key_for_literals(db, ctx, &property.key);
                        walk_expression_for_literals(db, ctx, &property.value);
                    }
                    ObjectPropertyKind::SpreadProperty(spread) => {
                        walk_expression_for_literals(db, ctx, &spread.argument);
                    }
                }
            }
        }
        JSXExpression::CallExpression(call) => {
            walk_expression_for_literals(db, ctx, &call.callee);
            for argument in &call.arguments {
                walk_argument_for_literals(db, ctx, argument);
            }
        }
        JSXExpression::ConditionalExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.test);
            walk_expression_for_literals(db, ctx, &expression.consequent);
            walk_expression_for_literals(db, ctx, &expression.alternate);
        }
        JSXExpression::LogicalExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.left);
            walk_expression_for_literals(db, ctx, &expression.right);
        }
        JSXExpression::ParenthesizedExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.expression);
        }
        JSXExpression::JSXElement(element) => extract_jsx_element_attributes(db, ctx, element),
        JSXExpression::JSXFragment(fragment) => walk_jsx_fragment_for_literals(db, ctx, fragment),
        JSXExpression::TSAsExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.expression);
        }
        JSXExpression::TSSatisfiesExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.expression);
        }
        JSXExpression::TSNonNullExpression(expression) => {
            walk_expression_for_literals(db, ctx, &expression.expression);
        }
        JSXExpression::TSTypeAssertion(expression) => {
            walk_expression_for_literals(db, ctx, &expression.expression);
        }
        JSXExpression::EmptyExpression(_) => {}
        _ => {}
    }
}

fn jsx_expression_static_value(expression: &JSXExpression<'_>) -> Option<String> {
    match expression {
        JSXExpression::Identifier(identifier) => Some(identifier.name.to_string()),
        JSXExpression::StringLiteral(literal) => Some(literal.value.to_string()),
        JSXExpression::NumericLiteral(literal) => Some(
            literal
                .raw
                .as_ref()
                .map_or_else(|| literal.value.to_string(), ToString::to_string),
        ),
        JSXExpression::TemplateLiteral(template) if template.expressions.is_empty() => {
            Some(template_literal_value(template))
        }
        JSXExpression::ParenthesizedExpression(expression) => {
            expression_static_value(&expression.expression)
        }
        JSXExpression::TSAsExpression(expression) => {
            expression_static_value(&expression.expression)
        }
        JSXExpression::TSSatisfiesExpression(expression) => {
            expression_static_value(&expression.expression)
        }
        JSXExpression::TSNonNullExpression(expression) => {
            expression_static_value(&expression.expression)
        }
        JSXExpression::TSTypeAssertion(expression) => {
            expression_static_value(&expression.expression)
        }
        _ => None,
    }
}

fn expression_static_value(expression: &Expression<'_>) -> Option<String> {
    match expression {
        Expression::Identifier(identifier) => Some(identifier.name.to_string()),
        Expression::StringLiteral(literal) => Some(literal.value.to_string()),
        Expression::NumericLiteral(literal) => Some(
            literal
                .raw
                .as_ref()
                .map_or_else(|| literal.value.to_string(), ToString::to_string),
        ),
        Expression::TemplateLiteral(template) if template.expressions.is_empty() => {
            Some(template_literal_value(template))
        }
        Expression::ParenthesizedExpression(expression) => {
            expression_static_value(&expression.expression)
        }
        Expression::TSAsExpression(expression) => expression_static_value(&expression.expression),
        Expression::TSSatisfiesExpression(expression) => {
            expression_static_value(&expression.expression)
        }
        Expression::TSNonNullExpression(expression) => {
            expression_static_value(&expression.expression)
        }
        Expression::TSTypeAssertion(expression) => expression_static_value(&expression.expression),
        _ => None,
    }
}

#[cfg(test)]
mod parser_identity_tests {
    use super::*;

    fn source() -> SourceFile {
        SourceFile::new(
            FileId::from_raw(0),
            std::path::PathBuf::from("src/app.ts"),
            "src/app.ts".to_string(),
            Language::TypeScript,
            "export {};\n".into(),
            "0123456789abcdef".to_string(),
        )
    }

    /// The parser label is a compile-time constant, so the guard is on the
    /// derivation: the layer key's toolchain component must be exactly the one
    /// built from the live label, and must move when the label moves.
    #[test]
    fn a_changed_parser_label_changes_the_ts_syntax_layer_key() {
        let file = source();
        let key = ts_syntax_layer_key(&[&file], "config");

        assert_eq!(
            key.toolchain_digest,
            ts_parser_toolchain_digest(TS_PARSER_BACKEND)
        );
        let moved = LayerCacheKeyParts {
            toolchain_digest: ts_parser_toolchain_digest("oxc-0.0.0"),
            ..key.clone()
        };
        assert_ne!(moved, key);
    }

    #[test]
    fn the_per_file_cache_key_carries_the_parser_identity() {
        let key = ts_file_cache_key(&source(), "config", "rule", "plan");
        assert_eq!(key.parser_identity, TS_PARSER_BACKEND);
    }
}

#[cfg(test)]
mod layer_digest_tests {
    use super::*;
    use crate::ts::local_db::LocalFactDb;
    use crate::ts::test_cache::FsAnalysisCache;
    use std::sync::atomic::{AtomicU64, Ordering};

    static CACHE_ROOT_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique_cache_root() -> std::path::PathBuf {
        let sequence = CACHE_ROOT_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "polint-ts-layer-digest-{}-{sequence}",
            std::process::id()
        ))
    }

    fn payload_for(source: &str) -> (LocalFactDb, SyntaxLayerPayload) {
        let mut db = LocalFactDb::new();
        FactDatabase::add_file(
            &mut db,
            std::path::PathBuf::from("main.ts"),
            "main.ts".to_string(),
            source.to_string(),
        );
        let owned = db.files().to_vec();
        let files = owned.iter().collect::<Vec<_>>();
        let payload = parse_ts_syntax_layer_payload(
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
        let (db, payload) = payload_for("export function main() {}\n");
        let owned = db.files().to_vec();
        let files = owned.iter().collect::<Vec<_>>();
        let layer_key = ts_syntax_layer_key(&files, "config");

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
                let recomputed = ts_syntax_output_digest_for_payload(&payload)
                    .expect("written payload can be independently digested");
                assert_eq!(recorded, recomputed);
                validated_digest = Some(recorded);
                validate_syntax_layer_payload(&payload, digests, TS_SYNTAX_LAYER_SCHEMA, &files)
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
        let (db, payload) = payload_for("export function main() {}\n");
        let owned = db.files().to_vec();
        let files = owned.iter().collect::<Vec<_>>();
        let layer_key = ts_syntax_layer_key(&files, "config");

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

        let (_, other) = payload_for("export function other() {}\n");
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
                validate_syntax_layer_payload(&payload, digests, TS_SYNTAX_LAYER_SCHEMA, &files)
            };
            cache.read_layer_json(&layer_key, &mut validate)
        };

        assert_eq!(read.status, LayerCacheReadStatus::InvalidEvicted);

        std::fs::remove_dir_all(&cache_root).ok();
    }
}
