//! Unified solver budget model (D-05, D-06).
//!
//! [`SolverBudget`] generalizes the points-to sub-domain's `PointsToBudget`: it
//! carries the cross-domain knobs (`max_steps` and an explicit bounded
//! outer-iteration cap per D-11) plus a per-sub-domain channel for the
//! points-to-specific knobs. [`BudgetStatus`] generalizes `PointsToBudgetStatus`
//! and models budget exhaustion as an explicit, honest signal (D-06) — never a
//! silent drop.
//!
//! D-05 (Claude's discretion: alias or wrap): the existing `PointsToBudget` and
//! `PointsToBudgetStatus` are treated as a **sub-domain projection** of the
//! unified types via explicit mapping functions
//! ([`SolverBudget::points_to_budget`] / [`BudgetStatus::from_points_to`]) rather
//! than editing `PointsToBudget`. The two defaults are equal, and the
//! points-to step ceiling is its own knob because `max_steps` also bounds the
//! engine's edge closure.

use serde::{Deserialize, Serialize};

use crate::analysis_neutral::adaptation::budget::AdaptationModelBudget;
use crate::analysis_neutral::points_to::facts::PointsToBudgetStatus;
use crate::analysis_neutral::points_to::solver::PointsToBudget;

/// Per-sub-domain budget knobs carried alongside the unified cross-domain knobs.
///
/// This is the "channel for per-sub-domain knobs" D-05 prescribes. Today it holds
/// the points-to-specific caps plus the Go RTA and TS token controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointsToSubBudget {
    pub max_steps: usize,
    pub max_objects_per_var: usize,
    pub max_dynamic_vars: usize,
}

impl Default for PointsToSubBudget {
    fn default() -> Self {
        let standalone = PointsToBudget::default();
        Self {
            max_steps: standalone.max_steps,
            max_objects_per_var: standalone.max_objects_per_var,
            max_dynamic_vars: standalone.max_dynamic_vars,
        }
    }
}

/// Per-sub-domain budget knobs for the Go RTA driver (D-10/D-11).
///
/// Mirrors [`PointsToSubBudget`] structurally: a `Copy` bag of caps hung on
/// [`SolverBudget`] as `go`. These bound the RTA fixpoint so runaway interface
/// dispatch latches [`BudgetStatus::BudgetExceeded`] honestly (D-13) rather than
/// looping unbounded:
///
/// - `address_taken_threshold` — the roadmap-named knob (D-10): if the accumulated
///   address-taken function set exceeds this, the run latches exhaustion.
/// - `max_candidates_per_callsite` — caps the candidate-callee fan-out resolved for
///   one dynamic callsite; exceeding it is run-level exhaustion (edges resolved
///   before the cap keep their honest status).
/// - `max_rta_rounds` — caps GENUINE dynamic-dispatch re-iteration (the number of
///   rounds that actually resolved a dynamic callsite, never static-call-graph depth);
///   exceeding it latches exhaustion. Static-reachability growth is bounded by
///   `max_worklist_steps`, not this cap, so a deep first-party static call chain whose
///   depth exceeds this value still converges (FIX 1).
/// - `max_worklist_steps` — the Go-scaled per-callsite-resolution worklist-step cap.
///   One step is one callsite resolution. This is sized like the points-to
///   `max_steps` default (10_000), NOT the cross-domain `max_outer_iterations` (64,
///   a policy-drain count): the RTA fixpoint can resolve thousands of dynamic
///   callsite-visits across rounds on a real repo, so reusing the policy-count cap
///   here would spuriously latch [`BudgetStatus::BudgetExceeded`] and drop real edges
///   (CR-01). Exceeding it is honest run-level exhaustion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GoRtaSubBudget {
    pub address_taken_threshold: usize,
    pub max_candidates_per_callsite: usize,
    pub max_rta_rounds: usize,
    pub max_worklist_steps: usize,
}

impl Default for GoRtaSubBudget {
    fn default() -> Self {
        // Honest defaults sized to comfortably accommodate real Go interface-dispatch
        // graphs while still bounding pathological fan-out / cyclic method-set graphs.
        Self {
            address_taken_threshold: 256,
            max_candidates_per_callsite: 128,
            max_rta_rounds: 32,
            // Go-scaled worklist-step cap (CR-01): mirrors the points-to `max_steps`
            // default (10_000), not the policy-count `max_outer_iterations` (64).
            max_worklist_steps: 10_000,
        }
    }
}

/// Unified solver budget generalizing `PointsToBudget` (D-05).
///
/// Cross-domain knobs: `max_steps` (per-step worklist cap, mirrors the points-to
/// default 10_000) and `max_outer_iterations` (the explicit bounded outer-iteration
/// cap mandated by D-11 — the unified core never loops unbounded). Per-sub-domain
/// knobs hang off [`PointsToSubBudget`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SolverBudget {
    pub max_steps: usize,
    pub max_outer_iterations: usize,
    pub points_to: PointsToSubBudget,
    pub go: GoRtaSubBudget,
    pub adaptation: AdaptationModelBudget,
}

impl Default for SolverBudget {
    fn default() -> Self {
        Self {
            // Matches the points-to default 10_000 so the folded sub-domain runs
            // identically when driven through the unified budget.
            max_steps: 10_000,
            // Bounded outer-iteration cap (D-11). A single fixpoint drain is one
            // outer iteration today; the cap keeps future multi-policy rounds
            // bounded rather than unbounded.
            max_outer_iterations: 64,
            points_to: PointsToSubBudget::default(),
            // Go RTA sub-budget (D-10/D-11). Adding this field MUST NOT perturb the
            // existing fields' values — `solver_budget_default_matches_points_to_defaults`
            // pins 10_000 / 64 / points-to defaults byte-identically.
            go: GoRtaSubBudget::default(),
            // Repo-local adaptation model caps (ADAPT-01). Appending this field keeps
            // existing solver defaults byte-identical while making model expansion
            // budgeted before graph lowering lands.
            adaptation: AdaptationModelBudget::default(),
        }
    }
}

impl SolverBudget {
    /// Project the unified budget onto the existing `PointsToBudget` (D-05).
    ///
    /// This is the sub-domain projection: it never mutates `PointsToBudget`'s own
    /// `Default`, so the points-to fixtures stay byte-identical.
    pub fn points_to_budget(&self) -> PointsToBudget {
        PointsToBudget {
            max_steps: self.points_to.max_steps,
            max_objects_per_var: self.points_to.max_objects_per_var,
            max_dynamic_vars: self.points_to.max_dynamic_vars,
        }
    }
}

/// Unified budget outcome, generalizing `PointsToBudgetStatus` (D-05/D-06).
///
/// Pinned declaration order drives the derived `Ord` + serde, making this enum
/// byte-stable without `#[repr(u8)]`. Budget exhaustion surfaces as an explicit
/// [`BudgetStatus::BudgetExceeded`] signal rather than a silent precision drop.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub enum BudgetStatus {
    /// The run converged within every budget ceiling.
    #[default]
    WithinBudget,
    /// A budget ceiling (steps, outer iterations, or a per-sub-domain cap) was
    /// hit; the unknown taxonomy categorizes this honestly downstream.
    BudgetExceeded,
    /// The solver did not run for this input.
    NotRun,
}

impl BudgetStatus {
    /// Stable lowercase tag used in stable keys / digest payloads.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WithinBudget => "within_budget",
            Self::BudgetExceeded => "budget_exceeded",
            Self::NotRun => "not_run",
        }
    }

    /// Lift a points-to sub-domain status into the unified status (D-05).
    pub fn from_points_to(status: PointsToBudgetStatus) -> Self {
        match status {
            PointsToBudgetStatus::WithinBudget => Self::WithinBudget,
            PointsToBudgetStatus::BudgetExceeded => Self::BudgetExceeded,
            PointsToBudgetStatus::NotRun => Self::NotRun,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BudgetReason {
    SolverMaxSteps,
    SolverMaxOuterIterations,
    PointsToMaxObjectsPerVar,
    PointsToMaxDynamicVars,
    GoAddressTakenThreshold,
    GoMaxCandidatesPerCallsite,
    GoMaxRtaRounds,
    GoMaxWorklistSteps,
    AdaptationMaxModelFiles,
    AdaptationMaxModelFacts,
    AdaptationMaxExpansionsPerModel,
    AdaptationMaxTargetsPerSource,
    AdaptationMaxModelDerivedEdges,
}

impl BudgetReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SolverMaxSteps => "solver.max_steps",
            Self::SolverMaxOuterIterations => "solver.max_outer_iterations",
            Self::PointsToMaxObjectsPerVar => "points_to.max_objects_per_var",
            Self::PointsToMaxDynamicVars => "points_to.max_dynamic_vars",
            Self::GoAddressTakenThreshold => "go.address_taken_threshold",
            Self::GoMaxCandidatesPerCallsite => "go.max_candidates_per_callsite",
            Self::GoMaxRtaRounds => "go.max_rta_rounds",
            Self::GoMaxWorklistSteps => "go.max_worklist_steps",
            Self::AdaptationMaxModelFiles => "adaptation.max_model_files",
            Self::AdaptationMaxModelFacts => "adaptation.max_model_facts",
            Self::AdaptationMaxExpansionsPerModel => "adaptation.max_expansions_per_model",
            Self::AdaptationMaxTargetsPerSource => "adaptation.max_targets_per_source",
            Self::AdaptationMaxModelDerivedEdges => "adaptation.max_model_derived_edges",
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::SolverMaxSteps,
            Self::SolverMaxOuterIterations,
            Self::PointsToMaxObjectsPerVar,
            Self::PointsToMaxDynamicVars,
            Self::GoAddressTakenThreshold,
            Self::GoMaxCandidatesPerCallsite,
            Self::GoMaxRtaRounds,
            Self::GoMaxWorklistSteps,
            Self::AdaptationMaxModelFiles,
            Self::AdaptationMaxModelFacts,
            Self::AdaptationMaxExpansionsPerModel,
            Self::AdaptationMaxTargetsPerSource,
            Self::AdaptationMaxModelDerivedEdges,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solver_budget_default_matches_points_to_defaults() {
        let budget = SolverBudget::default();
        assert_eq!(budget.max_steps, 10_000);
        assert!(budget.max_outer_iterations > 0);

        // The projection reproduces the standalone points-to defaults exactly.
        // The points-to step ceiling is its own knob: the cross-domain
        // `max_steps` also bounds the engine's edge closure.
        let projected = budget.points_to_budget();
        assert_eq!(projected, PointsToBudget::default());
        assert_eq!(projected.max_steps, 50_000_000);
        assert_eq!(projected.max_objects_per_var, 4_096);
        assert_eq!(projected.max_dynamic_vars, 4_000_000);
    }

    #[test]
    fn solver_budget_default_go_sub_budget_matches_go_defaults() {
        // The `go` sub-budget defaults are the roadmap-named honest caps. Adding the
        // field must not perturb the existing default fields (covered above).
        let budget = SolverBudget::default();
        assert_eq!(budget.go, GoRtaSubBudget::default());
        assert_eq!(budget.go.address_taken_threshold, 256);
        assert_eq!(budget.go.max_candidates_per_callsite, 128);
        assert_eq!(budget.go.max_rta_rounds, 32);
        // The Go-scaled worklist-step cap is sized like points-to `max_steps`
        // (10_000), never the policy-count 64 (CR-01).
        assert_eq!(budget.go.max_worklist_steps, 10_000);
        // The existing cross-domain/points-to defaults are still byte-identical.
        assert_eq!(budget.max_steps, 10_000);
        assert_eq!(budget.max_outer_iterations, 64);
        assert_eq!(budget.points_to, PointsToSubBudget::default());
    }

    #[test]
    fn solver_budget_default_adaptation_sub_budget_matches_adaptation_defaults() {
        let budget = SolverBudget::default();
        assert_eq!(budget.adaptation, AdaptationModelBudget::default());
        assert_eq!(budget.adaptation.max_model_files, 32);
        assert_eq!(budget.adaptation.max_model_facts, 512);
        assert_eq!(budget.adaptation.max_expansions_per_model, 64);
        assert_eq!(budget.adaptation.max_targets_per_source, 16);
        assert_eq!(budget.adaptation.max_model_derived_edges, 2_048);
    }

    #[test]
    fn points_to_budget_default_is_unchanged_by_projection() {
        // The standalone defaults are ceilings a real package stays under:
        // the old 10_000 / 64 / 512 stopped every package above about a
        // thousand call sites.
        let pt = PointsToBudget::default();
        assert_eq!(pt.max_steps, 50_000_000);
        assert_eq!(pt.max_objects_per_var, 4_096);
        assert_eq!(pt.max_dynamic_vars, 4_000_000);
    }

    #[test]
    fn budget_status_has_exactly_3_variants() {
        // Compile-time exhaustive match; adding a variant without updating this
        // test fails to compile, forcing a deliberate edit.
        fn assert_all(status: BudgetStatus) -> &'static str {
            match status {
                BudgetStatus::WithinBudget => "within_budget",
                BudgetStatus::BudgetExceeded => "budget_exceeded",
                BudgetStatus::NotRun => "not_run",
            }
        }
        let variants = [
            assert_all(BudgetStatus::WithinBudget),
            assert_all(BudgetStatus::BudgetExceeded),
            assert_all(BudgetStatus::NotRun),
        ];
        assert_eq!(variants.len(), 3);
    }

    #[test]
    fn budget_status_sorts_in_pinned_declaration_order() {
        // A permuted list sorts back to declaration order: the discriminant
        // ordering is declaration-driven, so the enum is byte-stable.
        let mut statuses = [
            BudgetStatus::NotRun,
            BudgetStatus::BudgetExceeded,
            BudgetStatus::WithinBudget,
        ];
        statuses.sort();
        let tags: Vec<&'static str> = statuses.iter().map(|s| s.as_str()).collect();
        assert_eq!(tags, vec!["within_budget", "budget_exceeded", "not_run"]);
    }

    #[test]
    fn budget_status_projects_from_points_to_status() {
        assert_eq!(
            BudgetStatus::from_points_to(PointsToBudgetStatus::WithinBudget),
            BudgetStatus::WithinBudget
        );
        assert_eq!(
            BudgetStatus::from_points_to(PointsToBudgetStatus::BudgetExceeded),
            BudgetStatus::BudgetExceeded
        );
        assert_eq!(
            BudgetStatus::from_points_to(PointsToBudgetStatus::NotRun),
            BudgetStatus::NotRun
        );
    }

    #[test]
    fn budget_reason_labels_are_stable_and_specific() {
        let labels = BudgetReason::all()
            .iter()
            .map(|reason| reason.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            labels,
            vec![
                "solver.max_steps",
                "solver.max_outer_iterations",
                "points_to.max_objects_per_var",
                "points_to.max_dynamic_vars",
                "go.address_taken_threshold",
                "go.max_candidates_per_callsite",
                "go.max_rta_rounds",
                "go.max_worklist_steps",
                "adaptation.max_model_files",
                "adaptation.max_model_facts",
                "adaptation.max_expansions_per_model",
                "adaptation.max_targets_per_source",
                "adaptation.max_model_derived_edges",
            ]
        );
    }
}
