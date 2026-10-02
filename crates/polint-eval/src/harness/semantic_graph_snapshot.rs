//! Byte-stable semantic-graph snapshot fixtures (GRAPH-02, D-12).
//!
//! For each checked-in `tests/eval-fixtures/semantic-graph/*` fixture this gate runs
//! the kernel over the fixture repo, observes the normalized semantic-graph debug
//! JSON, and asserts:
//!
//! 1. the graph emits >= 1 node, >= 1 edge, and >= 1 constraint (the minimal
//!    projection genuinely populated a graph, not an empty observation), and
//! 2. the observed debug JSON is byte-stable and total-ordered by stable key across
//!    repeated runs AND across N seeded provider-order / row-order permutations — the
//!    same cross-platform / cross-shuffle determinism contract the
//!    determinism gate enforces.
//!
//! The semantic-graph provider auto-enrolls into the determinism gate via
//! `provider_manifests()` (D-22), so no determinism-gate harness edit is required;
//! this snapshot gate is the GRAPH-02-specific byte-stable constraint-emission proof.

#![cfg(test)]

use std::path::Path;

use crate::analysis::semantic_graph::collect_ts_direct_bindings;
use crate::analysis::semantic_graph::debug::metadata_debug_json_for_test as semantic_graph_debug_json;
use crate::analysis_kernel::KernelOutput;
use crate::analysis_kernel::incremental::Digest;
use crate::analysis_plan::AnalysisPlan;
use crate::eval::fixtures::load_native_fixture;
use crate::eval::observed::{
    copy_fixture_repo_for_test, run_kernel_for_repo_for_test,
    run_kernel_for_repo_with_plan_for_test,
};
use crate::ts::binding::facts::{
    TsDirectBindingFact, TsDirectBindingKind, TsDirectBindingReason, TsDirectBindingStatus,
};

fn fixture_dir(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/eval-fixtures/semantic-graph")
        .join(name)
}

/// Runs the kernel over a fixture repo and returns the byte-stable semantic-graph
/// debug JSON (pretty, deterministic key order).
fn observe_semantic_graph(name: &str) -> serde_json::Value {
    let output = run_fixture_kernel(name);
    semantic_graph_debug_json(&output.db)
}

fn run_fixture_kernel(name: &str) -> KernelOutput {
    let fixture = load_native_fixture(&fixture_dir(name)).expect("fixture loads");
    let temp = copy_fixture_repo_for_test(&fixture).expect("copy fixture repo");
    if name == "ts_direct_bindings" {
        let plan = AnalysisPlan::from_capability_names_for_test(&["calls"]);
        return run_kernel_for_repo_with_plan_for_test(temp.path(), &plan)
            .expect("kernel runs over TS direct-binding fixture repo");
    }
    run_kernel_for_repo_for_test(temp.path()).expect("kernel runs over fixture repo")
}

fn provider_output_digest(output: &KernelOutput, provider_id: &str) -> Digest {
    let outcome = output
        .run_report
        .provider_outcomes
        .iter()
        .find(|row| row.provider_id == provider_id)
        .unwrap_or_else(|| panic!("provider outcome row {provider_id} exists"));
    assert_eq!(
        outcome.status,
        crate::analysis_kernel::ProviderOutcomeStatus::Succeeded,
        "provider {provider_id} must succeed; stage={:?} reason={:?} blockers={:?}; outcomes={:?}",
        outcome.failure_stage,
        outcome.failure_reason,
        outcome.blockers,
        output
            .run_report
            .provider_outcomes
            .iter()
            .map(|row| (
                &row.provider_id,
                row.status,
                row.failure_stage,
                row.failure_reason,
                &row.blockers
            ))
            .collect::<Vec<_>>()
    );
    outcome
        .output_identity
        .as_ref()
        .unwrap_or_else(|| panic!("succeeded provider {provider_id} has output identity"))
        .output_digest
        .clone()
}

/// Asserts the fixture emits >= 1 of each kind and that the observed debug JSON is
/// byte-identical across repeated runs (cross-run determinism).
fn assert_fixture_emits_each_kind_and_is_byte_stable(name: &str) {
    let first = observe_semantic_graph(name);
    let second = observe_semantic_graph(name);

    let first_json = serde_json::to_string_pretty(&first).expect("serialize first");
    let second_json = serde_json::to_string_pretty(&second).expect("serialize second");
    assert_eq!(
        first_json, second_json,
        "{name}: semantic-graph debug JSON diverged across two kernel runs"
    );

    let total_nodes = first["counts"]["total_nodes"].as_u64().unwrap_or(0);
    let total_edges = first["counts"]["total_edges"].as_u64().unwrap_or(0);
    let total_constraints = first["counts"]["total_constraints"].as_u64().unwrap_or(0);
    assert!(
        total_nodes >= 1,
        "{name}: expected >= 1 semantic node, got {total_nodes}: {first:#}"
    );
    assert!(
        total_edges >= 1,
        "{name}: expected >= 1 semantic edge, got {total_edges}: {first:#}"
    );
    assert!(
        total_constraints >= 1,
        "{name}: expected >= 1 semantic constraint, got {total_constraints}: {first:#}"
    );

    // The debug rows are total-ordered by stable key per family (the snapshot
    // observation sorts them) and carry no run-local dense IDs or absolute paths.
    assert_rows_sorted_by_stable_key(&first["nodes"], name, "nodes");
    assert_rows_sorted_by_stable_key(&first["edges"], name, "edges");
    assert_rows_sorted_by_stable_key(&first["constraints"], name, "constraints");
    let serialized = serde_json::to_string(&first).expect("serialize");
    assert!(
        !serialized.contains("/Users/") && !serialized.contains("\\Users\\"),
        "{name}: debug JSON must not contain absolute paths"
    );
}

fn assert_rows_sorted_by_stable_key(rows: &serde_json::Value, name: &str, family: &str) {
    let rows = rows.as_array().cloned().unwrap_or_default();
    let keys: Vec<String> = rows
        .iter()
        .map(|row| {
            row["stable_key"]
                .as_str()
                .expect("stable_key string")
                .to_string()
        })
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(
        keys, sorted,
        "{name}: {family} rows must be total-ordered by stable key"
    );
}

#[test]
fn go_graph_fixture_emits_each_kind_and_is_byte_stable() {
    assert_fixture_emits_each_kind_and_is_byte_stable("go_graph");
}

#[test]
fn ts_graph_fixture_emits_each_kind_and_is_byte_stable() {
    assert_fixture_emits_each_kind_and_is_byte_stable("ts_graph");
}

#[test]
fn ts_direct_bindings_fixture_emits_each_kind_and_is_byte_stable() {
    assert_fixture_emits_each_kind_and_is_byte_stable("ts_direct_bindings");
}

#[test]
fn go_graph_emits_call_edge_and_call_constraint() {
    let observed = observe_semantic_graph("go_graph");
    let call_edges = observed["counts"]["edges_by_kind"]["call"]
        .as_u64()
        .unwrap_or(0);
    let call_constraints = observed["counts"]["constraints_by_kind"]["call_constraint"]
        .as_u64()
        .unwrap_or(0);
    assert!(
        call_edges >= 1,
        "go_graph: expected >= 1 Call edge, got {call_edges}: {observed:#}"
    );
    assert!(
        call_constraints >= 1,
        "go_graph: expected >= 1 CallConstraint, got {call_constraints}: {observed:#}"
    );
}

#[test]
fn ts_graph_emits_call_edge_and_call_constraint() {
    let observed = observe_semantic_graph("ts_graph");
    let call_edges = observed["counts"]["edges_by_kind"]["call"]
        .as_u64()
        .unwrap_or(0);
    let call_constraints = observed["counts"]["constraints_by_kind"]["call_constraint"]
        .as_u64()
        .unwrap_or(0);
    assert!(
        call_edges >= 1,
        "ts_graph: expected >= 1 Call edge, got {call_edges}: {observed:#}"
    );
    assert!(
        call_constraints >= 1,
        "ts_graph: expected >= 1 CallConstraint, got {call_constraints}: {observed:#}"
    );
}

#[test]
fn ts_direct_bindings_fixture_emits_resolved_copy_and_call_constraints() {
    let observed = observe_semantic_graph("ts_direct_bindings");
    let copy_edges = observed["counts"]["constraints_by_kind"]["copy_edge"]
        .as_u64()
        .unwrap_or(0);
    let call_constraints = observed["counts"]["constraints_by_kind"]["call_constraint"]
        .as_u64()
        .unwrap_or(0);
    assert!(
        copy_edges >= 1,
        "ts_direct_bindings: expected >= 1 CopyEdge, got {copy_edges}: {observed:#}"
    );
    assert!(
        call_constraints >= 1,
        "ts_direct_bindings: expected >= 1 CallConstraint, got {call_constraints}: {observed:#}"
    );

    let total_nodes = observed["counts"]["total_nodes"].as_u64().unwrap_or(0);
    let constraints = observed["constraints"]
        .as_array()
        .expect("constraints array");
    let ts_direct_rows = constraints
        .iter()
        .filter(|row| row["source"] == "ts_direct_binding")
        .collect::<Vec<_>>();
    assert!(
        ts_direct_rows
            .iter()
            .any(|row| row["kind"].as_str() == Some("copy_edge")),
        "ts_direct_bindings: expected a TS direct-binding CopyEdge row: {observed:#}"
    );
    assert!(
        ts_direct_rows
            .iter()
            .any(|row| row["kind"].as_str() == Some("call_constraint")),
        "ts_direct_bindings: expected a TS direct-binding CallConstraint row: {observed:#}"
    );
    for row in ts_direct_rows {
        let refs = row["referenced_nodes"]
            .as_array()
            .expect("referenced_nodes array");
        assert!(
            !refs.is_empty(),
            "ts_direct_bindings: TS direct row must reference graph nodes: {row:#}"
        );
        for node in refs {
            let node = node.as_u64().expect("node ref is u64");
            assert!(
                node < total_nodes,
                "ts_direct_bindings: referenced node {node} must resolve within {total_nodes} nodes: {row:#}"
            );
        }
    }
}

#[test]
fn ts_direct_bindings_fixture_asserts_each_claimed_binding_form_projects_to_constraints() {
    let output = run_fixture_kernel("ts_direct_bindings");
    let observed = semantic_graph_debug_json(&output.db);
    let bindings = collect_ts_direct_bindings(&output.db);

    for expected in ts_direct_binding_expectations() {
        let binding = binding_for_callsite(
            &output.db.stable_key_interner(),
            &bindings.bindings,
            expected.callsite,
        );
        assert_eq!(
            binding.status, expected.status,
            "ts_direct_bindings: {} status mismatch: {binding:#?}",
            expected.callsite
        );
        assert_eq!(
            binding.kind, expected.kind,
            "ts_direct_bindings: {} binding kind mismatch: {binding:#?}",
            expected.callsite
        );
        assert_eq!(
            binding.reason, expected.reason,
            "ts_direct_bindings: {} unresolved reason mismatch: {binding:#?}",
            expected.callsite
        );
        if let Some(target) = expected.target {
            assert!(
                binding
                    .target_function_stable_key
                    .is_some_and(|key| output.db.resolve_stable_key(key).contains(target)),
                "ts_direct_bindings: {} target must contain {target}: {binding:#?}",
                expected.callsite
            );
        }
        if expected.status == TsDirectBindingStatus::Resolved {
            assert_ts_direct_constraint(
                &output.db.stable_key_interner(),
                &observed,
                "call_constraint",
                binding,
            );
            if expected.emits_copy_edge {
                assert_ts_direct_constraint(
                    &output.db.stable_key_interner(),
                    &observed,
                    "copy_edge",
                    binding,
                );
            }
        }
    }
}

#[test]
fn ts_direct_bindings_fixture_records_non_string_dynamic_import_reason() {
    let output = run_fixture_kernel("ts_direct_bindings");
    let bindings = collect_ts_direct_bindings(&output.db);

    assert!(
        bindings
            .bindings
            .iter()
            .any(|binding| binding.reason == Some(TsDirectBindingReason::NonStringDynamicImport)),
        "ts_direct_bindings: expected unresolved non-string dynamic import reason, got {:#?}",
        bindings.bindings
    );
}

#[test]
fn semantic_graph_digest_changes_when_ts_path_alias_fixture_changes() {
    let fixture = load_native_fixture(&fixture_dir("ts_direct_bindings")).expect("fixture loads");
    let temp = copy_fixture_repo_for_test(&fixture).expect("copy fixture repo");
    let plan = AnalysisPlan::from_capability_names_for_test(&["calls"]);
    let first =
        run_kernel_for_repo_with_plan_for_test(temp.path(), &plan).expect("first kernel run");
    let first_digest = provider_output_digest(&first, "polint.semantic_graph");
    let first_bindings = collect_ts_direct_bindings(&first.db);
    let first_alias = binding_for_callsite(
        &first.db.stable_key_interner(),
        &first_bindings.bindings,
        "aliasTarget",
    );
    assert_eq!(first_alias.status, TsDirectBindingStatus::Resolved);

    std::fs::write(
        temp.path().join("tsconfig.json"),
        r#"{
  "compilerOptions": {
    "baseUrl": ".",
    "paths": {
      "@lib/*": ["src/missing/*"]
    },
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "Bundler"
  },
  "include": ["src/**/*"]
}
"#,
    )
    .expect("break path alias in fixture copy");

    let second =
        run_kernel_for_repo_with_plan_for_test(temp.path(), &plan).expect("second kernel run");
    let second_digest = provider_output_digest(&second, "polint.semantic_graph");
    let second_bindings = collect_ts_direct_bindings(&second.db);
    let second_alias = binding_for_callsite(
        &second.db.stable_key_interner(),
        &second_bindings.bindings,
        "aliasTarget",
    );

    assert_ne!(
        first_digest, second_digest,
        "semantic graph output digest must change when tsconfig path alias resolution changes"
    );
    assert_ne!(
        second_alias.status,
        TsDirectBindingStatus::Resolved,
        "aliasTarget must stop resolving after the fixture path alias is broken: {second_alias:#?}"
    );
}

#[derive(Clone, Copy)]
struct TsDirectBindingExpectation {
    callsite: &'static str,
    kind: TsDirectBindingKind,
    status: TsDirectBindingStatus,
    reason: Option<TsDirectBindingReason>,
    target: Option<&'static str>,
    emits_copy_edge: bool,
}

fn ts_direct_binding_expectations() -> [TsDirectBindingExpectation; 9] {
    use TsDirectBindingKind::{
        CommonJsRequireMember, ImportedDefault, ImportedNamed, ImportedNamespaceMember, LocalAlias,
        LocalFunction, ReExportedAlias,
    };
    use TsDirectBindingReason::NonStringDynamicImport;
    use TsDirectBindingStatus::{Resolved, Unresolved};

    [
        TsDirectBindingExpectation {
            callsite: "localAlias",
            kind: LocalAlias,
            status: Resolved,
            reason: None,
            target: Some("localTarget"),
            emits_copy_edge: true,
        },
        TsDirectBindingExpectation {
            callsite: "namedAlias",
            kind: ImportedNamed,
            status: Resolved,
            reason: None,
            target: Some("namedTarget"),
            emits_copy_edge: true,
        },
        TsDirectBindingExpectation {
            callsite: "defaultTarget",
            kind: ImportedDefault,
            status: Resolved,
            reason: None,
            target: Some("defaultTarget"),
            emits_copy_edge: true,
        },
        TsDirectBindingExpectation {
            callsite: "ns.namespaceTarget",
            kind: ImportedNamespaceMember,
            status: Resolved,
            reason: None,
            target: Some("namespaceTarget"),
            emits_copy_edge: true,
        },
        TsDirectBindingExpectation {
            callsite: "viaBarrel",
            kind: ReExportedAlias,
            status: Resolved,
            reason: None,
            target: Some("namedTarget"),
            emits_copy_edge: true,
        },
        TsDirectBindingExpectation {
            callsite: "aliasTarget",
            kind: ImportedNamed,
            status: Resolved,
            reason: None,
            target: Some("aliasTarget"),
            emits_copy_edge: true,
        },
        TsDirectBindingExpectation {
            callsite: "cjsMember",
            kind: CommonJsRequireMember,
            status: Resolved,
            reason: None,
            target: Some("cjsTarget"),
            emits_copy_edge: true,
        },
        TsDirectBindingExpectation {
            callsite: "destructured",
            kind: LocalAlias,
            status: Resolved,
            reason: None,
            target: Some("localTarget"),
            emits_copy_edge: true,
        },
        TsDirectBindingExpectation {
            callsite: "import:<dynamic>",
            kind: LocalFunction,
            status: Unresolved,
            reason: Some(NonStringDynamicImport),
            target: None,
            emits_copy_edge: false,
        },
    ]
}

fn binding_for_callsite<'a>(
    interner: &crate::core::StableKeyInterner,
    bindings: &'a [TsDirectBindingFact],
    callsite: &str,
) -> &'a TsDirectBindingFact {
    bindings
        .iter()
        .find(|binding| {
            interner
                .resolve(binding.callsite_stable_key)
                .contains(callsite)
        })
        .unwrap_or_else(|| panic!("missing direct binding for callsite {callsite}: {bindings:#?}"))
}

fn assert_ts_direct_constraint(
    interner: &crate::core::StableKeyInterner,
    observed: &serde_json::Value,
    kind: &str,
    binding: &TsDirectBindingFact,
) {
    let constraints = observed["constraints"]
        .as_array()
        .expect("constraints array");
    assert!(
        constraints.iter().any(|row| {
            row["source"] == "ts_direct_binding"
                && row["kind"].as_str() == Some(kind)
                && row["stable_key"].as_str().is_some_and(|stable_key| {
                    stable_key.contains(
                        crate::analysis_api::compact_key_reference(
                            interner.resolve(binding.stable_key).as_ref(),
                        )
                        .as_ref(),
                    )
                })
        }),
        "missing TS direct-binding {kind} constraint for {}: {observed:#}",
        interner.resolve(binding.stable_key)
    );
}
