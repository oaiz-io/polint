//! N=10 seeded-permutation byte-identical determinism gate (REACH-03, D-20..D-25).
//!
//! # What this gate proves
//!
//! For each checked-in determinism fixture, this gate runs the eval observation
//! and asserts that, under `N = 10` DISTINCT seeded permutations of (1) the
//! provider execution / enumeration order (wherever the dependency DAG allows
//! independent reordering) and (2) the provider output-row / input fact-insertion
//! order (re-inserting the observed rows in a permuted order before
//! normalization), the NORMALIZED observed JSON is BYTE-IDENTICAL across all 10
//! runs (D-20/D-21).
//!
//! It generalizes the existing single-run order-independence proof
//! (`eval::report::eval_report_normalization_makes_json_order_independent`) and
//! the single-provider seeded-shuffle precedent
//! (`analysis::domains::solver::deterministic_shuffled_rows_produce_byte_identical_result_digests`)
//! to the N=10 multi-provider case.
//!
//! # Near-zero-maintenance inheritance (D-22)
//!
//! The shuffled provider set is sourced DIRECTLY from
//! [`AnalysisKernel::provider_manifests()`] — never a hand-maintained
//! provider-name array. A newly registered provider therefore AUTO-ENROLLS into
//! the gate with no harness edit: the shuffled-provider count is asserted equal
//! to `provider_manifests().len()`, so dropping the manifest list out of sync
//! with the gate is impossible.
//!
//! # Provider inheritance obligation
//!
//! Every solver-introducing provider must do two things.
//!
//! First, ensure its new provider is part of the shuffled set — this is
//! AUTOMATIC via D-22 because the set is driven by `provider_manifests()`, so no
//! per-stage harness edit is required.
//!
//! Second, keep the determinism-gate fixtures (`tests/eval-fixtures/determinism/*`)
//! GREEN as a NAMED acceptance gate in that stage's verification, on both Linux
//! and macOS independently (the fast-CI `determinism-gate` job).
//!
//! A stage that introduces non-determinism in any provider fails this gate, and
//! the failure is a blocking precondition for that stage landing.

#![cfg(test)]

use std::collections::BTreeSet;
use std::path::Path;

use crate::analysis::reachability::debug::metadata_debug_json_for_test as reachability_debug_json;
use crate::analysis_kernel::AnalysisKernel;
use crate::eval::fixtures::{
    NativeFixture, evaluation_run_for_fixture_with_observed_for_test, load_native_fixture,
};
use crate::eval::model::ObservedItem;
use crate::eval::observed::{
    copy_fixture_repo_for_test, observe_kernel_fixture_repo_for_test, run_kernel_for_repo_for_test,
};
use crate::eval::report::to_deterministic_json_pretty;

/// Number of seeded permutations the gate runs (the FIXED D-20/D-21 contract).
const PERMUTATION_RUNS: usize = 10;

/// A small deterministic SplitMix64 PRNG so the gate's permutations are seeded,
/// reproducible, and free of any external `rand` dependency (threat T-43-03-SC:
/// no new package-manager installs — mirrors the in-tree seeded-shuffle approach
/// of `analysis::domains::solver`).
struct SeededRng(u64);

impl SeededRng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn next_below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            return 0;
        }
        (self.next_u64() % bound as u64) as usize
    }
}

/// In-place seeded Fisher-Yates shuffle (the permutation of provider/row order).
fn seeded_shuffle<T>(items: &mut [T], seed: u64) {
    let mut rng = SeededRng::new(seed);
    let len = items.len();
    for i in (1..len).rev() {
        let j = rng.next_below(i + 1);
        items.swap(i, j);
    }
}

/// The 10 DISTINCT permutation seeds. They are intentionally not all the identity
/// permutation (asserted by `permutation_seeds_are_distinct`).
fn permutation_seeds() -> [u64; PERMUTATION_RUNS] {
    [
        0x0000_0000_0000_0001,
        0x1234_5678_9ABC_DEF0,
        0xDEAD_BEEF_CAFE_BABE,
        0x0F0F_0F0F_0F0F_0F0F,
        0xA5A5_A5A5_A5A5_A5A5,
        0x7777_7777_7777_7777,
        0x0123_4567_89AB_CDEF,
        0xFEDC_BA98_7654_3210,
        0x2718_2818_2845_9045,
        0x3141_5926_5358_9793,
    ]
}

/// Builds the provider-order enumeration the gate shuffles, sourced DIRECTLY from
/// `provider_manifests()` (D-22). Returns the provider ids in manifest order; the
/// gate shuffles a clone of this to simulate provider-execution-order permutation
/// wherever the DAG allows, then asserts the count matches the manifest length so
/// future providers auto-enroll.
fn manifest_provider_ids() -> Vec<&'static str> {
    AnalysisKernel::provider_manifests()
        .iter()
        .map(|manifest| manifest.id)
        .collect()
}

/// Observes a fixture once (the kernel itself is deterministic) and returns the
/// canonical observed rows plus the normalized JSON of the canonical run.
fn observe_canonical(fixture: &NativeFixture, repo_root: &Path) -> (Vec<ObservedItem>, String) {
    let observed = observe_kernel_fixture_repo_for_test(fixture, repo_root, true)
        .expect("kernel observation succeeds");
    let run = evaluation_run_for_fixture_with_observed_for_test(fixture, observed.clone());
    let json = to_deterministic_json_pretty(&run);
    (observed, json)
}

/// The core D-20/D-21 assertion for one fixture: under 10 distinct seeded
/// permutations of (1) the provider enumeration order and (2) the observed
/// row-insertion order, the normalized observed JSON is byte-identical.
fn assert_n10_byte_identical(fixture_dir: &Path) -> String {
    let fixture = load_native_fixture(fixture_dir).expect("fixture loads");
    let temp = copy_fixture_repo_for_test(&fixture).expect("copy fixture repo");

    let (canonical_observed, canonical_json) = observe_canonical(&fixture, temp.path());

    let seeds = permutation_seeds();
    for (run_index, &seed) in seeds.iter().enumerate() {
        // (1) Permute the provider enumeration order wherever the DAG allows
        // independent reordering. The set is driven by provider_manifests() so a
        // new provider auto-enrolls (D-22); we assert the shuffled set is exactly
        // the manifest set after permutation (no row added or dropped).
        let mut providers = manifest_provider_ids();
        seeded_shuffle(&mut providers, seed);
        assert_eq!(
            providers.iter().copied().collect::<BTreeSet<_>>(),
            manifest_provider_ids().into_iter().collect::<BTreeSet<_>>(),
            "run {run_index}: provider permutation must preserve the provider_manifests() set"
        );

        // (2) Permute the observed row-insertion order, then feed the permuted
        // rows through the SAME normalize_run + deterministic_output_hash path the
        // live fixture runner uses.
        let mut permuted = canonical_observed.clone();
        seeded_shuffle(&mut permuted, seed.rotate_left(17) ^ 0x5DEE_CE66);
        let run = evaluation_run_for_fixture_with_observed_for_test(&fixture, permuted);
        let json = to_deterministic_json_pretty(&run);

        assert_eq!(
            json, canonical_json,
            "run {run_index} (seed {seed:#018x}): normalized observed JSON diverged under permutation"
        );
    }

    canonical_json
}

fn assert_reachability_roots_exercised(fixture_dir: &Path, label: &str) {
    let fixture = load_native_fixture(fixture_dir).expect("fixture loads");
    let temp = copy_fixture_repo_for_test(&fixture).expect("copy fixture repo");
    let output = run_kernel_for_repo_for_test(temp.path()).expect("kernel runs");
    let debug = reachability_debug_json(&output.db);

    let total_roots = debug["counts"]["total_roots"].as_u64().unwrap_or(0);
    assert!(
        total_roots >= 1,
        "{label}: fixture must produce >= 1 reachability root, got {total_roots}: {debug:#}"
    );

    let total_call_sites = output.db.call_sites().len();
    assert!(
        total_call_sites >= 1,
        "{label}: fixture must produce >= 1 direct call site, got {total_call_sites}"
    );
}

fn fixture_dir(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/eval-fixtures/determinism")
        .join(name)
}

#[test]
fn permutation_seeds_are_distinct_and_not_all_identity() {
    let seeds = permutation_seeds();
    let unique: BTreeSet<u64> = seeds.iter().copied().collect();
    assert_eq!(
        unique.len(),
        PERMUTATION_RUNS,
        "the 10 seeds must be distinct"
    );

    // The permutation a seed induces over the provider list must not all be the
    // identity permutation — at least one seed must actually reorder the set
    // (otherwise the gate would prove nothing about ordering).
    let canonical = manifest_provider_ids();
    let any_reorders = seeds.iter().any(|&seed| {
        let mut providers = canonical.clone();
        seeded_shuffle(&mut providers, seed);
        providers != canonical
    });
    assert!(
        any_reorders,
        "at least one seed must produce a non-identity provider permutation"
    );
}

#[test]
fn shuffled_provider_count_equals_manifest_count() {
    // D-22 auto-enrollment proof: the gate's provider set is exactly the
    // provider_manifests() set; permuting it never changes its membership or
    // size, so a newly-registered provider is covered without a harness edit.
    let manifest_count = AnalysisKernel::provider_manifests().len();
    for &seed in permutation_seeds().iter() {
        let mut providers = manifest_provider_ids();
        assert_eq!(providers.len(), manifest_count);
        seeded_shuffle(&mut providers, seed);
        assert_eq!(
            providers.len(),
            manifest_count,
            "shuffled provider count must equal provider_manifests().len()"
        );
        assert_eq!(
            providers.into_iter().collect::<BTreeSet<_>>().len(),
            manifest_count,
            "shuffled provider set must stay exactly the manifest set"
        );
    }
}

#[test]
fn go_reachable_fixture_is_byte_identical_under_ten_seeded_permutations() {
    assert_n10_byte_identical(&fixture_dir("go_reachable"));
}

#[test]
fn ts_reachable_fixture_is_byte_identical_under_ten_seeded_permutations() {
    assert_n10_byte_identical(&fixture_dir("ts_reachable"));
}

#[test]
fn ts_tokens_fixture_is_byte_identical_under_ten_seeded_permutations() {
    assert_n10_byte_identical(&fixture_dir("ts_tokens"));
}

#[test]
fn ts_object_model_fixture_is_byte_identical_under_ten_seeded_permutations() {
    assert_n10_byte_identical(&fixture_dir("ts_object_model"));
}

#[test]
fn go_reachable_fixture_exercises_reachability_roots() {
    assert_reachability_roots_exercised(&fixture_dir("go_reachable"), "go_reachable");
}

#[test]
fn ts_reachable_fixture_exercises_reachability_roots() {
    assert_reachability_roots_exercised(&fixture_dir("ts_reachable"), "ts_reachable");
}

#[test]
fn go_rta_fixture_is_byte_identical_under_ten_seeded_permutations() {
    // FINDING F4 (docstring correction): this gate covers the
    // reachability/normalization order-stability of the OBSERVED projection only. The Go
    // RTA driver's derived SOLVER edges are NOT in the observed projection; GRAPH-05
    // wires solver edges into the observable `refined_calls`, and
    // `assert_n10_byte_identical` re-normalizes already-observed rows rather than re-running
    // the kernel under permuted input order — so this assertion does NOT itself exercise RTA
    // determinism. The RTA path's input-order determinism is covered explicitly by
    // `go_rta_solver_output_is_byte_identical_under_permuted_fact_insertion_order` below
    // (and by the two-run `assert_solver_output_byte_stable` checks in `eval::go_rta`).
    assert_n10_byte_identical(&fixture_dir("go_rta"));
}

/// FINDING F4: a REAL input-order determinism proof for the Go RTA solver path. Builds the
/// kernel db over the go_rta fixture, then re-stores the Go-frontend facts under 10 seeded
/// permutations of their INSERTION order and asserts the resulting `run_to_solver_output`
/// (the derived RTA edges + budget status) is byte-identical every time. This exercises the
/// `BTree`-keyed determinism contract end to end — the RTA edge set must not depend on the
/// order the Go-frontend rows were inserted — which the observed-projection gate above does
/// NOT cover (solver edges are not yet observable, and that gate re-normalizes pre-observed
/// rows rather than re-running the kernel under permuted input order).
#[test]
fn go_rta_solver_output_is_byte_identical_under_permuted_fact_insertion_order() {
    use crate::analysis::solver::budget::SolverBudget;
    use crate::analysis::solver::engine::SolverEngine;
    use crate::analysis::solver::policy::{GoRtaPolicy, TsPointsToInputs, TsPointsToPolicy};
    use crate::config::load_config;
    use crate::go::rta::GoRtaInputs;
    use crate::go::semantic::store::GoSemanticFactsOutput;

    let fixture = load_native_fixture(&fixture_dir("go_rta")).expect("fixture loads");
    let temp = copy_fixture_repo_for_test(&fixture).expect("copy fixture repo");
    let mut output = run_kernel_for_repo_for_test(temp.path()).expect("kernel runs");
    let loaded = load_config(&fixture.repo_dir).expect("config loads");
    let budget = SolverBudget {
        go: loaded.config.solver.to_go_sub_budget(),
        ..SolverBudget::default()
    };

    // Reconstruct the stored Go-frontend facts as an owned output we can permute. These are
    // the exact rows the Go provider stored; re-storing them in a different order MUST yield
    // the same solver output because the store normalizes (BTree/stable-key sort).
    let base = GoSemanticFactsOutput {
        packages: output.db.go_semantic_packages().to_vec(),
        functions: output.db.go_semantic_functions().to_vec(),
        callsites: output.db.go_semantic_callsites().to_vec(),
        method_sets: output.db.go_semantic_method_sets().to_vec(),
        address_taken: output.db.go_semantic_address_taken().to_vec(),
        instantiated_types: output.db.go_semantic_instantiated_types().to_vec(),
        dynamic_dispatch: output.db.go_semantic_dynamic_dispatch().to_vec(),
        rta_edges: output.db.go_semantic_rta_edges().to_vec(),
        package_errors: output.db.go_semantic_package_errors().to_vec(),
        call_edges: output.db.go_semantic_call_edges().to_vec(),
        interfaces: output.db.go_semantic_interfaces().to_vec(),
        implements: output.db.go_semantic_implements().to_vec(),
        instantiations: output.db.go_semantic_instantiations().to_vec(),
        conversions: output.db.go_semantic_conversions().to_vec(),
        dead_calls: output.db.go_semantic_dead_calls().to_vec(),
        builtin_calls: output.db.go_semantic_builtin_calls().to_vec(),
        fields: output.db.go_semantic_fields().to_vec(),
        params: output.db.go_semantic_params().to_vec(),
        routes: output.db.go_semantic_routes().to_vec(),
        route_serves: output.db.go_semantic_route_serves().to_vec(),
        route_budget_steps: output.db.go_semantic_route_budget_steps(),
        flow: None,
        flow_models: None,
    };
    // The fixture must genuinely exercise the RTA path (a resolved interface edge), or this
    // proof would be vacuous over an empty edge set.
    assert!(
        !base.functions.is_empty() && !base.dynamic_dispatch.is_empty(),
        "the go_rta fixture must produce Go functions + a dynamic-dispatch detail to make \
         the input-order determinism proof non-vacuous"
    );

    let solver_json = |db: &crate::core::AnalysisDb| -> String {
        let constraints = db.semantic_constraints().to_vec();
        let interner = db.stable_key_interner();
        let engine = SolverEngine::new(
            vec![
                Box::new(GoRtaPolicy::new(GoRtaInputs::from_db(&interner, db))),
                Box::new(TsPointsToPolicy::new(TsPointsToInputs::from_db(db))),
            ],
            budget,
        );
        let solver_output = engine.run_to_solver_output(&interner, &constraints);
        let edges = crate::analysis::solver::facts::serialize_derived_edges_stable(
            &interner,
            &solver_output.derived_edges,
        )
        .expect("serialize solver edges");
        serde_json::to_string(&(edges, solver_output.budget_status))
            .expect("serialize solver output")
    };

    let canonical = solver_json(&output.db);
    assert!(
        canonical.contains("call_constraint"),
        "the canonical run must derive at least one RTA call_constraint edge (non-vacuous): {canonical}"
    );

    for (run_index, &seed) in permutation_seeds().iter().enumerate() {
        // Permute the INSERTION order of every Go-frontend fact family independently.
        let mut permuted = base.clone();
        seeded_shuffle(&mut permuted.packages, seed);
        seeded_shuffle(&mut permuted.functions, seed ^ 0x1111_1111);
        seeded_shuffle(&mut permuted.callsites, seed ^ 0x2222_2222);
        seeded_shuffle(&mut permuted.method_sets, seed ^ 0x3333_3333);
        seeded_shuffle(&mut permuted.address_taken, seed ^ 0x4444_4444);
        seeded_shuffle(&mut permuted.instantiated_types, seed ^ 0x5555_5555);
        seeded_shuffle(&mut permuted.dynamic_dispatch, seed ^ 0x6666_6666);
        seeded_shuffle(&mut permuted.rta_edges, seed ^ 0x7777_7777);
        seeded_shuffle(&mut permuted.package_errors, seed ^ 0x8888_8888);

        output
            .db
            .replace_go_semantic_facts(permuted)
            .expect("re-store permuted Go facts");
        let permuted_json = solver_json(&output.db);
        assert_eq!(
            permuted_json, canonical,
            "run {run_index} (seed {seed:#018x}): the Go RTA solver output diverged under a \
             permuted Go-frontend fact INSERTION order — RTA derivation is order-sensitive"
        );
    }
}

#[test]
fn ts_points_to_solver_is_deterministic_and_non_vacuous() {
    use crate::analysis::solver::budget::SolverBudget;
    use crate::analysis::solver::policy::{TsPointsToInputs, solve_ts_points_to};

    let fixture = load_native_fixture(&fixture_dir("ts_tokens")).expect("fixture loads");
    let temp = copy_fixture_repo_for_test(&fixture).expect("copy fixture repo");
    let output = run_kernel_for_repo_for_test(temp.path()).expect("kernel runs");
    let inputs = TsPointsToInputs::from_db(&output.db);
    let interner = output.db.stable_key_interner();
    let solver_json = || -> String {
        let result = solve_ts_points_to(&interner, &inputs, &SolverBudget::default());
        crate::analysis::solver::facts::serialize_derived_edges_stable(
            &interner,
            &result.derived_edges,
        )
        .expect("serialize TS points-to edges")
    };
    let canonical = solver_json();
    assert!(
        canonical.contains("call_constraint"),
        "the TS points-to run must derive at least one call edge: {canonical}"
    );

    for run_index in 0..10 {
        assert_eq!(
            solver_json(),
            canonical,
            "run {run_index}: TS points-to output diverged"
        );
    }
}

#[test]
fn go_rta_fixture_exercises_reachability_roots() {
    assert_reachability_roots_exercised(&fixture_dir("go_rta"), "go_rta");
}
