//! Neutral solver policy contracts and the points-to composition policy.
//!
//! The engine owns scheduling and budget aggregation; policies own one closed,
//! deterministic input snapshot and report their own derivation result. This
//! module contains only language-neutral policy code. Frontend-specific policies
//! stay in their composition roots and implement [`SolverPolicy`] here.

use std::collections::BTreeSet;

use crate::analysis_neutral::points_to::facts::PointsToConstraintFact;
use crate::analysis_neutral::points_to::solver::{PointsToSolveResult, solve_points_to};
use crate::analysis_neutral::solver::budget::{BudgetStatus, SolverBudget};
use crate::analysis_neutral::solver::facts::DerivedEdgeFact;
use crate::internal_core::StableKeyInterner;

/// Outcome produced by a [`SolverPolicy`] when driven by the engine.
///
/// Unrelated to [`crate::sdk::policy::PolicyOutcome`], which is the rule-facing
/// verdict of a policy query.
///
/// A points-to policy carries its folded sub-domain result in `points_to`;
/// edge-producing policies place their normalized edges in `derived_edges`.
/// Budget status and reasons are retained independently so the engine can
/// combine only the policy outputs that contribute to the final solver output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolverPolicyOutcome {
    /// Folded points-to sub-domain result, when this is the points-to policy.
    pub points_to: Option<PointsToSolveResult>,
    /// Derived edges contributed by this policy.
    pub derived_edges: Vec<DerivedEdgeFact>,
    /// Budget outcome projected to the unified solver vocabulary.
    pub budget_status: BudgetStatus,
    /// Stable reason labels for every budget ceiling this policy exhausted.
    pub budget_reasons: BTreeSet<String>,
    /// Number of internal work steps reported by this policy.
    pub steps: u64,
}

impl SolverPolicyOutcome {
    /// Construct an honest empty outcome for policies with no derivation.
    pub fn empty() -> Self {
        Self {
            points_to: None,
            derived_edges: Vec::new(),
            budget_status: BudgetStatus::WithinBudget,
            budget_reasons: BTreeSet::new(),
            steps: 0,
        }
    }
}

/// A deterministic solver policy over a closed input snapshot.
pub trait SolverPolicy {
    /// Stable lowercase policy identifier.
    fn id(&self) -> &'static str;

    /// Drive one policy fixpoint under the supplied unified budget.
    fn solve(&self, interner: &StableKeyInterner, budget: &SolverBudget) -> SolverPolicyOutcome;
}

/// The language-neutral points-to policy.
///
/// It composes the existing Andersen fixpoint without rewriting it. Its derived
/// edges continue through the unified engine's CopyEdge closure; the folded
/// points-to result is retained only for budget/status accounting.
pub struct PointsToPolicy {
    constraints: Vec<PointsToConstraintFact>,
}

impl PointsToPolicy {
    pub fn new(constraints: Vec<PointsToConstraintFact>) -> Self {
        Self { constraints }
    }
}

impl SolverPolicy for PointsToPolicy {
    fn id(&self) -> &'static str {
        "points_to"
    }

    fn solve(&self, interner: &StableKeyInterner, budget: &SolverBudget) -> SolverPolicyOutcome {
        let result = solve_points_to(interner, &self.constraints, budget.points_to_budget());
        let budget_status = BudgetStatus::from_points_to(result.budget_status);
        let budget_reasons = result.budget_reasons.clone();
        SolverPolicyOutcome {
            points_to: Some(result),
            derived_edges: Vec::new(),
            budget_status,
            budget_reasons,
            steps: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_outcome_is_semantically_empty() {
        let outcome = SolverPolicyOutcome::empty();
        assert!(outcome.points_to.is_none());
        assert!(outcome.derived_edges.is_empty());
        assert_eq!(outcome.budget_status, BudgetStatus::WithinBudget);
        assert!(outcome.budget_reasons.is_empty());
        assert_eq!(outcome.steps, 0);
    }

    #[test]
    fn points_to_policy_id_is_stable() {
        assert_eq!(PointsToPolicy::new(Vec::new()).id(), "points_to");
    }
}
