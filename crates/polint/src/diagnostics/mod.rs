use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, IsTerminal};
use std::sync::Arc;

pub use crate::internal_core::{
    Diagnostic, DiagnosticRange as TextRange, Evidence, Fix, Label, Severity, StructuredEvidenceV1,
    Suggestion,
};
pub(crate) use crate::internal_core::{
    EvidenceBundleId, EvidenceBundleRef, diagnostic_fingerprint, fingerprint,
};

const _: () = {
    let _ = std::mem::size_of::<EvidenceBundleId>();
    let _ = std::mem::size_of::<EvidenceBundleRef>();
};

/// Version of every polint JSON report body.
///
/// Bumped to 2 when `summary` gained the provider-outcome and budget rows and
/// `summary.rules` gained its outcome fields. Every addition is optional, so a
/// v1 consumer still reads a v2 report.
pub(crate) const POLINT_REPORT_JSON_SCHEMA_V: u32 = 2;

/// Public URL of [`crate::diagnostics::PolintReport`] JSON Schema (v1); embedded in `--format json` when present.
pub const POLINT_REPORT_JSON_SCHEMA_V1_URL: &str =
    "https://raw.githubusercontent.com/oaiz-io/polint/main/docs/schemas/polint-report-v1.json";
/// Public URL of [`crate::diagnostics::AiFriendlyReport`] JSON Schema (v1); embedded in `--format ai-friendly` files.
pub(crate) const POLINT_AI_FRIENDLY_JSON_SCHEMA_V1_URL: &str =
    "https://raw.githubusercontent.com/oaiz-io/polint/main/docs/schemas/polint-ai-friendly-v1.json";

pub(crate) const AI_FRIENDLY_EXAMPLE_LIMIT: usize = 10;

pub(crate) fn extension_setup_diagnostic(
    failure_kind: &str,
    extension_id: &str,
    provider_id: Option<&str>,
    summary: &str,
) -> Diagnostic {
    let provider_label = provider_id.unwrap_or("<handshake>");
    Diagnostic::warning(
        "polint/extension",
        "",
        TextRange::point(1, 1),
        format!("Extension provider setup failed: {failure_kind}"),
    )
    .with_evidence("extension_id", extension_id)
    .with_evidence("provider_id", provider_label)
    .with_evidence("failure_kind", failure_kind)
    .with_evidence("summary", bounded_extension_summary(summary))
}

fn bounded_extension_summary(summary: &str) -> String {
    summary
        .replace('\\', "/")
        .lines()
        .take(4)
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(512)
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OutputFormat {
    Human,
    Github,
    Json,
    Sarif,
    AiFriendly,
}

/// Metadata embedded in `--format json` output (`PolintReport.tool`).
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct JsonReportMeta<'a> {
    pub tool_name: &'a str,
    pub tool_version: &'a str,
}

/// When to emit ANSI colors for `--format human` (honors `NO_COLOR` when [`ColorChoice::Auto`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ColorChoice {
    #[default]
    Auto,
    Always,
    Never,
}

impl ColorChoice {
    pub fn use_ansi_colors(self) -> bool {
        match self {
            ColorChoice::Never => false,
            ColorChoice::Always => true,
            ColorChoice::Auto => {
                io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct RenderOpts<'a> {
    pub json: JsonReportMeta<'a>,
    pub color: ColorChoice,
    /// Indexed by `Diagnostic.file`-style relative paths for human code snippets.
    pub sources: Option<&'a BTreeMap<String, Arc<str>>>,
    /// Per-rule execution telemetry embedded in `--format json` summary.
    pub(crate) rule_execution: &'a [RuleExecutionRow],
    /// Per-provider outcomes and per-budget trips for the same summary.
    pub(crate) run_summary: &'a RunSummary,
}

/// Tool identity in a [`PolintReport`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PolintToolInfo {
    pub name: String,
    pub version: String,
}

/// Versioned JSON report for `--format json` and repo-local rule host subprocesses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PolintReport {
    pub version: u32,
    pub tool: PolintToolInfo,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AiFriendlyReport {
    pub(crate) version: u32,
    pub(crate) schema: String,
    pub(crate) tool: PolintToolInfo,
    pub(crate) generated_at: String,
    pub(crate) summary: AiFriendlySummary,
    pub(crate) examples: Vec<AiFriendlyExample>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) truncation: Option<AiFriendlyTruncation>,
    pub(crate) diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AiFriendlySummary {
    pub(crate) total_diagnostics: usize,
    pub(crate) rules_triggered: usize,
    pub(crate) by_severity: BTreeMap<String, usize>,
    pub(crate) by_rule: Vec<AiFriendlyRuleSummary>,
    /// Per registered rule, including zero-finding and capability-skipped rules.
    pub(crate) rules: Vec<RuleExecutionRow>,
    /// Per provider, so a blocked rule names the provider that blocked it.
    #[serde(default)]
    pub(crate) providers: Vec<ProviderOutcomeRow>,
    /// Budgets this run exhausted.
    #[serde(default)]
    pub(crate) budgets: Vec<BudgetRow>,
    pub(crate) examples_limit: usize,
}

/// One row of rule-execution telemetry for silent-rule diagnosis.
///
/// `rules_triggered` stays the count of rules with ≥1 diagnostic; this list is
/// the registry view that also includes rules that planned, ran, and found nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RuleExecutionRow {
    pub(crate) rule_id: String,
    /// `true` when the rule survived capability planning and will execute.
    pub(crate) planned: bool,
    /// `true` when every requested capability is supported for this run.
    pub(crate) capabilities_ok: bool,
    /// Files matching this rule's scope after `files` / `allow_files` filtering
    /// against the analyzed file set (already narrowed by discovery scope).
    pub(crate) files_in_scope: usize,
    pub(crate) diagnostics_emitted: usize,
    /// Set when `!planned || !capabilities_ok`, using existing capability wording.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) skipped_reason: Option<String>,
    /// What happened to this rule: `analyzed`, `capability_blocked`, or
    /// `not_planned`. Zero diagnostics mean different things in each.
    #[serde(default = "default_rule_outcome")]
    pub(crate) outcome: String,
    /// Providers whose failure blocked this rule, when `outcome` is
    /// `capability_blocked`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) blocking_providers: Vec<String>,
    /// Operations this rule's policy queries examined.
    ///
    /// Zero with `outcome = analyzed` means the rule ran and matched nothing,
    /// which is a different state from "the analysis never reached it".
    #[serde(default)]
    pub(crate) observed_events: u64,
}

/// Outcome for rows decoded from an emitter that predates the field.
fn default_rule_outcome() -> String {
    RULE_OUTCOME_ANALYZED.to_string()
}

/// The rule ran.
pub(crate) const RULE_OUTCOME_ANALYZED: &str = "analyzed";
/// A provider the rule needed did not succeed, so the rule never ran.
pub(crate) const RULE_OUTCOME_CAPABILITY_BLOCKED: &str = "capability_blocked";
/// Capability planning rejected the rule before the run started.
pub(crate) const RULE_OUTCOME_NOT_PLANNED: &str = "not_planned";

/// One provider's outcome and cost for this run.
///
/// The kernel already decides every field here; before this row existed the
/// decision was computed each run and then dropped, so a blocked rule could not
/// be traced back to the provider that blocked it from the report alone.
///
/// Rows cover providers that failed or were blocked, providers that reported
/// counters, and providers whose ids the public output already names. A
/// provider this run never selected, or an internal fact family that quietly
/// succeeded, is not named: those are not a public vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProviderOutcomeRow {
    pub(crate) provider_id: String,
    pub(crate) status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    /// Wall time of the provider stage, absent when the provider did not run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) elapsed_ms: Option<u64>,
    /// Providers whose failure blocked this one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) blockers: Vec<String>,
    pub(crate) cache: ProviderCacheRow,
    /// Provider-specific counters, such as a sidecar's per-stage timings.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) counts: BTreeMap<String, u64>,
}

/// Cache counters for one provider, flattened for the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) struct ProviderCacheRow {
    pub(crate) hits: u64,
    pub(crate) misses: u64,
    pub(crate) recomputes: u64,
    pub(crate) writes: u64,
}

/// One budget this run exhausted.
///
/// Budgets already emit free-text diagnostics; this row is the machine-readable
/// form, so a consumer can tell "bounded itself" apart from "found nothing".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BudgetRow {
    pub(crate) budget: String,
    pub(crate) status: String,
    /// Diagnostic rule id that reported the trip.
    pub(crate) reported_by: String,
    pub(crate) detail: String,
}

/// Evidence label a budget diagnostic carries to be machine-readable.
pub(crate) const BUDGET_EVIDENCE_LABEL: &str = "budget";
/// Evidence label naming a budget's state.
pub(crate) const BUDGET_STATUS_EVIDENCE_LABEL: &str = "budget_status";

/// Shared empty summary for call sites that report no run telemetry.
#[cfg(test)]
pub(crate) static EMPTY_RUN_SUMMARY: &RunSummary = &RunSummary {
    rules: Vec::new(),
    providers: Vec::new(),
    budgets: Vec::new(),
};

/// Everything a run reports about itself besides the diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RunSummary {
    #[serde(default)]
    pub(crate) rules: Vec<RuleExecutionRow>,
    #[serde(default)]
    pub(crate) providers: Vec<ProviderOutcomeRow>,
    #[serde(default)]
    pub(crate) budgets: Vec<BudgetRow>,
}

impl RunSummary {
    /// Collects the machine-readable budget rows a run's diagnostics report.
    pub(crate) fn budget_rows(diagnostics: &[Diagnostic]) -> Vec<BudgetRow> {
        let mut rows = diagnostics
            .iter()
            .filter_map(|diagnostic| {
                let budget = evidence_value(diagnostic, BUDGET_EVIDENCE_LABEL)?;
                Some(BudgetRow {
                    budget: budget.to_string(),
                    status: evidence_value(diagnostic, BUDGET_STATUS_EVIDENCE_LABEL)
                        .unwrap_or("exceeded")
                        .to_string(),
                    reported_by: diagnostic.rule_id.clone(),
                    detail: diagnostic.message.clone(),
                })
            })
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| {
            (&left.budget, &left.reported_by).cmp(&(&right.budget, &right.reported_by))
        });
        rows.dedup();
        rows
    }
}

fn evidence_value<'a>(diagnostic: &'a Diagnostic, label: &str) -> Option<&'a str> {
    diagnostic
        .evidence
        .iter()
        .find(|evidence| evidence.label == label)
        .map(|evidence| evidence.value.as_str())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AiFriendlyRuleSummary {
    pub(crate) rule_id: String,
    pub(crate) total: usize,
    pub(crate) by_severity: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AiFriendlyExample {
    pub(crate) rule_id: String,
    pub(crate) severity: Severity,
    pub(crate) file: String,
    pub(crate) range: TextRange,
    pub(crate) message: String,
    pub(crate) labels: Vec<Label>,
    pub(crate) help: Option<String>,
    pub(crate) evidence: Vec<Evidence>,
    pub(crate) suggestions: Vec<Suggestion>,
    pub(crate) fix: Option<Fix>,
    pub(crate) stable_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AiFriendlyTruncation {
    pub(crate) total_diagnostics: usize,
    pub(crate) included_diagnostics: usize,
    pub(crate) reason: String,
}

#[derive(Serialize)]
struct PolintReportWire<'a> {
    version: u32,
    schema: &'static str,
    tool: PolintToolWire<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<PolintReportSummaryWire<'a>>,
    diagnostics: &'a [Diagnostic],
}

#[derive(Serialize)]
struct PolintReportSummaryWire<'a> {
    rules: &'a [RuleExecutionRow],
    providers: &'a [ProviderOutcomeRow],
    budgets: &'a [BudgetRow],
}

#[derive(Serialize)]
struct PolintToolWire<'a> {
    name: &'a str,
    version: &'a str,
}

#[derive(Debug, Deserialize)]
struct HostJsonSummary {
    #[serde(default)]
    rules: Vec<RuleExecutionRow>,
    #[serde(default)]
    providers: Vec<ProviderOutcomeRow>,
    #[serde(default)]
    budgets: Vec<BudgetRow>,
}

/// Parse stdout from a `polint-local-rules check --format json` process.
pub fn diagnostics_from_json_report(s: &str) -> Result<Vec<Diagnostic>, serde_json::Error> {
    let report: PolintReport = serde_json::from_str(s)?;
    Ok(report.diagnostics)
}

/// Parse diagnostics plus rule-execution telemetry from a rule-host JSON report.
///
/// `evidence_v1` is a trust boundary: validate and keep, or drop and emit an
/// internal diagnostic. `evidence_bundle` is always cleared — it is an in-process
/// store reference that cannot cross a process boundary.
pub(crate) fn diagnostics_and_rule_execution_from_public_json_report(
    s: &str,
) -> Result<(Vec<Diagnostic>, RunSummary), serde_json::Error> {
    let report: PublicJsonReportWire = serde_json::from_str(s)?;
    let mut diagnostics = Vec::with_capacity(report.diagnostics.len());
    let mut internal = Vec::new();
    for mut wire in report.diagnostics {
        let raw_evidence = wire.evidence_v1.take();
        let mut diagnostic = wire.into_diagnostic();
        diagnostic.evidence_bundle = None;
        diagnostic.evidence_v1 = None;
        if let Some(value) = raw_evidence {
            match StructuredEvidenceV1::try_from_value(value) {
                Ok(evidence) => diagnostic.evidence_v1 = Some(evidence),
                Err(error) => {
                    internal.push(Diagnostic::warning(
                        "polint/internal",
                        diagnostic.file.clone(),
                        diagnostic.range,
                        format!("dropped invalid evidence_v1 from rule host: {error}"),
                    ));
                }
            }
        }
        diagnostics.push(diagnostic);
    }
    diagnostics.extend(internal);
    let summary = report
        .summary
        .map(|summary| RunSummary {
            rules: summary.rules,
            providers: summary.providers,
            budgets: summary.budgets,
        })
        .unwrap_or_default();
    Ok((diagnostics, summary))
}

#[derive(Deserialize)]
struct PublicJsonReportWire {
    diagnostics: Vec<PublicDiagnosticWire>,
    #[serde(default)]
    summary: Option<HostJsonSummary>,
}

#[derive(Deserialize)]
struct PublicDiagnosticWire {
    rule_id: String,
    severity: Severity,
    file: String,
    range: TextRange,
    message: String,
    #[serde(default)]
    labels: Vec<Label>,
    #[serde(default)]
    help: Option<String>,
    #[serde(default)]
    evidence: Vec<Evidence>,
    #[serde(default)]
    evidence_v1: Option<serde_json::Value>,
    #[serde(default)]
    suggestions: Vec<Suggestion>,
    #[serde(default)]
    fix: Option<Fix>,
    #[serde(default)]
    stable_fingerprint: String,
}

impl PublicDiagnosticWire {
    fn into_diagnostic(self) -> Diagnostic {
        let stable_fingerprint = if self.stable_fingerprint.is_empty() {
            diagnostic_fingerprint(&self.rule_id, &self.file, self.range, &self.message)
        } else {
            self.stable_fingerprint
        };
        Diagnostic::from_parts(
            self.rule_id,
            self.severity,
            self.file,
            self.range,
            self.message,
            self.labels,
            self.help,
            self.evidence,
            None,
            None,
            self.suggestions,
            self.fix,
            stable_fingerprint,
        )
    }
}

pub(crate) fn build_ai_friendly_report(
    diagnostics: &[Diagnostic],
    persisted_diagnostics: &[Diagnostic],
    json_meta: JsonReportMeta<'_>,
    generated_at: impl Into<String>,
    rule_execution: &[RuleExecutionRow],
    run_summary: &RunSummary,
) -> AiFriendlyReport {
    let summary = ai_friendly_summary(diagnostics, rule_execution, run_summary);
    let examples = ai_friendly_examples(diagnostics, &summary.by_rule);
    let truncation = if persisted_diagnostics.len() < diagnostics.len() {
        Some(AiFriendlyTruncation {
            total_diagnostics: diagnostics.len(),
            included_diagnostics: persisted_diagnostics.len(),
            reason: "--max-diagnostics limited persisted diagnostics".to_string(),
        })
    } else {
        None
    };
    AiFriendlyReport {
        version: POLINT_REPORT_JSON_SCHEMA_V,
        schema: POLINT_AI_FRIENDLY_JSON_SCHEMA_V1_URL.to_string(),
        tool: PolintToolInfo {
            name: json_meta.tool_name.to_string(),
            version: json_meta.tool_version.to_string(),
        },
        generated_at: generated_at.into(),
        summary,
        examples,
        truncation,
        diagnostics: persisted_diagnostics.to_vec(),
    }
}

fn ai_friendly_summary(
    diagnostics: &[Diagnostic],
    rule_execution: &[RuleExecutionRow],
    run_summary: &RunSummary,
) -> AiFriendlySummary {
    let mut by_severity = empty_severity_counts();
    let mut by_rule: BTreeMap<String, RuleSummaryDraft> = BTreeMap::new();
    for diagnostic in diagnostics {
        increment_severity(&mut by_severity, diagnostic.severity);
        let draft = by_rule
            .entry(diagnostic.rule_id.clone())
            .or_insert_with(|| RuleSummaryDraft::new(diagnostic.rule_id.clone()));
        draft.total += 1;
        increment_severity(&mut draft.by_severity, diagnostic.severity);
        draft.highest_severity = draft
            .highest_severity
            .min(severity_sort_rank(diagnostic.severity));
    }

    let mut by_rule: Vec<RuleSummaryDraft> = by_rule.into_values().collect();
    by_rule.sort_by(|left, right| {
        (
            left.highest_severity,
            std::cmp::Reverse(left.total),
            left.rule_id.as_str(),
        )
            .cmp(&(
                right.highest_severity,
                std::cmp::Reverse(right.total),
                right.rule_id.as_str(),
            ))
    });

    AiFriendlySummary {
        providers: run_summary.providers.clone(),
        budgets: run_summary.budgets.clone(),
        total_diagnostics: diagnostics.len(),
        rules_triggered: by_rule.len(),
        by_severity,
        by_rule: by_rule
            .into_iter()
            .map(|draft| AiFriendlyRuleSummary {
                rule_id: draft.rule_id,
                total: draft.total,
                by_severity: draft.by_severity,
            })
            .collect(),
        rules: rule_execution.to_vec(),
        examples_limit: AI_FRIENDLY_EXAMPLE_LIMIT,
    }
}

#[derive(Debug, Clone)]
struct RuleSummaryDraft {
    rule_id: String,
    total: usize,
    by_severity: BTreeMap<String, usize>,
    highest_severity: u8,
}

impl RuleSummaryDraft {
    fn new(rule_id: String) -> Self {
        Self {
            rule_id,
            total: 0,
            by_severity: empty_severity_counts(),
            highest_severity: u8::MAX,
        }
    }
}

fn ai_friendly_examples(
    diagnostics: &[Diagnostic],
    by_rule: &[AiFriendlyRuleSummary],
) -> Vec<AiFriendlyExample> {
    let mut examples = BTreeMap::<String, &Diagnostic>::new();
    for diagnostic in diagnostics {
        examples
            .entry(diagnostic.rule_id.clone())
            .and_modify(|current| {
                if example_sort_key(diagnostic) < example_sort_key(current) {
                    *current = diagnostic;
                }
            })
            .or_insert(diagnostic);
    }

    by_rule
        .iter()
        .filter_map(|summary| examples.get(&summary.rule_id).copied())
        .take(AI_FRIENDLY_EXAMPLE_LIMIT)
        .map(AiFriendlyExample::from)
        .collect()
}

fn example_sort_key(diagnostic: &Diagnostic) -> (u8, &str, u32, u32, &str, &str) {
    (
        severity_sort_rank(diagnostic.severity),
        diagnostic.file.as_str(),
        diagnostic.range.start_line,
        diagnostic.range.start_col,
        diagnostic.message.as_str(),
        diagnostic.stable_fingerprint.as_str(),
    )
}

impl From<&Diagnostic> for AiFriendlyExample {
    fn from(diagnostic: &Diagnostic) -> Self {
        Self {
            rule_id: diagnostic.rule_id.clone(),
            severity: diagnostic.severity,
            file: diagnostic.file.clone(),
            range: diagnostic.range,
            message: diagnostic.message.clone(),
            labels: diagnostic.labels.clone(),
            help: diagnostic.help.clone(),
            evidence: diagnostic.evidence.clone(),
            suggestions: diagnostic.suggestions.clone(),
            fix: diagnostic.fix.clone(),
            stable_fingerprint: diagnostic.stable_fingerprint.clone(),
        }
    }
}

fn empty_severity_counts() -> BTreeMap<String, usize> {
    BTreeMap::from([
        ("error".to_string(), 0),
        ("warn".to_string(), 0),
        ("info".to_string(), 0),
    ])
}

fn increment_severity(counts: &mut BTreeMap<String, usize>, severity: Severity) {
    *counts
        .entry(severity_count_key(severity).to_string())
        .or_default() += 1;
}

fn severity_count_key(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warn => "warn",
        Severity::Info => "info",
        _ => "info",
    }
}

fn severity_sort_rank(severity: Severity) -> u8 {
    match severity {
        Severity::Error => 0,
        Severity::Warn => 1,
        Severity::Info => 2,
        _ => 2,
    }
}

pub(crate) fn render_ai_friendly_stdout(report: &AiFriendlyReport, json_path: &str) -> String {
    let total = report.summary.total_diagnostics;
    let rules = report.summary.rules_triggered;
    let mut out = String::new();
    out.push_str(&format!(
        "polint: {total} {} across {rules} {}. Full JSON: {json_path}\n",
        plural(total, "diagnostic", "diagnostics"),
        plural(rules, "rule", "rules")
    ));

    if let Some(truncation) = &report.truncation {
        out.push_str(&format!(
            "Persisted diagnostics were limited: {} of {} included ({})\n",
            truncation.included_diagnostics, truncation.total_diagnostics, truncation.reason
        ));
    }

    out.push_str("\nBy rule\n");
    if report.summary.by_rule.is_empty() {
        out.push_str("  none\n");
    } else {
        for rule in &report.summary.by_rule {
            out.push_str(&format!(
                "  {} {}: {}\n",
                dominant_rule_severity(rule),
                rule.rule_id,
                rule.total
            ));
        }
    }

    out.push_str("\nExamples, max 10\n");
    if report.examples.is_empty() {
        out.push_str("  none\n");
    } else {
        for example in &report.examples {
            push_ai_friendly_example(&mut out, example);
        }
    }

    out.push_str(
        "\nJSON format: versioned object with `summary`, `examples`, and `diagnostics`.\n",
    );
    out.push_str("Do not read the whole file into an AI prompt. Query it with bounded commands:\n");
    out.push_str(&format!("  jq '.summary.by_rule' {json_path}\n"));
    out.push_str(&format!(
        "  jq '.summary.rules[] | select(.diagnostics_emitted == 0)' {json_path}\n"
    ));
    if let Some(example) = report.examples.first() {
        let rule_id = jq_string_literal(&example.rule_id);
        let file = jq_string_literal(&example.file);
        out.push_str(&format!(
            "  jq '[.diagnostics[] | select(.rule_id=={rule_id})][0:20]' {json_path}\n"
        ));
        out.push_str(&format!(
            "  jq '.diagnostics[] | select(.file=={file}) | {{rule_id, range, message}}' {json_path} | head -c 12000\n"
        ));
    } else {
        out.push_str(&format!(
            "  jq '[.diagnostics[] | select(.rule_id==\"local/example\")][0:20]' {json_path}\n"
        ));
        out.push_str(&format!(
            "  jq '.diagnostics[] | select(.file==\"src/example.ts\") | {{rule_id, range, message}}' {json_path} | head -c 12000\n"
        ));
    }
    out
}

fn jq_string_literal(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

fn push_ai_friendly_example(out: &mut String, example: &AiFriendlyExample) {
    out.push_str(&format!(
        "  {}[{}]: {}\n    --> {}:{}\n",
        severity_label(example.severity),
        example.rule_id,
        example.message,
        example.file,
        format_range(example.range)
    ));
    for label in &example.labels {
        out.push_str(&format!(
            "    label {}: {}\n",
            format_range(label.range),
            label.message
        ));
    }
    for evidence in &example.evidence {
        out.push_str(&format!(
            "    evidence {}: {}\n",
            evidence.label, evidence.value
        ));
    }
    for suggestion in &example.suggestions {
        out.push_str(&format!("    suggestion: {}\n", suggestion.message));
    }
    if let Some(fix) = &example.fix {
        out.push_str(&format!("    fix: {}\n", fix.message));
        if let Some(replacement) = &fix.replacement {
            out.push_str(&format!("      replacement: {replacement}\n"));
        }
    }
    if let Some(help) = &example.help {
        out.push_str(&format!("    help: {help}\n"));
    }
    out.push_str(&format!(
        "    fingerprint: {}\n\n",
        example.stable_fingerprint
    ));
}

fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warn => "warn",
        Severity::Info => "info",
        _ => "info",
    }
}

fn dominant_rule_severity(rule: &AiFriendlyRuleSummary) -> &'static str {
    if rule.by_severity.get("error").copied().unwrap_or(0) > 0 {
        "error"
    } else if rule.by_severity.get("warn").copied().unwrap_or(0) > 0 {
        "warn"
    } else {
        "info"
    }
}

fn plural(count: usize, singular: &'static str, plural: &'static str) -> &'static str {
    if count == 1 { singular } else { plural }
}

pub(crate) fn sort_diagnostics(diagnostics: &mut [Diagnostic]) {
    diagnostics.sort_by(|a, b| {
        (
            a.file.as_str(),
            a.range.start_line,
            a.range.start_col,
            diagnostic_sort_priority(a),
            a.rule_id.as_str(),
            a.message.as_str(),
            a.stable_fingerprint.as_str(),
        )
            .cmp(&(
                b.file.as_str(),
                b.range.start_line,
                b.range.start_col,
                diagnostic_sort_priority(b),
                b.rule_id.as_str(),
                b.message.as_str(),
                b.stable_fingerprint.as_str(),
            ))
    });
}

fn diagnostic_sort_priority(diagnostic: &Diagnostic) -> u8 {
    if diagnostic.rule_id == "polint/capability" {
        0
    } else {
        1
    }
}

pub(crate) fn dedupe_diagnostics(diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
    let mut diagnostics = diagnostics;
    sort_diagnostics(&mut diagnostics);
    let mut seen = BTreeSet::new();
    diagnostics
        .into_iter()
        .filter(|diagnostic| seen.insert(diagnostic.stable_fingerprint.clone()))
        .collect()
}

/// Dedupe, optionally filter by `rule_id` pattern (see [`crate::core::rule_id_matches`]),
/// then sort for deterministic ordering.
pub(crate) fn apply_report_filters(
    diagnostics: Vec<Diagnostic>,
    only_rule: Option<&str>,
) -> Vec<Diagnostic> {
    let mut diagnostics = dedupe_diagnostics(diagnostics);
    if let Some(pattern) = only_rule {
        diagnostics.retain(|diagnostic| diagnostic_matches_rule_pattern(pattern, diagnostic));
    }
    sort_diagnostics(&mut diagnostics);
    diagnostics
}

fn diagnostic_matches_rule_pattern(pattern: &str, diagnostic: &Diagnostic) -> bool {
    crate::core::rule_id_matches(pattern, &diagnostic.rule_id)
        || diagnostic.rule_id.starts_with("parser/")
        || (diagnostic.rule_id == "polint/capability"
            && diagnostic.evidence.iter().any(|evidence| {
                evidence.label == "rule" && crate::core::rule_id_matches(pattern, &evidence.value)
            }))
        || (diagnostic.rule_id.starts_with("polint/")
            && diagnostic.evidence.iter().any(|evidence| {
                evidence.label == "selectors"
                    && evidence.value.split(',').map(str::trim).any(|selector| {
                        !selector.is_empty()
                            && (crate::core::rule_id_matches(pattern, selector)
                                || crate::core::rule_id_matches(selector, pattern))
                    })
            }))
}

/// Truncate diagnostics for rendering only. Exit status must be computed before this cap.
pub(crate) fn limit_report_diagnostics(
    mut diagnostics: Vec<Diagnostic>,
    max_diagnostics: Option<usize>,
) -> Vec<Diagnostic> {
    if let Some(max) = max_diagnostics {
        diagnostics.truncate(max);
    }
    diagnostics
}

#[cfg(test)]
pub(crate) fn render(
    format: OutputFormat,
    diagnostics: &[Diagnostic],
    opts: RenderOpts<'_>,
) -> String {
    render_with_sarif_help(format, diagnostics, opts, None)
}

pub(crate) fn render_with_sarif_help(
    format: OutputFormat,
    diagnostics: &[Diagnostic],
    opts: RenderOpts<'_>,
    sarif_rule_help_uri: Option<&BTreeMap<String, String>>,
) -> String {
    match format {
        OutputFormat::Human => render_human(diagnostics, opts.color, opts.sources),
        OutputFormat::Github => render_github(diagnostics),
        OutputFormat::Json => render_json(
            diagnostics,
            opts.json,
            opts.rule_execution,
            opts.run_summary,
        ),
        OutputFormat::Sarif => render_sarif(diagnostics, sarif_rule_help_uri),
        OutputFormat::AiFriendly => {
            let report = build_ai_friendly_report(
                diagnostics,
                diagnostics,
                opts.json,
                "unknown",
                opts.rule_execution,
                opts.run_summary,
            );
            render_ai_friendly_stdout(&report, ".polint/output/latest.json")
        }
    }
}

fn render_github(diagnostics: &[Diagnostic]) -> String {
    let mut output = String::new();
    for diagnostic in diagnostics {
        output.push_str("::");
        output.push_str(github_command(diagnostic.severity));
        output.push_str(" file=");
        output.push_str(&escape_github_property(&diagnostic.file));
        output.push_str(",line=");
        output.push_str(&github_line(diagnostic.range.start_line).to_string());
        output.push_str(",col=");
        output.push_str(&github_line(diagnostic.range.start_col).to_string());
        output.push_str(",endLine=");
        output.push_str(&github_line(diagnostic.range.end_line).to_string());
        output.push_str(",endColumn=");
        output.push_str(&github_line(diagnostic.range.end_col).to_string());
        output.push_str(",title=");
        output.push_str(&escape_github_property(&diagnostic.rule_id));
        output.push_str("::");
        output.push_str(&escape_github_message(&diagnostic.message));
        output.push('\n');
    }
    output
}

fn github_command(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warn => "warning",
        Severity::Info => "notice",
        _ => "notice",
    }
}

fn github_line(value: u32) -> u32 {
    value.max(1)
}

fn escape_github_property(value: &str) -> String {
    escape_github_message(value)
        .replace(':', "%3A")
        .replace(',', "%2C")
}

fn escape_github_message(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

fn render_json(
    diagnostics: &[Diagnostic],
    json_meta: JsonReportMeta<'_>,
    rule_execution: &[RuleExecutionRow],
    run_summary: &RunSummary,
) -> String {
    let summary = (!rule_execution.is_empty()).then_some(PolintReportSummaryWire {
        rules: rule_execution,
        providers: &run_summary.providers,
        budgets: &run_summary.budgets,
    });
    let wire = PolintReportWire {
        version: POLINT_REPORT_JSON_SCHEMA_V,
        schema: POLINT_REPORT_JSON_SCHEMA_V1_URL,
        tool: PolintToolWire {
            name: json_meta.tool_name,
            version: json_meta.tool_version,
        },
        summary,
        diagnostics,
    };
    serde_json::to_string_pretty(&wire).unwrap_or_else(|_| "{}".to_string())
}

struct HumanPalette {
    reset: &'static str,
    bold: &'static str,
    dim: &'static str,
    red: &'static str,
    yellow: &'static str,
    cyan: &'static str,
    green: &'static str,
    magenta: &'static str,
}

impl HumanPalette {
    fn new(color: bool) -> Self {
        if color {
            Self {
                reset: "\x1b[0m",
                bold: "\x1b[1m",
                dim: "\x1b[2m",
                red: "\x1b[1;31m",
                yellow: "\x1b[1;33m",
                cyan: "\x1b[1;36m",
                green: "\x1b[1;32m",
                magenta: "\x1b[1;35m",
            }
        } else {
            Self {
                reset: "",
                bold: "",
                dim: "",
                red: "",
                yellow: "",
                cyan: "",
                green: "",
                magenta: "",
            }
        }
    }

    fn severity_paint(&self, severity: Severity) -> &'static str {
        match severity {
            Severity::Error => self.red,
            Severity::Warn => self.yellow,
            Severity::Info => self.cyan,
            _ => self.cyan,
        }
    }
}

fn lookup_source<'a>(sources: &'a BTreeMap<String, Arc<str>>, file: &str) -> Option<&'a str> {
    if let Some(s) = sources.get(file) {
        return Some(s.as_ref());
    }
    let trimmed = file.trim_start_matches("./");
    if let Some(s) = sources.get(trimmed) {
        return Some(s.as_ref());
    }
    let with_dot = format!("./{trimmed}");
    sources.get(&with_dot).map(|a| a.as_ref())
}

/// 1-based column: return byte index at that column (first column is 1).
fn byte_idx_for_one_based_col(line: &str, col_1based: u32) -> usize {
    if col_1based <= 1 {
        return 0;
    }
    let mut c = 1_u32;
    for (idx, _) in line.char_indices() {
        if c == col_1based {
            return idx;
        }
        c = c.saturating_add(1);
    }
    line.len()
}

fn gutter_width(last_line_no: u32) -> usize {
    let d = last_line_no.max(1).ilog10() as usize + 1;
    d.max(3)
}

fn push_underline_row(
    out: &mut String,
    p: &HumanPalette,
    gutter_w: usize,
    start_col_1based: u32,
    line: &str,
    start_byte: usize,
    end_byte: usize,
) {
    out.push(' ');
    out.push_str(p.dim);
    for _ in 0..gutter_w {
        out.push(' ');
    }
    out.push_str(p.reset);
    out.push(' ');
    out.push_str(p.dim);
    out.push('|');
    out.push_str(p.reset);
    out.push(' ');
    out.push_str(p.red);
    let pad = (start_col_1based.saturating_sub(1)) as usize;
    for _ in 0..pad {
        out.push(' ');
    }
    let end_byte = end_byte.max(start_byte);
    let slice = line.get(start_byte..end_byte).unwrap_or("");
    let n = slice.chars().count().max(1);
    for _ in 0..n {
        out.push('^');
    }
    out.push_str(p.reset);
    out.push('\n');
}

fn push_code_snippet(out: &mut String, p: &HumanPalette, source: &str, range: TextRange) {
    let lines: Vec<&str> = source.lines().collect();
    if lines.is_empty() || range.start_line < 1 {
        return;
    }
    let start_idx = (range.start_line - 1) as usize;
    if start_idx >= lines.len() {
        return;
    }
    let end_idx = (range.end_line.saturating_sub(1) as usize)
        .min(lines.len() - 1)
        .max(start_idx);
    let w = gutter_width((end_idx + 1) as u32);

    out.push_str("  ");
    out.push_str(p.dim);
    for _ in 0..w {
        out.push(' ');
    }
    out.push('|');
    out.push_str(p.reset);
    out.push('\n');

    for (rel_idx, content) in lines[start_idx..=end_idx].iter().enumerate() {
        let line_no = (start_idx + rel_idx + 1) as u32;
        out.push(' ');
        out.push_str(p.dim);
        out.push_str(&format!("{:>width$}", line_no, width = w));
        out.push_str(p.reset);
        out.push(' ');
        out.push_str(p.dim);
        out.push('|');
        out.push_str(p.reset);
        out.push(' ');
        out.push_str(content);
        out.push('\n');
    }

    if range.start_line == range.end_line {
        let line = lines[start_idx];
        let start_b = byte_idx_for_one_based_col(line, range.start_col);
        let end_b = if range.end_col > range.start_col {
            byte_idx_for_one_based_col(line, range.end_col)
        } else {
            (start_b
                + line[start_b..]
                    .chars()
                    .next()
                    .map(|ch| ch.len_utf8())
                    .unwrap_or(1))
            .min(line.len())
        };
        push_underline_row(out, p, w, range.start_col, line, start_b, end_b);
    } else {
        let first = lines[start_idx];
        let start_b = byte_idx_for_one_based_col(first, range.start_col);
        push_underline_row(out, p, w, range.start_col, first, start_b, first.len());
        if end_idx > start_idx {
            let last = lines[end_idx];
            let end_b = byte_idx_for_one_based_col(last, range.end_col.max(1));
            push_underline_row(out, p, w, 1, last, 0, end_b);
        }
    }
}

/// Header line for human output: counts per `rule_id`, grouped by severity (errors, then warnings, then infos). Rule ids sorted lexicographically within each group.
fn format_human_summary_line(diagnostics: &[Diagnostic], p: &HumanPalette) -> String {
    let mut errors: BTreeMap<String, u32> = BTreeMap::new();
    let mut warns: BTreeMap<String, u32> = BTreeMap::new();
    let mut infos: BTreeMap<String, u32> = BTreeMap::new();
    for d in diagnostics {
        match d.severity {
            Severity::Error => *errors.entry(d.rule_id.clone()).or_insert(0) += 1,
            Severity::Warn => *warns.entry(d.rule_id.clone()).or_insert(0) += 1,
            Severity::Info => *infos.entry(d.rule_id.clone()).or_insert(0) += 1,
            _ => *infos.entry(d.rule_id.clone()).or_insert(0) += 1,
        }
    }

    fn push_group(
        parts: &mut Vec<String>,
        p: &HumanPalette,
        sev: Severity,
        count: u32,
        rules: &BTreeMap<String, u32>,
    ) {
        if rules.is_empty() {
            return;
        }
        let label = if count == 1 {
            match sev {
                Severity::Error => "error",
                Severity::Warn => "warning",
                Severity::Info => "info",
                _ => "info",
            }
        } else {
            match sev {
                Severity::Error => "errors",
                Severity::Warn => "warnings",
                Severity::Info => "infos",
                _ => "infos",
            }
        };
        let breakdown: Vec<String> = rules.iter().map(|(id, n)| format!("{id} ({n})")).collect();
        parts.push(format!(
            "{}{} {} — {}{}",
            p.severity_paint(sev),
            count,
            label,
            breakdown.join(", "),
            p.reset
        ));
    }

    let err_n: u32 = errors.values().sum();
    let warn_n: u32 = warns.values().sum();
    let info_n: u32 = infos.values().sum();

    let mut parts = Vec::new();
    push_group(&mut parts, p, Severity::Error, err_n, &errors);
    push_group(&mut parts, p, Severity::Warn, warn_n, &warns);
    push_group(&mut parts, p, Severity::Info, info_n, &infos);

    let mut out = String::new();
    out.push_str(&format!("{}Summary:{} ", p.bold, p.reset));
    out.push_str(&parts.join("; "));
    out.push('\n');
    out
}

pub(crate) fn render_human(
    diagnostics: &[Diagnostic],
    color: ColorChoice,
    sources: Option<&BTreeMap<String, Arc<str>>>,
) -> String {
    let p = HumanPalette::new(color.use_ansi_colors());
    if diagnostics.is_empty() {
        return format!("{}No diagnostics.{}\n", p.dim, p.reset);
    }

    let mut out = String::new();
    out.push_str(&format_human_summary_line(diagnostics, &p));
    out.push('\n');
    for diagnostic in diagnostics {
        let sev_word = match diagnostic.severity {
            Severity::Error => "error",
            Severity::Warn => "warn",
            Severity::Info => "info",
            _ => "info",
        };
        out.push_str(&format!(
            "{}{}{}{}[{}{}{}]: {}{}\n  {}{}{}{} {}{}{}:{}\n",
            p.severity_paint(diagnostic.severity),
            sev_word,
            p.reset,
            p.bold,
            p.magenta,
            diagnostic.rule_id,
            p.reset,
            p.bold,
            diagnostic.message,
            p.reset,
            p.cyan,
            "-->",
            p.reset,
            p.bold,
            diagnostic.file,
            p.reset,
            format_range(diagnostic.range),
        ));
        if let Some(map) = sources
            && let Some(src) = lookup_source(map, &diagnostic.file)
        {
            push_code_snippet(&mut out, &p, src, diagnostic.range);
        }
        for label in &diagnostic.labels {
            out.push_str(&format!(
                "  {}label{} {}: {}\n",
                p.dim,
                p.reset,
                format_range(label.range),
                label.message
            ));
        }
        if !diagnostic.evidence.is_empty() {
            for evidence in &diagnostic.evidence {
                out.push_str(&format!(
                    "  {}evidence{} {}: {}\n",
                    p.dim, p.reset, evidence.label, evidence.value
                ));
            }
        }
        for suggestion in &diagnostic.suggestions {
            out.push_str(&format!(
                "  {}suggestion:{} {}\n",
                p.dim, p.reset, suggestion.message
            ));
        }
        if let Some(fix) = &diagnostic.fix {
            out.push_str(&format!("  {}fix:{} {}\n", p.yellow, p.reset, fix.message));
            if let Some(replacement) = &fix.replacement {
                out.push_str(&format!(
                    "    {}replacement:{} {}\n",
                    p.green, p.reset, replacement
                ));
            }
        }
        if let Some(help) = &diagnostic.help {
            out.push_str(&format!("  {}help:{} {}\n", p.cyan, p.reset, help));
        }
        out.push_str(&format!(
            "  {}fingerprint: {}{}\n\n",
            p.dim, diagnostic.stable_fingerprint, p.reset
        ));
    }
    out
}

fn format_range(range: TextRange) -> String {
    format!(
        "{}:{}-{}:{}",
        range.start_line, range.start_col, range.end_line, range.end_col
    )
}

pub(crate) fn render_sarif(
    diagnostics: &[Diagnostic],
    rule_help_uri: Option<&BTreeMap<String, String>>,
) -> String {
    #[derive(Serialize)]
    struct SarifLog {
        version: &'static str,
        #[serde(rename = "$schema")]
        schema: &'static str,
        runs: Vec<SarifRun>,
    }

    #[derive(Serialize)]
    struct SarifRun {
        tool: SarifTool,
        results: Vec<SarifResult>,
    }

    #[derive(Serialize)]
    struct SarifTool {
        driver: SarifDriver,
    }

    #[derive(Serialize)]
    struct SarifDriver {
        name: &'static str,
        #[serde(rename = "informationUri")]
        information_uri: &'static str,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        rules: Vec<SarifReportingRule>,
    }

    #[derive(Serialize)]
    struct SarifReportingRule {
        id: String,
        #[serde(rename = "shortDescription")]
        short_description: SarifMessage,
        #[serde(rename = "fullDescription", skip_serializing_if = "Option::is_none")]
        full_description: Option<SarifMessage>,
        #[serde(rename = "helpUri", skip_serializing_if = "Option::is_none")]
        help_uri: Option<String>,
        properties: SarifProperties,
    }

    #[derive(Serialize)]
    struct SarifProperties {
        tags: Vec<String>,
    }

    #[derive(Serialize)]
    struct SarifResult {
        #[serde(rename = "ruleId")]
        rule_id: String,
        level: &'static str,
        message: SarifMessage,
        fingerprints: SarifFingerprints,
        locations: Vec<SarifLocation>,
        #[serde(rename = "relatedLocations", skip_serializing_if = "Vec::is_empty")]
        related_locations: Vec<SarifRelatedLocation>,
        #[serde(rename = "codeFlows", skip_serializing_if = "Vec::is_empty")]
        code_flows: Vec<SarifCodeFlow>,
        #[serde(skip_serializing_if = "Option::is_none")]
        fixes: Option<Vec<SarifFix>>,
    }

    #[derive(Serialize)]
    struct SarifFingerprints {
        #[serde(rename = "polint/v1")]
        polint_v1: String,
    }

    #[derive(Serialize)]
    struct SarifMessage {
        text: String,
    }

    #[derive(Serialize)]
    struct SarifLocation {
        #[serde(rename = "physicalLocation")]
        physical_location: SarifPhysicalLocation,
    }

    #[derive(Serialize)]
    struct SarifRelatedLocation {
        #[serde(rename = "physicalLocation")]
        physical_location: SarifPhysicalLocation,
        message: SarifMessage,
    }

    #[derive(Serialize)]
    struct SarifCodeFlow {
        #[serde(rename = "threadFlows")]
        thread_flows: Vec<SarifThreadFlow>,
    }

    #[derive(Serialize)]
    struct SarifThreadFlow {
        locations: Vec<SarifThreadFlowLocation>,
    }

    #[derive(Serialize)]
    struct SarifThreadFlowLocation {
        location: SarifLocationWithMessage,
    }

    #[derive(Serialize)]
    struct SarifLocationWithMessage {
        #[serde(rename = "physicalLocation")]
        physical_location: SarifPhysicalLocation,
        message: SarifMessage,
    }

    #[derive(Serialize)]
    struct SarifPhysicalLocation {
        #[serde(rename = "artifactLocation")]
        artifact_location: SarifArtifactLocation,
        region: SarifRegion,
    }

    #[derive(Serialize)]
    struct SarifArtifactLocation {
        uri: String,
    }

    #[derive(Serialize)]
    struct SarifRegion {
        #[serde(rename = "startLine")]
        start_line: u32,
        #[serde(rename = "startColumn")]
        start_column: u32,
        #[serde(rename = "endLine")]
        end_line: u32,
        #[serde(rename = "endColumn")]
        end_column: u32,
    }

    #[derive(Serialize)]
    struct SarifFix {
        description: SarifMessage,
        #[serde(rename = "artifactChanges")]
        artifact_changes: Vec<SarifArtifactChange>,
    }

    #[derive(Serialize)]
    struct SarifArtifactChange {
        #[serde(rename = "artifactLocation")]
        artifact_location: SarifArtifactLocation,
        replacements: Vec<SarifReplacement>,
    }

    #[derive(Serialize)]
    struct SarifReplacement {
        #[serde(rename = "deletedRegion")]
        deleted_region: SarifRegion,
        #[serde(rename = "insertedContent")]
        inserted_content: SarifArtifactContent,
    }

    #[derive(Serialize)]
    struct SarifArtifactContent {
        text: String,
    }

    fn region_from(range: TextRange) -> SarifRegion {
        SarifRegion {
            start_line: range.start_line,
            start_column: range.start_col,
            end_line: range.end_line,
            end_column: range.end_col,
        }
    }

    fn physical(uri: &str, range: TextRange) -> SarifPhysicalLocation {
        SarifPhysicalLocation {
            artifact_location: SarifArtifactLocation {
                uri: uri.to_string(),
            },
            region: region_from(range),
        }
    }

    let mut rule_descriptions: BTreeMap<String, String> = BTreeMap::new();
    for diagnostic in diagnostics {
        rule_descriptions
            .entry(diagnostic.rule_id.clone())
            .or_insert_with(|| diagnostic.message.clone());
    }
    let rules: Vec<SarifReportingRule> = rule_descriptions
        .into_iter()
        .map(|(id, text)| {
            let help_uri = rule_help_uri.and_then(|m| m.get(&id).cloned());
            SarifReportingRule {
                id,
                short_description: SarifMessage { text: text.clone() },
                full_description: Some(SarifMessage { text }),
                help_uri,
                properties: SarifProperties {
                    tags: vec!["polint".to_string()],
                },
            }
        })
        .collect();

    let results: Vec<SarifResult> = diagnostics
        .iter()
        .map(|diagnostic| {
            let uri = diagnostic.file.as_str();
            let related_locations: Vec<SarifRelatedLocation> = diagnostic
                .labels
                .iter()
                .map(|label| SarifRelatedLocation {
                    physical_location: physical(uri, label.range),
                    message: SarifMessage {
                        text: label.message.clone(),
                    },
                })
                .collect();

            let fixes = diagnostic.fix.as_ref().and_then(|fix| {
                fix.replacement.as_ref().map(|replacement| {
                    vec![SarifFix {
                        description: SarifMessage {
                            text: fix.message.clone(),
                        },
                        artifact_changes: vec![SarifArtifactChange {
                            artifact_location: SarifArtifactLocation {
                                uri: uri.to_string(),
                            },
                            replacements: vec![SarifReplacement {
                                deleted_region: region_from(diagnostic.range),
                                inserted_content: SarifArtifactContent {
                                    text: replacement.clone(),
                                },
                            }],
                        }],
                    }]
                })
            });
            let code_flows = diagnostic
                .evidence_v1
                .as_ref()
                .map_or_else(Vec::new, |value| {
                    let locations = crate::analysis::evidence::render::sarif_thread_flow_steps(
                        value.as_value(),
                    )
                    .into_iter()
                    .filter_map(|step| {
                        let location = step.location?;
                        Some(SarifThreadFlowLocation {
                            location: SarifLocationWithMessage {
                                physical_location: physical(&location.uri, location.range),
                                message: SarifMessage { text: step.message },
                            },
                        })
                    })
                    .collect::<Vec<_>>();
                    if locations.is_empty() {
                        Vec::new()
                    } else {
                        vec![SarifCodeFlow {
                            thread_flows: vec![SarifThreadFlow { locations }],
                        }]
                    }
                });

            SarifResult {
                rule_id: diagnostic.rule_id.clone(),
                level: match diagnostic.severity {
                    Severity::Info => "note",
                    Severity::Warn => "warning",
                    Severity::Error => "error",
                    _ => "error",
                },
                message: SarifMessage {
                    text: diagnostic.message.clone(),
                },
                fingerprints: SarifFingerprints {
                    polint_v1: diagnostic.stable_fingerprint.clone(),
                },
                locations: vec![SarifLocation {
                    physical_location: physical(uri, diagnostic.range),
                }],
                related_locations,
                code_flows,
                fixes,
            }
        })
        .collect();

    let log = SarifLog {
        version: "2.1.0",
        schema: "https://json.schemastore.org/sarif-2.1.0.json",
        runs: vec![SarifRun {
            tool: SarifTool {
                driver: SarifDriver {
                    name: "polint",
                    information_uri: "https://github.com/oaiz-io/polint",
                    rules,
                },
            },
            results,
        }],
    };

    serde_json::to_string_pretty(&log).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    fn test_opts() -> RenderOpts<'static> {
        RenderOpts {
            json: JsonReportMeta {
                tool_name: "polint",
                tool_version: env!("CARGO_PKG_VERSION"),
            },
            color: ColorChoice::Never,
            sources: None,
            rule_execution: &[],
            run_summary: EMPTY_RUN_SUMMARY,
        }
    }

    fn structured(value: serde_json::Value) -> StructuredEvidenceV1 {
        StructuredEvidenceV1::try_from_value(value).expect("valid structured evidence")
    }

    fn minimal_structured_evidence_value() -> serde_json::Value {
        serde_json::json!({
            "version": 1,
            "bundle": {
                "id": 0,
                "stable_key": "bundle:test",
                "diagnostic_stable_key": "diag:test",
                "status": "Partial",
                "precision": "SetupAware",
                "provenance": "Query",
                "validation": "RendererValidated",
                "confidence": "Medium",
                "replay_key": null
            },
            "paths": [],
            "unknowns": [],
            "omitted_regions": [],
            "limits": {
                "max_paths": 5,
                "max_edges_per_path": 96,
                "max_unknowns": 32,
                "max_omitted_regions": 32,
                "total_paths": 0,
                "rendered_paths": 0,
                "paths_truncated": false,
                "total_unknowns": 0,
                "rendered_unknowns": 0,
                "unknowns_truncated": false,
                "total_omitted_regions": 0,
                "rendered_omitted_regions": 0,
                "omitted_regions_truncated": false
            }
        })
    }

    #[test]
    fn sorting_is_deterministic() {
        let mut diagnostics = vec![
            Diagnostic::warning("b", "b.go", TextRange::point(2, 1), "b"),
            Diagnostic::warning("a", "a.go", TextRange::point(1, 1), "a"),
        ];

        sort_diagnostics(&mut diagnostics);

        assert_eq!(diagnostics[0].file, "a.go");
        assert_eq!(diagnostics[1].file, "b.go");
    }

    #[test]
    fn diagnostic_can_carry_internal_bundle_without_fingerprint_change() {
        let base = Diagnostic::warning("rule", "src/lib.rs", TextRange::point(1, 1), "message");
        let fingerprint = base.stable_fingerprint.clone();

        let diagnostic = base.with_evidence_bundle_ref(EvidenceBundleId(42));

        assert_eq!(diagnostic.stable_fingerprint, fingerprint);
        assert_eq!(
            diagnostic.evidence_bundle_ref().expect("bundle").id,
            EvidenceBundleId(42)
        );
    }

    #[test]
    fn structured_evidence_serializes_without_breaking_scalar_evidence() {
        let diagnostic =
            Diagnostic::warning("rule", "src/lib.rs", TextRange::point(1, 1), "message")
                .with_evidence("symbol", "unsafe_api")
                .with_structured_evidence_v1(structured(serde_json::json!({
                    "version": 1,
                    "bundle": {
                        "id": 0,
                        "stable_key": "bundle:test",
                        "diagnostic_stable_key": "diag:test",
                        "status": "Partial",
                        "precision": "SetupAware",
                        "provenance": "Query",
                        "validation": "RendererValidated",
                        "confidence": "Medium",
                        "replay_key": null
                    },
                    "paths": [],
                    "unknowns": [],
                    "omitted_regions": [],
                    "limits": {
                        "max_paths": 5,
                        "max_edges_per_path": 96,
                        "max_unknowns": 32,
                        "max_omitted_regions": 32,
                        "total_paths": 0,
                        "rendered_paths": 0,
                        "paths_truncated": false,
                        "total_unknowns": 0,
                        "rendered_unknowns": 0,
                        "unknowns_truncated": false,
                        "total_omitted_regions": 0,
                        "rendered_omitted_regions": 0,
                        "omitted_regions_truncated": false
                    }
                })));

        let value = serde_json::to_value(&diagnostic).expect("diagnostic serializes");

        assert_eq!(value["evidence"][0]["label"], "symbol");
        assert_eq!(value["evidence_v1"]["version"], 1);
        assert_eq!(
            diagnostic.stable_fingerprint,
            diagnostic_fingerprint("rule", "src/lib.rs", TextRange::point(1, 1), "message")
        );
    }

    #[test]
    fn diagnostic_rejects_arbitrary_structured_evidence_json() {
        let value = serde_json::json!({
            "rule_id": "rule",
            "severity": "warn",
            "file": "src/lib.rs",
            "range": {
                "start_line": 1,
                "start_col": 1,
                "end_line": 1,
                "end_col": 1
            },
            "message": "message",
            "evidence_v1": {
                "version": 1,
                "unbounded": [{"anything": ["goes"]}]
            }
        });

        let result = serde_json::from_value::<Diagnostic>(value);

        assert!(result.is_err());
    }

    #[test]
    fn diagnostic_rejects_untyped_structured_evidence_taxonomy_values() {
        let value = serde_json::json!({
            "version": 1,
            "bundle": {
                "id": 0,
                "stable_key": "bundle:test",
                "diagnostic_stable_key": "diag:test",
                "status": "Pretend",
                "precision": "SetupAware",
                "provenance": "Query",
                "validation": "RendererValidated",
                "confidence": "Medium",
                "replay_key": null
            },
            "paths": [],
            "unknowns": [],
            "omitted_regions": [],
            "limits": {
                "max_paths": 5,
                "max_edges_per_path": 96,
                "max_unknowns": 32,
                "max_omitted_regions": 32,
                "total_paths": 0,
                "rendered_paths": 0,
                "paths_truncated": false,
                "total_unknowns": 0,
                "rendered_unknowns": 0,
                "unknowns_truncated": false,
                "total_omitted_regions": 0,
                "rendered_omitted_regions": 0,
                "omitted_regions_truncated": false
            }
        });

        let err = StructuredEvidenceV1::try_from_value(value).unwrap_err();

        assert!(err.contains("evidence_v1.bundle.status"));
    }

    #[test]
    fn public_json_report_keeps_validated_structured_evidence_from_local_rule_output() {
        let diagnostic = Diagnostic::warning(
            "local/rule",
            "src/lib.rs",
            TextRange::point(1, 1),
            "message",
        )
        .with_structured_evidence_v1(structured(minimal_structured_evidence_value()));
        let json = render(OutputFormat::Json, &[diagnostic], test_opts());

        let (diagnostics, _rules) = diagnostics_and_rule_execution_from_public_json_report(&json)
            .expect("public diagnostics");

        assert_eq!(diagnostics.len(), 1);
        assert!(
            diagnostics[0].evidence_v1.is_some(),
            "validated evidence_v1 must survive the rule-host trust boundary"
        );
    }

    #[test]
    fn malformed_evidence_from_a_rule_host_is_dropped_not_propagated() {
        let json = serde_json::json!({
            "version": 1,
            "schema": POLINT_REPORT_JSON_SCHEMA_V1_URL,
            "tool": { "name": "polint", "version": "0.0.0" },
            "diagnostics": [{
                "rule_id": "local/rule",
                "severity": "warn",
                "file": "src/lib.rs",
                "range": { "start_line": 1, "start_col": 1, "end_line": 1, "end_col": 1 },
                "message": "message",
                "labels": [],
                "help": null,
                "evidence": [],
                "evidence_v1": {
                    "version": 1,
                    "bundle": {
                        "id": 0,
                        "stable_key": "bundle:bad",
                        "diagnostic_stable_key": "diag:bad",
                        "status": "not-a-real-status",
                        "precision": "Exact",
                        "provenance": "Query",
                        "validation": "RendererValidated",
                        "confidence": "High",
                        "replay_key": null
                    },
                    "paths": [],
                    "unknowns": [],
                    "omitted_regions": [],
                    "limits": {
                        "max_paths": 5,
                        "max_edges_per_path": 96,
                        "max_unknowns": 32,
                        "max_omitted_regions": 32,
                        "total_paths": 0,
                        "rendered_paths": 0,
                        "paths_truncated": false,
                        "total_unknowns": 0,
                        "rendered_unknowns": 0,
                        "unknowns_truncated": false,
                        "total_omitted_regions": 0,
                        "rendered_omitted_regions": 0,
                        "omitted_regions_truncated": false
                    }
                },
                "suggestions": [],
                "fix": null,
                "stable_fingerprint": "fp"
            }]
        })
        .to_string();

        let (diagnostics, _rules) = diagnostics_and_rule_execution_from_public_json_report(&json)
            .expect("public diagnostics");

        assert_eq!(diagnostics.len(), 2);
        assert!(diagnostics[0].evidence_v1.is_none());
        assert_eq!(diagnostics[0].rule_id, "local/rule");
        assert_eq!(diagnostics[1].rule_id, "polint/internal");
        assert!(
            diagnostics[1]
                .message
                .contains("dropped invalid evidence_v1"),
            "{}",
            diagnostics[1].message
        );
    }

    #[test]
    fn structured_evidence_string_limit_counts_characters_like_schema() {
        let mut value = minimal_structured_evidence_value();
        value["bundle"]["stable_key"] = serde_json::Value::String("é".repeat(4096));

        StructuredEvidenceV1::try_from_value(value).expect("4096 unicode chars are schema-valid");
    }

    #[test]
    fn structured_evidence_rejects_inconsistent_rendered_counts() {
        let mut value = minimal_structured_evidence_value();
        value["limits"]["rendered_paths"] = serde_json::json!(1);

        let err = StructuredEvidenceV1::try_from_value(value).unwrap_err();

        assert!(err.contains("paths rendered count"));
    }

    fn contract_diagnostic() -> Diagnostic {
        Diagnostic::error(
            "project/rule",
            "src/lib.rs",
            TextRange::new(10, 4, 10, 12),
            "policy failed",
        )
        .with_label(TextRange::new(11, 2, 11, 8), "related expression")
        .with_evidence("symbol", "unsafe_api")
        .with_suggestion("Prefer safe_api here")
        .with_fix("Replace unsafe_api", Some("safe_api()".to_string()))
        .with_help("Use the safe wrapper before crossing this boundary.")
        .with_fingerprint("fingerprint-123")
    }

    #[test]
    fn render_human_summary_line_groups_counts_by_severity_and_rule_id() {
        let d1 = Diagnostic::error("local/a", "a.go", TextRange::point(1, 1), "m");
        let d2 = Diagnostic::error("local/a", "b.go", TextRange::point(1, 1), "m");
        let d3 = Diagnostic::error("parser/go", "c.go", TextRange::point(1, 1), "m");
        let d4 = Diagnostic::warning("local/b", "d.go", TextRange::point(1, 1), "m");
        let d5 = Diagnostic::info("local/c", "e.go", TextRange::point(1, 1), "m");
        let d6 = Diagnostic::info("local/c", "f.go", TextRange::point(1, 1), "m");
        let rendered = render_human(&[d1, d2, d3, d4, d5, d6], ColorChoice::Never, None);
        assert!(
            rendered.starts_with("Summary:"),
            "unexpected prefix: {rendered:?}"
        );
        assert!(rendered.contains("3 errors"));
        assert!(rendered.contains("local/a (2)"));
        assert!(rendered.contains("parser/go (1)"));
        assert!(rendered.contains("1 warning"));
        assert!(rendered.contains("local/b (1)"));
        assert!(rendered.contains("2 infos"));
        assert!(rendered.contains("local/c (2)"));
    }

    #[test]
    fn only_rule_keeps_polint_ignore_health_diagnostics_for_matching_selectors() {
        let diagnostics = vec![
            Diagnostic::warning(
                "polint/unused-ignore",
                "src/a.ts",
                TextRange::point(1, 1),
                "unused",
            )
            .with_evidence("selectors", "local/*"),
            Diagnostic::warning(
                "polint/unused-ignore",
                "src/b.ts",
                TextRange::point(1, 1),
                "unused",
            )
            .with_evidence("selectors", "other/rule"),
            Diagnostic::warning(
                "local/no-todo",
                "src/c.ts",
                TextRange::point(1, 1),
                "finding",
            ),
        ];

        let filtered = apply_report_filters(diagnostics, Some("local/no-todo"));
        let rule_ids = filtered
            .iter()
            .map(|diagnostic| diagnostic.rule_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(rule_ids, ["polint/unused-ignore", "local/no-todo"]);
    }

    #[test]
    fn only_rule_keeps_parser_diagnostics() {
        let diagnostics = vec![
            Diagnostic::error("parser/ts", "src/a.ts", TextRange::point(1, 1), "parse"),
            Diagnostic::warning(
                "local/no-todo",
                "src/b.ts",
                TextRange::point(1, 1),
                "finding",
            ),
        ];

        let filtered = apply_report_filters(diagnostics, Some("local/no-todo"));
        let rule_ids = filtered
            .iter()
            .map(|diagnostic| diagnostic.rule_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(rule_ids, ["parser/ts", "local/no-todo"]);
    }

    #[test]
    fn render_human_always_color_includes_escape_sequences() {
        let diagnostic = Diagnostic::error("r", "f.go", TextRange::point(1, 1), "oops");
        let rendered = render_human(&[diagnostic], ColorChoice::Always, None);
        assert!(
            rendered.contains("\x1b["),
            "expected ANSI SGR sequences: {rendered:?}"
        );
    }

    #[test]
    fn render_human_snapshot_includes_contract_fields() {
        insta::assert_snapshot!(
            render(
                OutputFormat::Human,
                &[contract_diagnostic()],
                test_opts(),
            ),
            @r###"
        Summary: 1 error — project/rule (1)

        error[project/rule]: policy failed
          --> src/lib.rs:10:4-10:12
          label 11:2-11:8: related expression
          evidence symbol: unsafe_api
          suggestion: Prefer safe_api here
          fix: Replace unsafe_api
            replacement: safe_api()
          help: Use the safe wrapper before crossing this boundary.
          fingerprint: fingerprint-123

        "###,
        );
    }

    #[test]
    fn render_human_includes_snippet_when_sources_map_provided() {
        let source_text: String = (1..=12)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let mut sources = BTreeMap::new();
        sources.insert(
            "src/lib.rs".to_string(),
            Arc::from(source_text.into_boxed_str()),
        );
        let opts = RenderOpts {
            json: JsonReportMeta {
                tool_name: "polint",
                tool_version: env!("CARGO_PKG_VERSION"),
            },
            color: ColorChoice::Never,
            sources: Some(&sources),
            rule_execution: &[],
            run_summary: EMPTY_RUN_SUMMARY,
        };
        insta::assert_snapshot!(
            render(OutputFormat::Human, &[contract_diagnostic()], opts),
            @r###"
        Summary: 1 error — project/rule (1)

        error[project/rule]: policy failed
          --> src/lib.rs:10:4-10:12
             |
          10 | line 10
             |    ^^^^
          label 11:2-11:8: related expression
          evidence symbol: unsafe_api
          suggestion: Prefer safe_api here
          fix: Replace unsafe_api
            replacement: safe_api()
          help: Use the safe wrapper before crossing this boundary.
          fingerprint: fingerprint-123

        "###,
        );
    }

    #[test]
    fn render_json_snapshot_is_stable() {
        let rendered = render(OutputFormat::Json, &[contract_diagnostic()], test_opts());
        let parsed: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(parsed["version"], POLINT_REPORT_JSON_SCHEMA_V);
        assert_eq!(
            parsed["schema"].as_str().unwrap(),
            POLINT_REPORT_JSON_SCHEMA_V1_URL
        );
        assert_eq!(parsed["tool"]["name"], "polint");
        assert_eq!(parsed["tool"]["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(parsed["diagnostics"].as_array().unwrap().len(), 1);

        let normalized = rendered.replace(env!("CARGO_PKG_VERSION"), "<PKG_VERSION>");
        insta::assert_snapshot!(normalized, @r###"
        {
          "version": 2,
          "schema": "https://raw.githubusercontent.com/oaiz-io/polint/main/docs/schemas/polint-report-v1.json",
          "tool": {
            "name": "polint",
            "version": "<PKG_VERSION>"
          },
          "diagnostics": [
            {
              "rule_id": "project/rule",
              "severity": "error",
              "file": "src/lib.rs",
              "range": {
                "start_line": 10,
                "start_col": 4,
                "end_line": 10,
                "end_col": 12
              },
              "message": "policy failed",
              "labels": [
                {
                  "range": {
                    "start_line": 11,
                    "start_col": 2,
                    "end_line": 11,
                    "end_col": 8
                  },
                  "message": "related expression"
                }
              ],
              "help": "Use the safe wrapper before crossing this boundary.",
              "evidence": [
                {
                  "label": "symbol",
                  "value": "unsafe_api"
                }
              ],
              "suggestions": [
                {
                  "message": "Prefer safe_api here"
                }
              ],
              "fix": {
                "message": "Replace unsafe_api",
                "replacement": "safe_api()"
              },
              "stable_fingerprint": "fingerprint-123"
            }
          ]
        }
        "###);
    }

    #[test]
    fn ai_friendly_report_counts_rules_and_caps_examples() {
        let mut diagnostics = Vec::new();
        for index in 0..12 {
            diagnostics.push(Diagnostic::warning(
                format!("local/rule-{index:02}"),
                format!("src/{index:02}.ts"),
                TextRange::point(index + 1, 1),
                "policy failed",
            ));
        }
        diagnostics.push(Diagnostic::error(
            "local/rule-05",
            "src/error.ts",
            TextRange::point(1, 1),
            "more important",
        ));
        sort_diagnostics(&mut diagnostics);

        let persisted = diagnostics[..3].to_vec();
        let report = build_ai_friendly_report(
            &diagnostics,
            &persisted,
            test_opts().json,
            "123456",
            &[],
            &RunSummary::default(),
        );

        assert_eq!(report.version, POLINT_REPORT_JSON_SCHEMA_V);
        assert_eq!(report.schema, POLINT_AI_FRIENDLY_JSON_SCHEMA_V1_URL);
        assert_eq!(report.summary.total_diagnostics, 13);
        assert_eq!(report.summary.rules_triggered, 12);
        assert_eq!(report.summary.by_severity["error"], 1);
        assert_eq!(report.summary.by_severity["warn"], 12);
        assert_eq!(report.summary.by_rule[0].rule_id, "local/rule-05");
        assert_eq!(report.summary.by_rule[0].total, 2);
        assert!(report.summary.rules.is_empty());
        assert_eq!(report.examples.len(), AI_FRIENDLY_EXAMPLE_LIMIT);
        assert_eq!(report.examples[0].message, "more important");
        assert!(report.examples[0].labels.is_empty());
        assert!(report.examples[0].evidence.is_empty());
        assert_eq!(report.diagnostics.len(), 3);
        assert_eq!(report.truncation.as_ref().unwrap().total_diagnostics, 13);
    }

    #[test]
    fn render_ai_friendly_stdout_is_compact_and_query_oriented() {
        let diagnostics = vec![contract_diagnostic()];
        let report = build_ai_friendly_report(
            &diagnostics,
            &diagnostics,
            test_opts().json,
            "123456",
            &[],
            &RunSummary::default(),
        );
        let rendered = render_ai_friendly_stdout(&report, ".polint/output/latest.json");

        assert!(rendered.contains("polint: 1 diagnostic across 1 rule"));
        assert!(rendered.contains("Full JSON: .polint/output/latest.json"));
        assert!(rendered.contains("error[project/rule]: policy failed"));
        assert!(rendered.contains("--> src/lib.rs:10:4-10:12"));
        assert!(rendered.contains("label 11:2-11:8: related expression"));
        assert!(rendered.contains("evidence symbol: unsafe_api"));
        assert!(rendered.contains("suggestion: Prefer safe_api here"));
        assert!(rendered.contains("fix: Replace unsafe_api"));
        assert!(rendered.contains("help: Use the safe wrapper before crossing this boundary."));
        assert!(rendered.contains("jq '.summary.by_rule' .polint/output/latest.json"));
        assert!(rendered.contains(
            "jq '.summary.rules[] | select(.diagnostics_emitted == 0)' .polint/output/latest.json"
        ));
        assert!(
            rendered.contains(
                "jq '[.diagnostics[] | select(.rule_id==\"project/rule\")][0:20]' .polint/output/latest.json"
            ),
            "{rendered}"
        );
        assert!(
            rendered.contains(
                "jq '.diagnostics[] | select(.file==\"src/lib.rs\") | {rule_id, range, message}' .polint/output/latest.json | head -c 12000"
            ),
            "{rendered}"
        );
        assert!(rendered.contains("Do not read the whole file into an AI prompt"));
        assert!(!rendered.contains("stable_fingerprint"));
    }

    #[test]
    fn render_github_uses_workflow_annotations() {
        let diagnostic = Diagnostic::warning(
            "project/rule,one",
            "src/lib,one.rs",
            TextRange::new(10, 4, 10, 12),
            "policy % failed\nuse safe_api",
        );

        insta::assert_snapshot!(
            render(OutputFormat::Github, &[diagnostic], test_opts()),
            @"::warning file=src/lib%2Cone.rs,line=10,col=4,endLine=10,endColumn=12,title=project/rule%2Cone::policy %25 failed%0Ause safe_api
        "
        );
    }

    #[test]
    fn diagnostics_from_json_report_round_trips() {
        let input = vec![contract_diagnostic()];
        let json = render(OutputFormat::Json, &input, test_opts());
        let output = diagnostics_from_json_report(&json).unwrap();
        assert_eq!(output, input);
    }

    #[test]
    fn polint_report_json_schema_file_tracks_repo() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/schemas/polint-report-v1.json");
        let raw = std::fs::read_to_string(&path).expect("polint-report schema should exist");
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            value["$id"].as_str().unwrap(),
            POLINT_REPORT_JSON_SCHEMA_V1_URL
        );
        assert!(
            value["$defs"]["diagnostic"]["properties"]
                .get("evidence_v1")
                .is_some()
        );
    }

    #[test]
    fn render_sarif_includes_rule_help_uri_from_map() {
        let mut help = BTreeMap::new();
        help.insert(
            "project/rule".to_string(),
            "https://example.invalid/rules/project-rule".to_string(),
        );
        let rendered = render_with_sarif_help(
            OutputFormat::Sarif,
            &[contract_diagnostic()],
            RenderOpts {
                json: JsonReportMeta {
                    tool_name: "polint",
                    tool_version: env!("CARGO_PKG_VERSION"),
                },
                color: ColorChoice::Never,
                sources: None,
                rule_execution: &[],
                run_summary: EMPTY_RUN_SUMMARY,
            },
            Some(&help),
        );
        let parsed: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(
            parsed
                .pointer("/runs/0/tool/driver/rules/0/helpUri")
                .unwrap(),
            "https://example.invalid/rules/project-rule"
        );
        assert_eq!(
            parsed
                .pointer("/runs/0/tool/driver/rules/0/properties/tags/0")
                .unwrap(),
            "polint"
        );
        assert!(parsed.pointer("/runs/0/tool/driver/rules/0/tags").is_none());
    }

    #[test]
    fn render_sarif_projects_structured_evidence_to_code_flows() {
        let diagnostic = Diagnostic::warning(
            "project/rule",
            "src/lib.rs",
            TextRange::point(7, 3),
            "policy failed",
        )
        .with_structured_evidence_v1(structured(serde_json::json!({
            "version": 1,
            "bundle": {
                "id": 0,
                "stable_key": "bundle:diag",
                "diagnostic_stable_key": "diag:1",
                "status": "Partial",
                "precision": "SetupAware",
                "provenance": "Query",
                "validation": "RendererValidated",
                "confidence": "Medium",
                "replay_key": "replay:bundle"
            },
            "paths": [{
                "id": 0,
                "stable_key": "path:summary",
                "rank": 0,
                "status": "Partial",
                "hidden_node_count": 0,
                "nodes": [0, 1],
                "edges": [{
                    "id": 0,
                    "stable_key": "edge:summary",
                    "kind": "Summary",
                    "status": "Partial",
                    "precision": "SetupAware",
                    "provenance": "Summary",
                    "validation": "ReferentiallyValidated",
                    "confidence": "Medium",
                    "summary_stable_key": "summary:tito",
                    "expansion": {"state": "opaque", "reason": "summary_status=Unknown"},
                    "location": {
                        "uri": "src/evidence.rs",
                        "range": {
                            "start_line": 3,
                            "start_col": 5,
                            "end_line": 3,
                            "end_col": 11
                        }
                    }
                }],
                "omitted_regions": [],
                "total_edges": 1,
                "rendered_edges": 1,
                "edges_truncated": false
            }],
            "unknowns": [{
                "stable_key": "unknown:summary",
                "reason": "OpaqueSummary",
                "message": "opaque summary",
                "edge": 0
            }],
            "omitted_regions": [{
                "id": 0,
                "stable_key": "omitted:compact",
                "reason": "CompactRendering",
                "hidden_node_count": 1,
                "hidden_edge_count": 0,
                "budget_label": "test"
            }],
            "limits": {
                "max_paths": 5,
                "max_edges_per_path": 96,
                "max_unknowns": 32,
                "max_omitted_regions": 32,
                "total_paths": 1,
                "rendered_paths": 1,
                "paths_truncated": false,
                "total_unknowns": 1,
                "rendered_unknowns": 1,
                "unknowns_truncated": false,
                "total_omitted_regions": 1,
                "rendered_omitted_regions": 1,
                "omitted_regions_truncated": false
            }
        })));

        let rendered = render_sarif(&[diagnostic], None);
        let parsed: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        let code_flows = &parsed["runs"][0]["results"][0]["codeFlows"];

        assert_eq!(code_flows.as_array().expect("code flows").len(), 1);
        let text = serde_json::to_string(code_flows).unwrap();
        assert!(text.contains("Partial"));
        assert!(text.contains("unknown"));
        assert!(text.contains("omitted"));
    }

    #[test]
    fn render_sarif_omits_code_flows_without_evidence_step_locations() {
        let diagnostic = Diagnostic::warning(
            "project/rule",
            "src/lib.rs",
            TextRange::point(7, 3),
            "policy failed",
        )
        .with_structured_evidence_v1(structured(serde_json::json!({
            "version": 1,
            "bundle": {
                "id": 0,
                "stable_key": "bundle:diag",
                "diagnostic_stable_key": "diag:1",
                "status": "Partial",
                "precision": "SetupAware",
                "provenance": "Query",
                "validation": "RendererValidated",
                "confidence": "Medium",
                "replay_key": "replay:bundle"
            },
            "paths": [{
                "id": 0,
                "stable_key": "path:summary",
                "rank": 0,
                "status": "Partial",
                "hidden_node_count": 0,
                "nodes": [0, 1],
                "edges": [{
                    "id": 0,
                    "stable_key": "edge:summary",
                    "kind": "Summary",
                    "status": "Partial",
                    "precision": "SetupAware",
                    "provenance": "Summary",
                    "validation": "ReferentiallyValidated",
                    "confidence": "Medium",
                    "summary_stable_key": "summary:tito",
                    "expansion": {"state": "opaque", "reason": "summary_status=Unknown"}
                }],
                "omitted_regions": [],
                "total_edges": 1,
                "rendered_edges": 1,
                "edges_truncated": false
            }],
            "unknowns": [],
            "omitted_regions": [],
            "limits": {
                "max_paths": 5,
                "max_edges_per_path": 96,
                "max_unknowns": 32,
                "max_omitted_regions": 32,
                "total_paths": 1,
                "rendered_paths": 1,
                "paths_truncated": false,
                "total_unknowns": 0,
                "rendered_unknowns": 0,
                "unknowns_truncated": false,
                "total_omitted_regions": 0,
                "rendered_omitted_regions": 0,
                "omitted_regions_truncated": false
            }
        })));

        let rendered = render_sarif(&[diagnostic], None);
        let parsed: serde_json::Value = serde_json::from_str(&rendered).unwrap();

        assert!(parsed.pointer("/runs/0/results/0/codeFlows").is_none());
    }

    #[test]
    fn render_sarif_uses_evidence_step_locations_when_available() {
        let diagnostic = Diagnostic::warning(
            "project/rule",
            "src/diagnostic.rs",
            TextRange::point(7, 3),
            "policy failed",
        )
        .with_structured_evidence_v1(structured(serde_json::json!({
            "version": 1,
            "bundle": {
                "id": 0,
                "stable_key": "bundle:diag",
                "diagnostic_stable_key": "diag:1",
                "status": "Partial",
                "precision": "SetupAware",
                "provenance": "Query",
                "validation": "RendererValidated",
                "confidence": "Medium",
                "replay_key": "replay:bundle"
            },
            "paths": [{
                "id": 0,
                "stable_key": "path:summary",
                "rank": 0,
                "status": "Partial",
                "hidden_node_count": 0,
                "nodes": [0, 1],
                "edges": [{
                    "id": 0,
                    "stable_key": "edge:summary",
                    "kind": "Summary",
                    "status": "Partial",
                    "precision": "SetupAware",
                    "provenance": "Summary",
                    "validation": "ReferentiallyValidated",
                    "confidence": "Medium",
                    "summary_stable_key": "summary:tito",
                    "expansion": {"state": "opaque", "reason": "summary_status=Unknown"},
                    "location": {
                        "uri": "src/evidence.rs",
                        "range": {
                            "start_line": 3,
                            "start_col": 5,
                            "end_line": 3,
                            "end_col": 11
                        }
                    }
                }],
                "omitted_regions": [],
                "total_edges": 1,
                "rendered_edges": 1,
                "edges_truncated": false
            }],
            "unknowns": [],
            "omitted_regions": [],
            "limits": {
                "max_paths": 5,
                "max_edges_per_path": 96,
                "max_unknowns": 32,
                "max_omitted_regions": 32,
                "total_paths": 1,
                "rendered_paths": 1,
                "paths_truncated": false,
                "total_unknowns": 0,
                "rendered_unknowns": 0,
                "unknowns_truncated": false,
                "total_omitted_regions": 0,
                "rendered_omitted_regions": 0,
                "omitted_regions_truncated": false
            }
        })));

        let rendered = render_sarif(&[diagnostic], None);
        let parsed: serde_json::Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(
            parsed.pointer("/runs/0/results/0/codeFlows/0/threadFlows/0/locations/0/location/physicalLocation/artifactLocation/uri"),
            Some(&serde_json::json!("src/evidence.rs"))
        );
        assert_eq!(
            parsed.pointer("/runs/0/results/0/codeFlows/0/threadFlows/0/locations/0/location/physicalLocation/region/startLine"),
            Some(&serde_json::json!(3))
        );
        assert_eq!(
            parsed
                .pointer(
                    "/runs/0/results/0/codeFlows/0/threadFlows/0/locations/0/location/message/text"
                )
                .and_then(serde_json::Value::as_str)
                .map(|message| message.contains("confidence=Medium")),
            Some(true)
        );
    }

    #[test]
    fn render_sarif_snapshot_includes_ci_fields() {
        let rendered = render(OutputFormat::Sarif, &[contract_diagnostic()], test_opts());
        let parsed: serde_json::Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(parsed.pointer("/version").unwrap(), "2.1.0");
        assert_eq!(
            parsed.pointer("/runs/0/tool/driver/name").unwrap(),
            "polint"
        );
        assert_eq!(
            parsed.pointer("/runs/0/tool/driver/rules/0/id").unwrap(),
            "project/rule"
        );
        assert_eq!(
            parsed.pointer("/runs/0/results/0/ruleId").unwrap(),
            "project/rule"
        );
        assert_eq!(parsed.pointer("/runs/0/results/0/level").unwrap(), "error");
        assert_eq!(
            parsed.pointer("/runs/0/results/0/message/text").unwrap(),
            "policy failed"
        );
        assert_eq!(
            parsed
                .pointer("/runs/0/results/0/fingerprints/polint~1v1")
                .unwrap(),
            "fingerprint-123"
        );
        assert_eq!(
            parsed
                .pointer("/runs/0/results/0/locations/0/physicalLocation/artifactLocation/uri")
                .unwrap(),
            "src/lib.rs"
        );
        assert_eq!(
            parsed
                .pointer("/runs/0/results/0/relatedLocations/0/physicalLocation/region/startLine",)
                .and_then(|v| v.as_u64()),
            Some(11)
        );
        assert!(parsed
            .pointer("/runs/0/results/0/fixes/0/artifactChanges/0/replacements/0/insertedContent/text")
            .is_some());

        insta::assert_snapshot!(rendered, @r###"
        {
          "version": "2.1.0",
          "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
          "runs": [
            {
              "tool": {
                "driver": {
                  "name": "polint",
                  "informationUri": "https://github.com/oaiz-io/polint",
                  "rules": [
                    {
                      "id": "project/rule",
                      "shortDescription": {
                        "text": "policy failed"
                      },
                      "fullDescription": {
                        "text": "policy failed"
                      },
                      "properties": {
                        "tags": [
                          "polint"
                        ]
                      }
                    }
                  ]
                }
              },
              "results": [
                {
                  "ruleId": "project/rule",
                  "level": "error",
                  "message": {
                    "text": "policy failed"
                  },
                  "fingerprints": {
                    "polint/v1": "fingerprint-123"
                  },
                  "locations": [
                    {
                      "physicalLocation": {
                        "artifactLocation": {
                          "uri": "src/lib.rs"
                        },
                        "region": {
                          "startLine": 10,
                          "startColumn": 4,
                          "endLine": 10,
                          "endColumn": 12
                        }
                      }
                    }
                  ],
                  "relatedLocations": [
                    {
                      "physicalLocation": {
                        "artifactLocation": {
                          "uri": "src/lib.rs"
                        },
                        "region": {
                          "startLine": 11,
                          "startColumn": 2,
                          "endLine": 11,
                          "endColumn": 8
                        }
                      },
                      "message": {
                        "text": "related expression"
                      }
                    }
                  ],
                  "fixes": [
                    {
                      "description": {
                        "text": "Replace unsafe_api"
                      },
                      "artifactChanges": [
                        {
                          "artifactLocation": {
                            "uri": "src/lib.rs"
                          },
                          "replacements": [
                            {
                              "deletedRegion": {
                                "startLine": 10,
                                "startColumn": 4,
                                "endLine": 10,
                                "endColumn": 12
                              },
                              "insertedContent": {
                                "text": "safe_api()"
                              }
                            }
                          ]
                        }
                      ]
                    }
                  ]
                }
              ]
            }
          ]
        }
        "###);
    }

    #[test]
    fn diagnostic_deserializes_missing_phase3_fields_with_computed_fingerprint() {
        let diagnostic: Diagnostic = serde_json::from_str(
            r#"{
                "rule_id": "project/rule",
                "severity": "warn",
                "file": "src/lib.rs",
                "range": {
                    "start_line": 4,
                    "start_col": 2,
                    "end_line": 4,
                    "end_col": 9
                },
                "message": "policy failed"
            }"#,
        )
        .unwrap();

        let range = TextRange::new(4, 2, 4, 9);

        assert_eq!(diagnostic.rule_id, "project/rule");
        assert_eq!(diagnostic.severity, Severity::Warn);
        assert_eq!(diagnostic.file, "src/lib.rs");
        assert_eq!(diagnostic.range, range);
        assert_eq!(diagnostic.message, "policy failed");
        assert!(diagnostic.labels.is_empty());
        assert!(diagnostic.help.is_none());
        assert!(diagnostic.evidence.is_empty());
        assert!(diagnostic.suggestions.is_empty());
        assert!(diagnostic.fix.is_none());
        assert_eq!(
            diagnostic.stable_fingerprint,
            diagnostic_fingerprint("project/rule", "src/lib.rs", range, "policy failed")
        );
    }

    #[test]
    fn render_empty_human_output_is_stable() {
        assert_eq!(
            render(OutputFormat::Human, &[], test_opts()),
            "No diagnostics.\n"
        );
    }

    #[test]
    fn render_empty_json_report_is_stable() {
        let rendered = render(OutputFormat::Json, &[], test_opts());
        let parsed: PolintReport = serde_json::from_str(&rendered).unwrap();
        assert_eq!(parsed.version, POLINT_REPORT_JSON_SCHEMA_V);
        assert_eq!(parsed.tool.name, "polint");
        assert!(parsed.diagnostics.is_empty());
    }

    #[test]
    fn diagnostic_builders_cover_labels_suggestions_fixes_evidence_and_help() {
        let range = TextRange::new(10, 4, 10, 12);
        let label_range = TextRange::new(11, 2, 11, 8);

        let diagnostic = Diagnostic::error("project/rule", "src/lib.rs", range, "policy failed")
            .with_label(label_range, "related expression")
            .with_evidence("symbol", "unsafe_api")
            .with_suggestion("Prefer safe_api here")
            .with_fix("Replace unsafe_api", Some("safe_api".to_string()))
            .with_help("Use the safe wrapper before crossing this boundary.");

        assert_eq!(diagnostic.rule_id, "project/rule");
        assert_eq!(diagnostic.severity, Severity::Error);
        assert_eq!(diagnostic.file, "src/lib.rs");
        assert_eq!(diagnostic.range, range);
        assert_eq!(diagnostic.message, "policy failed");
        assert_eq!(
            diagnostic.labels,
            vec![Label::new(label_range, "related expression".to_string())]
        );
        assert_eq!(
            diagnostic.evidence,
            vec![Evidence::new(
                "symbol".to_string(),
                "unsafe_api".to_string(),
            )]
        );
        assert_eq!(
            diagnostic.suggestions,
            vec![Suggestion::new("Prefer safe_api here".to_string())]
        );
        assert_eq!(
            diagnostic.fix,
            Some(Fix::new(
                "Replace unsafe_api".to_string(),
                Some("safe_api".to_string())
            ))
        );
        assert_eq!(
            diagnostic.help,
            Some("Use the safe wrapper before crossing this boundary.".to_string())
        );
        assert!(!diagnostic.stable_fingerprint.is_empty());
    }

    #[test]
    fn fingerprint_includes_rule_file_full_range_and_message() {
        let range = TextRange::new(4, 2, 4, 9);
        let baseline = Diagnostic::warning("project/rule", "src/lib.rs", range, "policy failed")
            .stable_fingerprint;

        let changed_start_line = Diagnostic::warning(
            "project/rule",
            "src/lib.rs",
            TextRange::new(5, range.start_col, range.end_line, range.end_col),
            "policy failed",
        )
        .stable_fingerprint;
        let changed_start_col = Diagnostic::warning(
            "project/rule",
            "src/lib.rs",
            TextRange::new(range.start_line, 3, range.end_line, range.end_col),
            "policy failed",
        )
        .stable_fingerprint;
        let changed_end_line = Diagnostic::warning(
            "project/rule",
            "src/lib.rs",
            TextRange::new(range.start_line, range.start_col, 5, range.end_col),
            "policy failed",
        )
        .stable_fingerprint;
        let changed_end_col = Diagnostic::warning(
            "project/rule",
            "src/lib.rs",
            TextRange::new(range.start_line, range.start_col, range.end_line, 10),
            "policy failed",
        )
        .stable_fingerprint;
        let changed_file =
            Diagnostic::warning("project/rule", "src/main.rs", range, "policy failed")
                .stable_fingerprint;
        let changed_rule =
            Diagnostic::warning("project/other", "src/lib.rs", range, "policy failed")
                .stable_fingerprint;
        let changed_message =
            Diagnostic::warning("project/rule", "src/lib.rs", range, "different message")
                .stable_fingerprint;

        for changed in [
            changed_start_line,
            changed_start_col,
            changed_end_line,
            changed_end_col,
            changed_file,
            changed_rule,
            changed_message,
        ] {
            assert_ne!(baseline, changed);
        }

        let changed_non_identity_fields =
            Diagnostic::error("project/rule", "src/lib.rs", range, "policy failed")
                .with_label(range, "related")
                .with_evidence("name", "value")
                .with_suggestion("try another expression")
                .with_fix("replace it", Some("replacement".to_string()))
                .with_help("extra context");

        assert_eq!(baseline, changed_non_identity_fields.stable_fingerprint);
    }

    #[test]
    fn dedupe_diagnostics_collapses_same_fingerprint_after_sorting() {
        let duplicate = Diagnostic::warning("project/rule", "b.go", TextRange::point(2, 1), "b");
        let unique = Diagnostic::warning("project/rule", "a.go", TextRange::point(1, 1), "a");

        let diagnostics = dedupe_diagnostics(vec![duplicate.clone(), unique.clone(), duplicate]);

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].stable_fingerprint, unique.stable_fingerprint);
        assert_eq!(diagnostics[1].file, "b.go");
    }

    #[test]
    fn dedupe_diagnostics_removes_non_adjacent_duplicate_fingerprints() {
        let first =
            Diagnostic::warning("rule/a", "a.go", TextRange::point(1, 1), "first duplicate")
                .with_fingerprint("same-fingerprint");
        let unique = Diagnostic::warning("rule/b", "b.go", TextRange::point(1, 1), "unique")
            .with_fingerprint("unique-fingerprint");
        let second =
            Diagnostic::warning("rule/c", "c.go", TextRange::point(1, 1), "second duplicate")
                .with_fingerprint("same-fingerprint");

        let diagnostics = dedupe_diagnostics(vec![second, unique.clone(), first.clone()]);

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0], first);
        assert_eq!(diagnostics[1], unique);
    }

    fn diagnostic_from_index(index: usize) -> Diagnostic {
        let file = format!("src/{:02}.rs", index % 5);
        let rule_id = format!("rule/{:02}", index % 7);
        let line = (index % 11) as u32 + 1;
        let col = (index % 13) as u32 + 1;
        Diagnostic::warning(
            rule_id,
            file,
            TextRange::new(
                line,
                col,
                line + ((index % 3) as u32),
                col + ((index % 5) as u32),
            ),
            format!("message {index:02}"),
        )
    }

    fn sorted_keys(
        diagnostics: &mut [Diagnostic],
    ) -> Vec<(String, u32, u32, String, String, String)> {
        sort_diagnostics(diagnostics);
        diagnostics
            .iter()
            .map(|diagnostic| {
                (
                    diagnostic.file.clone(),
                    diagnostic.range.start_line,
                    diagnostic.range.start_col,
                    diagnostic.rule_id.clone(),
                    diagnostic.message.clone(),
                    diagnostic.stable_fingerprint.clone(),
                )
            })
            .collect()
    }

    proptest! {
        #[test]
        fn sort_diagnostics_is_input_order_independent(order in proptest::collection::vec(0usize..30, 1..80)) {
            let mut first: Vec<_> = order.iter().copied().map(diagnostic_from_index).collect();
            let mut second: Vec<_> = order.iter().rev().copied().map(diagnostic_from_index).collect();

            prop_assert_eq!(sorted_keys(&mut first), sorted_keys(&mut second));
        }
    }
}
