//! Production rule-dispatch coverage for kernel runtime provider blockers.
//!
//! These tests live beside the kernel rather than in `runner/` because the
//! public-surface leak gates forbid kernel vocabulary (`polint.refined_calls`,
//! `validation`, framework dispatch markers) anywhere under `src/runner`. The
//! runner's only job is to forward `KernelOutput::db`, `capability_support`,
//! and `runtime_blocked_rules` into `run_rules_with_runtime_provider_blockers`,
//! which is exactly the call these tests exercise.

#[cfg(all(feature = "lang-go", feature = "lang-typescript"))]
use super::ProviderOutcome;
use super::{AnalysisKernel, KernelInput, KernelOutput};
use crate::analysis_plan::AnalysisPlan;
use crate::cache::Cache;
use crate::config::{LoadedConfig, load_config};
#[cfg(all(feature = "lang-go", feature = "lang-typescript"))]
use crate::core::CapabilitySupportView;
use crate::core::{
    Capabilities, Rule, RuleKind, RuleMeta, RuleRuntimeViews,
    run_rules_with_runtime_provider_blockers,
};
#[cfg(all(feature = "lang-go", feature = "lang-typescript"))]
use crate::diagnostics::sort_diagnostics;
use crate::diagnostics::{Diagnostic, Severity, TextRange};
use crate::sdk::facts::{FactView, FileMetrics as Metrics};
use std::collections::BTreeMap;
#[cfg(all(feature = "lang-go", feature = "lang-typescript"))]
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};

type Counter = std::sync::Arc<AtomicUsize>;

/// Runs the rules exactly as `runner::analyze_and_run` does for a kernel output.
fn dispatch(output: &KernelOutput, rules: &[Rule], parallel: bool) -> Vec<Diagnostic> {
    let runtime = RuleRuntimeViews::new(
        &output.capability_support,
        &output.completeness,
        &output.runtime_blocked_rules,
    );
    run_rules_with_runtime_provider_blockers(
        &output.db,
        rules,
        &BTreeMap::new(),
        None,
        parallel,
        &runtime,
    )
}

#[cfg(all(feature = "lang-go", feature = "lang-typescript"))]
#[derive(Debug, PartialEq, Eq)]
struct Projection {
    support: CapabilitySupportView,
    provider_outcomes: Vec<ProviderOutcome>,
    runtime_blockers: BTreeSet<String>,
    kernel_diagnostics: Vec<Diagnostic>,
    policy_diagnostics: Vec<Diagnostic>,
    policy_answers: Vec<String>,
    combined_diagnostics: Vec<Diagnostic>,
    decisions: [usize; 2],
}

fn run_kernel(loaded: &LoadedConfig, cache: &Cache, plan: &AnalysisPlan) -> KernelOutput {
    AnalysisKernel::run(KernelInput {
        loaded,
        cache,
        config_digest: "config",
        rule_digest: "rules",
        plan,
        parallel: false,
    })
    .expect("kernel")
}

fn counted_rule(id: &'static str, caps: Capabilities, counter: &Counter) -> Rule {
    let counter = Counter::clone(counter);
    Rule::from_parts(
        move || RuleMeta {
            id: id.to_string(),
            description: id.to_string(),
            severity: Severity::Warn,
            kind: RuleKind::Check,
        },
        move || caps,
        move |db, ctx| {
            counter.fetch_add(1, Ordering::SeqCst);
            let answer = if caps.file_metrics {
                Metrics::build(db).iter().count()
            } else {
                0
            };
            let diagnostic = Diagnostic::warning(id, "", TextRange::point(1, 1), "policy answer");
            ctx.report(diagnostic.with_evidence("answer", format!("{id}={answer}")));
            Ok(())
        },
    )
}

fn counts(counters: &[Counter; 2]) -> [usize; 2] {
    [
        counters[0].load(Ordering::SeqCst),
        counters[1].load(Ordering::SeqCst),
    ]
}

fn blocker_fixture(
    source: (&str, &str),
    blocked_rules: &[(&'static str, Capabilities)],
    provider_ids: &[&str],
) -> (Vec<Diagnostic>, [usize; 2]) {
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp.path().join(source.0), source.1).expect("source");
    let loaded = load_config(temp.path()).expect("config");
    let counters = [Counter::default(), Counter::default()];
    let mut rules = blocked_rules
        .iter()
        .map(|(id, caps)| counted_rule(id, *caps, &counters[0]))
        .collect::<Vec<_>>();
    rules.push(counted_rule("allowed", Capabilities::new(), &counters[1]));
    let plan = AnalysisPlan::from_rules(&rules, None, &BTreeMap::new());
    let mut output = run_kernel(&loaded, &Cache::new("", false), &plan);
    let outcomes = &mut output.run_report.provider_outcomes;
    for provider_id in provider_ids {
        let outcome = outcomes
            .iter_mut()
            .find(|row| row.provider_id == *provider_id);
        outcome
            .expect("provider outcome")
            .reject_validation_for_test();
    }
    let (blocked, diagnostics) = AnalysisKernel::runtime_capability_blockers(
        &plan,
        &output.db,
        &output.run_report.provider_outcomes,
    );
    output.runtime_blocked_rules = blocked;
    dispatch(&output, &rules, true);
    (diagnostics, counts(&counters))
}

#[cfg(all(feature = "lang-go", feature = "lang-typescript"))]
fn project(output: &KernelOutput, rules: &[Rule], counters: &[Counter; 2]) -> Projection {
    let before = counts(counters);
    let mut policy_diagnostics = dispatch(output, rules, false);
    let after = counts(counters);
    let decisions = std::array::from_fn(|index| after[index] - before[index]);
    sort_diagnostics(&mut policy_diagnostics);
    let answers = policy_diagnostics
        .iter()
        .map(|diagnostic| diagnostic.evidence[0].value.clone());
    let policy_answers = answers.collect();
    let mut kernel_diagnostics = output.diagnostics.clone();
    sort_diagnostics(&mut kernel_diagnostics);
    let mut combined_diagnostics =
        [kernel_diagnostics.as_slice(), policy_diagnostics.as_slice()].concat();
    sort_diagnostics(&mut combined_diagnostics);
    Projection {
        support: output.capability_support.clone(),
        provider_outcomes: output.run_report.provider_outcomes.clone(),
        runtime_blockers: output.runtime_blocked_rules.clone(),
        kernel_diagnostics,
        policy_diagnostics,
        policy_answers,
        combined_diagnostics,
        decisions,
    }
}

#[test]
fn production_dispatch_blocks_events_from_rejected_scheduled_refinement() {
    let blocked_rules = [
        ("test/events", Capabilities::new().events()),
        ("test/calls", Capabilities::new().calls()),
    ];
    let (diagnostics, decisions) = blocker_fixture(
        ("main.ts", "export function f(){f()}"),
        &blocked_rules,
        &["polint.refined_calls"],
    );
    assert_eq!(diagnostics.len(), 2);
    assert_eq!(decisions, [0, 1]);
}

#[test]
fn production_dispatch_blocks_events_for_applicable_syntax_failure() {
    let blocked_rules = [("test/events", Capabilities::new().events())];
    let failed_providers = ["polint.go.syntax", "polint.ts.syntax"];
    let (diagnostics, decisions) = blocker_fixture(
        ("main.go", "package main\n"),
        &blocked_rules,
        &failed_providers,
    );
    assert_eq!(diagnostics[0].evidence[3].value, "polint.go.syntax");
    assert_eq!(decisions, [0, 1]);
}

#[cfg(all(feature = "lang-go", feature = "lang-typescript"))]
#[test]
fn cold_warm_production_semantic_projection_matches() {
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp.path().join("main.ts"), "export const n = 1;\n").expect("source");
    std::fs::write(
        temp.path().join("main.go"),
        "package main\nfunc answer() string { return \"yes\" }\n",
    )
    .expect("Go source");
    let loaded = load_config(temp.path()).expect("config");
    let cache = Cache::default_for_repo(temp.path(), true);
    let counters = [Counter::default(), Counter::default()];
    let rules = vec![
        counted_rule("metrics", Capabilities::new().file_metrics(), &counters[0]),
        counted_rule("unaffected", Capabilities::new(), &counters[1]),
    ];
    let plan = AnalysisPlan::from_rules(&rules, None, &BTreeMap::new());
    let cold = run_kernel(&loaded, &cache, &plan);
    let cold_projection = project(&cold, &rules, &counters);
    let warm = run_kernel(&loaded, &cache, &plan);
    let warm_projection = project(&warm, &rules, &counters);
    let disabled_cache = Cache::new(temp.path().join("disabled-cache"), false);
    let disabled = run_kernel(&loaded, &disabled_cache, &plan);
    let disabled_projection = project(&disabled, &rules, &counters);
    assert_eq!(cold_projection, warm_projection);
    let cold_metrics = cold.run_report.provider_outcomes.last().unwrap();
    let disabled_metrics = disabled.run_report.provider_outcomes.last().unwrap();
    assert_eq!(cold_metrics, disabled_metrics);
    let go = |output: &KernelOutput| {
        output
            .run_report
            .provider_outcomes
            .iter()
            .find(|outcome| outcome.provider_id == "polint.go.syntax")
            .unwrap()
            .clone()
    };
    assert_eq!(go(&cold), go(&disabled));
    assert!(go(&cold).output_identity.is_some());
    let ts = |output: &KernelOutput| {
        output
            .run_report
            .provider_outcomes
            .iter()
            .find(|outcome| outcome.provider_id == "polint.ts.syntax")
            .unwrap()
            .clone()
    };
    assert_eq!(ts(&cold), ts(&disabled));
    assert!(ts(&cold).output_identity.is_some());
    assert_eq!(cold_projection, disabled_projection);
    assert_eq!(cold_projection.decisions, [1, 1]);
    let answers = &cold_projection.policy_answers;
    assert_eq!(answers.as_slice(), ["metrics=2", "unaffected=0"]);
    let cold_telemetry = &cold.run_report.provider_telemetry;
    assert_ne!(cold_telemetry, &warm.run_report.provider_telemetry);
    assert_ne!(
        &warm.run_report.provider_telemetry,
        &disabled.run_report.provider_telemetry
    );
    let expected_outcomes = &warm.run_report.provider_outcomes;
    let blob = std::fs::read_dir(cache.layer_cache_dir().join("blobs"))
        .expect("cache blobs")
        .next()
        .expect("cached payload")
        .expect("blob")
        .path();
    std::fs::write(blob, b"corrupt").expect("corrupt");
    let repaired = run_kernel(&loaded, &cache, &plan);
    assert_eq!(expected_outcomes, &repaired.run_report.provider_outcomes);
    assert!(repaired.run_report.provider_telemetry.iter().any(|row| {
        row.cache_stats.invalid_evicted_reads > 0 && row.cache_stats.recomputes > 0
    }));
    let warning_cache = Cache::new(temp.path().join("warning-cache"), true);
    std::fs::create_dir_all(warning_cache.root()).expect("warning cache root");
    std::fs::write(warning_cache.layer_cache_dir(), b"not a directory").expect("block writes");
    let warned = run_kernel(&loaded, &warning_cache, &plan);
    let mut normalized_warned_outcomes = warned.run_report.provider_outcomes.clone();
    let expected_ts_identity = expected_outcomes
        .iter()
        .find(|outcome| outcome.provider_id == "polint.ts.syntax")
        .and_then(|outcome| outcome.output_identity.clone())
        .expect("warm TS provider identity");
    let warned_ts = normalized_warned_outcomes
        .iter_mut()
        .find(|outcome| outcome.provider_id == "polint.ts.syntax")
        .expect("TS provider outcome");
    let fallback_ts_identity = warned_ts
        .output_identity
        .replace(expected_ts_identity.clone())
        .expect("failed cache write still receives a deterministic kernel fallback");
    assert_ne!(fallback_ts_identity, expected_ts_identity);
    assert_eq!(expected_outcomes, &normalized_warned_outcomes);
    let mut warnings = warned.diagnostics.iter();
    let write_warning = warnings.any(|d| d.message.contains("cache write failed"));
    assert!(write_warning);
    std::fs::remove_file(warning_cache.layer_cache_dir()).expect("remove write blocker");
    let recovered = run_kernel(&loaded, &warning_cache, &plan);
    assert_eq!(expected_outcomes, &recovered.run_report.provider_outcomes);
    assert!(
        !recovered
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("cache write failed"))
    );
    let later_warm = run_kernel(&loaded, &warning_cache, &plan);
    assert_eq!(expected_outcomes, &later_warm.run_report.provider_outcomes);
    assert!(later_warm.run_report.provider_telemetry.iter().any(|row| {
        row.provider_id == "polint.go.syntax" && row.cache_stats.verified_reuse > 0
    }));
}
