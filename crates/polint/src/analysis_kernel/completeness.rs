use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::unknown_taxonomy::facts::{UnknownCategory, UnknownRow};
use crate::analysis_kernel::{AnalysisKernel, ProviderOutcome, ProviderOutcomeStatus};
use crate::analysis_plan::AnalysisPlan;
use crate::core::{
    AnalysisDb, CapabilityCompleteness, CapabilityCompletenessStatus, CapabilitySupportStatus,
    CompletenessView,
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

    let requested = BTreeSet::from([capability]);
    let relevant_providers = super::provider::providers_enabled_by_capability_closure(&requested);
    let relevant_unknowns = unknowns
        .iter()
        .filter(|row| {
            row.capability.as_deref() == Some(capability)
                || row.provider == "polint.kernel"
                || relevant_providers.contains(row.provider.as_str())
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
}
