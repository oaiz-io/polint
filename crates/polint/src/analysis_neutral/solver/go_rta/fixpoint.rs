//! The Go RTA fixpoint.
//!
//! [`solve_rta`] runs Rapid Type Analysis over a closed [`RtaInputs`] snapshot. It uses
//! `BTreeMap` and `BTreeSet` ordering, a globally monotonic `solver_step`, and assigns
//! dense IDs only after stable-key sorting through [`SolverOutput::normalized`].
//!
//! The fixpoint iterates **reachability ⊗ dispatch**:
//!
//! 1. Seed reachable functions from the reachability roots.
//! 2. Seed instantiated runtime types and address-taken functions from the Go frontend.
//!    These sets cover the reachable program because the sidecar builds SSA over
//!    `ssautil.AllPackages` and harvests `MakeInterface` and `MakeClosure` instructions.
//!    The facts have no per-function attribution, so filtering them further could drop
//!    valid RTA targets. Interface invokes still resolve only to instantiated receiver
//!    types.
//! 3. Process each newly reachable caller once. Static calls extend reachability without
//!    emitting derived edges. Dynamic callsites resolve through [`resolve_callsite`],
//!    emit their edges, and add newly resolved callees to the next frontier. Iteration
//!    stops when no new function becomes reachable.
//!
//! Three independent limits bound the work: `budget.go.max_rta_rounds` for dynamic
//! dispatch waves, `budget.go.max_worklist_steps` for callsite resolutions and static
//! edge expansions, and `budget.go.max_candidates_per_callsite` for fan-out. An
//! oversized address-taken set disables function-value resolution only when a reachable
//! signature-bearing callsite needs it; interface dispatch remains unaffected.
//!
//! Static reachability does not consume the dynamic-round budget. It is bounded by the
//! worklist-step limit, allowing deep finite static chains to reach their dynamic
//! callsites. A budget status is latched only when a limit actually prevents pending
//! work, and edges resolved before the limit retain their honest status.
//!
//! Processing only the frontier keeps the step count proportional to actual work.
//! Because instantiated and address-taken sets do not grow during the fixpoint, resolving
//! each caller once yields the same edge set as repeatedly scanning every reachable
//! caller.

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis_neutral::solver::budget::{BudgetReason, BudgetStatus, SolverBudget};
use crate::analysis_neutral::solver::facts::DerivedEdgeFact;
use crate::analysis_neutral::solver::store::SolverOutput;

use super::dispatch::resolve_callsite;
use super::snapshot::RtaInputs;

/// Run the RTA fixpoint over the closed snapshot, returning a normalized
/// [`SolverOutput`]. See the module docs for the model + budget.
pub(crate) fn solve_rta(
    interner: &crate::internal_core::StableKeyInterner,
    inputs: &RtaInputs,
    budget: &SolverBudget,
) -> SolverOutput {
    // Index callsites by caller for deterministic, efficient per-round scanning.
    let mut callsites_by_caller: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, callsite) in inputs.callsites.iter().enumerate() {
        callsites_by_caller
            .entry(callsite.caller.as_str())
            .or_default()
            .push(index);
    }

    // Reverse index `SemanticNodeId -> qualified` so a resolved callee node maps back
    // to its identity in O(1), built once (no per-edge linear scan of the function
    // index). Deterministic: the source map is BTree-keyed.
    let node_to_qualified: BTreeMap<crate::analysis_neutral::ids::SemanticNodeId, &str> = inputs
        .function_node
        .iter()
        .map(|(qualified, &node)| (node, qualified.as_str()))
        .collect();

    // Seed the reachable set from roots. The instantiated + address-taken sets
    // are the whole reachable program's rapid-type / address-taken sets (see module
    // docs); they do not grow during the fixpoint, so the loop is bounded by reachable
    // growth alone (plus the explicit caps). The instantiated-type FILTER is baked into
    // `inputs.interface_candidate_index` at build time, so the fixpoint no longer
    // threads the instantiated set into `resolve_callsite` — it consults the index.
    let mut reachable: BTreeSet<String> = inputs.roots.clone();
    let address_taken = &inputs.address_taken;

    // The FRONTIER: callers whose callsites still need resolving THIS round. Round 1's
    // frontier is the roots; each subsequent round's frontier is the callees newly
    // reached in the prior round. A caller is resolved exactly once, so the step count
    // tracks real work rather than re-scanning the whole reachable set every round
    // while preserving deterministic BTreeSet order.
    let mut frontier: BTreeSet<String> = inputs.roots.clone();

    // Membership-only dedup keyed by StableKeyId (the store rejects duplicate stable
    // keys; the same callee resolved in different rounds yields an identical stable
    // key, so we keep the first occurrence — its `solver_step` is the earliest, a
    // stable monotonic witness). Iteration order is NOT load-bearing: `finish`
    // always runs `SolverOutput::normalized` (resolved-text sort).
    let mut edges_by_key: std::collections::HashMap<
        crate::internal_core::StableKeyId,
        DerivedEdgeFact,
    > = std::collections::HashMap::new();

    let mut budget_exceeded = false;
    let mut budget_reasons = BTreeSet::new();
    // GLOBAL monotonic step counter: increments once per callsite resolution
    // across the WHOLE run and is never reset, so every derived edge's `solver_step`
    // is globally monotonic.
    let mut solver_step: u64 = 0;
    // DISPATCH-round counter: the round cap bounds GENUINE dynamic-dispatch
    // re-iteration, NOT static-call-graph DEPTH. Only a round that actually (re)resolved
    // a dynamic callsite — and could therefore grow the resolved-edge set — counts toward
    // `max_rta_rounds`. Pure static-reachability growth does NOT consume the round budget;
    // it runs to completion bounded solely by `max_worklist_steps`. Counting static-BFS
    // levels here would make `round` equal the static call-graph depth, so
    // any first-party function whose shortest static-call path from a root exceeded the cap
    // had its dispatch silently dropped AND the run falsely latched `BudgetExceeded`.
    // Termination does not depend on this cap: `reachable` grows monotonically and is
    // bounded by the finite function set, and every static-edge expansion + callsite
    // resolution consumes one `max_worklist_steps` step (the real runaway guard).
    let mut dispatch_rounds: usize = 0;

    // Address-taken threshold: a pathologically large func-value
    // surface disables FUNC-VALUE (signature) resolution rather than exploding the
    // candidate search — but it is SCOPED to func-value resolution only. Pure interface
    // dispatch never consults the address-taken set, so it must STILL proceed; aborting
    // the whole fixpoint here would drop real interface-invoke edges that have nothing to
    // do with the func-value surface. The set is seeded once and never grows, so this is
    // loop-invariant, so evaluate it once.
    let resolve_func_values = address_taken.len() <= budget.go.address_taken_threshold;
    // Latch BudgetExceeded for disabled func-value resolution ONLY when the fixpoint reaches
    // a signature-bearing callsite and therefore actually skips work. An unreachable
    // func-value callsite must not taint the run-level budget status.

    while !frontier.is_empty() {
        // Resolve only the callsites of THIS round's frontier (the callers newly
        // reached in the prior round, or the roots in round 1). Collect callees that
        // become newly reachable — they are the next round's frontier. Track whether this
        // round resolved any dynamic callsite, so only genuine dispatch waves count toward
        // the round cap.
        let mut newly_reachable: BTreeSet<String> = BTreeSet::new();
        let mut resolved_dispatch_this_round = false;
        for caller in &frontier {
            // Static-call edges GROW reachability: a function reached only via
            // a direct (static) call that is not itself a root must still enter the
            // worklist so dispatch inside it is resolved (standard RTA = closure over
            // static-call ⊗ dynamic-dispatch edges from roots). Static edges do NOT emit
            // a DerivedEdgeFact — only dynamic-dispatch resolution does. Each static-edge
            // expansion consumes one worklist step too (caps must account for deep
            // static chains), bounded by `max_worklist_steps`.
            if let Some(static_callees) = inputs.static_call_targets.get(caller.as_str()) {
                for callee in static_callees {
                    solver_step += 1;
                    if solver_step > budget.go.max_worklist_steps as u64 {
                        budget_reasons
                            .insert(BudgetReason::GoMaxWorklistSteps.as_str().to_string());
                        return finish(interner, edges_by_key, true, budget_reasons);
                    }
                    if !reachable.contains(callee) {
                        newly_reachable.insert(callee.clone());
                    }
                }
            }

            let Some(callsite_indices) = callsites_by_caller.get(caller.as_str()) else {
                continue;
            };
            // Round cap: bound only genuine dynamic-dispatch re-iteration.
            // Static-call expansion above is independent and remains governed by
            // `go.max_worklist_steps`; the dispatch cap must never block static
            // reachability closure or falsely latch when only static work remains.
            if dispatch_rounds >= budget.go.max_rta_rounds {
                budget_exceeded = true;
                budget_reasons.insert(BudgetReason::GoMaxRtaRounds.as_str().to_string());
                continue;
            }
            // This caller has at least one dynamic callsite: the round performs genuine
            // dispatch resolution, so it counts toward the round cap.
            resolved_dispatch_this_round = true;
            for &index in callsite_indices {
                // Worklist-step cap: one callsite resolution is one step,
                // bounded by the Go-scaled `max_worklist_steps` (10_000), NOT the
                // policy-count `max_outer_iterations` (64).
                solver_step += 1;
                if solver_step > budget.go.max_worklist_steps as u64 {
                    // Edges resolved before the cap keep their honest status.
                    budget_reasons.insert(BudgetReason::GoMaxWorklistSteps.as_str().to_string());
                    return finish(interner, edges_by_key, true, budget_reasons);
                }

                let callsite = &inputs.callsites[index];
                if !resolve_func_values
                    && callsite.interface_method.is_none()
                    && callsite.signature.is_some()
                {
                    budget_exceeded = true;
                    budget_reasons
                        .insert(BudgetReason::GoAddressTakenThreshold.as_str().to_string());
                }
                let resolution = resolve_callsite(
                    interner,
                    callsite,
                    inputs,
                    address_taken,
                    budget.go.max_candidates_per_callsite,
                    resolve_func_values,
                    solver_step,
                );
                if resolution.candidate_cap_exceeded {
                    budget_exceeded = true;
                    budget_reasons.insert(
                        BudgetReason::GoMaxCandidatesPerCallsite
                            .as_str()
                            .to_string(),
                    );
                }
                for edge in resolution.edges {
                    // A newly-resolved callee becomes reachable, so its own callsites
                    // are resolved next round (the fixpoint). Map the target node back
                    // to a qualified identity via the prebuilt reverse index.
                    if let Some(&callee) = node_to_qualified.get(&edge.target)
                        && !reachable.contains(callee)
                    {
                        newly_reachable.insert(callee.to_string());
                    }
                    edges_by_key.entry(edge.stable_key).or_insert(edge);
                }
            }
        }

        // Only a round that genuinely resolved dynamic dispatch counts toward the round
        // cap; a pure static-BFS round leaves `dispatch_rounds` untouched so deep
        // static chains do not consume the dispatch budget.
        if resolved_dispatch_this_round {
            dispatch_rounds += 1;
        }

        // The callees newly reached this round are the next round's frontier; fold them
        // into the reachable set so they are never re-resolved.
        reachable.extend(newly_reachable.iter().cloned());
        frontier = newly_reachable;
    }

    finish(interner, edges_by_key, budget_exceeded, budget_reasons)
}

/// Assemble the normalized output from the deduplicated edge accumulator + the
/// run-level budget status.
fn finish(
    interner: &crate::internal_core::StableKeyInterner,
    edges_by_key: std::collections::HashMap<crate::internal_core::StableKeyId, DerivedEdgeFact>,
    budget_exceeded: bool,
    budget_reasons: BTreeSet<String>,
) -> SolverOutput {
    let budget_status = if budget_exceeded {
        BudgetStatus::BudgetExceeded
    } else {
        BudgetStatus::WithinBudget
    };
    SolverOutput {
        derived_edges: edges_by_key.into_values().collect(),
        budget_status,
        budget_reasons,
        incomplete_sites: 0,
    }
    .normalized(interner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    use crate::analysis_neutral::ids::SemanticNodeId;
    use crate::analysis_neutral::points_to::facts::PointsToStatus;
    use crate::analysis_neutral::solver::go_rta::snapshot::{RtaCallsite, RtaInputs, RtaMethod};

    /// Build a small interface-dispatch scenario: a `main` caller, an interface invoke
    /// of method `Read` at a callsite, and a concrete type `pkg.File` with a `Read`
    /// method. Whether `pkg.File` is instantiated is the RTA discriminant.
    ///
    /// Nodes: main=1, callsite=2, File.Read=3, Buf.Read=4.
    fn interface_scenario(instantiated_types: &[&str]) -> RtaInputs {
        let interner = crate::internal_core::test_stable_key_interner();
        let mut function_node = BTreeMap::new();
        function_node.insert("main.main".to_string(), SemanticNodeId(1));
        function_node.insert("(pkg.File).Read".to_string(), SemanticNodeId(3));
        function_node.insert("(pkg.Buf).Read".to_string(), SemanticNodeId(4));

        let mut method_sets = BTreeMap::new();
        method_sets.insert(
            "pkg.File".to_string(),
            BTreeSet::from(["Read".to_string(), "Close".to_string()]),
        );
        method_sets.insert("pkg.Buf".to_string(), BTreeSet::from(["Read".to_string()]));

        let mut method_set_keys = BTreeMap::new();
        method_set_keys.insert("pkg.File".to_string(), interner.intern("ms|pkg.File"));
        method_set_keys.insert("pkg.Buf".to_string(), interner.intern("ms|pkg.Buf"));

        let mut methods_by_receiver = BTreeMap::new();
        methods_by_receiver.insert(
            "pkg.File".to_string(),
            vec![RtaMethod {
                method_name: "Read".to_string(),
                qualified: "(pkg.File).Read".to_string(),
                node: SemanticNodeId(3),
            }],
        );
        methods_by_receiver.insert(
            "pkg.Buf".to_string(),
            vec![RtaMethod {
                method_name: "Read".to_string(),
                qualified: "(pkg.Buf).Read".to_string(),
                node: SemanticNodeId(4),
            }],
        );

        let mut instantiated = BTreeSet::new();
        let mut instantiated_keys = BTreeMap::new();
        for type_name in instantiated_types {
            instantiated.insert((*type_name).to_string());
            instantiated_keys.insert(
                (*type_name).to_string(),
                interner.intern(format!("inst|{type_name}")),
            );
        }

        RtaInputs {
            roots: BTreeSet::from(["main.main".to_string()]),
            callsites: vec![RtaCallsite {
                caller: "main.main".to_string(),
                callsite_node: SemanticNodeId(2),
                callsite_stable_key: interner.intern("cs|main:read"),
                interface_method: Some("Read".to_string()),
                signature: None,
                dispatch_stable_key: interner.intern("dd|main:read"),
            }],
            method_sets,
            method_set_keys,
            instantiated,
            instantiated_keys,
            function_node,
            methods_by_receiver,
            ..RtaInputs::default()
        }
        // derive the interface-dispatch index from the hand-built primary fields so
        // dispatch resolution (which now reads the index) behaves as the scan would have.
        .finalize_indexes()
    }

    #[test]
    fn interface_invoke_resolves_only_instantiated_receiver() {
        // pkg.File is instantiated → exactly the pkg.File.Read edge is derived; pkg.Buf
        // is NOT instantiated → no Buf.Read edge (the RTA filter).
        let inputs = interface_scenario(&["pkg.File"]);
        let output = solve_rta(
            &crate::internal_core::test_stable_key_interner(),
            &inputs,
            &SolverBudget::default(),
        );

        assert_eq!(output.budget_status, BudgetStatus::WithinBudget);
        // main(1) -> File.Read(3) resolved; main -> Buf.Read(4) NOT resolved.
        assert!(
            output
                .derived_edges
                .iter()
                .any(|e| e.source == SemanticNodeId(1) && e.target == SemanticNodeId(3)),
            "instantiated receiver's method must be resolved"
        );
        assert!(
            !output
                .derived_edges
                .iter()
                .any(|e| e.target == SemanticNodeId(4)),
            "a NON-instantiated receiver's method must NOT be resolved (RTA filter)"
        );
        // The resolved edge is honest: Present status, never-exact precision, and its
        // provenance lists the producing CallConstraint + the contributing facts.
        let edge = output
            .derived_edges
            .iter()
            .find(|e| e.target == SemanticNodeId(3))
            .expect("File.Read edge");
        assert_eq!(edge.status, PointsToStatus::Present);
        assert!(edge.honors_precision_ceiling());
        assert_eq!(edge.provenance.constraint_kind, "call_constraint");
        // callsite + dispatch + method-set + instantiated-type facts.
        assert_eq!(edge.provenance.contributing_len(), 4);
    }

    #[test]
    fn static_call_edge_brings_non_root_caller_into_worklist_resolving_its_dispatch() {
        let interner = crate::internal_core::test_stable_key_interner();
        // `main` (the only root) makes a DIRECT (static) call to `run`, a
        // non-root helper. The interface invoke `s.Speak()` lives in `run`, not in main.
        // RTA reachability must close over the static main -> run edge so run enters the
        // worklist and its dispatch resolves to (pkg.File).Read. Without static-edge
        // growth, run is never scanned and the edge is silently dropped.
        let mut function_node = BTreeMap::new();
        function_node.insert("main.main".to_string(), SemanticNodeId(1));
        function_node.insert("main.run".to_string(), SemanticNodeId(7));
        function_node.insert("(pkg.File).Read".to_string(), SemanticNodeId(3));

        let mut method_sets = BTreeMap::new();
        method_sets.insert("pkg.File".to_string(), BTreeSet::from(["Read".to_string()]));
        let mut method_set_keys = BTreeMap::new();
        method_set_keys.insert("pkg.File".to_string(), interner.intern("ms|pkg.File"));

        let mut methods_by_receiver = BTreeMap::new();
        methods_by_receiver.insert(
            "pkg.File".to_string(),
            vec![RtaMethod {
                method_name: "Read".to_string(),
                qualified: "(pkg.File).Read".to_string(),
                node: SemanticNodeId(3),
            }],
        );

        let instantiated = BTreeSet::from(["pkg.File".to_string()]);
        let mut instantiated_keys = BTreeMap::new();
        instantiated_keys.insert("pkg.File".to_string(), interner.intern("inst|pkg.File"));

        // The dispatch obligation belongs to `run` (NOT main); main only statically calls run.
        let mut static_call_targets = BTreeMap::new();
        static_call_targets.insert(
            "main.main".to_string(),
            BTreeSet::from(["main.run".to_string()]),
        );

        let inputs = RtaInputs {
            roots: BTreeSet::from(["main.main".to_string()]),
            callsites: vec![RtaCallsite {
                caller: "main.run".to_string(),
                callsite_node: SemanticNodeId(2),
                callsite_stable_key: interner.intern("cs|run:read"),
                interface_method: Some("Read".to_string()),
                signature: None,
                dispatch_stable_key: interner.intern("dd|run:read"),
            }],
            method_sets,
            method_set_keys,
            instantiated,
            instantiated_keys,
            function_node,
            methods_by_receiver,
            static_call_targets,
            ..RtaInputs::default()
        }
        .finalize_indexes();

        let output = solve_rta(
            &crate::internal_core::test_stable_key_interner(),
            &inputs,
            &SolverBudget::default(),
        );
        assert_eq!(output.budget_status, BudgetStatus::WithinBudget);
        // run(7) -> File.Read(3): the dispatch in the statically-reached helper resolves.
        assert!(
            output
                .derived_edges
                .iter()
                .any(|e| e.source == SemanticNodeId(7) && e.target == SemanticNodeId(3)),
            "dispatch inside a statically-reached non-root function must resolve: {:#?}",
            output.derived_edges
        );
        // Static edges emit NO derived edge themselves: there is no main(1) -> run(7) edge.
        assert!(
            !output
                .derived_edges
                .iter()
                .any(|e| e.target == SemanticNodeId(7)),
            "static-call edges grow reachability only; they must not emit a derived edge"
        );
    }

    #[test]
    fn deep_static_chain_with_bottom_dispatch_converges_at_default_budget() {
        let interner = crate::internal_core::test_stable_key_interner();
        // The round cap bounds genuine dispatch re-iteration, not static-call-graph
        // depth. A 200-deep static chain whose bottom function performs dynamic dispatch
        // must converge within the default budget and resolve the deep edge. Static
        // reachability is bounded by `max_worklist_steps`, while this graph consumes only
        // one dynamic dispatch round.
        const CHAIN: usize = 200;
        let qualified = |i: usize| -> String {
            if i == 0 {
                "main.main".to_string()
            } else {
                format!("pkg.s{i}")
            }
        };

        // Node ids: chain function i -> SemanticNodeId(i+1); the dispatch target gets a
        // distinct high id so it cannot collide with a chain node.
        const TARGET_NODE: u64 = 100_000;
        let mut function_node = BTreeMap::new();
        let mut static_call_targets: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for i in 0..=CHAIN {
            function_node.insert(qualified(i), SemanticNodeId((i + 1) as u64));
        }
        for i in 0..CHAIN {
            static_call_targets
                .entry(qualified(i))
                .or_default()
                .insert(qualified(i + 1));
        }
        // The bottom of the chain (s200) is a method-bearing function whose body holds a
        // dynamic interface invoke of `Read`, resolving to the instantiated (pkg.File).Read.
        function_node.insert("(pkg.File).Read".to_string(), SemanticNodeId(TARGET_NODE));

        let mut method_sets = BTreeMap::new();
        method_sets.insert("pkg.File".to_string(), BTreeSet::from(["Read".to_string()]));
        let mut method_set_keys = BTreeMap::new();
        method_set_keys.insert("pkg.File".to_string(), interner.intern("ms|pkg.File"));

        let mut methods_by_receiver = BTreeMap::new();
        methods_by_receiver.insert(
            "pkg.File".to_string(),
            vec![RtaMethod {
                method_name: "Read".to_string(),
                qualified: "(pkg.File).Read".to_string(),
                node: SemanticNodeId(TARGET_NODE),
            }],
        );

        let instantiated = BTreeSet::from(["pkg.File".to_string()]);
        let mut instantiated_keys = BTreeMap::new();
        instantiated_keys.insert("pkg.File".to_string(), interner.intern("inst|pkg.File"));

        let inputs = RtaInputs {
            roots: BTreeSet::from(["main.main".to_string()]),
            // The dynamic dispatch lives in the DEEPEST statically-reached function.
            callsites: vec![RtaCallsite {
                caller: qualified(CHAIN),
                callsite_node: SemanticNodeId(900_000),
                callsite_stable_key: interner.intern("cs|deep:read"),
                interface_method: Some("Read".to_string()),
                signature: None,
                dispatch_stable_key: interner.intern("dd|deep:read"),
            }],
            method_sets,
            method_set_keys,
            instantiated,
            instantiated_keys,
            methods_by_receiver,
            function_node,
            static_call_targets,
            ..RtaInputs::default()
        }
        .finalize_indexes();

        // THE DEFAULT BUDGET — no inflated `max_rta_rounds`. The default `max_rta_rounds`
        // (32) is far below the chain depth (200), yet static closure must converge.
        let output = solve_rta(
            &crate::internal_core::test_stable_key_interner(),
            &inputs,
            &SolverBudget::default(),
        );

        // (a) The deep dispatch edge resolves: s200 -> (pkg.File).Read.
        assert!(
            output
                .derived_edges
                .iter()
                .any(|e| e.source == SemanticNodeId((CHAIN + 1) as u64)
                    && e.target == SemanticNodeId(TARGET_NODE)),
            "the dispatch at the bottom of a deep static chain must resolve at the DEFAULT \
             budget: {:#?}",
            output.derived_edges
        );
        // (b) The run converged honestly — a deep first-party static chain is NOT a runaway.
        assert_eq!(
            output.budget_status,
            BudgetStatus::WithinBudget,
            "a deep finite static chain (depth {CHAIN} >> default max_rta_rounds) must \
             converge WithinBudget at the DEFAULT budget, not latch exhaustion"
        );
    }

    #[test]
    fn pure_deep_static_chain_converges_at_default_budget() {
        // A deep-but-finite STATIC chain with NO dispatch anywhere must
        // also converge WithinBudget AT THE DEFAULT budget. Static-edge expansion consumes
        // the worklist-step budget (10_000, comfortably admitting the chain) and grows no
        // dispatch waves, so the round cap is never approached. No dynamic callsites → no
        // derived edges.
        const CHAIN: usize = 200;
        let qualified = |i: usize| -> String {
            if i == 0 {
                "main.main".to_string()
            } else {
                format!("pkg.s{i}")
            }
        };

        let mut function_node = BTreeMap::new();
        let mut static_call_targets: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for i in 0..=CHAIN {
            function_node.insert(qualified(i), SemanticNodeId((i + 1) as u64));
        }
        for i in 0..CHAIN {
            static_call_targets
                .entry(qualified(i))
                .or_default()
                .insert(qualified(i + 1));
        }

        let inputs = RtaInputs {
            roots: BTreeSet::from(["main.main".to_string()]),
            function_node,
            static_call_targets,
            ..RtaInputs::default()
        }
        .finalize_indexes();

        let output = solve_rta(
            &crate::internal_core::test_stable_key_interner(),
            &inputs,
            &SolverBudget::default(),
        );
        assert_eq!(
            output.budget_status,
            BudgetStatus::WithinBudget,
            "a deep finite static chain must converge WithinBudget at the DEFAULT budget"
        );
        // No dynamic callsites anywhere, so no edges are derived (static edges emit none).
        assert!(output.derived_edges.is_empty());
    }

    #[test]
    fn type_not_in_instantiated_set_derives_no_edge() {
        // No type instantiated at all → the interface invoke resolves to nothing
        // (honest unresolved, not a fabricated edge).
        let inputs = interface_scenario(&[]);
        let output = solve_rta(
            &crate::internal_core::test_stable_key_interner(),
            &inputs,
            &SolverBudget::default(),
        );
        assert!(output.derived_edges.is_empty());
        assert_eq!(output.budget_status, BudgetStatus::WithinBudget);
    }

    #[test]
    fn func_value_resolves_via_address_taken_by_signature() {
        let interner = crate::internal_core::test_stable_key_interner();
        // A func-value callsite resolves to address-taken functions whose signature
        // matches. handler (addr-taken, sig "func()") matches; other (sig "func(int)")
        // does not.
        let mut function_node = BTreeMap::new();
        function_node.insert("main.main".to_string(), SemanticNodeId(1));
        function_node.insert("pkg.handler".to_string(), SemanticNodeId(3));
        function_node.insert("pkg.other".to_string(), SemanticNodeId(4));

        let mut function_signature = BTreeMap::new();
        function_signature.insert("pkg.handler".to_string(), "func()".to_string());
        function_signature.insert("pkg.other".to_string(), "func(int)".to_string());

        let mut address_taken = BTreeSet::new();
        address_taken.insert("pkg.handler".to_string());
        address_taken.insert("pkg.other".to_string());
        let mut address_taken_keys = BTreeMap::new();
        address_taken_keys.insert("pkg.handler".to_string(), interner.intern("at|pkg.handler"));
        address_taken_keys.insert("pkg.other".to_string(), interner.intern("at|pkg.other"));

        let inputs = RtaInputs {
            roots: BTreeSet::from(["main.main".to_string()]),
            callsites: vec![RtaCallsite {
                caller: "main.main".to_string(),
                callsite_node: SemanticNodeId(2),
                callsite_stable_key: interner.intern("cs|main:fv"),
                interface_method: None,
                signature: Some("func()".to_string()),
                dispatch_stable_key: interner.intern("dd|main:fv"),
            }],
            address_taken,
            address_taken_keys,
            function_node,
            function_signature,
            ..RtaInputs::default()
        }
        .finalize_indexes();

        let output = solve_rta(
            &crate::internal_core::test_stable_key_interner(),
            &inputs,
            &SolverBudget::default(),
        );
        assert!(
            output
                .derived_edges
                .iter()
                .any(|e| e.source == SemanticNodeId(1) && e.target == SemanticNodeId(3)),
            "the signature-matching address-taken function must resolve"
        );
        assert!(
            !output
                .derived_edges
                .iter()
                .any(|e| e.target == SemanticNodeId(4)),
            "a signature-mismatching function must NOT resolve"
        );
    }

    #[test]
    fn honest_unresolved_when_method_set_has_no_match() {
        // The instantiated type's method-set does NOT contain the invoked method →
        // no edge (honest unresolved).
        let mut inputs = interface_scenario(&["pkg.File"]);
        // The callsite invokes a method nobody declares.
        inputs.callsites[0].interface_method = Some("Frobnicate".to_string());
        let output = solve_rta(
            &crate::internal_core::test_stable_key_interner(),
            &inputs,
            &SolverBudget::default(),
        );
        assert!(output.derived_edges.is_empty());
    }

    #[test]
    fn deleting_a_contributing_fact_invalidates_the_rta_edge() {
        // The resolved edge depends on its contributing instantiated-type
        // method-set, callsite, and dispatch facts. Removing the instantiated-type fact
        // (i.e. the type is no longer instantiated) does NOT reproduce the SAME edge.
        let baseline = solve_rta(
            &crate::internal_core::test_stable_key_interner(),
            &interface_scenario(&["pkg.File"]),
            &SolverBudget::default(),
        );
        let edge = baseline
            .derived_edges
            .iter()
            .find(|e| e.target == SemanticNodeId(3))
            .expect("baseline File.Read edge");
        let baseline_key = edge.stable_key;

        // Remove pkg.File from the instantiated set (delete the contributing fact).
        let without_instantiated = solve_rta(
            &crate::internal_core::test_stable_key_interner(),
            &interface_scenario(&[]),
            &SolverBudget::default(),
        );
        assert!(
            !without_instantiated
                .derived_edges
                .iter()
                .any(|e| e.stable_key == baseline_key),
            "deleting the instantiated-type contributing fact must invalidate THAT edge"
        );

        // Removing the method-set match (the type no longer declares the method) also
        // invalidates the edge. Mutating a primary field requires rebuilding the derived
        // dispatch index — exactly what re-running `from_db` would do.
        let mut no_method_set = interface_scenario(&["pkg.File"]);
        no_method_set
            .method_sets
            .get_mut("pkg.File")
            .unwrap()
            .remove("Read");
        let no_method_set = no_method_set.finalize_indexes();
        let rerun = solve_rta(
            &crate::internal_core::test_stable_key_interner(),
            &no_method_set,
            &SolverBudget::default(),
        );
        assert!(
            !rerun
                .derived_edges
                .iter()
                .any(|e| e.stable_key == baseline_key),
            "deleting the method-set contributing fact must invalidate THAT edge"
        );
    }

    #[test]
    fn solve_rta_is_shuffle_stable() {
        // BTree-keyed inputs make the output insensitive to insertion order. Build the
        // same scenario via two different instantiated-type insertion orders and assert
        // byte-identical normalized output.
        let forward = solve_rta(
            &crate::internal_core::test_stable_key_interner(),
            &interface_scenario(&["pkg.File", "pkg.Buf"]),
            &SolverBudget::default(),
        );
        let reversed = solve_rta(
            &crate::internal_core::test_stable_key_interner(),
            &interface_scenario(&["pkg.Buf", "pkg.File"]),
            &SolverBudget::default(),
        );

        let interner = crate::internal_core::test_stable_key_interner();
        let forward_json = crate::analysis_neutral::solver::facts::serialize_derived_edges_stable(
            &interner,
            &forward.derived_edges,
        )
        .expect("serialize forward");
        let reversed_json = crate::analysis_neutral::solver::facts::serialize_derived_edges_stable(
            &interner,
            &reversed.derived_edges,
        )
        .expect("serialize reversed");
        assert_eq!(forward_json, reversed_json);
    }

    #[test]
    fn runaway_dispatch_latches_budget_exceeded_not_unbounded() {
        let interner = crate::internal_core::test_stable_key_interner();
        // A deliberately tight round cap forces the fixpoint to latch BudgetExceeded
        // rather than loop unbounded. Build a reachable chain main -> a -> b so
        // resolution needs more than one round, then cap rounds at 1.
        let mut function_node = BTreeMap::new();
        function_node.insert("main.main".to_string(), SemanticNodeId(1));
        function_node.insert("(pkg.A).Step".to_string(), SemanticNodeId(3));
        function_node.insert("(pkg.B).Step".to_string(), SemanticNodeId(5));

        let mut method_sets = BTreeMap::new();
        method_sets.insert("pkg.A".to_string(), BTreeSet::from(["Step".to_string()]));
        method_sets.insert("pkg.B".to_string(), BTreeSet::from(["Step".to_string()]));
        let mut method_set_keys = BTreeMap::new();
        method_set_keys.insert("pkg.A".to_string(), interner.intern("ms|A"));
        method_set_keys.insert("pkg.B".to_string(), interner.intern("ms|B"));

        let mut methods_by_receiver = BTreeMap::new();
        methods_by_receiver.insert(
            "pkg.A".to_string(),
            vec![RtaMethod {
                method_name: "Step".to_string(),
                qualified: "(pkg.A).Step".to_string(),
                node: SemanticNodeId(3),
            }],
        );
        methods_by_receiver.insert(
            "pkg.B".to_string(),
            vec![RtaMethod {
                method_name: "Step".to_string(),
                qualified: "(pkg.B).Step".to_string(),
                node: SemanticNodeId(5),
            }],
        );

        let instantiated = BTreeSet::from(["pkg.A".to_string(), "pkg.B".to_string()]);
        let mut instantiated_keys = BTreeMap::new();
        instantiated_keys.insert("pkg.A".to_string(), interner.intern("inst|A"));
        instantiated_keys.insert("pkg.B".to_string(), interner.intern("inst|B"));

        let inputs = RtaInputs {
            roots: BTreeSet::from(["main.main".to_string()]),
            callsites: vec![
                // main -> A.Step (resolvable round 1)
                RtaCallsite {
                    caller: "main.main".to_string(),
                    callsite_node: SemanticNodeId(2),
                    callsite_stable_key: interner.intern("cs|main"),
                    interface_method: Some("Step".to_string()),
                    signature: None,
                    dispatch_stable_key: interner.intern("dd|main"),
                },
                // A.Step -> B.Step (only reachable in round 2)
                RtaCallsite {
                    caller: "(pkg.A).Step".to_string(),
                    callsite_node: SemanticNodeId(4),
                    callsite_stable_key: interner.intern("cs|a"),
                    interface_method: Some("Step".to_string()),
                    signature: None,
                    dispatch_stable_key: interner.intern("dd|a"),
                },
            ],
            method_sets,
            method_set_keys,
            instantiated,
            instantiated_keys,
            methods_by_receiver,
            function_node,
            ..RtaInputs::default()
        }
        .finalize_indexes();

        let mut budget = SolverBudget::default();
        budget.go.max_rta_rounds = 1;
        let output = solve_rta(
            &crate::internal_core::test_stable_key_interner(),
            &inputs,
            &budget,
        );
        // The cap is hit before the second round resolves A.Step's callsite, so the
        // run latches BudgetExceeded honestly rather than looping.
        assert_eq!(output.budget_status, BudgetStatus::BudgetExceeded);
    }

    #[test]
    fn address_taken_threshold_suppresses_only_func_value_not_interface_dispatch() {
        let interner = crate::internal_core::test_stable_key_interner();
        // a pathologically large address-taken (func-value) surface must
        // disable only FUNC-VALUE (signature) resolution; pure INTERFACE dispatch (which
        // does not consult the address-taken set) must still resolve. `main` has an
        // interface invoke of `Read` (resolves to the instantiated (pkg.File).Read) AND a
        // func-value call of `func()` (would resolve to the address-taken handlers). With
        // address_taken_threshold = 1 and TWO address-taken handlers, func-value resolution
        // is suppressed but the interface edge must still be derived; the run latches
        // BudgetExceeded honestly (the func-value surface was too large to resolve).
        let mut function_node = BTreeMap::new();
        function_node.insert("main.main".to_string(), SemanticNodeId(1));
        function_node.insert("(pkg.File).Read".to_string(), SemanticNodeId(3));
        function_node.insert("pkg.h1".to_string(), SemanticNodeId(5));
        function_node.insert("pkg.h2".to_string(), SemanticNodeId(6));

        let mut method_sets = BTreeMap::new();
        method_sets.insert("pkg.File".to_string(), BTreeSet::from(["Read".to_string()]));
        let mut method_set_keys = BTreeMap::new();
        method_set_keys.insert("pkg.File".to_string(), interner.intern("ms|pkg.File"));

        let mut methods_by_receiver = BTreeMap::new();
        methods_by_receiver.insert(
            "pkg.File".to_string(),
            vec![RtaMethod {
                method_name: "Read".to_string(),
                qualified: "(pkg.File).Read".to_string(),
                node: SemanticNodeId(3),
            }],
        );

        let instantiated = BTreeSet::from(["pkg.File".to_string()]);
        let mut instantiated_keys = BTreeMap::new();
        instantiated_keys.insert("pkg.File".to_string(), interner.intern("inst|pkg.File"));

        // Two address-taken func() handlers; threshold = 1, so the func-value surface is
        // "too large" and func-value resolution is suppressed.
        let address_taken = BTreeSet::from(["pkg.h1".to_string(), "pkg.h2".to_string()]);
        let mut address_taken_keys = BTreeMap::new();
        address_taken_keys.insert("pkg.h1".to_string(), interner.intern("at|h1"));
        address_taken_keys.insert("pkg.h2".to_string(), interner.intern("at|h2"));
        let mut function_signature = BTreeMap::new();
        function_signature.insert("pkg.h1".to_string(), "func()".to_string());
        function_signature.insert("pkg.h2".to_string(), "func()".to_string());

        let inputs = RtaInputs {
            roots: BTreeSet::from(["main.main".to_string()]),
            callsites: vec![
                // Interface invoke of Read (must still resolve).
                RtaCallsite {
                    caller: "main.main".to_string(),
                    callsite_node: SemanticNodeId(2),
                    callsite_stable_key: interner.intern("cs|main:read"),
                    interface_method: Some("Read".to_string()),
                    signature: None,
                    dispatch_stable_key: interner.intern("dd|main:read"),
                },
                // Func-value call of func() (must be suppressed under the threshold).
                RtaCallsite {
                    caller: "main.main".to_string(),
                    callsite_node: SemanticNodeId(4),
                    callsite_stable_key: interner.intern("cs|main:fv"),
                    interface_method: None,
                    signature: Some("func()".to_string()),
                    dispatch_stable_key: interner.intern("dd|main:fv"),
                },
            ],
            method_sets,
            method_set_keys,
            instantiated,
            instantiated_keys,
            address_taken,
            address_taken_keys,
            function_node,
            function_signature,
            methods_by_receiver,
            ..RtaInputs::default()
        }
        .finalize_indexes();

        let mut budget = SolverBudget::default();
        budget.go.address_taken_threshold = 1;
        let output = solve_rta(
            &crate::internal_core::test_stable_key_interner(),
            &inputs,
            &budget,
        );

        // Interface dispatch still resolves despite the large address-taken set.
        assert!(
            output
                .derived_edges
                .iter()
                .any(|e| e.source == SemanticNodeId(1) && e.target == SemanticNodeId(3)),
            "interface dispatch must resolve even when the address-taken surface exceeds the threshold: {:#?}",
            output.derived_edges
        );
        // Func-value resolution is suppressed: no edge targets an address-taken handler.
        assert!(
            !output
                .derived_edges
                .iter()
                .any(|e| e.target == SemanticNodeId(5) || e.target == SemanticNodeId(6)),
            "func-value resolution must be suppressed under the address-taken threshold"
        );
        // The suppression is signalled honestly.
        assert_eq!(output.budget_status, BudgetStatus::BudgetExceeded);
    }

    #[test]
    fn large_address_taken_with_no_func_value_callsite_stays_within_budget() {
        let interner = crate::internal_core::test_stable_key_interner();
        // an oversized address-taken (func-value) surface must latch
        // BudgetExceeded ONLY when func-value resolution is actually NEEDED — i.e. at
        // least one reachable callsite is a func-value (signature-bearing) callsite.
        // Here the only callsite is an INTERFACE invoke (no signature), so disabling
        // func-value resolution costs nothing: interface dispatch still resolves and the
        // run must stay WithinBudget. Latching unconditionally would produce a false
        // exhaustion signal in `solver_output_digest` for codebases with many
        // address-taken functions but no indirect function-value calls.
        let mut inputs = interface_scenario(&["pkg.File"]);
        // A large address-taken surface (3 funcs) with threshold 1: func-value resolution
        // is "disabled", but there is NO func-value callsite to resolve. The scenario's
        // sole callsite (main -> Read) is a pure interface invoke (signature: None).
        inputs.address_taken = BTreeSet::from([
            "pkg.h1".to_string(),
            "pkg.h2".to_string(),
            "pkg.h3".to_string(),
        ]);
        inputs.address_taken_keys = BTreeMap::from([
            ("pkg.h1".to_string(), interner.intern("at|h1")),
            ("pkg.h2".to_string(), interner.intern("at|h2")),
            ("pkg.h3".to_string(), interner.intern("at|h3")),
        ]);
        debug_assert!(inputs.callsites.iter().all(|c| c.signature.is_none()));

        let mut budget = SolverBudget::default();
        budget.go.address_taken_threshold = 1;
        let output = solve_rta(
            &crate::internal_core::test_stable_key_interner(),
            &inputs,
            &budget,
        );

        // Interface dispatch still resolves: main(1) -> (pkg.File).Read(3).
        assert!(
            output
                .derived_edges
                .iter()
                .any(|e| e.source == SemanticNodeId(1) && e.target == SemanticNodeId(3)),
            "interface dispatch must still resolve: {:#?}",
            output.derived_edges
        );
        // No func-value callsite exists, so disabling func-value resolution cost nothing —
        // the run is WithinBudget, NOT a false BudgetExceeded.
        assert_eq!(
            output.budget_status,
            BudgetStatus::WithinBudget,
            "a large address-taken surface with NO func-value callsite must NOT latch \
             BudgetExceeded (func-value resolution was never needed)"
        );
    }

    #[test]
    fn large_address_taken_with_only_unreachable_func_value_callsite_stays_within_budget() {
        let interner = crate::internal_core::test_stable_key_interner();
        // A signature-bearing func-value callsite that is NOT reachable from any RTA root
        // must not latch BudgetExceeded. The threshold signal is about skipped reachable
        // work, not dormant obligations in disconnected functions.
        let mut inputs = interface_scenario(&["pkg.File"]);
        inputs
            .function_node
            .insert("pkg.unreachable".to_string(), SemanticNodeId(50));
        inputs
            .function_node
            .insert("pkg.h1".to_string(), SemanticNodeId(51));
        inputs
            .function_node
            .insert("pkg.h2".to_string(), SemanticNodeId(52));
        inputs
            .function_signature
            .insert("pkg.h1".to_string(), "func()".to_string());
        inputs
            .function_signature
            .insert("pkg.h2".to_string(), "func()".to_string());
        inputs.address_taken = BTreeSet::from(["pkg.h1".to_string(), "pkg.h2".to_string()]);
        inputs.address_taken_keys = BTreeMap::from([
            ("pkg.h1".to_string(), interner.intern("at|h1")),
            ("pkg.h2".to_string(), interner.intern("at|h2")),
        ]);
        inputs.callsites.push(RtaCallsite {
            caller: "pkg.unreachable".to_string(),
            callsite_node: SemanticNodeId(53),
            callsite_stable_key: interner.intern("cs|unreachable:fv"),
            interface_method: None,
            signature: Some("func()".to_string()),
            dispatch_stable_key: interner.intern("dd|unreachable:fv"),
        });

        let mut budget = SolverBudget::default();
        budget.go.address_taken_threshold = 1;
        let output = solve_rta(
            &crate::internal_core::test_stable_key_interner(),
            &inputs.finalize_indexes(),
            &budget,
        );

        assert!(
            output
                .derived_edges
                .iter()
                .any(|e| e.source == SemanticNodeId(1) && e.target == SemanticNodeId(3)),
            "reachable interface dispatch must still resolve: {:#?}",
            output.derived_edges
        );
        assert!(
            !output
                .derived_edges
                .iter()
                .any(|e| e.target == SemanticNodeId(51) || e.target == SemanticNodeId(52)),
            "the unreachable func-value callsite must not resolve"
        );
        assert_eq!(
            output.budget_status,
            BudgetStatus::WithinBudget,
            "an unreachable func-value callsite must not produce a false BudgetExceeded"
        );
    }

    #[test]
    fn convergence_at_round_boundary_with_no_pending_work_stays_within_budget() {
        let interner = crate::internal_core::test_stable_key_interner();
        // at the `max_rta_rounds` boundary the cap must latch BudgetExceeded
        // ONLY when there is genuinely pending dispatch work. Here `main` (root) invokes
        // `Step`, resolving to (pkg.A).Step in round 1; (pkg.A).Step has NO callsites
        // and only a static edge back to already-reachable main, so the round-2 frontier
        // {A.Step} has nothing left to resolve — the analysis has CONVERGED. With
        // `max_rta_rounds = 1`, the static-call row must not by itself latch
        // `BudgetExceeded`. By contrast, A.Step invoking B.Step leaves genuine pending
        // work and must still latch.
        let mut function_node = BTreeMap::new();
        function_node.insert("main.main".to_string(), SemanticNodeId(1));
        function_node.insert("(pkg.A).Step".to_string(), SemanticNodeId(3));

        let mut method_sets = BTreeMap::new();
        method_sets.insert("pkg.A".to_string(), BTreeSet::from(["Step".to_string()]));
        let mut method_set_keys = BTreeMap::new();
        method_set_keys.insert("pkg.A".to_string(), interner.intern("ms|A"));

        let mut methods_by_receiver = BTreeMap::new();
        methods_by_receiver.insert(
            "pkg.A".to_string(),
            vec![RtaMethod {
                method_name: "Step".to_string(),
                qualified: "(pkg.A).Step".to_string(),
                node: SemanticNodeId(3),
            }],
        );

        let instantiated = BTreeSet::from(["pkg.A".to_string()]);
        let mut instantiated_keys = BTreeMap::new();
        instantiated_keys.insert("pkg.A".to_string(), interner.intern("inst|A"));

        let inputs = RtaInputs {
            roots: BTreeSet::from(["main.main".to_string()]),
            // Only main has a callsite; (pkg.A).Step has none, and its static edge is
            // already closed over reachable main → the round-2 frontier is converged
            // with no pending work.
            callsites: vec![RtaCallsite {
                caller: "main.main".to_string(),
                callsite_node: SemanticNodeId(2),
                callsite_stable_key: interner.intern("cs|main"),
                interface_method: Some("Step".to_string()),
                signature: None,
                dispatch_stable_key: interner.intern("dd|main"),
            }],
            method_sets,
            method_set_keys,
            instantiated,
            instantiated_keys,
            methods_by_receiver,
            function_node,
            static_call_targets: BTreeMap::from([(
                "(pkg.A).Step".to_string(),
                BTreeSet::from(["main.main".to_string()]),
            )]),
            ..RtaInputs::default()
        }
        .finalize_indexes();

        let mut budget = SolverBudget::default();
        budget.go.max_rta_rounds = 1;
        let output = solve_rta(
            &crate::internal_core::test_stable_key_interner(),
            &inputs,
            &budget,
        );
        // The single resolvable edge is derived AND the run is WithinBudget (converged at
        // the boundary, no pending work the cap prevented).
        assert!(
            output
                .derived_edges
                .iter()
                .any(|e| e.source == SemanticNodeId(1) && e.target == SemanticNodeId(3)),
            "the resolvable edge must be derived"
        );
        assert_eq!(
            output.budget_status,
            BudgetStatus::WithinBudget,
            "convergence at the round boundary with no pending work must NOT latch BudgetExceeded"
        );
    }

    #[test]
    fn static_only_expansion_after_dispatch_round_cap_stays_within_budget() {
        let interner = crate::internal_core::test_stable_key_interner();
        // The dispatch round cap is a dynamic-dispatch cap, not a static reachability
        // cap. Round 1 resolves main -> A dynamically. At the cap boundary, A has only
        // a static call to B, so the analysis must still close over B without latching
        // go.max_rta_rounds.
        let mut function_node = BTreeMap::new();
        function_node.insert("main.main".to_string(), SemanticNodeId(1));
        function_node.insert("(pkg.A).Step".to_string(), SemanticNodeId(3));
        function_node.insert("pkg.B".to_string(), SemanticNodeId(4));

        let mut method_sets = BTreeMap::new();
        method_sets.insert("pkg.A".to_string(), BTreeSet::from(["Step".to_string()]));
        let mut method_set_keys = BTreeMap::new();
        method_set_keys.insert("pkg.A".to_string(), interner.intern("ms|A"));

        let mut methods_by_receiver = BTreeMap::new();
        methods_by_receiver.insert(
            "pkg.A".to_string(),
            vec![RtaMethod {
                method_name: "Step".to_string(),
                qualified: "(pkg.A).Step".to_string(),
                node: SemanticNodeId(3),
            }],
        );

        let inputs = RtaInputs {
            roots: BTreeSet::from(["main.main".to_string()]),
            callsites: vec![RtaCallsite {
                caller: "main.main".to_string(),
                callsite_node: SemanticNodeId(2),
                callsite_stable_key: interner.intern("cs|main"),
                interface_method: Some("Step".to_string()),
                signature: None,
                dispatch_stable_key: interner.intern("dd|main"),
            }],
            method_sets,
            method_set_keys,
            instantiated: BTreeSet::from(["pkg.A".to_string()]),
            instantiated_keys: BTreeMap::from([("pkg.A".to_string(), interner.intern("inst|A"))]),
            methods_by_receiver,
            function_node,
            static_call_targets: BTreeMap::from([(
                "(pkg.A).Step".to_string(),
                BTreeSet::from(["pkg.B".to_string()]),
            )]),
            ..RtaInputs::default()
        }
        .finalize_indexes();

        let mut budget = SolverBudget::default();
        budget.go.max_rta_rounds = 1;

        let output = solve_rta(
            &crate::internal_core::test_stable_key_interner(),
            &inputs,
            &budget,
        );

        assert!(
            output
                .derived_edges
                .iter()
                .any(|edge| edge.source == SemanticNodeId(1) && edge.target == SemanticNodeId(3)),
            "round 1 must still resolve the dynamic main -> A edge"
        );
        assert_eq!(
            output.budget_status,
            BudgetStatus::WithinBudget,
            "static-only expansion after the dispatch cap must not latch BudgetExceeded"
        );
        assert!(
            output.budget_reasons.is_empty(),
            "static-only expansion must not record a dispatch budget reason"
        );
    }

    /// A multi-round RTA convergence may exceed the policy iteration count without
    /// being runaway dispatch. The worklist uses `max_worklist_steps`, and frontier-only
    /// scanning counts each reachable caller's actual work once. This graph must converge
    /// `WithinBudget` and resolve every chain edge.
    ///
    /// Shape: a deep reachable chain `step.0 -> step.1 -> ... -> step.N`. Each `step.i`
    /// is a method on a distinct instantiated type `pkg.S{i}`; it has one interface
    /// invoke of the uniquely-named method `M{i+1}`, which resolves ONLY to `step.{i+1}`
    /// (on `pkg.S{i+1}`). Resolving the whole chain therefore needs N rounds and N
    /// callsite-visits. With N = 80, the graph also exceeds the policy iteration count.
    #[test]
    fn deep_multi_round_chain_exceeding_64_visits_stays_within_budget() {
        let interner = crate::internal_core::test_stable_key_interner();
        const CHAIN: usize = 80;

        let mut function_node = BTreeMap::new();
        let mut method_sets = BTreeMap::new();
        let mut method_set_keys = BTreeMap::new();
        let mut methods_by_receiver = BTreeMap::new();
        let mut instantiated = BTreeSet::new();
        let mut instantiated_keys = BTreeMap::new();
        let mut callsites = Vec::new();

        // step.0 is the root (a plain function, the entry); step.1..=CHAIN are methods
        // on instantiated types pkg.S1..pkg.S{CHAIN}. Node ids: step.i -> SemanticNodeId(i+1).
        let qualified = |i: usize| -> String {
            if i == 0 {
                "main.main".to_string()
            } else {
                format!("(pkg.S{i}).M{i}")
            }
        };
        for i in 0..=CHAIN {
            function_node.insert(qualified(i), SemanticNodeId((i + 1) as u64));
        }
        // Each instantiated type pkg.S{i} declares its method M{i} and is in the
        // instantiated set, so an invoke of M{i} resolves to (pkg.S{i}).M{i}.
        for i in 1..=CHAIN {
            let type_name = format!("pkg.S{i}");
            let method = format!("M{i}");
            method_sets.insert(type_name.clone(), BTreeSet::from([method.clone()]));
            method_set_keys.insert(
                type_name.clone(),
                interner.intern(format!("ms|{type_name}")),
            );
            methods_by_receiver.insert(
                type_name.clone(),
                vec![RtaMethod {
                    method_name: method,
                    qualified: qualified(i),
                    node: SemanticNodeId((i + 1) as u64),
                }],
            );
            instantiated.insert(type_name.clone());
            instantiated_keys.insert(
                type_name.clone(),
                interner.intern(format!("inst|{type_name}")),
            );
        }
        // Caller step.i (for i in 0..CHAIN) invokes M{i+1}, reaching step.{i+1}.
        for i in 0..CHAIN {
            let next = i + 1;
            callsites.push(RtaCallsite {
                caller: qualified(i),
                callsite_node: SemanticNodeId((1000 + i) as u64),
                callsite_stable_key: interner.intern(format!("cs|{i}")),
                interface_method: Some(format!("M{next}")),
                signature: None,
                dispatch_stable_key: interner.intern(format!("dd|{i}")),
            });
        }

        let inputs = RtaInputs {
            roots: BTreeSet::from(["main.main".to_string()]),
            callsites,
            method_sets,
            method_set_keys,
            instantiated,
            instantiated_keys,
            methods_by_receiver,
            function_node,
            ..RtaInputs::default()
        }
        .finalize_indexes();

        // Default worklist-step cap (10_000) easily admits the 80 visits; the round cap
        // must be raised above the chain depth so the round cap itself is not what trips
        // This pins the step cap's scale. The default
        // worklist-step cap stays untouched, proving 80 visits remain within budget.
        let mut budget = SolverBudget::default();
        budget.go.max_rta_rounds = CHAIN + 1;
        assert!(
            CHAIN > budget.max_outer_iterations,
            "the chain must exceed the policy iteration count ({})",
            budget.max_outer_iterations
        );

        let output = solve_rta(
            &crate::internal_core::test_stable_key_interner(),
            &inputs,
            &budget,
        );

        // The fixpoint converges honestly — NOT a spurious BudgetExceeded.
        assert_eq!(
            output.budget_status,
            BudgetStatus::WithinBudget,
            "an {CHAIN}-visit multi-round convergence must not trip the worklist-step cap"
        );
        // Every chain edge step.i -> step.{i+1} is resolved (no edge silently dropped).
        assert_eq!(
            output.derived_edges.len(),
            CHAIN,
            "every chain edge must resolve; a mid-chain cap would drop the tail"
        );
        for i in 0..CHAIN {
            let source = SemanticNodeId((i + 1) as u64);
            let target = SemanticNodeId((i + 2) as u64);
            assert!(
                output
                    .derived_edges
                    .iter()
                    .any(|edge| edge.source == source && edge.target == target),
                "chain edge step.{i} -> step.{} must be resolved",
                i + 1
            );
        }
    }

    /// Genuine runaway dispatch must still latch `BudgetExceeded`
    /// once it exceeds the (Go-scaled) worklist-step cap. A tight `max_worklist_steps`
    /// forces the same chain to truncate mid-resolution, proving the cap still bites
    /// when work genuinely exceeds it (the `iteration-cap` fixture stays green for the
    /// candidate-cap path; this pins the step-cap path).
    #[test]
    fn worklist_step_cap_still_latches_budget_exceeded_when_genuinely_exceeded() {
        let interner = crate::internal_core::test_stable_key_interner();
        let mut function_node = BTreeMap::new();
        function_node.insert("main.main".to_string(), SemanticNodeId(1));
        function_node.insert("(pkg.A).Step".to_string(), SemanticNodeId(3));
        function_node.insert("(pkg.B).Step".to_string(), SemanticNodeId(5));

        let mut method_sets = BTreeMap::new();
        method_sets.insert("pkg.A".to_string(), BTreeSet::from(["Step".to_string()]));
        method_sets.insert("pkg.B".to_string(), BTreeSet::from(["Step".to_string()]));
        let mut method_set_keys = BTreeMap::new();
        method_set_keys.insert("pkg.A".to_string(), interner.intern("ms|A"));
        method_set_keys.insert("pkg.B".to_string(), interner.intern("ms|B"));

        let mut methods_by_receiver = BTreeMap::new();
        methods_by_receiver.insert(
            "pkg.A".to_string(),
            vec![RtaMethod {
                method_name: "Step".to_string(),
                qualified: "(pkg.A).Step".to_string(),
                node: SemanticNodeId(3),
            }],
        );
        methods_by_receiver.insert(
            "pkg.B".to_string(),
            vec![RtaMethod {
                method_name: "Step".to_string(),
                qualified: "(pkg.B).Step".to_string(),
                node: SemanticNodeId(5),
            }],
        );

        let instantiated = BTreeSet::from(["pkg.A".to_string(), "pkg.B".to_string()]);
        let mut instantiated_keys = BTreeMap::new();
        instantiated_keys.insert("pkg.A".to_string(), interner.intern("inst|A"));
        instantiated_keys.insert("pkg.B".to_string(), interner.intern("inst|B"));

        let inputs = RtaInputs {
            roots: BTreeSet::from(["main.main".to_string()]),
            callsites: vec![
                RtaCallsite {
                    caller: "main.main".to_string(),
                    callsite_node: SemanticNodeId(2),
                    callsite_stable_key: interner.intern("cs|main"),
                    interface_method: Some("Step".to_string()),
                    signature: None,
                    dispatch_stable_key: interner.intern("dd|main"),
                },
                RtaCallsite {
                    caller: "(pkg.A).Step".to_string(),
                    callsite_node: SemanticNodeId(4),
                    callsite_stable_key: interner.intern("cs|a"),
                    interface_method: Some("Step".to_string()),
                    signature: None,
                    dispatch_stable_key: interner.intern("dd|a"),
                },
            ],
            method_sets,
            method_set_keys,
            instantiated,
            instantiated_keys,
            methods_by_receiver,
            function_node,
            ..RtaInputs::default()
        }
        .finalize_indexes();

        // A worklist-step cap of 1 admits only the first callsite resolution, so the
        // chain truncates and the run latches BudgetExceeded honestly.
        let mut budget = SolverBudget::default();
        budget.go.max_worklist_steps = 1;
        let output = solve_rta(
            &crate::internal_core::test_stable_key_interner(),
            &inputs,
            &budget,
        );
        assert_eq!(output.budget_status, BudgetStatus::BudgetExceeded);
    }
}
