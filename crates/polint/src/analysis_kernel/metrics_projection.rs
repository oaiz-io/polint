use super::incremental::{Digest, DigestKind, PrecisionTier};
use super::outcome::ProviderOutcomeError;
use super::{
    ProviderManifest, ProviderOutcome, ProviderOutcomeStatus, ProviderOutputIdentity, provider,
};
use crate::analysis_api::is_synthetic_ts_js_module_function;
use crate::analysis_neutral::metrics::MetricFileSummary;
use crate::core::{
    AnalysisDb, ComplexityMetricFact, FileId, FileMetricFact, FunctionFact, FunctionId,
    FunctionMetricFact, Language, Span,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
const MAX_METRIC_ROWS: usize = 1_000_000;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CanonicalMetricSource {
    pub(crate) path: String,
    pub(crate) language: Language,
    pub(crate) source_digest: Digest,
    pub(crate) byte_count: u32,
    pub(crate) line_count: u32,
    pub(crate) non_empty_line_count: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CanonicalMetricFunction {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) start_byte: u32,
    pub(crate) end_byte: u32,
    pub(crate) start_line: u32,
    pub(crate) end_line: u32,
    pub(crate) language: Language,
    pub(crate) cyclomatic_complexity: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CanonicalMetricsInputs {
    pub(crate) sources: Vec<CanonicalMetricSource>,
    pub(crate) functions: Vec<CanonicalMetricFunction>,
}
type CanonicalFileMetric = (String, Language, u32, u32, u32, u32);
type CanonicalFunctionMetric = (CanonicalMetricFunction, u32, u32);
type CanonicalComplexityMetric = (CanonicalMetricFunction, u32);
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CanonicalMetricsOutput {
    file_metrics: Vec<CanonicalFileMetric>,
    function_metrics: Vec<CanonicalFunctionMetric>,
    complexity_metrics: Vec<CanonicalComplexityMetric>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MetricsProviderProjection {
    pub(crate) manifest: ProviderManifest,
    pub(crate) outcome: ProviderOutcome,
    pub(crate) inputs: Option<CanonicalMetricsInputs>,
}
#[derive(Debug, thiserror::Error)]
pub(crate) enum MetricsProjectionError {
    #[error("metrics projection contains an invalid source")]
    Source,
    #[error("metrics projection contains inconsistent metric output")]
    Output,
    #[error("metrics projection contains an invalid sealed outcome")]
    Outcome(#[from] ProviderOutcomeError),
}
pub(crate) struct CanonicalMetricsContext {
    inputs: CanonicalMetricsInputs,
    sources: BTreeMap<FileId, CanonicalMetricSource>,
    functions: BTreeMap<FunctionId, (CanonicalMetricFunction, FunctionFact)>,
    file_summaries: BTreeMap<FileId, MetricFileSummary>,
}
#[cfg(test)]
thread_local! {
    /// Canonical metrics contexts built on this thread, so a test can tell a
    /// run that skipped the projection from one that built it.
    static CONTEXTS_BUILT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[cfg(test)]
pub(crate) fn canonical_metrics_contexts_built_for_test() -> usize {
    CONTEXTS_BUILT.with(std::cell::Cell::get)
}
impl CanonicalMetricsInputs {
    #[cfg(test)]
    pub(crate) fn from_db(db: &AnalysisDb) -> Result<Self, MetricsProjectionError> {
        Ok(CanonicalMetricsContext::from_db(db)?.inputs)
    }
    pub(crate) fn source_digests(&self) -> Vec<Digest> {
        self.sources
            .iter()
            .map(CanonicalMetricSource::digest)
            .collect()
    }
    pub(crate) fn function_digests(&self) -> Vec<Digest> {
        self.functions
            .iter()
            .map(CanonicalMetricFunction::digest)
            .collect()
    }
}
impl CanonicalMetricSource {
    fn digest(&self) -> Digest {
        let mut digest = Digest::builder(DigestKind::SourceText, "metrics-source-input-v1");
        digest.field("path", &self.path);
        digest.field("language", language_label(self.language));
        digest.field("source-kind", self.source_digest.kind.as_str());
        digest.field("source-value", &self.source_digest.value);
        digest.u64_field("byte-count", u64::from(self.byte_count));
        digest.u64_field("line-count", u64::from(self.line_count));
        digest.u64_field("non-empty-line-count", u64::from(self.non_empty_line_count));
        digest.finish()
    }
}
impl CanonicalMetricFunction {
    fn digest(&self) -> Digest {
        let mut digest = Digest::builder(DigestKind::ProviderParameters, "metrics-function-v1");
        append_function(&mut digest, self);
        digest.finish()
    }
}
impl CanonicalMetricsOutput {
    #[cfg(test)]
    pub(crate) fn from_db(db: &AnalysisDb) -> Result<Self, MetricsProjectionError> {
        let context = CanonicalMetricsContext::from_db(db)?;
        Self::from_metric_facts_with_context(
            &context,
            db.file_metrics(),
            db.function_metrics(),
            db.complexity_metrics(),
        )
    }
    pub(crate) fn from_db_with_context(
        context: &CanonicalMetricsContext,
        db: &AnalysisDb,
    ) -> Result<Self, MetricsProjectionError> {
        Self::from_metric_facts_with_context(
            context,
            db.file_metrics(),
            db.function_metrics(),
            db.complexity_metrics(),
        )
    }
    pub(crate) fn from_metric_facts_with_context(
        context: &CanonicalMetricsContext,
        stored_file_metrics: &[FileMetricFact],
        stored_function_metrics: &[FunctionMetricFact],
        stored_complexity_metrics: &[ComplexityMetricFact],
    ) -> Result<Self, MetricsProjectionError> {
        let mut seen_files = BTreeSet::new();
        let mut file_metrics = Vec::with_capacity(stored_file_metrics.len());
        for metric in stored_file_metrics {
            let source = context
                .sources
                .get(&metric.file)
                .ok_or(MetricsProjectionError::Output)?;
            let function_count = context
                .file_summaries
                .get(&metric.file)
                .map(|summary| summary.function_count)
                .ok_or(MetricsProjectionError::Output)?;
            if !seen_files.insert(metric.file)
                || (
                    metric.language,
                    metric.line_count,
                    metric.non_empty_line_count,
                ) != (
                    source.language,
                    source.line_count,
                    source.non_empty_line_count,
                )
                || (metric.byte_count, metric.function_count) != (source.byte_count, function_count)
            {
                return Err(MetricsProjectionError::Output);
            }
            file_metrics.push((
                source.path.clone(),
                metric.language,
                metric.line_count,
                metric.non_empty_line_count,
                metric.byte_count,
                metric.function_count,
            ));
        }
        file_metrics.sort();
        if seen_files.len() != context.sources.len() {
            return Err(MetricsProjectionError::Output);
        }
        let mut seen_functions = BTreeSet::new();
        let mut function_metrics = Vec::with_capacity(stored_function_metrics.len());
        for metric in stored_function_metrics {
            let function = linked_function(
                context,
                metric.function,
                metric.file,
                &metric.span,
                &metric.name,
                metric.language,
            )?;
            if !seen_functions.insert(metric.function)
                || metric.line_count != function.end_line - function.start_line + 1
                || metric.byte_count != function.end_byte - function.start_byte
            {
                return Err(MetricsProjectionError::Output);
            }
            function_metrics.push((function.clone(), metric.line_count, metric.byte_count));
        }
        function_metrics.sort();
        if seen_functions.len() != context.functions.len() {
            return Err(MetricsProjectionError::Output);
        }
        seen_functions.clear();
        let mut complexity_metrics = Vec::with_capacity(stored_complexity_metrics.len());
        for metric in stored_complexity_metrics {
            let function = linked_function(
                context,
                metric.function,
                metric.file,
                &metric.span,
                &metric.name,
                metric.language,
            )?;
            if !seen_functions.insert(metric.function)
                || metric.cyclomatic_complexity != function.cyclomatic_complexity
            {
                return Err(MetricsProjectionError::Output);
            }
            complexity_metrics.push((function.clone(), metric.cyclomatic_complexity));
        }
        complexity_metrics.sort();
        if seen_functions.len() != context.functions.len() {
            return Err(MetricsProjectionError::Output);
        }
        Ok(Self {
            file_metrics,
            function_metrics,
            complexity_metrics,
        })
    }
    pub(crate) fn digest(&self) -> Digest {
        let mut digest = Digest::builder(DigestKind::ProviderOutput, "metrics-output-v1");
        digest.u64_field("file-count", self.file_metrics.len() as u64);
        for (path, language, lines, non_empty, bytes, functions) in &self.file_metrics {
            digest.field("file-path", path);
            digest.field("file-language", language_label(*language));
            digest.u64_field("line-count", u64::from(*lines));
            digest.u64_field("non-empty-line-count", u64::from(*non_empty));
            digest.u64_field("byte-count", u64::from(*bytes));
            digest.u64_field("function-count", u64::from(*functions));
        }
        digest.u64_field("function-count", self.function_metrics.len() as u64);
        for (function, lines, bytes) in &self.function_metrics {
            append_function(&mut digest, function);
            digest.u64_field("line-count", u64::from(*lines));
            digest.u64_field("byte-count", u64::from(*bytes));
        }
        digest.u64_field("complexity-count", self.complexity_metrics.len() as u64);
        for (function, complexity) in &self.complexity_metrics {
            append_function(&mut digest, function);
            digest.u64_field("complexity", u64::from(*complexity));
        }
        digest.finish()
    }
}
impl MetricsProviderProjection {
    #[cfg_attr(not(test), expect(dead_code))]
    pub(crate) fn from_db(
        outcome: ProviderOutcome,
        db: &AnalysisDb,
    ) -> Result<Self, MetricsProjectionError> {
        let inputs = match outcome.status {
            ProviderOutcomeStatus::Succeeded => {
                let identity = outcome
                    .output_identity
                    .as_ref()
                    .ok_or(MetricsProjectionError::Output)?;
                let context = CanonicalMetricsContext::from_db(db)?;
                if identity.output_digest
                    != CanonicalMetricsOutput::from_db_with_context(&context, db)?.digest()
                {
                    return Err(MetricsProjectionError::Output);
                }
                Some(context.inputs)
            }
            _ => None,
        };
        Self::from_durable_parts(outcome, inputs)
    }
    pub(crate) fn from_durable_parts(
        outcome: ProviderOutcome,
        inputs: Option<CanonicalMetricsInputs>,
    ) -> Result<Self, MetricsProjectionError> {
        let manifest = *provider::provider_manifests()
            .iter()
            .find(|manifest| manifest.id == "polint.metrics")
            .expect("static provider inventory contains polint.metrics");
        let outcome = ProviderOutcome::from_closed_parts(
            outcome.provider_id,
            outcome.status,
            outcome.output_identity,
            outcome.failure_stage,
            outcome.failure_reason,
            outcome.blockers,
        )?;
        if outcome.provider_id != manifest.id
            || outcome.blockers.iter().any(|blocker| {
                !super::outcome::hard_dependencies(manifest.id).contains(&blocker.as_str())
            })
        {
            return Err(MetricsProjectionError::Output);
        }
        match (&outcome.output_identity, &inputs) {
            (Some(identity), Some(inputs)) => {
                validate_identity(identity, &manifest)?;
                validate_inputs(inputs)?;
            }
            (None, None) if outcome.status != ProviderOutcomeStatus::Succeeded => {}
            _ => return Err(MetricsProjectionError::Output),
        }
        Ok(Self {
            manifest,
            outcome,
            inputs,
        })
    }
}
fn validate_inputs(inputs: &CanonicalMetricsInputs) -> Result<(), MetricsProjectionError> {
    if inputs.sources.len() > MAX_METRIC_ROWS
        || inputs.functions.len() > MAX_METRIC_ROWS
        || !inputs.sources.is_sorted()
        || !inputs.functions.is_sorted()
    {
        return Err(MetricsProjectionError::Source);
    }
    let mut sources = BTreeMap::new();
    for source in &inputs.sources {
        if canonical_path(&source.path)? != source.path
            || source.language == Language::Unknown
            || source.source_digest.kind != DigestKind::SourceText
            || !valid_digest_value(&source.source_digest.value)
            || source.non_empty_line_count > source.line_count
            || (source.byte_count == 0) != (source.line_count == 0)
            || sources.insert(source.path.as_str(), source).is_some()
        {
            return Err(MetricsProjectionError::Source);
        }
    }
    for function in &inputs.functions {
        let source = sources
            .get(function.path.as_str())
            .ok_or(MetricsProjectionError::Source)?;
        if function.language != source.language
            || function.start_byte > function.end_byte
            || function.end_byte > source.byte_count
            || function.start_line == 0
            || function.start_line > function.end_line
            || function.end_line > source.line_count
        {
            return Err(MetricsProjectionError::Source);
        }
    }
    Ok(())
}
impl CanonicalMetricsContext {
    pub(crate) fn from_db(db: &AnalysisDb) -> Result<Self, MetricsProjectionError> {
        #[cfg(test)]
        CONTEXTS_BUILT.with(|count| count.set(count.get() + 1));
        if db.files().len() > MAX_METRIC_ROWS || db.functions().len() > MAX_METRIC_ROWS {
            return Err(MetricsProjectionError::Source);
        }
        let mut sources = BTreeMap::new();
        let mut file_summaries = BTreeMap::new();
        let mut paths = BTreeSet::new();
        for file in db.files() {
            let path = canonical_path(&file.relative_path)?;
            if file.language == Language::Unknown
                || file.content_hash != crate::diagnostics::fingerprint(&[file.source.as_ref()])
                || !valid_digest_value(&file.content_hash)
                || !paths.insert(path.clone())
            {
                return Err(MetricsProjectionError::Source);
            }
            let summary = MetricFileSummary {
                byte_count: u32::try_from(file.byte_count())
                    .map_err(|_| MetricsProjectionError::Source)?,
                line_count: u32::try_from(file.line_count())
                    .map_err(|_| MetricsProjectionError::Source)?,
                non_empty_line_count: u32::try_from(file.non_empty_line_count())
                    .map_err(|_| MetricsProjectionError::Source)?,
                function_count: 0,
            };
            let row = CanonicalMetricSource {
                path,
                language: file.language,
                source_digest: Digest {
                    kind: DigestKind::SourceText,
                    value: file.content_hash.clone(),
                },
                byte_count: summary.byte_count,
                line_count: summary.line_count,
                non_empty_line_count: summary.non_empty_line_count,
            };
            if sources.insert(file.id, row).is_some()
                || file_summaries.insert(file.id, summary).is_some()
            {
                return Err(MetricsProjectionError::Source);
            }
        }
        let mut functions = BTreeMap::new();
        for function in db
            .functions()
            .iter()
            .filter(|function| !is_synthetic_ts_js_module_function(function))
        {
            let source = sources
                .get(&function.file)
                .ok_or(MetricsProjectionError::Source)?;
            let span = &function.span;
            if span.file != function.file
                || function.language != source.language
                || span.start_byte > span.end_byte
                || span.end_byte > source.byte_count
                || span.start_line == 0
                || span.start_line > span.end_line
                || span.end_line > source.line_count
            {
                return Err(MetricsProjectionError::Source);
            }
            let row = CanonicalMetricFunction {
                path: source.path.clone(),
                name: function.name.clone(),
                start_byte: span.start_byte,
                end_byte: span.end_byte,
                start_line: span.start_line,
                end_line: span.end_line,
                language: function.language,
                cyclomatic_complexity: function.cyclomatic_complexity,
            };
            if functions
                .insert(function.id, (row, function.clone()))
                .is_some()
            {
                return Err(MetricsProjectionError::Source);
            }
            let summary = file_summaries
                .get_mut(&function.file)
                .ok_or(MetricsProjectionError::Source)?;
            summary.function_count = summary
                .function_count
                .checked_add(1)
                .ok_or(MetricsProjectionError::Source)?;
        }
        let mut source_rows = sources.values().cloned().collect::<Vec<_>>();
        let mut function_rows = functions
            .values()
            .map(|(row, _)| row.clone())
            .collect::<Vec<_>>();
        source_rows.sort();
        function_rows.sort();
        Ok(Self {
            inputs: CanonicalMetricsInputs {
                sources: source_rows,
                functions: function_rows,
            },
            sources,
            functions,
            file_summaries,
        })
    }

    pub(crate) fn inputs(&self) -> &CanonicalMetricsInputs {
        &self.inputs
    }

    pub(crate) fn file_summaries(&self) -> &BTreeMap<FileId, MetricFileSummary> {
        &self.file_summaries
    }
}
fn linked_function<'a>(
    context: &'a CanonicalMetricsContext,
    id: FunctionId,
    file: FileId,
    span: &Span,
    name: &str,
    language: Language,
) -> Result<&'a CanonicalMetricFunction, MetricsProjectionError> {
    let (function, raw) = context
        .functions
        .get(&id)
        .ok_or(MetricsProjectionError::Output)?;
    if (file, span, name, language) != (raw.file, &raw.span, raw.name.as_str(), raw.language) {
        return Err(MetricsProjectionError::Output);
    }
    Ok(function)
}
fn canonical_path(path: &str) -> Result<String, MetricsProjectionError> {
    let bytes = path.as_bytes();
    if Path::new(path).is_absolute()
        || path.starts_with(['/', '\\'])
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
    {
        return Err(MetricsProjectionError::Source);
    }
    let normalized =
        crate::repo_fs::normalize_repo_relative(path).ok_or(MetricsProjectionError::Source)?;
    if normalized == "." || normalized != path {
        return Err(MetricsProjectionError::Source);
    }
    Ok(normalized)
}
fn validate_identity(
    identity: &ProviderOutputIdentity,
    manifest: &ProviderManifest,
) -> Result<(), MetricsProjectionError> {
    if identity.provider_id != manifest.id
        || identity.provider_version != manifest.provider_version()
        || identity.schema_version != manifest.primary_schema_label()
        || identity.precision != PrecisionTier::Syntax
        || identity.output_digest.kind != DigestKind::ProviderOutput
        || !valid_digest_value(&identity.output_digest.value)
    {
        return Err(MetricsProjectionError::Output);
    }
    Ok(())
}
fn valid_digest_value(value: &str) -> bool {
    value.len() == 16
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn append_function(digest: &mut super::incremental::DigestBuilder, row: &CanonicalMetricFunction) {
    digest.field("function-path", &row.path);
    digest.field("function-name", &row.name);
    digest.u64_field("start-byte", u64::from(row.start_byte));
    digest.u64_field("end-byte", u64::from(row.end_byte));
    digest.u64_field("start-line", u64::from(row.start_line));
    digest.u64_field("end-line", u64::from(row.end_line));
    digest.field("function-language", language_label(row.language));
    digest.u64_field("complexity", u64::from(row.cyclomatic_complexity));
}
pub(crate) const fn language_label(language: Language) -> &'static str {
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
#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_plan::AnalysisPlan;
    use crate::core::FunctionFact;
    fn fixture(order_reversed: bool, excluded_seed: bool, duplicates: usize) -> AnalysisDb {
        let rows = if order_reversed {
            [("src/b.ts", "b"), ("src/a.ts", "a")]
        } else {
            [("src/a.ts", "a"), ("src/b.ts", "b")]
        };
        let mut db = AnalysisDb::new();
        for (path, name) in rows {
            let source = format!("export function {name}() {{}}\n");
            let file = db.add_file(path.into(), path.to_string(), source.clone());
            let start = source.find(name).expect("function name") as u32;
            for _ in 0..duplicates.max(1) {
                db.push_function(FunctionFact::new(
                    FunctionId::from_raw(99),
                    file,
                    name.to_string(),
                    Span::new(file, start, source.len() as u32, 1, 1, 1, 1),
                    Language::TypeScript,
                    excluded_seed,
                    !excluded_seed,
                    2,
                    excluded_seed
                        .then(|| "ignored".into())
                        .into_iter()
                        .collect(),
                ));
            }
        }
        crate::metrics::derive_requested_metrics(
            &mut db,
            &AnalysisPlan::from_capability_names_for_test(&["file_metrics"]),
        );
        db
    }
    #[test]
    fn canonical_rows_ignore_transient_ids_order_and_unused_function_fields() {
        let first = fixture(false, false, 1);
        let second = fixture(true, true, 1);
        assert_eq!(
            CanonicalMetricsInputs::from_db(&first).unwrap(),
            CanonicalMetricsInputs::from_db(&second).unwrap()
        );
        let first_output = CanonicalMetricsOutput::from_db(&first).unwrap();
        let second_output = CanonicalMetricsOutput::from_db(&second).unwrap();
        assert_eq!(first_output, second_output);
        assert_eq!(first_output.digest(), second_output.digest());
        let duplicated = fixture(false, false, 2);
        let inputs = CanonicalMetricsInputs::from_db(&duplicated).unwrap();
        let output = CanonicalMetricsOutput::from_db(&duplicated).unwrap();
        assert_eq!(inputs.functions.len(), 4);
        assert_eq!(output.function_metrics.len(), 4);
    }

    #[test]
    fn dependency_blockers_must_be_actual_metrics_dependencies() {
        let forged = ProviderOutcome::from_closed_parts(
            "polint.metrics".into(),
            ProviderOutcomeStatus::DependencyBlocked,
            None,
            Some(crate::analysis_kernel::ProviderFailureStage::Dependency),
            Some(crate::analysis_kernel::ProviderFailureReason::DependencyUnavailable),
            vec!["polint.evidence".into()],
        )
        .unwrap();
        assert!(MetricsProviderProjection::from_db(forged, &AnalysisDb::new()).is_err());
    }
}
