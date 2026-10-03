use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::unknown_taxonomy::facts::{UnknownCategory, UnknownRow, UnknownRowInput};
use crate::analysis_kernel::{AnalysisKernel, ProviderOutcome, ProviderOutcomeStatus};
use crate::analysis_plan::AnalysisPlan;
use crate::core::{
    AnalysisDb, CapabilityCompleteness, CapabilityCompletenessStatus, CapabilitySupportStatus,
    CapabilitySupportView, CompletenessView,
};
use crate::diagnostics::Diagnostic;

pub(super) fn view_from_run(
    plan: &AnalysisPlan,
    db: &AnalysisDb,
    outcomes: &[ProviderOutcome],
    diagnostics: &[Diagnostic],
) -> CompletenessView {
    let rules_by_capability = direct_rules_by_capability(plan);
    let unknowns =
        crate::analysis::unknown_taxonomy::collect::all_unknowns_with_diagnostics(db, diagnostics);
    let outcomes_by_provider = outcomes
        .iter()
        .map(|outcome| (outcome.provider_id.as_str(), outcome))
        .collect::<BTreeMap<_, _>>();

    let entries = rules_by_capability
        .into_iter()
        .map(|(capability, rules)| {
            let (status, reason) =
                capability_status(plan, db, &capability, &outcomes_by_provider, &unknowns);
            CapabilityCompleteness::new(capability, status, reason, rules.into_iter().collect())
        })
        .collect();
    CompletenessView::new(entries)
}

fn direct_rules_by_capability(plan: &AnalysisPlan) -> BTreeMap<String, BTreeSet<String>> {
    let mut rules_by_capability = BTreeMap::<String, BTreeSet<String>>::new();
    for rule in plan.rules() {
        for capability in &rule.requested_capabilities {
            rules_by_capability
                .entry(capability.clone())
                .or_default()
                .insert(rule.id.clone());
        }
    }
    rules_by_capability
}

fn capability_status(
    plan: &AnalysisPlan,
    db: &AnalysisDb,
    capability: &str,
    outcomes: &BTreeMap<&str, &ProviderOutcome>,
    unknowns: &[UnknownRow],
) -> (CapabilityCompletenessStatus, Option<String>) {
    if plan.support_view().status_for(capability) != Some(CapabilitySupportStatus::Supported) {
        let reason = plan
            .support_view()
            .entries()
            .iter()
            .find(|entry| entry.capability == capability)
            .and_then(|entry| entry.reason.clone())
            .or_else(|| Some("capability support is unavailable".to_string()));
        return (CapabilityCompletenessStatus::Unknown, reason);
    }

    let providers = AnalysisKernel::capability_providers(capability, db);
    if providers.is_empty() {
        return (
            CapabilityCompletenessStatus::Unknown,
            Some("no completeness source is registered for this capability".to_string()),
        );
    }

    let provider_rows = providers
        .iter()
        .filter_map(|provider| outcomes.get(provider).copied())
        .collect::<Vec<_>>();
    if provider_rows.len() != providers.len() {
        return (
            CapabilityCompletenessStatus::Unknown,
            Some("provider outcome information is unavailable".to_string()),
        );
    }

    if let Some(outcome) = provider_rows
        .iter()
        .find(|outcome| outcome.status == ProviderOutcomeStatus::BudgetExceeded)
    {
        return (
            CapabilityCompletenessStatus::BudgetExceeded,
            Some(provider_outcome_reason(outcome)),
        );
    }

    if let Some(outcome) = provider_rows.iter().find(|outcome| {
        matches!(
            outcome.status,
            ProviderOutcomeStatus::Failed | ProviderOutcomeStatus::DependencyBlocked
        )
    }) {
        return (
            CapabilityCompletenessStatus::ProviderFailed,
            Some(provider_outcome_reason(outcome)),
        );
    }

    if let Some(outcome) = provider_rows
        .iter()
        .find(|outcome| outcome.status != ProviderOutcomeStatus::Succeeded)
    {
        return (
            CapabilityCompletenessStatus::Unknown,
            Some(provider_outcome_reason(outcome)),
        );
    }

    if super::provider::reads_typed_go_frontend(capability)
        && super::provider::go_types_unloaded(db)
    {
        return (
            CapabilityCompletenessStatus::Unknown,
            Some("the typed Go frontend loaded no package for the scanned Go files".to_string()),
        );
    }

    let requested = BTreeSet::from([capability]);
    let relevant_providers = super::provider::providers_enabled_by_capability_closure(&requested);
    let relevant_unknowns = unknowns
        .iter()
        .filter(|row| {
            row.capability.as_deref() == Some(capability)
                || (!scoped_to_another_capability(row, capability)
                    && (row.provider == "polint.kernel"
                        || relevant_providers.contains(row.provider.as_str())))
        })
        .collect::<Vec<_>>();

    if relevant_unknowns
        .iter()
        .any(|row| row.category == UnknownCategory::BudgetExceeded)
    {
        return (
            CapabilityCompletenessStatus::BudgetExceeded,
            Some(unknown_reasons(&relevant_unknowns, true)),
        );
    }
    if !relevant_unknowns.is_empty() {
        return (
            CapabilityCompletenessStatus::Degraded,
            Some(unknown_reasons(&relevant_unknowns, false)),
        );
    }

    (CapabilityCompletenessStatus::Complete, None)
}

/// Unknown rows that describe one capability's answer only: a route
/// interpretation budget stop says nothing about the call or type facts the
/// same provider produced.
fn scoped_to_another_capability(row: &UnknownRow, capability: &str) -> bool {
    const SCOPED_CAPABILITIES: &[&str] = &["routes"];
    row.capability
        .as_deref()
        .is_some_and(|scoped| scoped != capability && SCOPED_CAPABILITIES.contains(&scoped))
}

/// The part of `capabilities`' analysis pipeline that did not run, as one error
/// row per cause: every stage in their own closure that failed, is
/// setup-missing or unsupported, or was skipped by the resource budget, and
/// every setup-missing language support a requested capability (or one it
/// depends on) reported at run time. Stages that only waited on one of those
/// add no row of their own; the cause is the row.
///
/// Without these rows a capability whose pipeline never ran answers with an
/// empty or partial list, which reads as "nothing (else) is unknown". The rows
/// name stages in public terms: provider identifiers stay out of the report.
pub(crate) fn pipeline_failure_unknowns(
    capabilities: &[&str],
    outcomes: &[ProviderOutcome],
    support: &CapabilitySupportView,
    diagnostics: &[Diagnostic],
) -> Vec<UnknownRow> {
    let requested = capabilities.iter().copied().collect::<BTreeSet<_>>();
    let closure = super::provider::providers_required_by_capabilities(&requested);
    let in_closure = outcomes
        .iter()
        .filter(|outcome| closure.contains(outcome.provider_id.as_str()))
        .collect::<Vec<_>>();
    let causes = in_closure
        .iter()
        .filter(|outcome| {
            matches!(
                outcome.status,
                ProviderOutcomeStatus::Failed
                    | ProviderOutcomeStatus::SetupMissing
                    | ProviderOutcomeStatus::Unsupported
                    | ProviderOutcomeStatus::BudgetExceeded
            )
        })
        .collect::<Vec<_>>();
    let mut rows = causes
        .iter()
        .map(|outcome| stage_row(outcome, public_stage_label(&outcome.provider_id)))
        .collect::<Vec<_>>();
    // A stage blocked by a failure outside the closure has no cause row here.
    if causes.is_empty() {
        rows.extend(
            in_closure
                .iter()
                .filter(|outcome| outcome.status == ProviderOutcomeStatus::DependencyBlocked)
                .map(|outcome| stage_row(outcome, public_stage_label(&outcome.provider_id))),
        );
    }

    let depended_on = requested
        .iter()
        .flat_map(|capability| {
            std::iter::once(*capability).chain(
                crate::analysis_plan::capability_dependencies(capability)
                    .iter()
                    .copied(),
            )
        })
        .collect::<BTreeSet<_>>();
    rows.extend(
        support
            .entries()
            .iter()
            .filter(|entry| depended_on.contains(entry.capability.as_str()))
            .filter(|entry| entry.status == CapabilitySupportStatus::SetupMissing)
            .map(|entry| {
                let language = entry
                    .language
                    .map_or("workspace", crate::symbol_graph::language_name);
                let detail = entry.reason.as_deref().unwrap_or("no reason was reported");
                UnknownRow::new(UnknownRowInput {
                    category: UnknownCategory::SetupMissing,
                    capability: Some(entry.capability.clone()),
                    family: Some("CapabilitySupport".to_string()),
                    provider: "polint.kernel".to_string(),
                    file: "<workspace>".to_string(),
                    span: None,
                    status: "setup_missing".to_string(),
                    reason: Some(format!(
                        "{language} `{}` support is setup-missing: {detail}",
                        entry.capability
                    )),
                    precision: Some("unknown".to_string()),
                    docs_path: Some(
                        entry
                            .docs_path
                            .clone()
                            .unwrap_or_else(|| PIPELINE_DOCS_PATH.to_string()),
                    ),
                    suggested_artifact: Some("setup_or_budget".to_string()),
                    source_stable_key: Some(format!(
                        "capability_support:{}:{language}",
                        entry.capability
                    )),
                })
            }),
    );
    // The outcome says that the sidecar failed; its diagnostics say why.
    if causes
        .iter()
        .any(|outcome| outcome.provider_id == "polint.go.semantic")
    {
        rows.extend(
            crate::analysis::unknown_taxonomy::collect::go_semantic_diagnostic_unknowns(
                diagnostics,
            ),
        );
    }
    crate::analysis::unknown_taxonomy::facts::normalize_rows(rows)
}

fn stage_row(outcome: &ProviderOutcome, stage: &str) -> UnknownRow {
    let category = match outcome.status {
        ProviderOutcomeStatus::SetupMissing => UnknownCategory::SetupMissing,
        ProviderOutcomeStatus::BudgetExceeded => UnknownCategory::BudgetExceeded,
        _ => UnknownCategory::ProviderFailed,
    };
    let mut reason = format!("{stage} did not run: {}", outcome.status.label());
    if let (Some(failed_at), Some(failure)) = (outcome.failure_stage, outcome.failure_reason) {
        reason.push_str(&format!(" ({}, {})", failed_at.label(), failure.label()));
    }
    UnknownRow::new(UnknownRowInput {
        category,
        capability: None,
        family: Some("ProviderOutcome".to_string()),
        provider: outcome.provider_id.clone(),
        file: "<workspace>".to_string(),
        span: None,
        status: outcome.status.label().to_string(),
        reason: Some(reason),
        precision: Some("unknown".to_string()),
        docs_path: Some(PIPELINE_DOCS_PATH.to_string()),
        suggested_artifact: Some("setup_or_budget".to_string()),
        source_stable_key: Some(format!("provider_outcome:{}", outcome.provider_id)),
    })
}

/// How a failed stage is named in a public report. Provider identifiers are
/// internal, and most of the deep pipeline's stages have no public name of
/// their own.
fn public_stage_label(provider_id: &str) -> &'static str {
    match provider_id {
        "polint.source" => "source discovery",
        "polint.go.syntax" => "Go syntax analysis",
        "polint.ts.syntax" => "TypeScript and JavaScript syntax analysis",
        "polint.module_graph" | "polint.module_topology" => "module graph analysis",
        "polint.symbol_graph" => "symbol and reference analysis",
        "polint.go.semantic" => "the Go semantic sidecar",
        "polint.ts.types" => "the TypeScript type sidecar",
        "polint.metrics" => "metrics",
        "polint.extensions" => "repository extensions",
        _ => "a deep analysis stage",
    }
}

const PIPELINE_DOCS_PATH: &str = "docs/facts/capability-plans.md";

fn provider_outcome_reason(outcome: &ProviderOutcome) -> String {
    let mut reason = format!("{}: {}", outcome.provider_id, outcome.status.label());
    if let (Some(stage), Some(failure)) = (outcome.failure_stage, outcome.failure_reason) {
        reason.push_str(&format!(":{}:{}", stage.label(), failure.label()));
    }
    if !outcome.blockers.is_empty() {
        reason.push_str(&format!(" (blockers: {})", outcome.blockers.join(",")));
    }
    reason
}

fn unknown_reasons(rows: &[&UnknownRow], budget_only: bool) -> String {
    rows.iter()
        .filter(|row| !budget_only || row.category == UnknownCategory::BudgetExceeded)
        .map(|row| {
            let detail = row.reason.as_deref().unwrap_or(row.status.as_str());
            format!("{}: {detail}", row.provider)
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_kernel::{KernelInput, ProviderFailureReason, ProviderFailureStage};
    use crate::cache::Cache;
    use crate::config::load_config;
    use crate::core::{Capabilities, Rule, RuleKind, RuleMeta};
    use crate::diagnostics::Severity;

    fn metrics_fixture() -> (AnalysisPlan, crate::analysis_kernel::KernelOutput) {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("main.ts"), "export const n = 1;\n").expect("source");
        let loaded = load_config(temp.path()).expect("config");
        let rule = Rule::from_parts(
            || RuleMeta {
                id: "test/metrics".to_string(),
                description: "metrics".to_string(),
                severity: Severity::Warn,
                kind: RuleKind::Check,
            },
            || Capabilities::new().file_metrics(),
            |_, _| Ok(()),
        );
        let plan = AnalysisPlan::from_rules(&[rule], None, &BTreeMap::new());
        let output = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &Cache::new("", false),
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("kernel");
        (plan, output)
    }

    #[cfg(all(feature = "lang-go", feature = "lang-typescript"))]
    #[test]
    fn complete_provider_run_builds_complete_view() {
        let (plan, output) = metrics_fixture();
        let view = view_from_run(
            &plan,
            &output.db,
            &output.run_report.provider_outcomes,
            &output.diagnostics,
        );

        assert_eq!(
            view.status_for("file_metrics"),
            CapabilityCompletenessStatus::Complete
        );
        assert!(view.is_complete());
    }

    #[test]
    fn budget_stopped_provider_builds_budget_exceeded_view() {
        let (plan, mut output) = metrics_fixture();
        let outcome = output
            .run_report
            .provider_outcomes
            .iter_mut()
            .find(|outcome| outcome.provider_id == "polint.metrics")
            .expect("metrics outcome");
        *outcome = ProviderOutcome::from_closed_parts(
            "polint.metrics".to_string(),
            ProviderOutcomeStatus::BudgetExceeded,
            None,
            Some(ProviderFailureStage::Execution),
            Some(ProviderFailureReason::MemoryCeiling),
            Vec::new(),
        )
        .expect("valid budget outcome");

        let view = view_from_run(
            &plan,
            &output.db,
            &output.run_report.provider_outcomes,
            &output.diagnostics,
        );

        assert_eq!(
            view.status_for("file_metrics"),
            CapabilityCompletenessStatus::BudgetExceeded
        );
        assert!(view.budget_exceeded());
    }

    #[cfg(feature = "lang-go")]
    #[test]
    fn a_route_budget_stop_marks_routes_incomplete_and_nothing_else() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join("go.mod"),
            "module example.com/m\n\ngo 1.22\n",
        )
        .expect("go.mod");
        std::fs::write(
            temp.path().join("main.go"),
            "package main\n\nfunc main() {}\n",
        )
        .expect("source");
        let loaded = load_config(temp.path()).expect("config");
        let rule = Rule::from_parts(
            || RuleMeta {
                id: "test/routes".to_string(),
                description: "routes".to_string(),
                severity: Severity::Warn,
                kind: RuleKind::Check,
            },
            || Capabilities::new().routes().go_types(),
            |_, _| Ok(()),
        );
        let plan = AnalysisPlan::from_rules(&[rule], None, &BTreeMap::new());
        let mut output = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &Cache::new("", false),
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("kernel");
        let view = |output: &crate::analysis_kernel::KernelOutput| {
            view_from_run(
                &plan,
                &output.db,
                &output.run_report.provider_outcomes,
                &output.diagnostics,
            )
        };
        assert_eq!(
            view(&output).status_for("routes"),
            CapabilityCompletenessStatus::Complete
        );

        let facts = crate::go::semantic::store::GoSemanticFactsOutput {
            packages: output.db.go_semantic_packages().to_vec(),
            functions: output.db.go_semantic_functions().to_vec(),
            route_budget_steps: Some(5_000_001),
            ..Default::default()
        };
        output
            .db
            .replace_go_semantic_facts(facts)
            .expect("facts store");

        let stopped = view(&output);
        assert_eq!(
            stopped.status_for("routes"),
            CapabilityCompletenessStatus::BudgetExceeded
        );
        assert_eq!(
            stopped.status_for("go_types"),
            CapabilityCompletenessStatus::Complete,
            "a route budget stop says nothing about the type facts"
        );
    }

    #[test]
    fn failed_provider_builds_provider_failed_view() {
        let (plan, mut output) = metrics_fixture();
        let outcome = output
            .run_report
            .provider_outcomes
            .iter_mut()
            .find(|outcome| outcome.provider_id == "polint.metrics")
            .expect("metrics outcome");
        *outcome = ProviderOutcome::from_closed_parts(
            "polint.metrics".to_string(),
            ProviderOutcomeStatus::Failed,
            None,
            Some(ProviderFailureStage::Execution),
            Some(ProviderFailureReason::ExecutionFailed),
            Vec::new(),
        )
        .expect("valid failed outcome");

        let view = view_from_run(
            &plan,
            &output.db,
            &output.run_report.provider_outcomes,
            &output.diagnostics,
        );

        assert_eq!(
            view.status_for("file_metrics"),
            CapabilityCompletenessStatus::ProviderFailed
        );
        assert!(view.reason_for("file_metrics").is_some());
    }

    #[cfg(all(feature = "lang-go", feature = "lang-typescript"))]
    #[test]
    fn a_healthy_pipeline_reports_no_failure_rows() {
        let (_plan, output) = metrics_fixture();
        let rows = pipeline_failure_unknowns(
            &["file_metrics"],
            &output.run_report.provider_outcomes,
            &output.capability_support,
            &output.diagnostics,
        );
        assert_eq!(rows, Vec::new());
    }

    #[cfg(all(feature = "lang-go", feature = "lang-typescript"))]
    #[test]
    fn a_failed_provider_in_the_closure_becomes_an_error_row_naming_it() {
        let (_plan, mut output) = metrics_fixture();
        let outcome = output
            .run_report
            .provider_outcomes
            .iter_mut()
            .find(|outcome| outcome.provider_id == "polint.metrics")
            .expect("metrics outcome");
        *outcome = ProviderOutcome::from_closed_parts(
            "polint.metrics".to_string(),
            ProviderOutcomeStatus::Failed,
            None,
            Some(ProviderFailureStage::Execution),
            Some(ProviderFailureReason::ExecutionFailed),
            Vec::new(),
        )
        .expect("valid failed outcome");

        let rows = pipeline_failure_unknowns(
            &["file_metrics"],
            &output.run_report.provider_outcomes,
            &output.capability_support,
            &output.diagnostics,
        );

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].provider, "polint.metrics");
        assert_eq!(rows[0].category, UnknownCategory::ProviderFailed);
        assert_eq!(rows[0].status, "failed");
        assert_eq!(rows[0].file, "<workspace>");
        assert_eq!(
            rows[0].reason.as_deref(),
            Some("metrics did not run: failed (execution, execution_failed)")
        );
        // A provider outside the requested closure is not this capability's problem.
        assert_eq!(
            pipeline_failure_unknowns(
                &["source_files"],
                &output.run_report.provider_outcomes,
                &output.capability_support,
                &output.diagnostics,
            ),
            Vec::new()
        );
    }

    #[cfg(all(feature = "lang-go", feature = "lang-typescript"))]
    #[test]
    fn setup_missing_support_a_capability_depends_on_becomes_an_error_row() {
        let (_plan, output) = metrics_fixture();
        let support = CapabilitySupportView::new(vec![crate::core::CapabilitySupport {
            capability: "references".to_string(),
            language: Some(crate::core::Language::Go),
            status: CapabilitySupportStatus::SetupMissing,
            rules: vec!["polint/requested-capabilities".to_string()],
            reason: Some("go.work lists go 1.27.0".to_string()),
            hint: None,
            docs_path: None,
        }]);

        let rows = pipeline_failure_unknowns(
            &["calls"],
            &output.run_report.provider_outcomes,
            &support,
            &output.diagnostics,
        );

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].capability.as_deref(), Some("references"));
        assert_eq!(rows[0].status, "setup_missing");
        assert!(
            rows[0]
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("go.work lists go 1.27.0")),
            "the row must carry the provider's reason"
        );
        // A capability that does not depend on references is unaffected.
        assert_eq!(
            pipeline_failure_unknowns(
                &["file_metrics"],
                &output.run_report.provider_outcomes,
                &support,
                &output.diagnostics,
            ),
            Vec::new()
        );
    }
}
