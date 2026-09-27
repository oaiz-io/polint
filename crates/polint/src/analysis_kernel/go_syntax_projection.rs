use super::incremental::{Digest, DigestBuilder, DigestKind, PrecisionTier};
use super::outcome::ProviderOutcomeError;
use super::{
    ProviderManifest, ProviderOutcome, ProviderOutcomeStatus, ProviderOutputIdentity, provider,
};
use crate::analysis_api::{GO_PARSER_BACKEND, GO_PARSER_GRAMMAR};
use crate::cache::{Cache, CacheKey, CacheReadStatus};
use crate::core::{AnalysisDb, FileId, FunctionId, Language, SourceFile, Span};
use crate::diagnostics::{Diagnostic, TextRange};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const MAX_GO_ROWS: usize = 1_000_000;
pub(crate) const GO_FACT_SCHEMA: &str = "go-facts-v2";
pub(crate) const GO_PAYLOAD_SCHEMA: &str = "go-syntax-layer-v1";

fn ensure_capacity(
    count: usize,
    error: GoSyntaxProjectionError,
) -> Result<(), GoSyntaxProjectionError> {
    (count < MAX_GO_ROWS).then_some(()).ok_or(error)
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CanonicalGoSyntaxSource {
    pub(crate) path: String,
    pub(crate) language: Language,
    pub(crate) source_digest: Digest,
}

impl CanonicalGoSyntaxSource {
    #[cfg(test)]
    pub(crate) fn digest(&self) -> Digest {
        let mut digest = Digest::builder(DigestKind::SourceText, "go-syntax-source-v1");
        digest.field("path", &self.path);
        digest.field("language", "go");
        digest.field("source-kind", self.source_digest.kind.as_str());
        digest.field("source-value", &self.source_digest.value);
        digest.finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CanonicalGoSyntaxInputs {
    pub(crate) sources: Vec<CanonicalGoSyntaxSource>,
}

impl CanonicalGoSyntaxInputs {
    pub(crate) fn from_db(db: &AnalysisDb) -> Result<Self, GoSyntaxProjectionError> {
        let mut sources = Vec::new();
        for file in db.files() {
            if file.relative_path.ends_with(".go") && file.language != Language::Go {
                return Err(GoSyntaxProjectionError::Source);
            }
            if file.language != Language::Go {
                continue;
            }
            ensure_capacity(sources.len(), GoSyntaxProjectionError::Source)?;
            let path = canonical_path(&file.relative_path)?;
            let value = crate::diagnostics::fingerprint(&[file.source.as_ref()]);
            if value != file.content_hash || !valid_digest_value(&value) {
                return Err(GoSyntaxProjectionError::Source);
            }
            sources.push(CanonicalGoSyntaxSource {
                path,
                language: Language::Go,
                source_digest: Digest {
                    kind: DigestKind::SourceText,
                    value,
                },
            });
        }
        sources.sort();
        validate_inputs(&sources)?;
        Ok(Self { sources })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GoSyntaxParserContract {
    pub(crate) provider_id: String,
    pub(crate) provider_version: String,
    pub(crate) fact_schema: String,
    pub(crate) payload_schema: String,
    pub(crate) backend: String,
    pub(crate) grammar: String,
}

impl GoSyntaxParserContract {
    pub(crate) fn current() -> Self {
        let manifest = go_manifest();
        Self {
            provider_id: manifest.id.to_string(),
            provider_version: manifest.provider_version().to_string(),
            fact_schema: GO_FACT_SCHEMA.to_string(),
            payload_schema: GO_PAYLOAD_SCHEMA.to_string(),
            backend: GO_PARSER_BACKEND.to_string(),
            grammar: GO_PARSER_GRAMMAR.to_string(),
        }
    }

    #[cfg(test)]
    pub(crate) fn digest(&self) -> Digest {
        let mut digest = Digest::builder(DigestKind::ProviderParameters, "go-parser-contract-v1");
        for (label, value) in [
            ("provider-id", self.provider_id.as_str()),
            ("provider-version", self.provider_version.as_str()),
            ("fact-schema", self.fact_schema.as_str()),
            ("payload-schema", self.payload_schema.as_str()),
            ("backend", self.backend.as_str()),
            ("grammar", self.grammar.as_str()),
        ] {
            digest.field(label, value);
        }
        digest.finish()
    }

    pub(crate) fn validate(&self) -> Result<(), GoSyntaxProjectionError> {
        (self == &Self::current())
            .then_some(())
            .ok_or(GoSyntaxProjectionError::Parser)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CanonicalGoSyntaxOutput {
    families: [Vec<String>; 7],
}

impl CanonicalGoSyntaxOutput {
    pub(crate) fn from_db(
        db: &AnalysisDb,
        diagnostics: &[Diagnostic],
    ) -> Result<Self, GoSyntaxProjectionError> {
        let context = GoContext::new(db)?;
        let mut functions = BTreeMap::new();
        let mut function_rows = BTreeSet::new();
        for fact in db.functions().iter().filter(|fact| context.owns(fact.file)) {
            ensure_capacity(function_rows.len(), GoSyntaxProjectionError::Output)?;
            let source = context.source(fact.file)?;
            if fact.language != Language::Go || !fact.calls.is_sorted() {
                return Err(GoSyntaxProjectionError::Output);
            }
            let span = canonical_span(&fact.span, source, context.line_count(fact.file)?)?;
            let row = row_digest("function", |row| {
                append_path_span(row, &source.relative_path, span);
                row.field("name", &fact.name);
                row.field("language", "go");
                row.bool_part(fact.is_test);
                row.bool_part(fact.is_exported);
                row.u64_field("complexity", u64::from(fact.cyclomatic_complexity));
                for call in &fact.calls {
                    row.field("call", call);
                }
            });
            let function = FunctionOccurrence {
                row: row.clone(),
                file: fact.file,
                span,
            };
            if !function_rows.insert(row) || functions.insert(fact.id, function).is_some() {
                return Err(GoSyntaxProjectionError::Output);
            }
        }
        let mut packages = Vec::new();
        for fact in db.packages().iter().filter(|fact| context.owns(fact.file)) {
            ensure_capacity(packages.len(), GoSyntaxProjectionError::Output)?;
            let source = context.source(fact.file)?;
            if fact.language != Language::Go {
                return Err(GoSyntaxProjectionError::Output);
            }
            let span = canonical_span(&fact.span, source, context.line_count(fact.file)?)?;
            packages.push(row_digest("package", |row| {
                append_path_span(row, &source.relative_path, span);
                row.field("name", &fact.name);
                row.field("language", "go");
            }));
        }
        let mut imports = Vec::new();
        for fact in db.imports().iter().filter(|fact| context.owns(fact.file)) {
            ensure_capacity(imports.len(), GoSyntaxProjectionError::Output)?;
            let source = context.source(fact.file)?;
            if fact.language != Language::Go {
                return Err(GoSyntaxProjectionError::Output);
            }
            let span = canonical_span(&fact.span, source, context.line_count(fact.file)?)?;
            imports.push(row_digest("import", |row| {
                append_path_span(row, &source.relative_path, span);
                row.field("package", fact.package.as_deref().unwrap_or(""));
                row.field("path", &fact.path);
                row.field("language", "go");
            }));
        }
        let mut tests = Vec::new();
        for fact in db.tests().iter().filter(|fact| context.owns(fact.file)) {
            ensure_capacity(tests.len(), GoSyntaxProjectionError::Output)?;
            let source = context.source(fact.file)?;
            let span = canonical_span(&fact.span, source, context.line_count(fact.file)?)?;
            let function = function_ref(fact.function, fact.file, span, true, &functions)?;
            tests.push(row_digest("test", |row| {
                append_path_span(row, &source.relative_path, span);
                row.field("function", function);
                row.field("name", &fact.name);
                for term in &fact.evidence_terms {
                    row.field("evidence", term);
                }
                row.u64_field("assertions", u64::from(fact.assertion_count));
                row.u64_field("subtests", u64::from(fact.subtest_count));
                for name in &fact.subtest_names {
                    row.field("subtest-name", name);
                }
                row.u64_field("table-rows", u64::from(fact.table_rows));
            }));
        }
        let mut branches = Vec::new();
        for fact in db.branches().iter().filter(|fact| context.owns(fact.file)) {
            ensure_capacity(branches.len(), GoSyntaxProjectionError::Output)?;
            let source = context.source(fact.file)?;
            let span = canonical_span(&fact.decision_span, source, context.line_count(fact.file)?)?;
            let function = function_ref(fact.function, fact.file, span, false, &functions)?;
            branches.push(row_digest("branch", |row| {
                append_path_span(row, &source.relative_path, span);
                row.field("function", function);
                row.field("condition", &fact.condition_text);
                row.field("edge", &fact.edge_label);
                row.bool_part(fact.is_error_path);
                row.field("fingerprint", &fact.stable_fingerprint);
            }));
        }
        let mut literals = Vec::new();
        for fact in db
            .string_literals()
            .iter()
            .filter(|fact| context.owns(fact.file))
        {
            ensure_capacity(literals.len(), GoSyntaxProjectionError::Output)?;
            let source = context.source(fact.file)?;
            if fact.language != Language::Go {
                return Err(GoSyntaxProjectionError::Output);
            }
            let span = canonical_span(&fact.span, source, context.line_count(fact.file)?)?;
            literals.push(row_digest("string-literal", |row| {
                append_path_span(row, &source.relative_path, span);
                row.field("value", &fact.value);
                row.field("language", "go");
            }));
        }
        let mut parser_diagnostics = Vec::new();
        for diagnostic in diagnostics {
            if diagnostic.rule_id == "internal/cache" {
                return Err(GoSyntaxProjectionError::Output);
            }
            if diagnostic.rule_id != "parser/go" {
                continue;
            }
            ensure_capacity(parser_diagnostics.len(), GoSyntaxProjectionError::Output)?;
            let source = context.by_path(&diagnostic.file)?;
            validate_range(diagnostic.range, source)?;
            parser_diagnostics.push(row_digest("parser-diagnostic", |row| {
                row.field("severity", &diagnostic.severity.to_string());
                row.field("path", &diagnostic.file);
                append_range(row, diagnostic.range);
                row.field("message", &diagnostic.message);
                row.field("fingerprint", &diagnostic.stable_fingerprint);
            }));
        }
        let mut families = [
            packages,
            function_rows.into_iter().collect(),
            imports,
            tests,
            branches,
            literals,
            parser_diagnostics,
        ];
        families.iter_mut().for_each(|rows| rows.sort());
        Ok(Self { families })
    }

    pub(crate) fn digest(&self) -> Digest {
        let mut digest = Digest::builder(DigestKind::ProviderOutput, "go-syntax-output-v1");
        for (family, rows) in [
            "packages",
            "functions",
            "imports",
            "go-tests",
            "branch-obligations",
            "string-literals",
            "parser-diagnostics",
        ]
        .into_iter()
        .zip(&self.families)
        {
            digest.field("family", family);
            digest.u64_field("row-count", rows.len() as u64);
            rows.iter().for_each(|row| digest.field("row", row));
        }
        digest.finish()
    }
}

/// Version of the canonical Go syntax output recipe: the rows
/// [`CanonicalGoSyntaxOutput::from_db`] builds and the fold in
/// [`CanonicalGoSyntaxOutput::digest`]. Memoized digests are keyed by it, so any
/// change to either must change this string.
const CANONICAL_OUTPUT_MEMO_SCHEMA: &str = "go-syntax-canonical-output-memo-v1";

/// Stands in for a source path in the memo's cache key; it names no file.
const CANONICAL_OUTPUT_MEMO_KEY: &str = "<go-syntax-canonical-output>";

#[derive(Serialize, Deserialize)]
struct CanonicalOutputMemo {
    schema: String,
    native: Digest,
    canonical: Digest,
}

/// The canonical Go syntax output digest, read from the analysis cache when the
/// Go syntax provider's native output digest was seen by an earlier run.
///
/// The native digest names every Go file's path, content hash, and serialized
/// facts and parser diagnostics, and the canonical projection is a function of
/// exactly those restored facts and diagnostics, so one native digest has one
/// canonical digest. A warm run restores the facts from an unchanged layer and
/// would otherwise recompute the projection — every Go fact projected,
/// digested, and sorted on the scheduler thread — to arrive at the same value.
/// A projection that fails is recomputed every time rather than remembered.
pub(crate) fn canonical_go_syntax_digest(
    db: &AnalysisDb,
    cache: &Cache,
    native: Option<&Digest>,
    parser_diagnostics: &[Diagnostic],
) -> Option<Digest> {
    let key = native.map(canonical_output_memo_key);
    if let (Some(key), Some(native)) = (&key, native)
        && let Some(canonical) = read_canonical_output_memo(cache, key, native)
    {
        return Some(canonical);
    }
    let canonical = CanonicalGoSyntaxOutput::from_db(db, parser_diagnostics)
        .ok()?
        .digest();
    if let (Some(key), Some(native)) = (&key, native) {
        let memo = CanonicalOutputMemo {
            schema: CANONICAL_OUTPUT_MEMO_SCHEMA.to_string(),
            native: native.clone(),
            canonical: canonical.clone(),
        };
        // Best effort: a memo that cannot be written only costs the next run
        // the projection.
        if let Ok(bytes) = serde_json::to_vec(&memo) {
            let _ = cache.write_json_bytes_with_status(key, &bytes);
        }
    }
    Some(canonical)
}

fn canonical_output_memo_key(native: &Digest) -> CacheKey {
    CacheKey::for_file(
        CANONICAL_OUTPUT_MEMO_KEY,
        &native.value,
        native.kind.as_str(),
        "",
        "",
        CANONICAL_OUTPUT_MEMO_SCHEMA,
        GO_PAYLOAD_SCHEMA,
    )
}

fn read_canonical_output_memo(cache: &Cache, key: &CacheKey, native: &Digest) -> Option<Digest> {
    let read = cache.read_json_bytes_with_status(key);
    if read.status != CacheReadStatus::Hit {
        return None;
    }
    let memo = serde_json::from_slice::<CanonicalOutputMemo>(&read.value?).ok()?;
    (memo.schema == CANONICAL_OUTPUT_MEMO_SCHEMA
        && memo.native == *native
        && memo.canonical.kind == DigestKind::ProviderOutput)
        .then_some(memo.canonical)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GoSyntaxProviderProjection {
    pub(crate) manifest: ProviderManifest,
    pub(crate) outcome: ProviderOutcome,
    pub(crate) inputs: Option<CanonicalGoSyntaxInputs>,
    pub(crate) parser: Option<GoSyntaxParserContract>,
}

impl GoSyntaxProviderProjection {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the private publication path is not consumed by normal kernel runs"
        )
    )]
    pub(crate) fn from_db(
        outcome: ProviderOutcome,
        db: &AnalysisDb,
        diagnostics: &[Diagnostic],
    ) -> Result<Self, GoSyntaxProjectionError> {
        let (inputs, parser) = if outcome.status == ProviderOutcomeStatus::Succeeded {
            let identity = outcome
                .output_identity
                .as_ref()
                .ok_or(GoSyntaxProjectionError::Output)?;
            if identity.output_digest != CanonicalGoSyntaxOutput::from_db(db, diagnostics)?.digest()
            {
                return Err(GoSyntaxProjectionError::Output);
            }
            (
                Some(CanonicalGoSyntaxInputs::from_db(db)?),
                Some(GoSyntaxParserContract::current()),
            )
        } else {
            (None, None)
        };
        Self::from_durable_parts(outcome, inputs, parser)
    }

    pub(crate) fn from_durable_parts(
        outcome: ProviderOutcome,
        inputs: Option<CanonicalGoSyntaxInputs>,
        parser: Option<GoSyntaxParserContract>,
    ) -> Result<Self, GoSyntaxProjectionError> {
        let manifest = go_manifest();
        let outcome = ProviderOutcome::from_closed_parts(
            outcome.provider_id,
            outcome.status,
            outcome.output_identity,
            outcome.failure_stage,
            outcome.failure_reason,
            outcome.blockers,
        )?;
        if outcome.provider_id != manifest.id
            || outcome
                .blockers
                .iter()
                .any(|blocker| blocker != "polint.source")
        {
            return Err(GoSyntaxProjectionError::Output);
        }
        match (&outcome.output_identity, &inputs, &parser) {
            (Some(identity), Some(inputs), Some(parser)) => {
                validate_identity(identity, &manifest)?;
                validate_inputs(&inputs.sources)?;
                parser.validate()?;
            }
            (None, None, None) if outcome.status != ProviderOutcomeStatus::Succeeded => {}
            _ => return Err(GoSyntaxProjectionError::Output),
        }
        Ok(Self {
            manifest,
            outcome,
            inputs,
            parser,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GoSyntaxProjectionError {
    #[error("Go syntax projection contains an invalid source")]
    Source,
    #[error("Go syntax projection contains an invalid parser contract")]
    Parser,
    #[error("Go syntax projection contains inconsistent output")]
    Output,
    #[error("Go syntax projection contains an invalid sealed outcome")]
    Outcome(#[from] ProviderOutcomeError),
}

struct GoContext<'a> {
    files: BTreeMap<FileId, &'a SourceFile>,
    paths: BTreeMap<&'a str, &'a SourceFile>,
    /// Line count per owned file, counted once here rather than per fact.
    ///
    /// Span validation needs the count for every fact, and every fact belongs to
    /// a file this context already owns, so the whole set is derived up front.
    /// The count is a pure function of the immutable source text, which makes the
    /// memo indistinguishable from recounting at each use.
    line_counts: BTreeMap<FileId, u32>,
}

impl<'a> GoContext<'a> {
    fn new(db: &'a AnalysisDb) -> Result<Self, GoSyntaxProjectionError> {
        let inputs = CanonicalGoSyntaxInputs::from_db(db)?;
        let allowed = inputs
            .sources
            .iter()
            .map(|source| source.path.as_str())
            .collect::<BTreeSet<_>>();
        let mut files = BTreeMap::new();
        let mut paths = BTreeMap::new();
        let mut line_counts = BTreeMap::new();
        for file in db
            .files()
            .iter()
            .filter(|file| file.language == Language::Go)
        {
            if !allowed.contains(file.relative_path.as_str())
                || files.insert(file.id, file).is_some()
                || paths.insert(file.relative_path.as_str(), file).is_some()
            {
                return Err(GoSyntaxProjectionError::Source);
            }
            let lines = u32::try_from(file.source.lines().count())
                .map_err(|_| GoSyntaxProjectionError::Output)?;
            line_counts.insert(file.id, lines);
        }
        Ok(Self {
            files,
            paths,
            line_counts,
        })
    }
    fn owns(&self, file: FileId) -> bool {
        self.files.contains_key(&file)
    }
    fn source(&self, file: FileId) -> Result<&'a SourceFile, GoSyntaxProjectionError> {
        self.files
            .get(&file)
            .copied()
            .ok_or(GoSyntaxProjectionError::Output)
    }
    fn line_count(&self, file: FileId) -> Result<u32, GoSyntaxProjectionError> {
        self.line_counts
            .get(&file)
            .copied()
            .ok_or(GoSyntaxProjectionError::Output)
    }
    fn by_path(&self, path: &str) -> Result<&'a SourceFile, GoSyntaxProjectionError> {
        self.paths
            .get(path)
            .copied()
            .ok_or(GoSyntaxProjectionError::Output)
    }
}

type CanonicalSpan = (u32, u32, u32, u32, u32, u32);

struct FunctionOccurrence {
    row: String,
    file: FileId,
    span: CanonicalSpan,
}

/// Validate `span` against `source` and reduce it to its canonical tuple.
///
/// `line_count` must be the line count of `source`; it is passed in because the
/// caller holds it for every owned file already.
fn canonical_span(
    span: &Span,
    source: &SourceFile,
    line_count: u32,
) -> Result<CanonicalSpan, GoSyntaxProjectionError> {
    if span.file != source.id
        || span.start_byte > span.end_byte
        || usize::try_from(span.end_byte).map_or(true, |end| end > source.source.len())
        || span.start_line == 0
        || span.start_line > span.end_line
        || span.end_line > line_count
        || span.start_col == 0
        || (span.start_line, span.start_col) > (span.end_line, span.end_col)
    {
        return Err(GoSyntaxProjectionError::Output);
    }
    Ok((
        span.start_byte,
        span.end_byte,
        span.start_line,
        span.start_col,
        span.end_line,
        span.end_col,
    ))
}

fn validate_range(range: TextRange, source: &SourceFile) -> Result<(), GoSyntaxProjectionError> {
    let lines = u32::try_from(source.source.lines().count())
        .map_err(|_| GoSyntaxProjectionError::Output)?;
    if range.start_line == 0
        || range.start_line > range.end_line
        || range.end_line > lines
        || range.start_col == 0
        || (range.start_line, range.start_col) > (range.end_line, range.end_col)
    {
        return Err(GoSyntaxProjectionError::Output);
    }
    Ok(())
}

fn function_ref(
    id: Option<FunctionId>,
    file: FileId,
    span: CanonicalSpan,
    exact_span: bool,
    functions: &BTreeMap<FunctionId, FunctionOccurrence>,
) -> Result<&str, GoSyntaxProjectionError> {
    let function = functions
        .get(&id.ok_or(GoSyntaxProjectionError::Output)?)
        .ok_or(GoSyntaxProjectionError::Output)?;
    let span_matches = if exact_span {
        function.span == span
    } else {
        span_contains(function.span, span)
    };
    (function.file == file && span_matches)
        .then_some(function.row.as_str())
        .ok_or(GoSyntaxProjectionError::Output)
}

fn span_contains(outer: CanonicalSpan, inner: CanonicalSpan) -> bool {
    outer.0 <= inner.0
        && inner.1 <= outer.1
        && (outer.2, outer.3) <= (inner.2, inner.3)
        && (inner.4, inner.5) <= (outer.4, outer.5)
}

fn row_digest(label: &'static str, append: impl FnOnce(&mut DigestBuilder)) -> String {
    let mut digest = Digest::builder(DigestKind::ProviderParameters, label);
    append(&mut digest);
    digest.finish().to_string()
}

fn append_path_span(digest: &mut DigestBuilder, path: &str, span: CanonicalSpan) {
    digest.field("path", path);
    digest.u64_field("start-byte", u64::from(span.0));
    digest.u64_field("end-byte", u64::from(span.1));
    digest.u64_field("start-line", u64::from(span.2));
    digest.u64_field("start-col", u64::from(span.3));
    digest.u64_field("end-line", u64::from(span.4));
    digest.u64_field("end-col", u64::from(span.5));
}

fn append_range(digest: &mut DigestBuilder, range: TextRange) {
    digest.u64_field("start-line", u64::from(range.start_line));
    digest.u64_field("start-col", u64::from(range.start_col));
    digest.u64_field("end-line", u64::from(range.end_line));
    digest.u64_field("end-col", u64::from(range.end_col));
}

fn validate_inputs(sources: &[CanonicalGoSyntaxSource]) -> Result<(), GoSyntaxProjectionError> {
    if sources.len() > MAX_GO_ROWS || !sources.is_sorted() {
        return Err(GoSyntaxProjectionError::Source);
    }
    let mut paths = BTreeSet::new();
    for source in sources {
        if canonical_path(&source.path)? != source.path
            || source.language != Language::Go
            || source.source_digest.kind != DigestKind::SourceText
            || !valid_digest_value(&source.source_digest.value)
            || !paths.insert(source.path.as_str())
        {
            return Err(GoSyntaxProjectionError::Source);
        }
    }
    Ok(())
}

fn validate_identity(
    identity: &ProviderOutputIdentity,
    manifest: &ProviderManifest,
) -> Result<(), GoSyntaxProjectionError> {
    if identity.provider_id != manifest.id
        || identity.provider_version != manifest.provider_version()
        || identity.schema_version != manifest.primary_schema_label()
        || identity.precision != PrecisionTier::Syntax
        || identity.output_digest.kind != DigestKind::ProviderOutput
        || !valid_digest_value(&identity.output_digest.value)
    {
        return Err(GoSyntaxProjectionError::Output);
    }
    Ok(())
}

fn canonical_path(path: &str) -> Result<String, GoSyntaxProjectionError> {
    let bytes = path.as_bytes();
    if Path::new(path).is_absolute()
        || path.starts_with(['/', '\\'])
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
    {
        return Err(GoSyntaxProjectionError::Source);
    }
    let normalized =
        crate::repo_fs::normalize_repo_relative(path).ok_or(GoSyntaxProjectionError::Source)?;
    if normalized == "." || normalized != path {
        return Err(GoSyntaxProjectionError::Source);
    }
    Ok(normalized)
}

fn valid_digest_value(value: &str) -> bool {
    value.len() == 16
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn go_manifest() -> ProviderManifest {
    *provider::provider_manifests()
        .iter()
        .find(|manifest| manifest.id == "polint.go.syntax")
        .expect("static provider inventory contains polint.go.syntax")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{BranchId, BranchObligation, FunctionFact, TestFact};

    #[rustfmt::skip] fn literal_digest(path: &str, value: &str, line: u32, language: Language, count: usize) -> Result<Digest, GoSyntaxProjectionError> {
        let mut db = AnalysisDb::new(); let file = db.add_file(path.into(), path.into(), "first\nsecond\n".into());
        for _ in 0..count { db.push_string_literal(crate::core::StringLiteralFact::new(file, value.into(), Span::point(file, line, 1), language)); }
        Ok(CanonicalGoSyntaxOutput::from_db(&db, &[])?.digest())
    }
    #[derive(Clone, Copy, Debug)] #[rustfmt::skip]
    enum RelatedFact { Test, Branch }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)] #[rustfmt::skip]
    enum RelationshipMutation { Valid, Missing, Dangling, CrossFile, Span }
    #[rustfmt::skip] fn span(file: FileId, start: u32, end: u32) -> Span { Span::new(file, start, end, 1, start + 1, 1, end + 1) }
    #[rustfmt::skip] fn function_fact(file: FileId, name: String, span: Span) -> FunctionFact { FunctionFact::new(FunctionId::from_raw(0), file, name, span, Language::Go, true, true, 1, Vec::new()) }
    #[rustfmt::skip] fn relationship_projection(family: RelatedFact, mutation: RelationshipMutation) -> Result<CanonicalGoSyntaxOutput, GoSyntaxProjectionError> {
        let mut db = AnalysisDb::new(); let source = "012345678901234567890123456789012345678901234567890123456789\n";
        let file = db.add_file("a_test.go".into(), "a_test.go".into(), source.into()); let other_file = db.add_file("b_test.go".into(), "b_test.go".into(), source.into()); let function_span = span(file, 5, 40);
        let function = db.push_function(function_fact(file, "TestA".into(), function_span.clone())); let other = db.push_function(function_fact(other_file, "TestB".into(), span(other_file, 5, 40)));
        let function = match mutation { RelationshipMutation::Missing => None, RelationshipMutation::Dangling => Some(FunctionId::from_raw(99)), RelationshipMutation::CrossFile => Some(other), _ => Some(function) };
        match family {
            RelatedFact::Test => db.push_test(TestFact::new(file, function, "TestA".into(), if mutation == RelationshipMutation::Span { span(file, 6, 39) } else { function_span }, Vec::new(), 0, 0, Vec::new(), 0)),
            RelatedFact::Branch => { db.push_branch(BranchObligation::new(BranchId::from_raw(0), function, file, if mutation == RelationshipMutation::Span { span(file, 45, 46) } else { span(file, 20, 21) }, "condition".into(), "true".into(), false, "fingerprint".into())); }
        }
        CanonicalGoSyntaxOutput::from_db(&db, &[])
    }

    #[test]
    fn canonical_digest_memo_answers_only_for_the_native_digest_it_recorded() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(temp.path().join("analysis"), true);
        let mut go = AnalysisDb::new();
        let file = go.add_file(
            "a.go".into(),
            "a.go".into(),
            "package a\nfunc A() {}\n".into(),
        );
        go.push_function(function_fact(file, "A".into(), span(file, 10, 21)));
        let empty = AnalysisDb::new();
        let projected = CanonicalGoSyntaxOutput::from_db(&go, &[]).unwrap().digest();
        let empty_projection = CanonicalGoSyntaxOutput::from_db(&empty, &[])
            .unwrap()
            .digest();
        assert_ne!(projected, empty_projection);
        let native = Digest {
            kind: DigestKind::ProviderOutput,
            value: "0123456789abcdef".into(),
        };

        // A first run projects and records the answer for its native digest.
        assert_eq!(
            canonical_go_syntax_digest(&go, &cache, Some(&native), &[]),
            Some(projected.clone())
        );
        // A later run with that native digest is answered from the memo, without
        // projecting: the empty database would project to a different digest.
        assert_eq!(
            canonical_go_syntax_digest(&empty, &cache, Some(&native), &[]),
            Some(projected)
        );
        // Any other native digest, no native digest, or a disabled cache projects.
        let other = Digest {
            value: "fedcba9876543210".into(),
            ..native
        };
        assert_eq!(
            canonical_go_syntax_digest(&empty, &cache, Some(&other), &[]),
            Some(empty_projection.clone())
        );
        assert_eq!(
            canonical_go_syntax_digest(&empty, &cache, None, &[]),
            Some(empty_projection.clone())
        );
        let disabled = Cache::new(temp.path().join("analysis"), false);
        assert_eq!(
            canonical_go_syntax_digest(&empty, &disabled, Some(&native), &[]),
            Some(empty_projection)
        );
    }

    #[test]
    fn canonical_inputs_are_ordered_and_parser_contract_is_closed() {
        let mut db = AnalysisDb::new();
        db.add_file("b.go".into(), "b.go".into(), "package b\n".into());
        db.add_file("a.go".into(), "a.go".into(), "package a\n".into());
        db.add_file("x.ts".into(), "x.ts".into(), "export {};\n".into());
        let inputs = CanonicalGoSyntaxInputs::from_db(&db).unwrap();
        assert_eq!(
            inputs
                .sources
                .iter()
                .map(|source| source.path.as_str())
                .collect::<Vec<_>>(),
            ["a.go", "b.go"]
        );
        GoSyntaxParserContract::current().validate().unwrap();
    }

    #[test] #[rustfmt::skip]
    fn unrelated_language_volume_is_filtered_before_owned_bounds() {
        let mut go = AnalysisDb::new(); go.add_file("a.go".into(), "a.go".into(), "package a\n".into());
        let expected = (CanonicalGoSyntaxInputs::from_db(&go).unwrap(), CanonicalGoSyntaxOutput::from_db(&go, &[]).unwrap());
        let mut mixed = go.clone(); mixed.add_file("unrelated.ts".into(), "unrelated.ts".into(), "import value from 'pkg'; export function pick(flag: boolean) { return flag ? 'yes' : value; }\n".into());
        let ts_file = mixed
            .files()
            .iter()
            .find(|file| file.relative_path == "unrelated.ts")
            .map(|file| file.id)
            .expect("mixed TypeScript file");
        mixed.push_function(crate::core::FunctionFact::new(
            crate::core::FunctionId::from_raw(100),
            ts_file,
            "pick".into(),
            Span::point(ts_file, 1, 1),
            Language::TypeScript,
            false,
            true,
            1,
            Vec::new(),
        ));
        mixed.push_import(crate::core::ImportFact::new(
            crate::core::ImportId::from_raw(100),
            ts_file,
            Some("pkg".into()),
            "pkg".into(),
            Span::point(ts_file, 1, 1),
            Language::TypeScript,
        ));
        mixed.push_string_literal(crate::core::StringLiteralFact::new(
            ts_file,
            "yes".into(),
            Span::point(ts_file, 1, 1),
            Language::TypeScript,
        ));
        let diagnostics = vec![Diagnostic::warning(
            "parser/ts",
            "unrelated.ts",
            TextRange::point(1, 1),
            "unrelated",
        )];
        assert!(!mixed.functions().is_empty() && !mixed.imports().is_empty() && !mixed.string_literals().is_empty());
        assert_eq!((CanonicalGoSyntaxInputs::from_db(&mixed).unwrap(), CanonicalGoSyntaxOutput::from_db(&mixed, &diagnostics).unwrap()), expected);
        let mut selected = 0;
        (0..10_000).map(|_| false).chain([true]).filter(|owned| *owned).try_for_each(|_| { ensure_capacity(selected, GoSyntaxProjectionError::Output)?; selected += 1; Ok::<(), GoSyntaxProjectionError>(()) }).unwrap();
        assert_eq!((selected, ensure_capacity(MAX_GO_ROWS, GoSyntaxProjectionError::Output).is_err()), (1, true));
    }
    #[test] #[rustfmt::skip]
    fn test_and_branch_function_relationships_are_exact() { for family in [RelatedFact::Test, RelatedFact::Branch] { for mutation in [RelationshipMutation::Valid, RelationshipMutation::Missing, RelationshipMutation::Dangling, RelationshipMutation::CrossFile, RelationshipMutation::Span] { assert_eq!(relationship_projection(family, mutation).is_ok(), mutation == RelationshipMutation::Valid, "{family:?} {mutation:?}"); } } }
    #[test] #[rustfmt::skip]
    fn function_row_uniqueness_scales_through_the_sorted_set() {
        let mut db = AnalysisDb::new(); let file = db.add_file("many.go".into(), "many.go".into(), "0123456789\n".into()); let span = span(file, 0, 1);
        for index in 0..4_096 { db.push_function(function_fact(file, format!("f{index}"), span.clone())); }
        let unique = CanonicalGoSyntaxOutput::from_db(&db, &[]).is_ok();
        db.push_function(function_fact(file, "f0".into(), span));
        assert_eq!((unique, CanonicalGoSyntaxOutput::from_db(&db, &[]).is_err()), (true, true));
    }
    /// The memoized line count must agree with counting the source at each use,
    /// and each fact must be validated against *its own* file, not whichever file
    /// happens to be longest in the projection.
    #[test]
    fn line_counts_are_memoized_per_file_and_match_a_per_fact_recount() {
        let mut db = AnalysisDb::new();
        let short = db.add_file("short.go".into(), "short.go".into(), "package a\n".into());
        let long = db.add_file(
            "long.go".into(),
            "long.go".into(),
            "package b\nconst x = 1\nconst y = 2\n".into(),
        );
        db.push_function(function_fact(short, "A".into(), Span::point(short, 1, 1)));
        db.push_function(function_fact(long, "B".into(), Span::point(long, 3, 1)));
        let context = GoContext::new(&db).expect("context");
        for fact in db.functions() {
            let source = context.source(fact.file).expect("owned source");
            let recounted =
                u32::try_from(source.source.lines().count()).expect("line count fits in u32");
            assert_eq!(context.line_count(fact.file).expect("memo"), recounted);
            assert_eq!(
                canonical_span(
                    &fact.span,
                    source,
                    context.line_count(fact.file).expect("memo")
                )
                .expect("memoized span"),
                canonical_span(&fact.span, source, recounted).expect("recounted span"),
            );
        }
        assert!(CanonicalGoSyntaxOutput::from_db(&db, &[]).is_ok());

        // A span past the short file's last line stays invalid even though the
        // longer file in the same projection would admit it.
        let mut beyond = db.clone();
        beyond.push_function(function_fact(short, "C".into(), Span::point(short, 3, 1)));
        assert!(CanonicalGoSyntaxOutput::from_db(&beyond, &[]).is_err());
    }

    #[test]
    fn every_parser_contract_member_changes_its_digest() {
        let contract = GoSyntaxParserContract::current();
        for mutate in [
            |value: &mut GoSyntaxParserContract| value.provider_id.push('x'),
            |value: &mut GoSyntaxParserContract| value.provider_version.push('x'),
            |value: &mut GoSyntaxParserContract| value.fact_schema.push('x'),
            |value: &mut GoSyntaxParserContract| value.payload_schema.push('x'),
            |value: &mut GoSyntaxParserContract| value.backend.push('x'),
            |value: &mut GoSyntaxParserContract| value.grammar.push('x'),
        ] {
            let mut changed = contract.clone();
            mutate(&mut changed);
            assert_ne!(changed.digest(), contract.digest());
            assert!(changed.validate().is_err());
        }
    }

    #[test]
    fn operational_cache_diagnostics_are_not_semantic_output() {
        let mut db = AnalysisDb::new();
        db.add_file("a.go".into(), "a.go".into(), "package a\n".into());
        let warning = Diagnostic::warning(
            "internal/cache",
            "a.go",
            TextRange::point(1, 1),
            "write failed",
        );
        assert!(CanonicalGoSyntaxOutput::from_db(&db, &[warning]).is_err());
    }

    #[test] #[rustfmt::skip]
    fn every_produced_family_and_parser_diagnostic_changes_output_identity() -> Result<(), GoSyntaxProjectionError> {
        let baseline = CanonicalGoSyntaxOutput { families: std::array::from_fn(|index| vec![format!("row-{index}")]) };
        for family in 0..baseline.families.len() { let mut changed = baseline.clone(); changed.families[family].push("additional-row".into()); assert_ne!(changed.digest(), baseline.digest(), "family {family}"); }
        let mut db = AnalysisDb::new(); db.add_file("a.go".into(), "a.go".into(), "package a\n".into());
        let diagnostic = Diagnostic::warning("parser/go", "a.go", TextRange::point(1, 1), "recoverable parse error");
        assert_ne!(CanonicalGoSyntaxOutput::from_db(&db, &[])?.digest(), CanonicalGoSyntaxOutput::from_db(&db, &[diagnostic])?.digest());
        Ok(())
    }
    #[test] #[rustfmt::skip]
    fn string_literal_path_value_span_language_and_multiplicity_are_consumed() {
        let baseline = literal_digest("a.go", "alpha", 1, Language::Go, 1).unwrap();
        for changed in [literal_digest("b.go", "alpha", 1, Language::Go, 1).unwrap(), literal_digest("a.go", "beta", 1, Language::Go, 1).unwrap(), literal_digest("a.go", "alpha", 2, Language::Go, 1).unwrap(), literal_digest("a.go", "alpha", 1, Language::Go, 2).unwrap()] { assert_ne!(changed, baseline); }
        assert!(literal_digest("a.go", "alpha", 1, Language::JavaScript, 1).is_err());
    }
}
