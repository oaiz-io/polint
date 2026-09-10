//! Preview policy-query vocabulary for repo-local rules.
//!
//! These types define the public shape for v1.4 policy queries. They are
//! intentionally plain data structs with `new(required...)` constructors and
//! named option fields. Runtime query behavior is backed only for documented
//! preview scopes; unsupported patterns return no policy matches rather than
//! exposing internal graph APIs.

use crate::cache::stable_hash;
use crate::diagnostics::{Diagnostic, StructuredEvidenceV1, TextRange};

pub(crate) const POLICY_QUERY_VERSION: &str = "policy-query-preview-v1";

/// Edge taxonomy used when rendering policy-query path hops into `evidence_v1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PolicyEvidenceEdgeKind {
    Call,
    Control,
    DataTaint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PolicyOperation {
    EventsMatching,
    CallsForbiddenReachable,
    ControlFlowMissingGuard,
    ControlFlowGuardOutcomes,
    ControlFlowMissingCleanup,
    DataFlowForbidden,
}

impl PolicyOperation {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::EventsMatching => "events.matching",
            Self::CallsForbiddenReachable => "calls.forbidden_reachable",
            Self::ControlFlowMissingGuard => "control_flow.missing_guard",
            Self::ControlFlowGuardOutcomes => "control_flow.guard_outcomes",
            Self::ControlFlowMissingCleanup => "control_flow.missing_cleanup",
            Self::DataFlowForbidden => "data_flow.forbidden",
        }
    }
}

/// Result status for a policy-query match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PolicyStatus {
    /// The query result is exact for the supported scope.
    Exact,
    /// The query result is heuristic and must be described as such.
    Heuristic,
    /// The query could not be answered with the available analysis support.
    Unknown,
    /// The query family is known but not supported in the current stage.
    Unsupported,
    /// The query exceeded its configured budget.
    BudgetExceeded,
}

/// Precision level attached to a policy-query result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PolicyPrecision {
    /// Exact semantic evidence.
    Exact,
    /// Setup-aware evidence that may depend on local lifecycle configuration.
    SetupAware,
    /// Syntax-level evidence.
    Syntax,
    /// Conservative evidence.
    Conservative,
    /// Heuristic evidence.
    Heuristic,
    /// Unknown precision.
    Unknown,
}

/// Confidence level attached to a policy-query result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PolicyConfidence {
    /// High-confidence evidence from resolved or validated facts.
    High,
    /// Medium-confidence evidence from setup-aware or conservative facts.
    Medium,
    /// Low-confidence evidence from heuristic or unknown facts.
    Low,
}

/// Preview violation returned by policy-query views.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PolicyViolation {
    query: PolicyOperation,
    query_digest: String,
    file: String,
    range: TextRange,
    status: PolicyStatus,
    precision: PolicyPrecision,
    evidence: Vec<(String, String)>,
    structured_evidence: Option<StructuredEvidenceV1>,
}

impl PolicyViolation {
    pub(crate) fn new(
        query: PolicyOperation,
        query_digest: impl Into<String>,
        file: impl Into<String>,
        range: TextRange,
        status: PolicyStatus,
        precision: PolicyPrecision,
        evidence: Vec<(String, String)>,
    ) -> Self {
        Self {
            query,
            query_digest: query_digest.into(),
            file: file.into(),
            range,
            status,
            precision,
            evidence,
            structured_evidence: None,
        }
    }

    pub(crate) fn set_status(&mut self, status: PolicyStatus) {
        self.status = status;
    }

    pub(crate) fn set_precision(&mut self, precision: PolicyPrecision) {
        self.precision = precision;
    }

    pub(crate) fn push_evidence(&mut self, label: impl Into<String>, value: impl Into<String>) {
        self.evidence.push((label.into(), value.into()));
    }

    /// Attaches a bounded `evidence_v1` envelope built from engine path hops.
    pub(crate) fn with_path_evidence(
        mut self,
        hops: impl IntoIterator<Item = impl Into<String>>,
        edge_kind: PolicyEvidenceEdgeKind,
    ) -> Self {
        let hops = hops.into_iter().map(Into::into).collect::<Vec<_>>();
        if hops.len() < 2 {
            return self;
        }
        let diagnostic_stable_key = self.stable_key();
        match render_policy_path_evidence(
            &diagnostic_stable_key,
            &self.query_digest,
            self.status,
            self.precision,
            &hops,
            edge_kind,
        ) {
            Ok(evidence) => self.structured_evidence = Some(evidence),
            Err(_) => {
                // Engine-produced envelopes must validate; leave unset rather than
                // fabricate an invalid claim.
            }
        }
        self
    }

    pub(crate) fn stable_key(&self) -> String {
        let mut parts = vec![
            format!("query={}", encode_str(self.query.label())),
            format!("query_digest={}", encode_str(&self.query_digest)),
            format!("file={}", encode_str(&self.file)),
            format!("start_line={}", self.range.start_line),
            format!("start_col={}", self.range.start_col),
            format!("end_line={}", self.range.end_line),
            format!("end_col={}", self.range.end_col),
            format!("status={}", policy_status_label(self.status)),
            format!("precision={}", policy_precision_label(self.precision)),
        ];
        for (label, value) in &self.evidence {
            parts.push(format!(
                "evidence:{}={}",
                encode_str(label),
                encode_str(value)
            ));
        }
        let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
        stable_hash(&refs)
    }

    /// Returns the result status.
    pub fn status(&self) -> PolicyStatus {
        self.status
    }

    /// Returns the result precision.
    pub fn precision(&self) -> PolicyPrecision {
        self.precision
    }

    pub(crate) fn file(&self) -> &str {
        &self.file
    }

    #[cfg(test)]
    pub(crate) fn evidence(&self) -> &[(String, String)] {
        &self.evidence
    }

    pub(crate) fn range(&self) -> TextRange {
        self.range
    }

    /// Builds a diagnostic for this violation using the current rule ID.
    pub fn diagnostic(&self, rule_id: &str, message: impl Into<String>) -> Diagnostic {
        let mut diagnostic =
            Diagnostic::error(rule_id.to_string(), self.file.clone(), self.range, message)
                .with_evidence("policy_query", self.query.label())
                .with_evidence("policy_query_version", POLICY_QUERY_VERSION)
                .with_evidence("query_digest", self.query_digest.clone())
                .with_evidence("policy_status", policy_status_label(self.status))
                .with_evidence("policy_precision", policy_precision_label(self.precision));
        for (label, value) in &self.evidence {
            diagnostic = diagnostic.with_evidence(label.clone(), value.clone());
        }
        if let Some(evidence) = self.structured_evidence.clone() {
            diagnostic = diagnostic.with_structured_evidence_v1(evidence);
        }
        diagnostic
    }
}

/// What a policy query decided about one operation.
///
/// Unlike a bare violation list, an outcome distinguishes "proved" from "not
/// decided" from "never examined", so an empty diagnostic list can no longer be
/// read as a proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PolicyOutcome {
    /// The contract was proved for this operation, within the documented scope.
    Covered,
    /// The contract was refuted for this operation.
    Violation,
    /// The operation was examined and the engine could not decide. The result's
    /// `reason` evidence names what was missing.
    Unknown,
    /// The operation was never examined, because the analysis the query needed
    /// did not run. Queries never return this; the run report does.
    NotAnalyzed,
}

/// One operation's outcome under one policy query.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PolicyResult {
    outcome: PolicyOutcome,
    result: PolicyViolation,
}

impl PolicyResult {
    pub(crate) fn new(outcome: PolicyOutcome, result: PolicyViolation) -> Self {
        Self { outcome, result }
    }

    pub(crate) fn stable_key(&self) -> String {
        format!(
            "{}:{}",
            policy_outcome_label(self.outcome),
            self.result.stable_key()
        )
    }

    pub(crate) fn set_status(&mut self, status: PolicyStatus) {
        self.result.set_status(status);
    }

    pub(crate) fn push_evidence(&mut self, label: impl Into<String>, value: impl Into<String>) {
        self.result.push_evidence(label, value);
    }

    #[cfg(test)]
    pub(crate) fn evidence(&self) -> &[(String, String)] {
        self.result.evidence()
    }

    /// Returns what the query decided about this operation.
    pub fn outcome(&self) -> PolicyOutcome {
        self.outcome
    }

    /// Returns the result status.
    pub fn status(&self) -> PolicyStatus {
        self.result.status()
    }

    /// Returns the result precision.
    pub fn precision(&self) -> PolicyPrecision {
        self.result.precision()
    }

    /// Returns the repo-relative path of the operation this result describes.
    pub fn file(&self) -> &str {
        self.result.file()
    }

    /// Returns the source range of the operation this result describes.
    pub fn range(&self) -> TextRange {
        self.result.range()
    }

    /// Builds a diagnostic for this result using the current rule ID.
    ///
    /// Returns `None` for [`PolicyOutcome::Covered`]: a proved operation has
    /// nothing to report as a finding. Report covered operations through the
    /// run summary, or build a rule-owned informational diagnostic from
    /// [`PolicyResult::file`] and [`PolicyResult::range`].
    pub fn diagnostic(&self, rule_id: &str, message: impl Into<String>) -> Option<Diagnostic> {
        match self.outcome {
            PolicyOutcome::Covered => None,
            _ => Some(self.result.diagnostic(rule_id, message)),
        }
    }
}

/// Query for reachable event/call policies.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReachQuery {
    /// Target event that must not be reachable.
    pub target: EventPattern,
    /// Optional root events to constrain reachability.
    pub roots: Vec<EventPattern>,
    /// Whether tests are included in the query.
    pub include_tests: bool,
    /// Maximum call depth explored by the query.
    pub max_depth: usize,
    /// Maximum result paths returned by the query.
    pub max_paths: usize,
    /// Minimum acceptable precision.
    pub minimum_precision: PolicyPrecision,
    /// Minimum acceptable confidence.
    pub minimum_confidence: PolicyConfidence,
}

impl ReachQuery {
    /// Creates a reachability query for a required target event.
    pub fn new(target: EventPattern) -> Self {
        Self {
            target,
            roots: Vec::new(),
            include_tests: false,
            max_depth: 20,
            max_paths: 20,
            minimum_precision: PolicyPrecision::Conservative,
            minimum_confidence: PolicyConfidence::Low,
        }
    }

    pub(crate) fn query_digest(&self) -> String {
        policy_query_digest([
            encode_str(PolicyOperation::CallsForbiddenReachable.label()),
            encode_event_pattern("target", &self.target),
            encode_event_pattern_list("roots", &self.roots),
            format!("include_tests={}", self.include_tests),
            format!("max_depth={}", self.max_depth),
            format!("max_paths={}", self.max_paths),
            format!(
                "minimum_precision={}",
                policy_precision_label(self.minimum_precision)
            ),
            format!(
                "minimum_confidence={}",
                policy_confidence_label(self.minimum_confidence)
            ),
        ])
    }
}

/// Query for missing guard policies.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct GuardQuery {
    /// Sensitive event that requires a guard.
    pub event: EventPattern,
    /// Required guard pattern.
    pub guard: GuardPattern,
    /// Maximum interprocedural depth.
    pub max_depth: usize,
    /// Maximum uncovered paths returned by the query.
    pub max_paths: usize,
    /// Minimum acceptable precision.
    pub minimum_precision: PolicyPrecision,
    /// Report an explicit unknown result when block dominance cannot be
    /// established, instead of treating the missing relation as coverage.
    ///
    /// Defaults to `false`, which keeps the ordering-plus-dominance behavior
    /// that suppresses a result whenever the relation is unavailable.
    /// `guard_outcomes` always reports unestablished dominance and ignores
    /// this field.
    pub report_unknown_coverage: bool,
    /// Require the guard's returned error to be tested, and the error path to
    /// not reach the protected operation, before an operation counts as
    /// covered.
    ///
    /// Only `guard_outcomes` reads this; `missing_guard` keeps its documented
    /// ordering-plus-dominance meaning. Defaults to `false`, which proves
    /// coverage on the ordering-plus-dominance dimension alone.
    pub require_checked_error: bool,
}

impl GuardQuery {
    /// Creates a guard query for an event and required guard.
    pub fn new(event: EventPattern, guard: GuardPattern) -> Self {
        Self {
            event,
            guard,
            max_depth: 4,
            max_paths: 20,
            minimum_precision: PolicyPrecision::Conservative,
            report_unknown_coverage: false,
            require_checked_error: false,
        }
    }

    /// Digest for the guard-outcome form of this query.
    ///
    /// Distinct from [`GuardQuery::query_digest`] because the two queries
    /// answer different questions from the same query object.
    pub(crate) fn outcome_query_digest(&self) -> String {
        policy_query_digest([
            encode_str(PolicyOperation::ControlFlowGuardOutcomes.label()),
            encode_event_pattern("event", &self.event),
            encode_guard_pattern("guard", &self.guard),
            format!("max_depth={}", self.max_depth),
            format!("max_paths={}", self.max_paths),
            format!(
                "minimum_precision={}",
                policy_precision_label(self.minimum_precision)
            ),
            format!("require_checked_error={}", self.require_checked_error),
        ])
    }

    pub(crate) fn query_digest(&self) -> String {
        policy_query_digest([
            encode_str(PolicyOperation::ControlFlowMissingGuard.label()),
            encode_event_pattern("event", &self.event),
            encode_guard_pattern("guard", &self.guard),
            format!("max_depth={}", self.max_depth),
            format!("max_paths={}", self.max_paths),
            format!(
                "minimum_precision={}",
                policy_precision_label(self.minimum_precision)
            ),
            format!("report_unknown_coverage={}", self.report_unknown_coverage),
            format!("require_checked_error={}", self.require_checked_error),
        ])
    }
}

/// Query for resource lifecycle policies.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LifecycleQuery {
    /// Event that acquires or opens a resource.
    pub start: EventPattern,
    /// Event that must close, release, commit, rollback, or otherwise clean up.
    pub cleanup: EventPattern,
    /// Whether cleanup is required on error exits.
    pub require_error_cleanup: bool,
    /// Maximum interprocedural depth.
    pub max_depth: usize,
    /// Maximum uncovered paths returned by the query.
    pub max_paths: usize,
    /// Minimum acceptable precision.
    pub minimum_precision: PolicyPrecision,
    /// Report an explicit unknown result when block post-dominance cannot be
    /// established, instead of treating the missing relation as cleanup.
    ///
    /// Defaults to `false`, which keeps the ordering-plus-post-dominance
    /// behavior that suppresses a result whenever the relation is unavailable.
    pub report_unknown_coverage: bool,
}

impl LifecycleQuery {
    /// Creates a lifecycle query from a start event and required cleanup event.
    pub fn new(start: EventPattern, cleanup: EventPattern) -> Self {
        Self {
            start,
            cleanup,
            require_error_cleanup: true,
            max_depth: 4,
            max_paths: 20,
            minimum_precision: PolicyPrecision::Conservative,
            report_unknown_coverage: false,
        }
    }

    pub(crate) fn query_digest(&self) -> String {
        policy_query_digest([
            encode_str(PolicyOperation::ControlFlowMissingCleanup.label()),
            encode_event_pattern("start", &self.start),
            encode_event_pattern("cleanup", &self.cleanup),
            format!("require_error_cleanup={}", self.require_error_cleanup),
            format!("max_depth={}", self.max_depth),
            format!("max_paths={}", self.max_paths),
            format!(
                "minimum_precision={}",
                policy_precision_label(self.minimum_precision)
            ),
            format!("report_unknown_coverage={}", self.report_unknown_coverage),
        ])
    }
}

/// Query for source-to-sink data-flow policies.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FlowQuery {
    /// Required source pattern.
    pub source: SourcePattern,
    /// Required sink pattern.
    pub sink: SinkPattern,
    /// Optional barrier/sanitizer pattern.
    pub barriers: BarrierPattern,
    /// Maximum interprocedural path depth.
    pub max_depth: usize,
    /// Maximum paths returned by the query.
    pub max_paths: usize,
    /// Minimum acceptable precision.
    pub minimum_precision: PolicyPrecision,
}

impl FlowQuery {
    /// Creates a data-flow query for a required source and sink.
    pub fn new(source: SourcePattern, sink: SinkPattern) -> Self {
        Self {
            source,
            sink,
            barriers: BarrierPattern::none(),
            max_depth: 8,
            max_paths: 20,
            minimum_precision: PolicyPrecision::Conservative,
        }
    }

    pub(crate) fn query_digest(&self) -> String {
        policy_query_digest([
            encode_str(PolicyOperation::DataFlowForbidden.label()),
            encode_source_pattern("source", &self.source),
            encode_sink_pattern("sink", &self.sink),
            encode_barrier_pattern("barriers", &self.barriers),
            format!("max_depth={}", self.max_depth),
            format!("max_paths={}", self.max_paths),
            format!(
                "minimum_precision={}",
                policy_precision_label(self.minimum_precision)
            ),
        ])
    }
}

/// Pattern for semantic events such as calls or field writes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct EventPattern {
    kind: EventPatternKind,
    values: Vec<String>,
}

impl EventPattern {
    /// Matches a call with an exact canonical target name.
    pub fn call(target: impl Into<String>) -> Self {
        Self {
            kind: EventPatternKind::Call,
            values: vec![target.into()],
        }
    }

    /// Matches a write to an exact canonical field or property name.
    pub fn write_field(field: impl Into<String>) -> Self {
        Self {
            kind: EventPatternKind::WriteField,
            values: vec![field.into()],
        }
    }

    pub(crate) fn kind(&self) -> EventPatternKind {
        self.kind
    }

    pub(crate) fn values(&self) -> &[String] {
        &self.values
    }

    pub(crate) fn query_digest(&self) -> String {
        policy_query_digest([
            encode_str(PolicyOperation::EventsMatching.label()),
            encode_event_pattern("event", self),
        ])
    }
}

/// Pattern for data-flow sources.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SourcePattern {
    kind: SourcePatternKind,
    values: Vec<String>,
}

impl SourcePattern {
    /// Matches HTTP request input sources.
    pub fn http_request() -> Self {
        Self {
            kind: SourcePatternKind::HttpRequest,
            values: Vec::new(),
        }
    }

    /// Matches secret-like names from an explicit list of canonical strings.
    pub fn secret_like<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            kind: SourcePatternKind::SecretLike,
            values: collect_strings(names),
        }
    }

    pub(crate) fn kind(&self) -> SourcePatternKind {
        self.kind
    }

    pub(crate) fn values(&self) -> &[String] {
        &self.values
    }
}

/// Pattern for data-flow sinks.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SinkPattern {
    kind: SinkPatternKind,
    values: Vec<String>,
}

impl SinkPattern {
    /// Matches a call sink by exact canonical target name.
    pub fn call(target: impl Into<String>) -> Self {
        Self {
            kind: SinkPatternKind::Call,
            values: vec![target.into()],
        }
    }

    /// Matches common logger sinks.
    pub fn logger() -> Self {
        Self {
            kind: SinkPatternKind::Logger,
            values: Vec::new(),
        }
    }

    pub(crate) fn kind(&self) -> SinkPatternKind {
        self.kind
    }

    pub(crate) fn values(&self) -> &[String] {
        &self.values
    }
}

/// Pattern for guard checks.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct GuardPattern {
    kind: GuardPatternKind,
    values: Vec<String>,
}

impl GuardPattern {
    /// Matches any call in an explicit list of canonical target names.
    pub fn call_any<I, S>(targets: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            kind: GuardPatternKind::CallAny,
            values: collect_strings(targets),
        }
    }

    pub(crate) fn kind(&self) -> GuardPatternKind {
        self.kind
    }

    pub(crate) fn values(&self) -> &[String] {
        &self.values
    }
}

/// Pattern for data-flow barriers or sanitizers.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BarrierPattern {
    kind: BarrierPatternKind,
    values: Vec<String>,
}

impl BarrierPattern {
    /// Matches no barrier.
    pub fn none() -> Self {
        Self {
            kind: BarrierPatternKind::None,
            values: Vec::new(),
        }
    }

    /// Matches any call in an explicit list of canonical target names.
    pub fn call_any<I, S>(targets: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            kind: BarrierPatternKind::CallAny,
            values: collect_strings(targets),
        }
    }

    pub(crate) fn kind(&self) -> BarrierPatternKind {
        self.kind
    }

    pub(crate) fn values(&self) -> &[String] {
        &self.values
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EventPatternKind {
    Call,
    WriteField,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourcePatternKind {
    HttpRequest,
    SecretLike,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SinkPatternKind {
    Call,
    Logger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuardPatternKind {
    CallAny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BarrierPatternKind {
    None,
    CallAny,
}

fn collect_strings<I, S>(values: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    values.into_iter().map(Into::into).collect()
}

fn policy_query_digest<const N: usize>(parts: [String; N]) -> String {
    let mut all_parts = vec![format!("version={}", encode_str(POLICY_QUERY_VERSION))];
    all_parts.extend(parts);
    let refs = all_parts.iter().map(String::as_str).collect::<Vec<_>>();
    stable_hash(&refs)
}

fn encode_event_pattern(label: &str, pattern: &EventPattern) -> String {
    format!(
        "{}=kind:{};values:{}",
        encode_str(label),
        event_pattern_kind_label(pattern.kind),
        encode_string_set(&pattern.values)
    )
}

fn encode_event_pattern_list(label: &str, patterns: &[EventPattern]) -> String {
    let mut encoded = patterns
        .iter()
        .map(|pattern| encode_event_pattern("pattern", pattern))
        .collect::<Vec<_>>();
    encoded.sort();
    format!("{}=[{}]", encode_str(label), encoded.join("|"))
}

fn encode_source_pattern(label: &str, pattern: &SourcePattern) -> String {
    format!(
        "{}=kind:{};values:{}",
        encode_str(label),
        source_pattern_kind_label(pattern.kind),
        encode_string_set(&pattern.values)
    )
}

fn encode_sink_pattern(label: &str, pattern: &SinkPattern) -> String {
    format!(
        "{}=kind:{};values:{}",
        encode_str(label),
        sink_pattern_kind_label(pattern.kind),
        encode_string_set(&pattern.values)
    )
}

fn encode_guard_pattern(label: &str, pattern: &GuardPattern) -> String {
    format!(
        "{}=kind:{};values:{}",
        encode_str(label),
        guard_pattern_kind_label(pattern.kind),
        encode_string_set(&pattern.values)
    )
}

fn encode_barrier_pattern(label: &str, pattern: &BarrierPattern) -> String {
    format!(
        "{}=kind:{};values:{}",
        encode_str(label),
        barrier_pattern_kind_label(pattern.kind),
        encode_string_set(&pattern.values)
    )
}

fn encode_string_set(values: &[String]) -> String {
    let mut values = values
        .iter()
        .map(|value| encode_str(value))
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values.join(",")
}

fn encode_str(value: &str) -> String {
    format!("{}:{value}", value.len())
}

fn event_pattern_kind_label(kind: EventPatternKind) -> &'static str {
    match kind {
        EventPatternKind::Call => "call",
        EventPatternKind::WriteField => "write_field",
    }
}

fn source_pattern_kind_label(kind: SourcePatternKind) -> &'static str {
    match kind {
        SourcePatternKind::HttpRequest => "http_request",
        SourcePatternKind::SecretLike => "secret_like",
    }
}

fn sink_pattern_kind_label(kind: SinkPatternKind) -> &'static str {
    match kind {
        SinkPatternKind::Call => "call",
        SinkPatternKind::Logger => "logger",
    }
}

fn guard_pattern_kind_label(kind: GuardPatternKind) -> &'static str {
    match kind {
        GuardPatternKind::CallAny => "call_any",
    }
}

fn barrier_pattern_kind_label(kind: BarrierPatternKind) -> &'static str {
    match kind {
        BarrierPatternKind::None => "none",
        BarrierPatternKind::CallAny => "call_any",
    }
}

fn policy_outcome_label(outcome: PolicyOutcome) -> &'static str {
    match outcome {
        PolicyOutcome::Covered => "covered",
        PolicyOutcome::Violation => "violation",
        PolicyOutcome::Unknown => "unknown",
        PolicyOutcome::NotAnalyzed => "not_analyzed",
    }
}

fn policy_status_label(status: PolicyStatus) -> &'static str {
    match status {
        PolicyStatus::Exact => "exact",
        PolicyStatus::Heuristic => "heuristic",
        PolicyStatus::Unknown => "unknown",
        PolicyStatus::Unsupported => "unsupported",
        PolicyStatus::BudgetExceeded => "budget_exceeded",
    }
}

fn policy_precision_label(precision: PolicyPrecision) -> &'static str {
    match precision {
        PolicyPrecision::Exact => "exact",
        PolicyPrecision::SetupAware => "setup_aware",
        PolicyPrecision::Syntax => "syntax",
        PolicyPrecision::Conservative => "conservative",
        PolicyPrecision::Heuristic => "heuristic",
        PolicyPrecision::Unknown => "unknown",
    }
}

fn policy_confidence_label(confidence: PolicyConfidence) -> &'static str {
    match confidence {
        PolicyConfidence::High => "high",
        PolicyConfidence::Medium => "medium",
        PolicyConfidence::Low => "low",
    }
}

fn render_policy_path_evidence(
    diagnostic_stable_key: &str,
    query_digest: &str,
    status: PolicyStatus,
    precision: PolicyPrecision,
    hops: &[String],
    edge_kind: PolicyEvidenceEdgeKind,
) -> Result<StructuredEvidenceV1, String> {
    const MAX_PATHS: u64 = 5;
    const MAX_EDGES_PER_PATH: u64 = 96;
    const MAX_UNKNOWNS: u64 = 32;
    const MAX_OMITTED_REGIONS: u64 = 32;

    let max_edges = usize::try_from(MAX_EDGES_PER_PATH).unwrap_or(usize::MAX);
    let hop_limit = max_edges.saturating_add(1);
    let hops = if hops.len() > hop_limit {
        &hops[..hop_limit]
    } else {
        hops
    };
    let total_edges = hops.len().saturating_sub(1) as u64;
    let rendered_edges = total_edges.min(MAX_EDGES_PER_PATH);
    let edges_truncated = total_edges > rendered_edges;

    let nodes = (0..hops.len() as u64).collect::<Vec<_>>();
    let edges = hops
        .windows(2)
        .take(rendered_edges as usize)
        .enumerate()
        .map(|(index, pair)| {
            serde_json::json!({
                "id": index as u64,
                "stable_key": format!("edge:{}:{}:{}", index, pair[0], pair[1]),
                "kind": edge_kind_label(edge_kind),
                "status": evidence_status_label(status),
                "precision": evidence_precision_label(precision),
                "provenance": "Query",
                "validation": "RendererValidated",
                "confidence": evidence_confidence_label(status, precision),
                "summary_stable_key": null,
                "expansion": { "state": "none" },
            })
        })
        .collect::<Vec<_>>();

    let path_key = hops.join(" -> ");
    let replay_key = format!(
        "replay:policy:{}:{}",
        query_digest,
        stable_hash(&[&path_key])
    );
    let value = serde_json::json!({
        "version": 1,
        "bundle": {
            "id": 0,
            "stable_key": format!("bundle:policy:{diagnostic_stable_key}"),
            "diagnostic_stable_key": diagnostic_stable_key,
            "status": evidence_status_label(status),
            "precision": evidence_precision_label(precision),
            "provenance": "Query",
            "validation": "RendererValidated",
            "confidence": evidence_confidence_label(status, precision),
            "replay_key": replay_key,
        },
        "paths": [{
            "id": 0,
            "stable_key": format!("path:policy:{path_key}"),
            "rank": 0,
            "status": evidence_status_label(status),
            "hidden_node_count": 0,
            "nodes": nodes,
            "edges": edges,
            "omitted_regions": [],
            "total_edges": total_edges,
            "rendered_edges": rendered_edges,
            "edges_truncated": edges_truncated,
        }],
        "unknowns": [],
        "omitted_regions": [],
        "limits": {
            "max_paths": MAX_PATHS,
            "max_edges_per_path": MAX_EDGES_PER_PATH,
            "max_unknowns": MAX_UNKNOWNS,
            "max_omitted_regions": MAX_OMITTED_REGIONS,
            "total_paths": 1,
            "rendered_paths": 1,
            "paths_truncated": false,
            "total_unknowns": 0,
            "rendered_unknowns": 0,
            "unknowns_truncated": false,
            "total_omitted_regions": 0,
            "rendered_omitted_regions": 0,
            "omitted_regions_truncated": false,
        },
    });
    StructuredEvidenceV1::try_from_value(value)
}

fn edge_kind_label(kind: PolicyEvidenceEdgeKind) -> &'static str {
    match kind {
        PolicyEvidenceEdgeKind::Call => "Call",
        PolicyEvidenceEdgeKind::Control => "Control",
        PolicyEvidenceEdgeKind::DataTaint => "DataTaint",
    }
}

fn evidence_status_label(status: PolicyStatus) -> &'static str {
    match status {
        PolicyStatus::Exact | PolicyStatus::Heuristic => "Present",
        PolicyStatus::Unknown => "Unknown",
        PolicyStatus::Unsupported => "Unsupported",
        PolicyStatus::BudgetExceeded => "BudgetExceeded",
    }
}

fn evidence_precision_label(precision: PolicyPrecision) -> &'static str {
    match precision {
        PolicyPrecision::Exact => "Exact",
        PolicyPrecision::SetupAware => "SetupAware",
        PolicyPrecision::Syntax => "Syntax",
        PolicyPrecision::Conservative => "Conservative",
        PolicyPrecision::Heuristic => "Heuristic",
        PolicyPrecision::Unknown => "Unknown",
    }
}

fn evidence_confidence_label(status: PolicyStatus, precision: PolicyPrecision) -> &'static str {
    match (status, precision) {
        (PolicyStatus::Exact, PolicyPrecision::Exact | PolicyPrecision::SetupAware) => "High",
        (PolicyStatus::BudgetExceeded | PolicyStatus::Unsupported | PolicyStatus::Unknown, _) => {
            "Low"
        }
        _ => "Medium",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_query_digest_is_stable_for_same_query() {
        let mut left = FlowQuery::new(
            SourcePattern::secret_like(["token", "password"]),
            SinkPattern::logger(),
        );
        left.barriers = BarrierPattern::call_any(["redact", "mask_secret"]);

        let mut right = FlowQuery::new(
            SourcePattern::secret_like(["password", "token"]),
            SinkPattern::logger(),
        );
        right.barriers = BarrierPattern::call_any(["mask_secret", "redact"]);

        assert_eq!(left.query_digest(), right.query_digest());
    }

    #[test]
    fn policy_query_digest_changes_when_options_change() {
        let baseline = ReachQuery::new(EventPattern::call("dangerous"));
        let mut changed = baseline.clone();
        changed.max_depth += 1;

        assert_ne!(baseline.query_digest(), changed.query_digest());
    }

    #[test]
    fn policy_path_evidence_is_replayable_and_bounded() {
        let violation = PolicyViolation::new(
            PolicyOperation::CallsForbiddenReachable,
            "digest",
            "src/main.go",
            TextRange::point(1, 1),
            PolicyStatus::Exact,
            PolicyPrecision::SetupAware,
            vec![("path".into(), "main -> helper -> dangerous".into())],
        )
        .with_path_evidence(
            ["main", "helper", "dangerous"],
            PolicyEvidenceEdgeKind::Call,
        );
        let diagnostic = violation.diagnostic("local/reach", "reachable");
        let evidence = diagnostic
            .evidence_v1
            .as_ref()
            .expect("structured evidence");
        let value = evidence.as_value();
        assert_eq!(value["version"], 1);
        assert!(value["bundle"]["replay_key"].as_str().is_some());
        assert!(
            value["paths"]
                .as_array()
                .map(|paths| !paths.is_empty())
                .unwrap_or(false)
        );
        assert!(value["unknowns"].as_array().is_some());
        assert!(value["limits"].as_object().is_some());
    }
}
