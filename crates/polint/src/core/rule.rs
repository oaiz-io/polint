//! Rule registration, options, context, and execution.
//!
//! Extracted from the core monolith without behaviour changes.

use super::AnalysisDb;
use super::capability::{Capabilities, CapabilitySupportStatus, CapabilitySupportView};
use super::completeness::CompletenessView;
use super::ids::FileId;
use super::span::Span;
use crate::diagnostics::{Diagnostic, Severity, TextRange as DiagnosticRange, dedupe_diagnostics};
use crate::rule_error::RuleResult;
use crate::rule_manifest::{FactViewRequirement, RuleManifest};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

/// Arbitrary per-rule configuration value from `.polint.toml`.
///
/// Rules read these through [`RuleOptions::settings`] when the built-in shortcut
/// fields (`max`, `deny`, `forbidden_imports`, etc.) are not expressive enough.
pub type RuleConfigValue = toml::Value;

/// Whether a rule runs under `polint check` (`Check`) or `polint review` (`Review`).
///
/// Authored via `#[polint::rule(..., kind = "check" | "review")]`. Defaults to
/// `Check`, so every existing rule keeps its behavior. `polint check` executes
/// only `Check` rules; `polint review` executes only `Review` rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    /// A normal rule that runs under `polint check`.
    #[default]
    Check,
    /// A diff-gated rule that runs under `polint review`.
    Review,
}

/// Static metadata for a rule as shown in diagnostics, config, and registries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleMeta {
    pub id: String,
    pub description: String,
    pub severity: Severity,
    /// Which command runs this rule (`check` vs `review`); defaults to `check`.
    #[serde(default)]
    pub kind: RuleKind,
}

type RuleMetaFn = dyn Fn() -> RuleMeta + Send + Sync;
type RuleCapabilitiesFn = dyn Fn() -> Capabilities + Send + Sync;
type RuleRunFn = dyn Fn(&AnalysisDb, &mut RuleCtx<'_>) -> RuleResult + Send + Sync;

/// Opaque static-analysis rule registered with the runner.
///
/// Rule authors create this through `#[polint::rule]`. The value is intentionally
/// not a trait: repo-local rules must be written in the analyzable macro shape so
/// capabilities come from typed fact-view parameters.
#[derive(Clone)]
#[non_exhaustive]
pub struct Rule {
    meta: Arc<RuleMetaFn>,
    capabilities: Arc<RuleCapabilitiesFn>,
    fact_views: Arc<[FactViewRequirement]>,
    run: Arc<RuleRunFn>,
}

impl Rule {
    pub(crate) fn from_parts<M, C, R>(meta: M, capabilities: C, run: R) -> Self
    where
        M: Fn() -> RuleMeta + Send + Sync + 'static,
        C: Fn() -> Capabilities + Send + Sync + 'static,
        R: Fn(&AnalysisDb, &mut RuleCtx<'_>) -> RuleResult + Send + Sync + 'static,
    {
        Self {
            meta: Arc::new(meta),
            capabilities: Arc::new(capabilities),
            fact_views: Arc::from(Vec::new().into_boxed_slice()),
            run: Arc::new(run),
        }
    }

    pub(crate) fn from_parts_with_fact_views<M, C, R>(
        meta: M,
        capabilities: C,
        fact_views: Vec<FactViewRequirement>,
        run: R,
    ) -> Self
    where
        M: Fn() -> RuleMeta + Send + Sync + 'static,
        C: Fn() -> Capabilities + Send + Sync + 'static,
        R: Fn(&AnalysisDb, &mut RuleCtx<'_>) -> RuleResult + Send + Sync + 'static,
    {
        Self {
            meta: Arc::new(meta),
            capabilities: Arc::new(capabilities),
            fact_views: Arc::from(fact_views.into_boxed_slice()),
            run: Arc::new(run),
        }
    }

    pub(crate) fn meta(&self) -> RuleMeta {
        (self.meta)()
    }

    pub(crate) fn capabilities(&self) -> Capabilities {
        (self.capabilities)()
    }

    pub(crate) fn manifest(&self, options: Option<&RuleOptions>) -> RuleManifest {
        RuleManifest::from_parts(
            self.meta(),
            self.capabilities(),
            self.fact_views.iter().cloned().collect(),
            options,
        )
    }

    pub(crate) fn run(&self, db: &AnalysisDb, ctx: &mut RuleCtx<'_>) -> RuleResult {
        (self.run)(db, ctx)
    }
}

/// Resolved per-rule options from configuration.
///
/// Built-in and repo-local rules can read these values through
/// [`RuleCtx::options`] to apply severity overrides, file filters, thresholds,
/// denied values, or import-boundary settings.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct RuleOptions {
    /// Severity override from config, if any.
    pub severity: Option<Severity>,
    /// File include globs for this rule.
    pub files: Vec<String>,
    /// File globs to skip for this rule.
    pub allow_files: Vec<String>,
    /// Rule-defined allow list. Helpers treat this as exact file paths only when documented.
    pub allow: Vec<String>,
    /// Common numeric threshold used by example complexity/test-size rules.
    pub max: Option<u32>,
    /// Common deny list used by literal/pattern rules.
    pub deny: Vec<String>,
    /// Common import-boundary map: source glob -> forbidden import patterns.
    pub forbidden_imports: BTreeMap<String, Vec<String>>,
    /// Arbitrary extra fields from this rule's `[[rules.config]]` table.
    pub settings: BTreeMap<String, RuleConfigValue>,
}

/// Borrowed execution context passed to a single rule run.
///
/// The context owns reporting, options, path lookup, and support metadata.
/// Analysis facts are exposed through typed SDK fact views requested in a
/// `#[polint::rule]` function signature.
#[non_exhaustive]
pub struct RuleCtx<'a> {
    db: &'a AnalysisDb,
    diagnostics: Vec<Diagnostic>,
    rule: RuleMeta,
    options: RuleOptions,
    capability_support: CapabilitySupportView,
    completeness: CompletenessView,
}

impl<'a> RuleCtx<'a> {
    /// Creates a rule context for one rule execution.
    #[cfg(test)]
    pub(crate) fn new(db: &'a AnalysisDb, rule: RuleMeta, options: RuleOptions) -> Self {
        Self::with_capability_support(db, rule, options, CapabilitySupportView::empty())
    }

    #[cfg(test)]
    pub(crate) fn with_capability_support(
        db: &'a AnalysisDb,
        rule: RuleMeta,
        options: RuleOptions,
        capability_support: CapabilitySupportView,
    ) -> Self {
        Self::with_runtime_views(
            db,
            rule,
            options,
            capability_support,
            CompletenessView::unknown(),
        )
    }

    pub(crate) fn with_runtime_views(
        db: &'a AnalysisDb,
        rule: RuleMeta,
        options: RuleOptions,
        capability_support: CapabilitySupportView,
        completeness: CompletenessView,
    ) -> Self {
        Self {
            db,
            diagnostics: Vec::new(),
            rule,
            options,
            capability_support,
            completeness,
        }
    }

    /// Returns resolved options for the current rule.
    pub fn options(&self) -> &RuleOptions {
        &self.options
    }

    /// Returns the stable ID of the current rule.
    pub fn rule_id(&self) -> &str {
        &self.rule.id
    }

    /// Returns read-only capability support information for the resolved plan.
    pub fn capability_support(&self) -> &CapabilitySupportView {
        &self.capability_support
    }

    /// Returns completeness information for the current rule's requested capabilities.
    ///
    /// A status of [`CapabilityCompletenessStatus::Unknown`](super::CapabilityCompletenessStatus)
    /// means the host could not prove completeness for that capability. Rules should
    /// not interpret missing telemetry as a clean repository.
    pub fn completeness(&self) -> &CompletenessView {
        &self.completeness
    }

    /// Paths paired with `file`'s relative path under the named `[path_contexts]` rule.
    pub fn path_context_related(&self, pair_name: &str, file: FileId) -> Vec<String> {
        let Some(source) = self.db.file(file) else {
            return Vec::new();
        };
        self.db
            .path_context_related(pair_name, &source.relative_path)
    }

    /// Returns a display path for a file ID, or `<unknown>` if missing.
    pub fn file_path(&self, file: FileId) -> String {
        self.db.path_for(file)
    }

    /// Adds a diagnostic for the current rule, applying severity overrides.
    pub fn report(&mut self, mut diagnostic: Diagnostic) {
        if let Some(severity) = self.options.severity {
            diagnostic.severity = severity;
        }
        self.diagnostics.push(diagnostic);
    }

    /// Reports an error diagnostic at a core span.
    pub fn error(&mut self, span: &Span, message: impl Into<String>) {
        let file = self.file_path(span.file);
        self.report(Diagnostic::error(
            self.rule.id.clone(),
            file,
            span.diagnostic_range(),
            message,
        ));
    }

    /// Reports a warning diagnostic at a core span.
    pub fn warn(&mut self, span: &Span, message: impl Into<String>) {
        let file = self.file_path(span.file);
        self.report(Diagnostic::warning(
            self.rule.id.clone(),
            file,
            span.diagnostic_range(),
            message,
        ));
    }

    /// Consumes the context and returns buffered diagnostics.
    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }
}

/// Test-only registry that exposes the [`Capabilities`]/[`Rule`] shape independent of `run_rules`.
/// Production runs use [`run_rules`] directly with `&[Rule]`.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct RuleRegistry {
    rules: Vec<Rule>,
}

#[cfg(test)]
impl RuleRegistry {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn register(&mut self, rule: Rule) {
        self.rules.push(rule);
    }

    pub(crate) fn rules(&self) -> &[Rule] {
        &self.rules
    }
}

// Intentionally `pub` (not `pub(crate)`): `polint::_bench::core` glob-re-exports this for
// `polint-bench`, an external crate. `unreachable_pub` does not follow that path.
#[allow(unreachable_pub)]
pub fn run_rules(
    db: &AnalysisDb,
    rules: &[Rule],
    options: &BTreeMap<String, RuleOptions>,
    enabled: Option<&BTreeSet<String>>,
    parallel: bool,
) -> Vec<Diagnostic> {
    let capability_support = CapabilitySupportView::empty();
    run_rules_with_capability_support(db, rules, options, enabled, parallel, &capability_support)
}

pub(crate) struct RuleRuntimeViews<'a> {
    capability_support: &'a CapabilitySupportView,
    completeness: &'a CompletenessView,
    runtime_blocked_rules: &'a BTreeSet<String>,
}

impl<'a> RuleRuntimeViews<'a> {
    pub(crate) fn new(
        capability_support: &'a CapabilitySupportView,
        completeness: &'a CompletenessView,
        runtime_blocked_rules: &'a BTreeSet<String>,
    ) -> Self {
        Self {
            capability_support,
            completeness,
            runtime_blocked_rules,
        }
    }
}

pub(crate) fn run_rules_with_capability_support(
    db: &AnalysisDb,
    rules: &[Rule],
    options: &BTreeMap<String, RuleOptions>,
    enabled: Option<&BTreeSet<String>>,
    parallel: bool,
    capability_support: &CapabilitySupportView,
) -> Vec<Diagnostic> {
    let completeness = CompletenessView::unknown();
    let runtime_blocked_rules = BTreeSet::new();
    let runtime = RuleRuntimeViews::new(capability_support, &completeness, &runtime_blocked_rules);
    run_rules_with_runtime_provider_blockers(db, rules, options, enabled, parallel, &runtime)
}

pub(crate) fn run_rules_with_runtime_provider_blockers(
    db: &AnalysisDb,
    rules: &[Rule],
    options: &BTreeMap<String, RuleOptions>,
    enabled: Option<&BTreeSet<String>>,
    parallel: bool,
    runtime: &RuleRuntimeViews<'_>,
) -> Vec<Diagnostic> {
    let run_one = |rule: &Rule| {
        let meta = match catch_unwind(AssertUnwindSafe(|| rule.meta())) {
            Ok(meta) => meta,
            Err(_) => {
                return vec![internal_rule_error_for_id(
                    db,
                    "unknown",
                    "rule metadata panicked".to_string(),
                )];
            }
        };
        if let Some(enabled) = enabled
            && !enabled
                .iter()
                .any(|pattern| rule_id_matches(pattern, &meta.id))
        {
            return Vec::new();
        }
        if has_blocking_capability(&meta.id, runtime.capability_support)
            || runtime.runtime_blocked_rules.contains(&meta.id)
        {
            return Vec::new();
        }
        let rule_options = options.get(&meta.id).cloned().unwrap_or_default();
        let mut ctx = RuleCtx::with_runtime_views(
            db,
            meta.clone(),
            rule_options,
            runtime.capability_support.clone(),
            runtime.completeness.for_rule(&meta.id),
        );
        let started = std::time::Instant::now();
        let result = catch_unwind(AssertUnwindSafe(|| rule.run(db, &mut ctx)));
        tracing::info!(
            target: "polint::rules",
            rule = %meta.id,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "rule finished"
        );
        match result {
            Ok(Ok(())) => ctx.into_diagnostics(),
            Ok(Err(error)) => vec![internal_rule_error(db, &meta, error.to_string())],
            Err(_) => vec![internal_rule_error(db, &meta, "rule panicked".to_string())],
        }
    };

    let diagnostics = if parallel {
        rules
            .par_iter()
            .map(run_one)
            .collect::<Vec<_>>()
            .into_iter()
            .flatten()
            .collect()
    } else {
        rules.iter().flat_map(run_one).collect()
    };

    dedupe_diagnostics(diagnostics)
}

fn has_blocking_capability(rule_id: &str, support: &CapabilitySupportView) -> bool {
    support.entries().iter().any(|entry| {
        entry.rules.iter().any(|entry_rule| entry_rule == rule_id)
            && entry.status != CapabilitySupportStatus::Supported
    })
}

fn internal_rule_error(db: &AnalysisDb, meta: &RuleMeta, message: String) -> Diagnostic {
    internal_rule_error_for_id(db, &meta.id, message)
}

fn internal_rule_error_for_id(db: &AnalysisDb, rule_id: &str, message: String) -> Diagnostic {
    let (file, range) = db
        .files()
        .first()
        .map(|file| (file.relative_path.clone(), DiagnosticRange::point(1, 1)))
        .unwrap_or_else(|| ("<workspace>".to_string(), DiagnosticRange::point(1, 1)));

    Diagnostic::error(
        format!("internal/{rule_id}"),
        file,
        range,
        format!("Rule `{rule_id}` failed: {message}"),
    )
}

pub(crate) fn rule_id_matches(pattern: &str, rule_id: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix("/*") {
        return rule_id.starts_with(&format!("{prefix}/"));
    }
    pattern == rule_id
}

#[cfg(test)]
pub(crate) fn span_from_byte_range(
    file: FileId,
    source: &str,
    start_byte: usize,
    end_byte: usize,
) -> Span {
    let start_byte = start_byte.min(source.len());
    let end_byte = end_byte.min(source.len()).max(start_byte);
    let (start_line, start_col) = line_col(source, start_byte);
    let (end_line, end_col) = line_col(source, end_byte);
    Span::new(
        file,
        start_byte as u32,
        end_byte as u32,
        start_line,
        start_col,
        end_line,
        end_col,
    )
}

#[cfg(test)]
pub(crate) fn line_col(source: &str, byte_offset: usize) -> (u32, u32) {
    let mut line = 1_u32;
    let mut col = 1_u32;
    let limit = byte_offset.min(source.len());
    for (idx, ch) in source.char_indices() {
        if idx >= limit {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}
