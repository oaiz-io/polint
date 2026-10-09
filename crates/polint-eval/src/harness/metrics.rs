use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::eval::matcher::{MatchItemKind, MatchOutcome};
use crate::eval::model::{ExpectedItem, ObservedItem};
use crate::eval::report::{
    CategorizedFailureSection, GraphMetricSection, JellyOracleCoverageSection, JellyUnmatchedSpan,
    MatchSummary, MetricSections, MetricSummary, PathMetricSection, PerformanceMetricSection,
    ScannerMetricSection, SolverMetricSection, UnknownMetricSection,
};
#[cfg(test)]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) struct ComputedMetrics {
    pub(crate) true_positives: u64,
    pub(crate) false_positives: u64,
    pub(crate) false_negatives: u64,
    pub(crate) true_negatives: u64,
    pub(crate) unconfirmed: u64,
    pub(crate) false_positive_trap_hits: u64,
    pub(crate) forbidden_hits: u64,
    pub(crate) unknown_count: u64,
    pub(crate) facts_present: u64,
    pub(crate) facts_accepted: u64,
    pub(crate) facts_rejected: u64,
    pub(crate) graph_edges_expected: u64,
    pub(crate) graph_edges_observed: u64,
    pub(crate) graph_edges_unconfirmed: u64,
    pub(crate) paths_expected: u64,
    pub(crate) paths_observed: u64,
    pub(crate) paths_unconfirmed: u64,
    pub(crate) runtime_budget_passed: u64,
    pub(crate) runtime_budget_failed: u64,
    pub(crate) unknown_by_status: BTreeMap<String, u64>,
    pub(crate) precision: Option<f64>,
    pub(crate) recall: Option<f64>,
    pub(crate) f1: Option<f64>,
    pub(crate) f0_5: Option<f64>,
    pub(crate) f2: Option<f64>,
    pub(crate) f3: Option<f64>,
    pub(crate) false_positive_rate: Option<f64>,
}

impl From<ComputedMetrics> for MetricSummary {
    fn from(metrics: ComputedMetrics) -> Self {
        let sections = MetricSections {
            scanner: ScannerMetricSection {
                true_positives: metrics.true_positives,
                false_positives: metrics.false_positives,
                false_negatives: metrics.false_negatives,
                true_negatives: metrics.true_negatives,
                precision: metrics.precision,
                recall: metrics.recall,
                f1: metrics.f1,
                f0_5: metrics.f0_5,
                f2: metrics.f2,
                f3: metrics.f3,
                false_positive_rate: metrics.false_positive_rate,
            },
            graph: GraphMetricSection {
                edges_expected: metrics.graph_edges_expected,
                edges_observed: metrics.graph_edges_observed,
                edges_unconfirmed: metrics.graph_edges_unconfirmed,
            },
            paths: PathMetricSection {
                paths_expected: metrics.paths_expected,
                paths_observed: metrics.paths_observed,
                paths_unconfirmed: metrics.paths_unconfirmed,
            },
            unknowns: UnknownMetricSection {
                total: metrics.unknown_count,
                by_status: metrics.unknown_by_status.clone(),
            },
            performance: PerformanceMetricSection {
                runtime_budget_passed: metrics.runtime_budget_passed,
                runtime_budget_failed: metrics.runtime_budget_failed,
                cache_hits: 0,
                cache_misses: 0,
            },
            suite_native: std::collections::BTreeMap::new(),
            per_language_deltas: Vec::new(),
            adaptation: None,
            jelly_oracle_coverage: JellyOracleCoverageSection::default(),
            categorized_failures: CategorizedFailureSection::default(),
            // RESERVED (D-23): defaulted to step_count = 0 / empty reasons in
            // zero so observed/report JSON always surfaces the `solver` section
            // and the N=10 determinism gate stays byte-stable. Real values can
            // populate the existing shape without changing it.
            solver: SolverMetricSection::default(),
        };
        Self {
            true_positives: metrics.true_positives,
            false_positives: metrics.false_positives,
            false_negatives: metrics.false_negatives,
            true_negatives: metrics.true_negatives,
            unconfirmed: metrics.unconfirmed,
            false_positive_trap_hits: metrics.false_positive_trap_hits,
            forbidden_hits: metrics.forbidden_hits,
            unknown_count: metrics.unknown_count,
            facts_present: metrics.facts_present,
            facts_accepted: metrics.facts_accepted,
            facts_rejected: metrics.facts_rejected,
            graph_edges_expected: metrics.graph_edges_expected,
            graph_edges_observed: metrics.graph_edges_observed,
            graph_edges_unconfirmed: metrics.graph_edges_unconfirmed,
            paths_expected: metrics.paths_expected,
            paths_observed: metrics.paths_observed,
            paths_unconfirmed: metrics.paths_unconfirmed,
            runtime_budget_passed: metrics.runtime_budget_passed,
            runtime_budget_failed: metrics.runtime_budget_failed,
            precision: metrics.precision,
            recall: metrics.recall,
            f1: metrics.f1,
            f2: metrics.f2,
            f3: metrics.f3,
            false_positive_rate: metrics.false_positive_rate,
            sections,
        }
    }
}

pub(crate) fn compute_metrics(matches: &[MatchSummary]) -> ComputedMetrics {
    let mut metrics = ComputedMetrics {
        true_positives: 0,
        false_positives: 0,
        false_negatives: 0,
        true_negatives: 0,
        unconfirmed: 0,
        false_positive_trap_hits: 0,
        forbidden_hits: 0,
        unknown_count: 0,
        facts_present: 0,
        facts_accepted: 0,
        facts_rejected: 0,
        graph_edges_expected: 0,
        graph_edges_observed: 0,
        graph_edges_unconfirmed: 0,
        paths_expected: 0,
        paths_observed: 0,
        paths_unconfirmed: 0,
        runtime_budget_passed: 0,
        runtime_budget_failed: 0,
        unknown_by_status: BTreeMap::new(),
        precision: None,
        recall: None,
        f1: None,
        f0_5: None,
        f2: None,
        f3: None,
        false_positive_rate: None,
    };

    for summary in matches {
        let score_bearing = score_bearing_item_kind(summary.item_kind);
        match summary.outcome {
            MatchOutcome::TruePositive if score_bearing => metrics.true_positives += 1,
            MatchOutcome::FalsePositive if score_bearing => metrics.false_positives += 1,
            MatchOutcome::FalseNegative if score_bearing => metrics.false_negatives += 1,
            MatchOutcome::TrueNegative if score_bearing => metrics.true_negatives += 1,
            MatchOutcome::TruePositive
            | MatchOutcome::FalsePositive
            | MatchOutcome::FalseNegative
            | MatchOutcome::TrueNegative => {}
            MatchOutcome::Unconfirmed => metrics.unconfirmed += 1,
            MatchOutcome::ForbiddenHit => metrics.forbidden_hits += 1,
            MatchOutcome::TrapHit => metrics.false_positive_trap_hits += 1,
            MatchOutcome::Unknown => metrics.unknown_count += 1,
            MatchOutcome::RuntimeBudgetPassed => metrics.runtime_budget_passed += 1,
            MatchOutcome::RuntimeBudgetFailed => metrics.runtime_budget_failed += 1,
        }
        if summary.outcome == MatchOutcome::Unknown
            && let Some(status) = summary.observed_status
        {
            *metrics
                .unknown_by_status
                .entry(status_label(status).to_string())
                .or_insert(0) += 1;
        }

        match summary.item_kind {
            MatchItemKind::GraphEdge => {
                if summary.expected_key.is_some() {
                    metrics.graph_edges_expected += 1;
                }
                if summary.observed_key.is_some() {
                    metrics.graph_edges_observed += 1;
                }
                if summary.outcome == MatchOutcome::Unconfirmed {
                    metrics.graph_edges_unconfirmed += 1;
                }
            }
            MatchItemKind::Fact => match summary.observed_status {
                Some(
                    crate::eval::model::ObservedStatus::Present
                    | crate::eval::model::ObservedStatus::Resolved,
                ) => metrics.facts_present += 1,
                Some(crate::eval::model::ObservedStatus::Accepted) => metrics.facts_accepted += 1,
                Some(crate::eval::model::ObservedStatus::Rejected) => metrics.facts_rejected += 1,
                Some(
                    crate::eval::model::ObservedStatus::Unknown
                    | crate::eval::model::ObservedStatus::Partial
                    | crate::eval::model::ObservedStatus::Top
                    | crate::eval::model::ObservedStatus::Unresolved
                    | crate::eval::model::ObservedStatus::Ambiguous
                    | crate::eval::model::ObservedStatus::Dynamic
                    | crate::eval::model::ObservedStatus::SetupMissing
                    | crate::eval::model::ObservedStatus::MissingLockfile
                    | crate::eval::model::ObservedStatus::Unsupported
                    | crate::eval::model::ObservedStatus::External
                    | crate::eval::model::ObservedStatus::Cycle
                    | crate::eval::model::ObservedStatus::Generated
                    | crate::eval::model::ObservedStatus::Undeclared
                    | crate::eval::model::ObservedStatus::OutsideWorkspace
                    | crate::eval::model::ObservedStatus::BudgetExceeded,
                )
                | None => {}
            },
            MatchItemKind::Path => {
                if summary.expected_key.is_some() {
                    metrics.paths_expected += 1;
                }
                if summary.observed_key.is_some() {
                    metrics.paths_observed += 1;
                }
                if summary.outcome == MatchOutcome::Unconfirmed {
                    metrics.paths_unconfirmed += 1;
                }
            }
            MatchItemKind::Diagnostic | MatchItemKind::Invariant | MatchItemKind::RuntimeBudget => {
            }
        }
    }

    metrics.precision = ratio(
        metrics.true_positives,
        metrics.true_positives + metrics.false_positives,
    );
    metrics.recall = ratio(
        metrics.true_positives,
        metrics.true_positives + metrics.false_negatives,
    );
    metrics.f1 = f_score(
        1.0,
        metrics.true_positives,
        metrics.false_positives,
        metrics.false_negatives,
    );
    metrics.f0_5 = f_score(
        0.5,
        metrics.true_positives,
        metrics.false_positives,
        metrics.false_negatives,
    );
    metrics.f2 = f_score(
        2.0,
        metrics.true_positives,
        metrics.false_positives,
        metrics.false_negatives,
    );
    metrics.f3 = f_score(
        3.0,
        metrics.true_positives,
        metrics.false_positives,
        metrics.false_negatives,
    );
    metrics.false_positive_rate = ratio(
        metrics.false_positives,
        metrics.false_positives + metrics.true_negatives,
    );

    metrics
}

fn score_bearing_item_kind(kind: MatchItemKind) -> bool {
    matches!(
        kind,
        MatchItemKind::Diagnostic
            | MatchItemKind::Fact
            | MatchItemKind::GraphEdge
            | MatchItemKind::Path
    )
}

/// Computes deterministic Jelly oracle-span coverage (D-20, D-21).
///
/// Every distinct Jelly graph-edge endpoint span in the `expected` (oracle) set
/// is checked against the `observed` (renderer-produced) endpoint span set. A
/// span counts as `matched` when at least one observed Jelly graph edge renders
/// that exact span string; unmatched oracle spans are surfaced individually for
/// debuggability. `ratio = matched / total`, or `1.0` when there are no oracle
/// spans (an empty oracle is vacuously fully covered, so it never drags a
/// suite-wide ratio below the 0.99 threshold).
///
/// The renderer-produced observed spans are the single source of truth (D-05):
/// the eval adapter populates `observed` via
/// `analysis::identity::render::jelly_span::render`, so this function never
/// re-renders — it counts agreement between the oracle and the rendered spans.
pub(crate) fn jelly_oracle_coverage(
    expected: &[ExpectedItem],
    observed: &[ObservedItem],
) -> JellyOracleCoverageSection {
    use std::collections::{BTreeMap, BTreeSet};

    let observed_spans: BTreeSet<&str> = observed
        .iter()
        .filter_map(jelly_graph_edge_observed)
        .flat_map(|edge| [edge.0, edge.1])
        .collect();

    // Deterministic distinct oracle spans, keyed by span string -> source file
    // (the file portion before the first `:` of the Jelly span shape).
    let mut oracle_spans: BTreeMap<&str, &str> = BTreeMap::new();
    for edge in expected.iter().filter_map(jelly_graph_edge_expected) {
        for span in [edge.0, edge.1] {
            oracle_spans
                .entry(span)
                .or_insert_with(|| jelly_span_file(span));
        }
    }

    let total = oracle_spans.len() as u32;
    let mut matched = 0u32;
    let mut unmatched = Vec::new();
    for (span, file) in &oracle_spans {
        if observed_spans.contains(span) {
            matched += 1;
        } else {
            unmatched.push(JellyUnmatchedSpan {
                file: (*file).to_string(),
                span: (*span).to_string(),
                reason: "no identity record renders this span".to_string(),
            });
        }
    }

    let ratio = if total == 0 {
        1.0
    } else {
        f64::from(matched) / f64::from(total)
    };

    JellyOracleCoverageSection {
        matched,
        total,
        ratio,
        unmatched,
    }
}

/// Aggregates Jelly oracle-span coverage across an entire suite run (D-20,
/// D-22). The per-platform aggregate is the deterministic union of every case's
/// oracle spans matched against every case's observed rendered spans; there is
/// no cross-platform averaging — each platform's CI run computes this
/// independently and asserts the threshold on its own host.
pub(crate) fn jelly_oracle_coverage_for_cases(
    cases: &[crate::eval::report::CaseResult],
) -> JellyOracleCoverageSection {
    let expected = cases
        .iter()
        .flat_map(|case| case.expected.iter().cloned())
        .collect::<Vec<_>>();
    let observed = cases
        .iter()
        .flat_map(|case| case.observed.iter().cloned())
        .collect::<Vec<_>>();
    jelly_oracle_coverage(&expected, &observed)
}

/// Projects the live analysis facts into the closed [`CategorizedFailureSection`]
/// counter map (D-15, D-16; Plan 42-03).
///
/// Each failing fact is categorized at most once via direct dispatch — the pass
/// is O(n) over the fact tables, no nested loops (threat T-42-03-06):
///
/// - every `unresolved_calls` fact carries an `UnresolvedCallReason` ->
///   [`category_for_unresolved`];
/// - every `call_targets` fact with a non-`Resolved`/non-`Ambiguous`
///   `CallTargetStatus` -> [`category_for_unsupported`] (`None` skips success
///   and ambiguity, which are not failures);
/// - every callsite identity record whose `(file_id, span)` overlaps an oracle
///   entry's span without matching its container/digest is a wrong-identity
///   event -> [`category_for_wrong_identity`].
///
/// `oracle_overlap` is computed here by comparing observed callsite spans to the
/// `oracle_callsite_spans` set the caller derives from the expected (oracle)
/// items. When that set is empty (no oracle, e.g. native fixtures) no
/// wrong-identity events fire — a callsite with no overlapping oracle entry is a
/// miss, not a wrong-identity (D-16).
pub(crate) fn categorized_failures_from_db(
    db: &crate::core::AnalysisDb,
    oracle_callsite_spans: &std::collections::BTreeSet<(u32, u32, u32)>,
) -> CategorizedFailureSection {
    use crate::analysis::calls::facts::CallTargetStatus;
    use crate::analysis::identity::categorize::{
        category_for_unresolved, category_for_unsupported, category_for_wrong_identity,
    };
    use crate::analysis::identity::facts::IdentityKind;

    let mut section = CategorizedFailureSection::default();

    for unresolved in db.unresolved_calls() {
        section.record_category(category_for_unresolved(unresolved.reason));
    }

    for target in db.call_targets() {
        if !matches!(
            target.status,
            CallTargetStatus::Resolved | CallTargetStatus::Ambiguous
        ) && let Some(category) = category_for_unsupported(target.status)
        {
            section.record_category(category);
        }
    }

    if !oracle_callsite_spans.is_empty() {
        for record in db.identity_records() {
            if record.kind != IdentityKind::Callsite {
                continue;
            }
            let key = (
                record.file_id.0,
                record.span.start_byte,
                record.span.end_byte,
            );
            let oracle_overlap = oracle_callsite_spans.contains(&key);
            if let Some(category) = category_for_wrong_identity(record, oracle_overlap) {
                section.record_category(category);
            }
        }
    }

    section
}

/// Reconstructs the [`CategorizedFailureSection`] from the per-category observed
/// invariants emitted by `eval::observed::identity_categorized_failure_invariants`.
///
/// The observation layer computes the section from the live `AnalysisDb`; this
/// rehydrates it into the report shape so the section appears in the report JSON
/// for a native fixture. Missing or unparsable invariants default to `0`.
pub(crate) fn categorized_failures_from_observed(
    observed: &[ObservedItem],
) -> CategorizedFailureSection {
    let mut section = CategorizedFailureSection::default();
    for item in observed {
        let ObservedItem::Invariant(invariant) = item else {
            continue;
        };
        let Ok(value) = invariant.value.parse::<u32>() else {
            continue;
        };
        match invariant.name.as_str() {
            "identity.categorized_failures.wrong_identity" => section.wrong_identity = value,
            "identity.categorized_failures.unsupported_edge" => section.unsupported_edge = value,
            "identity.categorized_failures.unresolved_edge" => section.unresolved_edge = value,
            "identity.categorized_failures.package_load_limitation" => {
                section.package_load_limitation = value;
            }
            "identity.categorized_failures.model_missing" => section.model_missing = value,
            _ => {}
        }
    }
    section
}

fn jelly_graph_edge_expected(item: &ExpectedItem) -> Option<(&str, &str)> {
    match item {
        ExpectedItem::GraphEdge(edge) if edge.graph.starts_with("jelly.call_graph.") => {
            Some((edge.from.as_str(), edge.to.as_str()))
        }
        _ => None,
    }
}

fn jelly_graph_edge_observed(item: &ObservedItem) -> Option<(&str, &str)> {
    match item {
        ObservedItem::GraphEdge(edge) if edge.graph.starts_with("jelly.call_graph.") => {
            Some((edge.from.as_str(), edge.to.as_str()))
        }
        _ => None,
    }
}

/// Extracts the source-file portion of a Jelly span string
/// (`file:start_line:start_col:end_line:end_col`).
///
/// WR-05 path invariant: this `rsplitn(5, ':')` split from the right is only correct
/// because Jelly spans are normalized to forward-slash, repo-relative paths with NO
/// colon in the file portion. The renderer enforces exactly this — see
/// `eval::observed::identity_render_invariants`, which asserts the rendered Jelly
/// path is not absolute and never contains `:\` (no Windows drive letters or
/// backslash separators). If that invariant ever regresses, a path containing a `:`
/// would push the line/col tail past five pieces and mis-attribute the `file`
/// segment, so the cheap debug assertion below pins the contract at the use site.
fn jelly_span_file(span: &str) -> &str {
    let file = span.rsplitn(5, ':').last().unwrap_or(span);
    debug_assert!(
        !file.contains(":\\"),
        "Jelly span file segment must be a colon-free forward-slash path \
         (identity_render_invariants), got: {file:?} from span {span:?}"
    );
    file
}

fn status_label(status: crate::eval::model::ObservedStatus) -> &'static str {
    match status {
        crate::eval::model::ObservedStatus::Present => "present",
        crate::eval::model::ObservedStatus::Resolved => "resolved",
        crate::eval::model::ObservedStatus::Partial => "partial",
        crate::eval::model::ObservedStatus::Top => "top",
        crate::eval::model::ObservedStatus::Unknown => "unknown",
        crate::eval::model::ObservedStatus::Unresolved => "unresolved",
        crate::eval::model::ObservedStatus::Ambiguous => "ambiguous",
        crate::eval::model::ObservedStatus::Dynamic => "dynamic",
        crate::eval::model::ObservedStatus::SetupMissing => "setup_missing",
        crate::eval::model::ObservedStatus::MissingLockfile => "missing_lockfile",
        crate::eval::model::ObservedStatus::Unsupported => "unsupported",
        crate::eval::model::ObservedStatus::External => "external",
        crate::eval::model::ObservedStatus::Cycle => "cycle",
        crate::eval::model::ObservedStatus::Generated => "generated",
        crate::eval::model::ObservedStatus::Undeclared => "undeclared",
        crate::eval::model::ObservedStatus::OutsideWorkspace => "outside_workspace",
        crate::eval::model::ObservedStatus::BudgetExceeded => "budget_exceeded",
        crate::eval::model::ObservedStatus::Rejected => "rejected",
        crate::eval::model::ObservedStatus::Accepted => "accepted",
    }
}

fn ratio(numerator: u64, denominator: u64) -> Option<f64> {
    if denominator == 0 {
        None
    } else {
        Some(numerator as f64 / denominator as f64)
    }
}

fn f_score(
    beta: f64,
    true_positives: u64,
    false_positives: u64,
    false_negatives: u64,
) -> Option<f64> {
    let beta_squared = beta * beta;
    let true_positives = true_positives as f64;
    let denominator = (1.0 + beta_squared) * true_positives
        + beta_squared * false_negatives as f64
        + false_positives as f64;
    if denominator == 0.0 {
        None
    } else {
        Some((1.0 + beta_squared) * true_positives / denominator)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::matcher::{MatchItemKind, MatchOutcome, MatcherConfig, match_case};
    use crate::eval::model::{
        AssertionMode, ExpectedItem, ExpectedRuntimeBudget, ObservedItem, ObservedRuntimeBudget,
    };
    use crate::eval::report::{MatchSummary, MetricSummary};

    #[test]
    fn eval_metrics_count_mixed_match_outcomes() {
        let metrics = compute_metrics(&[
            summary(
                MatchOutcome::TruePositive,
                MatchItemKind::Diagnostic,
                true,
                true,
            ),
            summary(
                MatchOutcome::FalsePositive,
                MatchItemKind::Diagnostic,
                false,
                true,
            ),
            summary(
                MatchOutcome::FalseNegative,
                MatchItemKind::Fact,
                true,
                false,
            ),
            summary(MatchOutcome::TrueNegative, MatchItemKind::Fact, true, false),
            summary(
                MatchOutcome::Unconfirmed,
                MatchItemKind::GraphEdge,
                false,
                true,
            ),
            summary(MatchOutcome::TrapHit, MatchItemKind::Diagnostic, true, true),
            summary(MatchOutcome::ForbiddenHit, MatchItemKind::Fact, true, true),
            summary(MatchOutcome::Unknown, MatchItemKind::Path, true, true),
            summary(
                MatchOutcome::RuntimeBudgetPassed,
                MatchItemKind::RuntimeBudget,
                true,
                true,
            ),
            summary(
                MatchOutcome::RuntimeBudgetFailed,
                MatchItemKind::RuntimeBudget,
                true,
                true,
            ),
            summary(
                MatchOutcome::TruePositive,
                MatchItemKind::GraphEdge,
                true,
                true,
            ),
            summary(
                MatchOutcome::FalseNegative,
                MatchItemKind::Path,
                true,
                false,
            ),
            summary(MatchOutcome::Unconfirmed, MatchItemKind::Path, false, true),
        ]);

        assert_eq!(
            metrics,
            ComputedMetrics {
                true_positives: 2,
                false_positives: 1,
                false_negatives: 2,
                true_negatives: 1,
                unconfirmed: 2,
                false_positive_trap_hits: 1,
                forbidden_hits: 1,
                unknown_count: 1,
                facts_present: 1,
                facts_accepted: 0,
                facts_rejected: 0,
                graph_edges_expected: 1,
                graph_edges_observed: 2,
                graph_edges_unconfirmed: 1,
                paths_expected: 2,
                paths_observed: 2,
                paths_unconfirmed: 1,
                runtime_budget_passed: 1,
                runtime_budget_failed: 1,
                unknown_by_status: [("present".to_string(), 1)].into_iter().collect(),
                precision: Some(2.0 / 3.0),
                recall: Some(0.5),
                f1: Some(4.0 / 7.0),
                f0_5: Some(5.0 / 8.0),
                f2: Some(10.0 / 19.0),
                f3: Some(20.0 / 39.0),
                false_positive_rate: Some(0.5),
            }
        );
    }

    #[test]
    fn eval_metrics_do_not_score_unmatched_invariants_as_false_positives() {
        let metrics = compute_metrics(&[
            summary(
                MatchOutcome::TruePositive,
                MatchItemKind::GraphEdge,
                true,
                true,
            ),
            summary(
                MatchOutcome::FalsePositive,
                MatchItemKind::Invariant,
                false,
                true,
            ),
        ]);

        assert_eq!(metrics.true_positives, 1);
        assert_eq!(metrics.false_positives, 0);
        assert_eq!(metrics.false_negatives, 0);
        assert_eq!(metrics.precision, Some(1.0));
        assert_eq!(metrics.recall, Some(1.0));
        assert_eq!(metrics.f1, Some(1.0));
        assert_eq!(metrics.graph_edges_expected, 1);
        assert_eq!(metrics.graph_edges_observed, 1);
    }

    #[test]
    fn eval_metrics_zero_denominators_are_none() {
        let metrics = compute_metrics(&[]);

        assert_eq!(metrics.precision, None);
        assert_eq!(metrics.recall, None);
        assert_eq!(metrics.f1, None);
        assert_eq!(metrics.f0_5, None);
        assert_eq!(metrics.f2, None);
        assert_eq!(metrics.f3, None);
        assert_eq!(metrics.false_positive_rate, None);
    }

    #[test]
    fn eval_metrics_serialization_is_byte_stable_with_deterministic_field_order() {
        let metrics = compute_metrics(&[
            summary(
                MatchOutcome::TruePositive,
                MatchItemKind::Diagnostic,
                true,
                true,
            ),
            summary(
                MatchOutcome::FalsePositive,
                MatchItemKind::Diagnostic,
                false,
                true,
            ),
        ]);

        let left = serde_json::to_string(&metrics).unwrap();
        let right = serde_json::to_string(&metrics).unwrap();

        assert_eq!(left, right);
        assert!(left.starts_with("{\"true_positives\":"));
        assert!(
            left.find("\"false_positives\"").unwrap() < left.find("\"false_negatives\"").unwrap()
        );
        assert!(
            left.find("\"runtime_budget_failed\"").unwrap() < left.find("\"precision\"").unwrap()
        );
    }

    #[test]
    fn eval_metrics_runtime_budget_counts_follow_observed_budget_passed() {
        let matches = match_case(
            &[
                ExpectedItem::RuntimeBudget(ExpectedRuntimeBudget {
                    name: "fast-ci".to_string(),
                    max_runtime_ms: 500,
                    mode: AssertionMode::Exact,
                }),
                ExpectedItem::RuntimeBudget(ExpectedRuntimeBudget {
                    name: "slow-ci".to_string(),
                    max_runtime_ms: 800,
                    mode: AssertionMode::Exact,
                }),
            ],
            &[
                ObservedItem::RuntimeBudget(ObservedRuntimeBudget {
                    name: "fast-ci".to_string(),
                    budget_passed: true,
                    observed_runtime_ms: Some(900),
                    peak_rss_bytes: None,
                }),
                ObservedItem::RuntimeBudget(ObservedRuntimeBudget {
                    name: "slow-ci".to_string(),
                    budget_passed: false,
                    observed_runtime_ms: Some(400),
                    peak_rss_bytes: None,
                }),
            ],
            MatcherConfig::default(),
        );

        let metrics = compute_metrics(&matches);

        assert_eq!(metrics.runtime_budget_passed, 1);
        assert_eq!(metrics.runtime_budget_failed, 1);
    }

    #[test]
    fn eval_metrics_compute_precision_weighted_f0_5() {
        let metrics = compute_metrics(&[
            summary(
                MatchOutcome::TruePositive,
                MatchItemKind::Diagnostic,
                true,
                true,
            ),
            summary(
                MatchOutcome::TruePositive,
                MatchItemKind::Diagnostic,
                true,
                true,
            ),
            summary(
                MatchOutcome::TruePositive,
                MatchItemKind::Diagnostic,
                true,
                true,
            ),
            summary(
                MatchOutcome::FalsePositive,
                MatchItemKind::Diagnostic,
                false,
                true,
            ),
            summary(
                MatchOutcome::FalseNegative,
                MatchItemKind::Diagnostic,
                true,
                false,
            ),
            summary(
                MatchOutcome::FalseNegative,
                MatchItemKind::Diagnostic,
                true,
                false,
            ),
        ]);

        assert_eq!(metrics.precision, Some(0.75));
        assert_eq!(metrics.recall, Some(0.6));
        assert_eq!(metrics.f0_5, Some(5.0 / 7.0));
    }

    #[test]
    fn eval_metrics_count_fact_statuses_separately() {
        let metrics = compute_metrics(&[
            summary_with_status(
                MatchOutcome::TruePositive,
                MatchItemKind::Fact,
                crate::eval::model::ObservedStatus::Present,
            ),
            summary_with_status(
                MatchOutcome::TruePositive,
                MatchItemKind::Fact,
                crate::eval::model::ObservedStatus::Accepted,
            ),
            summary_with_status(
                MatchOutcome::TruePositive,
                MatchItemKind::Fact,
                crate::eval::model::ObservedStatus::Rejected,
            ),
        ]);

        assert_eq!(metrics.facts_present, 1);
        assert_eq!(metrics.facts_accepted, 1);
        assert_eq!(metrics.facts_rejected, 1);
    }

    #[test]
    fn eval_metrics_convert_into_report_metric_summary() {
        let metrics = compute_metrics(&[
            summary(
                MatchOutcome::TruePositive,
                MatchItemKind::Diagnostic,
                true,
                true,
            ),
            summary(MatchOutcome::TrapHit, MatchItemKind::Diagnostic, true, true),
            summary(
                MatchOutcome::RuntimeBudgetFailed,
                MatchItemKind::RuntimeBudget,
                true,
                true,
            ),
        ]);

        let summary: MetricSummary = metrics.into();

        assert_eq!(summary.true_positives, 1);
        assert_eq!(summary.false_positive_trap_hits, 1);
        assert_eq!(summary.runtime_budget_failed, 1);
        assert_eq!(summary.sections.scanner.true_positives, 1);
        assert_eq!(summary.sections.scanner.f0_5, Some(1.0));
        assert_eq!(summary.sections.performance.runtime_budget_failed, 1);
    }

    #[test]
    fn eval_metrics_sections_include_graph_path_unknown_and_suite_native_slots() {
        let mut metrics = compute_metrics(&[
            summary(
                MatchOutcome::TruePositive,
                MatchItemKind::GraphEdge,
                true,
                true,
            ),
            summary(MatchOutcome::Unconfirmed, MatchItemKind::Path, true, true),
            summary_with_status(
                MatchOutcome::Unknown,
                MatchItemKind::Fact,
                crate::eval::model::ObservedStatus::SetupMissing,
            ),
        ]);
        metrics.unknown_by_status.insert("unknown".to_string(), 2);

        let mut summary: MetricSummary = metrics.into();
        summary
            .sections
            .suite_native
            .insert("secbench_js.test_file_count".to_string(), 704.0);

        assert_eq!(summary.sections.graph.edges_expected, 1);
        assert_eq!(summary.sections.graph.edges_observed, 1);
        assert_eq!(summary.sections.paths.paths_unconfirmed, 1);
        assert_eq!(summary.sections.unknowns.total, 1);
        assert_eq!(
            summary.sections.unknowns.by_status.get("setup_missing"),
            Some(&1)
        );
        assert_eq!(
            summary
                .sections
                .suite_native
                .get("secbench_js.test_file_count"),
            Some(&704.0)
        );
    }

    // -- Plan 42-03 categorized-failures pass over synthetic live facts --------
    //
    // These exercise `categorized_failures_from_db` end-to-end (the real
    // projection logic, not just `record_category`) for the three categories the
    // syntactic frontend does not naturally emit on native fixtures:
    // `wrong_identity` (needs oracle span overlap, D-16), `package_load_limitation`
    // (needs CallTargetStatus::SetupMissing), and `model_missing` (needs
    // CallTargetStatus::Rejected). Together with the categorized-failures fixture
    // (which proves `unsupported_edge` + `unresolved_edge` from real source) all
    // FIVE counters are proven non-zero across the test corpus (BLOCKER #4, D-15).

    use crate::analysis::calls::facts::{
        CallAlgorithm, CallCallee, CallEdgeKind, CallPrecision, CallProvenance, CallSiteFact,
        CallSyntaxKind, CallTargetFact, CallTargetStatus, UnresolvedCallFact, UnresolvedCallReason,
    };
    use crate::analysis::identity::facts::{
        IdentityKind, IdentityRecord, IdentityRecordId, LanguageTag, compute_signature_digest,
    };
    use crate::analysis::ids::{CallSiteId, CallTargetId, MirBodyId, MirOpId};
    use crate::core::{AnalysisDb, FileId, FunctionFact, FunctionId, Language, Span};

    fn db_with_one_site() -> (AnalysisDb, FileId, CallSiteFact) {
        let mut db = AnalysisDb::new();
        let file = db.add_file(
            std::path::PathBuf::from("src/main.go"),
            "src/main.go".to_string(),
            "package main\nfunc main() {}\n".to_string(),
        );
        db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "main".to_string(),
            Span::point(file, 2, 1),
            Language::Go,
            false,
            true,
            1,
            Vec::new(),
        ));
        let site = CallSiteFact {
            in_throw: false,
            id: CallSiteId(0),
            language: Language::Go,
            file,
            caller: FunctionId::from_raw(0),
            owner_symbol: None,
            body: MirBodyId(0),
            operation: MirOpId(0),
            span: Span::point(file, 2, 5),
            kind: CallSyntaxKind::Function,
            callee: CallCallee::Identifier {
                reference: None,
                name: "callee".to_string(),
            },
            receiver: None,
            arguments: Vec::new(),
            result: None,
            status: CallTargetStatus::Unresolved,
            precision: CallPrecision::Unknown,
            stable_key: crate::core::StableKeyId(0),
        };
        (db, file, site)
    }

    fn target_with_status(site: CallSiteId, status: CallTargetStatus) -> CallTargetFact {
        CallTargetFact {
            id: CallTargetId(0),
            site,
            caller: FunctionId::from_raw(0),
            target_function: None,
            target_symbol: None,
            synthetic_target: None,
            edge_kind: CallEdgeKind::Unknown,
            algorithm: CallAlgorithm::Unsupported,
            status,
            reason: None,
            provenance: CallProvenance::Native,
            precision: CallPrecision::Unsupported,
            stable_key: crate::core::StableKeyId(status as u32 + 1),
        }
    }

    fn callsite_identity_at(file: FileId, span: Span) -> IdentityRecord {
        let language = LanguageTag::Go;
        IdentityRecord {
            id: IdentityRecordId(0),
            kind: IdentityKind::Callsite,
            file_id: file,
            span,
            language,
            package_or_module: std::sync::Arc::from("src/main.go"),
            container_path: std::sync::Arc::from("main"),
            display_name: std::sync::Arc::from("callee"),
            signature_digest: compute_signature_digest(
                language,
                "src/main.go",
                "main",
                "callee",
                None,
                None,
            ),
            multiplicity: 1,
            stable_key: crate::core::stable_key_for_test("identity|callsite|synthetic"),
            originating_call_site_id: None,
            originating_call_target_id: None,
        }
    }

    #[test]
    fn categorized_failures_unresolved_and_unsupported_from_unresolved_calls() {
        use crate::analysis::calls::store::CallOutput;
        let (mut db, _file, site) = db_with_one_site();
        let unresolved = vec![
            UnresolvedCallFact {
                site: site.id,
                caller: FunctionId::from_raw(0),
                status: CallTargetStatus::Unresolved,
                reason: UnresolvedCallReason::DynamicProperty,
                algorithm: CallAlgorithm::Unsupported,
                provenance: CallProvenance::Native,
                precision: CallPrecision::Unknown,
                stable_key: crate::core::StableKeyId(1),
            },
            UnresolvedCallFact {
                site: site.id,
                caller: FunctionId::from_raw(0),
                status: CallTargetStatus::Unsupported,
                reason: UnresolvedCallReason::Reflection,
                algorithm: CallAlgorithm::Unsupported,
                provenance: CallProvenance::Native,
                precision: CallPrecision::Unsupported,
                stable_key: crate::core::StableKeyId(2),
            },
        ];
        db.replace_call_facts(CallOutput {
            sites: vec![site],
            targets: Vec::new(),
            unresolved,
        })
        .unwrap();

        let section = categorized_failures_from_db(&db, &std::collections::BTreeSet::new());
        assert_eq!(section.unresolved_edge, 1);
        assert_eq!(section.unsupported_edge, 1);
        assert_eq!(section.package_load_limitation, 0);
        assert_eq!(section.model_missing, 0);
        assert_eq!(section.wrong_identity, 0);
    }

    #[test]
    fn categorized_failures_package_load_limitation_fires_on_setup_missing() {
        use crate::analysis::calls::store::CallOutput;
        let (mut db, _file, site) = db_with_one_site();
        let target = target_with_status(site.id, CallTargetStatus::SetupMissing);
        db.replace_call_facts(CallOutput {
            sites: vec![site],
            targets: vec![target],
            unresolved: Vec::new(),
        })
        .unwrap();

        let section = categorized_failures_from_db(&db, &std::collections::BTreeSet::new());
        assert_eq!(section.package_load_limitation, 1);
        assert_eq!(section.model_missing, 0);
        assert_eq!(section.unresolved_edge, 0);
        assert_eq!(section.unsupported_edge, 0);
    }

    #[test]
    fn categorized_failures_model_missing_fires_on_rejected_target() {
        use crate::analysis::calls::store::CallOutput;
        let (mut db, _file, site) = db_with_one_site();
        let target = target_with_status(site.id, CallTargetStatus::Rejected);
        db.replace_call_facts(CallOutput {
            sites: vec![site],
            targets: vec![target],
            unresolved: Vec::new(),
        })
        .unwrap();

        let section = categorized_failures_from_db(&db, &std::collections::BTreeSet::new());
        assert_eq!(section.model_missing, 1);
        assert_eq!(section.package_load_limitation, 0);
        assert_eq!(section.unresolved_edge, 0);
        assert_eq!(section.unsupported_edge, 0);
    }

    #[test]
    fn categorized_failures_resolved_and_ambiguous_targets_are_not_failures() {
        use crate::analysis::calls::store::CallOutput;
        let (mut db, _file, site) = db_with_one_site();
        let resolved = CallTargetFact {
            id: CallTargetId(0),
            stable_key: crate::core::StableKeyId(1),
            ..target_with_status(site.id, CallTargetStatus::Resolved)
        };
        let ambiguous = CallTargetFact {
            id: CallTargetId(1),
            stable_key: crate::core::StableKeyId(2),
            ..target_with_status(site.id, CallTargetStatus::Ambiguous)
        };
        db.replace_call_facts(CallOutput {
            sites: vec![site],
            targets: vec![resolved, ambiguous],
            unresolved: Vec::new(),
        })
        .unwrap();

        let section = categorized_failures_from_db(&db, &std::collections::BTreeSet::new());
        assert_eq!(section, CategorizedFailureSection::default());
    }

    #[test]
    fn categorized_failures_wrong_identity_fires_on_oracle_span_overlap() {
        let (mut db, file, _site) = db_with_one_site();
        let span = Span::new(file, 20, 30, 2, 5, 2, 15);
        db.set_identity_records_for_test(vec![callsite_identity_at(file, span.clone())]);

        // An oracle entry overlapping the observed callsite's (file, span) makes it
        // a wrong-identity event (D-16). The key matches the (file_id, start_byte,
        // end_byte) tuple `categorized_failures_from_db` derives.
        let oracle = std::collections::BTreeSet::from([(file.0, span.start_byte, span.end_byte)]);
        let section = categorized_failures_from_db(&db, &oracle);
        assert_eq!(section.wrong_identity, 1);

        // With no overlapping oracle the same record is a miss, not a wrong-identity.
        let empty = std::collections::BTreeSet::new();
        assert_eq!(categorized_failures_from_db(&db, &empty).wrong_identity, 0);
    }

    fn summary(
        outcome: MatchOutcome,
        item_kind: MatchItemKind,
        has_expected: bool,
        has_observed: bool,
    ) -> MatchSummary {
        MatchSummary {
            item_key: format!("{item_kind:?}:{outcome:?}"),
            outcome,
            item_kind,
            expected_key: has_expected.then(|| "expected".to_string()),
            observed_key: has_observed.then(|| "observed".to_string()),
            observed_status: has_observed.then_some(crate::eval::model::ObservedStatus::Present),
            expected_runtime_budget_ms: None,
            expected_mode: has_expected.then_some(AssertionMode::Exact),
            observed_runtime_ms: None,
        }
    }

    fn summary_with_status(
        outcome: MatchOutcome,
        item_kind: MatchItemKind,
        observed_status: crate::eval::model::ObservedStatus,
    ) -> MatchSummary {
        MatchSummary {
            item_key: format!("{item_kind:?}:{outcome:?}:{observed_status:?}"),
            outcome,
            item_kind,
            expected_key: Some("expected".to_string()),
            observed_key: Some("observed".to_string()),
            observed_status: Some(observed_status),
            expected_runtime_budget_ms: None,
            expected_mode: Some(AssertionMode::Exact),
            observed_runtime_ms: None,
        }
    }
}
