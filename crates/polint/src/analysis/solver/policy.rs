//! Composition-root solver policies.
//!
//! The points-to policy contract and neutral Andersen composition live in
//! `polint-analysis`. This module retains only policies that adapt the facade's
//! concrete frontend snapshots: Go RTA and the TS/JS callsite projection.

pub(crate) use crate::analysis_neutral::solver::policy::{SolverPolicy, SolverPolicyOutcome};

use super::budget::SolverBudget;
use crate::go::rta::{GoRtaInputs, solve_go_rta};

pub(crate) use crate::ts::points_to::{TsPointsToInputs, budget_status, solve_ts_points_to};

pub(crate) fn ts_points_to_inputs_from_db(db: &crate::core::AnalysisDb) -> TsPointsToInputs {
    TsPointsToInputs::from_db(db)
}

/// The real Go RTA policy (GO-05). Owns a CLOSED snapshot of the Go RTA
/// inputs (reachability roots + the Go-frontend address-taken / instantiated-type /
/// dispatch facts + method-sets + callsites), mirroring the neutral points-to
/// policy's closed snapshot. [`SolverPolicy::solve`] runs the RTA fixpoint
/// ([`solve_go_rta`]) and returns the resolved call edges + budget status.
pub(crate) struct GoRtaPolicy {
    inputs: GoRtaInputs,
}

impl GoRtaPolicy {
    pub(crate) fn new(inputs: GoRtaInputs) -> Self {
        Self { inputs }
    }
}

impl SolverPolicy for GoRtaPolicy {
    fn id(&self) -> &'static str {
        "go_rta"
    }

    fn solve(
        &self,
        interner: &crate::core::StableKeyInterner,
        budget: &SolverBudget,
    ) -> SolverPolicyOutcome {
        // Run the RTA fixpoint over the closed snapshot (composition over the engine
        // worklist, mirroring the neutral points-to policy's fold). The output is
        // already normalized.
        let output = solve_go_rta(interner, &self.inputs, budget);
        SolverPolicyOutcome {
            points_to: None,
            derived_edges: output.derived_edges,
            budget_status: output.budget_status,
            budget_reasons: output.budget_reasons,
            steps: 0,
        }
    }
}

/// JS/TS points-to policy. It projects the semantic graph into the shared
/// field-sensitive Andersen solver and emits indirect call edges from solved sets.
pub(crate) struct TsPointsToPolicy {
    inputs: TsPointsToInputs,
}

impl TsPointsToPolicy {
    pub(crate) fn new(inputs: TsPointsToInputs) -> Self {
        Self { inputs }
    }
}

impl SolverPolicy for TsPointsToPolicy {
    fn id(&self) -> &'static str {
        "ts_points_to"
    }

    fn solve(
        &self,
        interner: &crate::core::StableKeyInterner,
        budget: &SolverBudget,
    ) -> SolverPolicyOutcome {
        let output = solve_ts_points_to(interner, &self.inputs, budget);
        SolverPolicyOutcome {
            points_to: None,
            derived_edges: output.derived_edges,
            budget_status: budget_status(&output.points_to),
            budget_reasons: output.points_to.budget_reasons,
            steps: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    use crate::analysis::ids::SemanticNodeId;
    use crate::analysis::solver::budget::BudgetStatus;
    use crate::analysis::solver::engine::SolverEngine;
    use crate::analysis_neutral::solver::policy::PointsToPolicy;
    use crate::go::rta::inputs::{GoRtaCallsite, GoRtaMethod};

    #[test]
    fn ts_policy_id_is_stable() {
        let budget = SolverBudget::default();
        let ts = TsPointsToPolicy::new(TsPointsToInputs::default());
        let ts_outcome = ts.solve(&crate::core::test_stable_key_interner(), &budget);
        assert_eq!(ts.id(), "ts_points_to");
        assert!(ts_outcome.points_to.is_none());
        assert!(ts_outcome.derived_edges.is_empty());
        assert_eq!(ts_outcome.budget_status, BudgetStatus::WithinBudget);
    }
    #[test]
    fn go_rta_policy_derives_edges_from_a_resolvable_dispatch() {
        let interner = crate::core::test_stable_key_interner();
        // The real Go RTA policy now derives ≥1 edge from a resolvable interface
        // dispatch (an instantiated receiver whose method-set declares the method).
        let mut function_node = BTreeMap::new();
        function_node.insert("main.main".to_string(), SemanticNodeId(1));
        function_node.insert("(pkg.File).Read".to_string(), SemanticNodeId(3));

        let mut method_sets = BTreeMap::new();
        method_sets.insert("pkg.File".to_string(), BTreeSet::from(["Read".to_string()]));
        let mut method_set_keys = BTreeMap::new();
        method_set_keys.insert("pkg.File".to_string(), interner.intern("ms|pkg.File"));

        let mut methods_by_receiver = BTreeMap::new();
        methods_by_receiver.insert(
            "pkg.File".to_string(),
            vec![GoRtaMethod {
                method_name: "Read".to_string(),
                qualified: "(pkg.File).Read".to_string(),
                node: SemanticNodeId(3),
            }],
        );

        let mut instantiated_keys = BTreeMap::new();
        instantiated_keys.insert("pkg.File".to_string(), interner.intern("inst|pkg.File"));

        let inputs = GoRtaInputs {
            roots: BTreeSet::from(["main.main".to_string()]),
            callsites: vec![GoRtaCallsite {
                caller: "main.main".to_string(),
                callsite_node: SemanticNodeId(2),
                callsite_stable_key: interner.intern("cs|main:read"),
                interface_method: Some("Read".to_string()),
                signature: None,
                dispatch_stable_key: interner.intern("dd|main:read"),
            }],
            method_sets,
            method_set_keys,
            instantiated: BTreeSet::from(["pkg.File".to_string()]),
            instantiated_keys,
            methods_by_receiver,
            function_node,
            ..GoRtaInputs::default()
        }
        .finalize_indexes();

        let policy = GoRtaPolicy::new(inputs);
        assert_eq!(policy.id(), "go_rta");
        let outcome = policy.solve(
            &crate::core::test_stable_key_interner(),
            &SolverBudget::default(),
        );
        assert!(outcome.points_to.is_none());
        assert!(
            !outcome.derived_edges.is_empty(),
            "the Go RTA policy must derive ≥1 edge from a resolvable dispatch"
        );
        assert_eq!(outcome.budget_status, BudgetStatus::WithinBudget);
    }

    #[test]
    fn engine_drives_empty_go_rta_and_ts_inputs_to_zero_results() {
        // Empty Go RTA and TS token snapshots derive nothing, proving the composition
        // root drives both frontend policies without fabricating edges.
        let budget = SolverBudget::default();
        let engine = SolverEngine::new(
            vec![
                Box::new(GoRtaPolicy::new(GoRtaInputs::default())),
                Box::new(TsPointsToPolicy::new(TsPointsToInputs::default())),
            ],
            budget,
        );
        let run = engine.run(&crate::core::test_stable_key_interner());

        assert_eq!(run.policy_outcomes.len(), 2);
        assert_eq!(run.policy_outcomes[0].policy_id, "go_rta");
        assert_eq!(run.policy_outcomes[1].policy_id, "ts_points_to");
        assert!(
            run.policy_outcomes
                .iter()
                .all(|record| record.outcome.points_to.is_none()
                    && record.outcome.derived_edges.is_empty())
        );
        assert_eq!(run.budget_status, BudgetStatus::WithinBudget);
    }

    #[test]
    fn run_to_solver_output_still_surfaces_go_rta_budget_exhaustion() {
        let interner = crate::core::test_stable_key_interner();
        // Excluding the discarded points-to fold must not mask a policy whose edges
        // enter the output. A Go RTA policy that exhausts its round cap must still
        // surface BudgetExceeded at the merged output.
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
            vec![GoRtaMethod {
                method_name: "Step".to_string(),
                qualified: "(pkg.A).Step".to_string(),
                node: SemanticNodeId(3),
            }],
        );
        methods_by_receiver.insert(
            "pkg.B".to_string(),
            vec![GoRtaMethod {
                method_name: "Step".to_string(),
                qualified: "(pkg.B).Step".to_string(),
                node: SemanticNodeId(5),
            }],
        );
        let mut instantiated_keys = BTreeMap::new();
        instantiated_keys.insert("pkg.A".to_string(), interner.intern("inst|A"));
        instantiated_keys.insert("pkg.B".to_string(), interner.intern("inst|B"));
        let go_inputs = GoRtaInputs {
            roots: BTreeSet::from(["main.main".to_string()]),
            callsites: vec![
                GoRtaCallsite {
                    caller: "main.main".to_string(),
                    callsite_node: SemanticNodeId(2),
                    callsite_stable_key: interner.intern("cs|main"),
                    interface_method: Some("Step".to_string()),
                    signature: None,
                    dispatch_stable_key: interner.intern("dd|main"),
                },
                GoRtaCallsite {
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
            instantiated: BTreeSet::from(["pkg.A".to_string(), "pkg.B".to_string()]),
            instantiated_keys,
            methods_by_receiver,
            function_node,
            ..GoRtaInputs::default()
        }
        .finalize_indexes();

        let mut budget = SolverBudget::default();
        budget.go.max_rta_rounds = 1;
        let engine = SolverEngine::new(
            vec![
                Box::new(PointsToPolicy::new(Vec::new())),
                Box::new(GoRtaPolicy::new(go_inputs)),
            ],
            budget,
        );
        let output = engine.run_to_solver_output(&crate::core::test_stable_key_interner(), &[]);
        assert_eq!(
            output.budget_status,
            BudgetStatus::BudgetExceeded,
            "a Go RTA policy whose edges enter the output must still surface exhaustion"
        );
    }
}
