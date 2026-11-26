//! Weighs the solver's answer at each typed call site against the type
//! checker's.
//!
//! Types do not decide what flows: the solve and every other tier have run
//! before this step, and it neither adds to nor removes from what reaches a
//! callee set. It changes how strongly an edge is claimed and which candidates
//! an open site lists:
//!
//! - At a complete site, an exact edge to a function the checker's declared
//!   kind or arity says the call cannot run moves to the conservative tier
//!   with `type_mismatch`. It is never deleted: a value the checker was told
//!   the wrong type of still runs.
//! - At an open site, a method the checker found on the class that names the
//!   receiver is listed as a conservative candidate with `type_seed`.
//! - At a site no other tier answered, the checker's targets are listed at
//!   the conservative tier, and the site stays open: a type answer never
//!   claims a site's whole callee set.
//!
//! Every other typed row is dropped: where the solver answered, the type
//! tier's own rows would only repeat it or contradict it.

use std::collections::BTreeSet;

use super::facts::{RefinedCallConfidence, RefinedCallEdgeFact, RefinedCallTier};
use super::ts_types::{
    TsTypeDeclarationKind, TsTypeSiteStatus, TypedFunction, TypedRows, TypedSite,
};
use crate::analysis_api::{FactFamily, stable_key_from_parts};
use crate::analysis_neutral::calls::facts::{
    CallAlgorithm, CallEdgeKind, CallPrecision, CallProvenance, CallTargetStatus, ClaimTier,
    UnresolvedCallReason,
};
use crate::analysis_neutral::ids::{CallSiteId, RefinedCallEdgeId};
use crate::analysis_neutral::refined_calls::facts::RefinedCallValidation;
use crate::internal_core::{FunctionId, StableKeyInterner};

/// Evidence on an exact edge the checker's declarations moved to the
/// conservative tier.
pub(crate) const TYPE_MISMATCH: &str = "type_mismatch";
/// Evidence on a candidate the checker listed at an open site.
pub(crate) const TYPE_SEED: &str = "type_seed";

/// How many edges the step changed, for the stage log.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TypeEffects {
    pub(crate) mismatched: usize,
    pub(crate) seeded: usize,
    pub(crate) typed_only_sites: usize,
}

/// Applies the checker's answers to `edges`, the edges of every other tier.
pub(crate) fn assign_type_tiers(
    interner: &StableKeyInterner,
    edges: &mut Vec<RefinedCallEdgeFact>,
    typed: TypedRows,
) -> TypeEffects {
    let mut effects = TypeEffects::default();
    let mut answered = BTreeSet::new();
    let mut marked = BTreeSet::new();
    let mut listed: BTreeSet<(CallSiteId, FunctionId)> = BTreeSet::new();
    for edge in edges.iter() {
        if edge.status != CallTargetStatus::Resolved {
            marked.insert(edge.site);
        } else if edge.precision.tier() <= ClaimTier::Conservative {
            answered.insert(edge.site);
            if let Some(function) = edge.target_function {
                listed.insert((edge.site, function));
            }
        }
    }

    for edge in edges.iter_mut() {
        if edge.status != CallTargetStatus::Resolved
            || edge.precision.tier() != ClaimTier::Exact
            || marked.contains(&edge.site)
        {
            continue;
        }
        let Some(site) = typed.sites.get(&edge.site) else {
            continue;
        };
        let Some(function) = edge
            .target_function
            .and_then(|function| typed.functions.get(&function))
        else {
            continue;
        };
        if let Some(reason) = mismatch(site, function) {
            edge.precision = CallPrecision::Conservative;
            edge.assumptions.clear();
            edge.confidence = edge.confidence.weaker(RefinedCallConfidence::Medium);
            edge.evidence.push(TYPE_MISMATCH.to_string());
            edge.evidence.push(format!("{TYPE_MISMATCH}={reason}"));
            effects.mismatched += 1;
        }
    }

    let mut opened = BTreeSet::new();
    let mut typed_only = BTreeSet::new();
    for row in typed.rows {
        let mut edge = row.edge;
        let site = edge.site;
        let resolved = edge.status == CallTargetStatus::Resolved;
        if answered.contains(&site) {
            let unlisted = edge
                .target_function
                .is_some_and(|function| listed.insert((site, function)));
            if row.seeds && resolved && marked.contains(&site) && unlisted {
                at_most_conservative(&mut edge);
                edge.evidence.push(TYPE_SEED.to_string());
                edges.push(edge);
                effects.seeded += 1;
            }
            continue;
        }
        if resolved {
            at_most_conservative(&mut edge);
            typed_only.insert(site);
            if !marked.contains(&site) && opened.insert(site) {
                edges.push(open_typed_site_row(interner, &edge));
            }
        }
        edges.push(edge);
    }
    effects.typed_only_sites = typed_only.len();
    effects
}

/// Why the checker says the call cannot run `function`, if it does.
///
/// Kind is the runtime's rule: an arrow function, a method and an accessor
/// throw when constructed, and a class throws when called. Arity is the
/// checker's: a function that needs more arguments than the signature the
/// call resolved to declares is not assignable to it, so it reaches the call
/// only through a value the checker was told the wrong type of.
fn mismatch(site: &TypedSite, function: &TypedFunction) -> Option<&'static str> {
    let constructible = !matches!(
        function.kind,
        TsTypeDeclarationKind::Arrow
            | TsTypeDeclarationKind::Method
            | TsTypeDeclarationKind::Accessor
    );
    if site.constructs && !constructible {
        return Some("constructs_a_non_constructor");
    }
    if !site.constructs && function.kind == TsTypeDeclarationKind::Class {
        return Some("calls_a_class");
    }
    if site.status == TsTypeSiteStatus::Resolved
        && let (Some(required), Some(declared)) =
            (function.required_parameters, site.signature_parameters)
        && required > declared
    {
        return Some("arity");
    }
    None
}

fn at_most_conservative(edge: &mut RefinedCallEdgeFact) {
    if edge.precision.tier() < ClaimTier::Conservative {
        edge.precision = CallPrecision::Conservative;
        edge.assumptions.clear();
    }
}

/// The row that keeps a site the checker alone answered open: the checker
/// names targets the call may reach, not every callee the call can run.
fn open_typed_site_row(
    interner: &StableKeyInterner,
    typed: &RefinedCallEdgeFact,
) -> RefinedCallEdgeFact {
    let site_key = typed.input_stable_keys.first().cloned().unwrap_or_default();
    RefinedCallEdgeFact {
        id: RefinedCallEdgeId(0),
        site: typed.site,
        base_target: None,
        caller: typed.caller,
        target_function: None,
        target_symbol: None,
        synthetic_target: None,
        language: typed.language,
        edge_kind: CallEdgeKind::Unknown,
        algorithm: CallAlgorithm::TypeHierarchy,
        tier: RefinedCallTier::TypeDirected,
        status: CallTargetStatus::Unresolved,
        reason: Some(UnresolvedCallReason::UnknownCallee),
        provenance: CallProvenance::Native,
        precision: CallPrecision::Unknown,
        assumptions: Vec::new(),
        validation: RefinedCallValidation::ReferentiallyValidated,
        confidence: RefinedCallConfidence::Low,
        evidence: vec!["ts_type_checker".to_string(), "ts_type_open".to_string()],
        input_stable_keys: vec![site_key.clone()],
        stable_key: stable_key_from_parts(
            interner,
            FactFamily::RefinedCallEdge,
            &[("tier", "ts_type_directed".to_string()), ("open", site_key)],
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::points_to::tiers::Assumption;
    use crate::analysis_neutral::refined_calls::ts_types::TypedRow;
    use crate::internal_core::Language;

    const SITE: CallSiteId = CallSiteId(7);

    fn interner() -> StableKeyInterner {
        StableKeyInterner::default()
    }

    fn edge(
        interner: &StableKeyInterner,
        key: &str,
        target: Option<u64>,
        tier: RefinedCallTier,
        status: CallTargetStatus,
        precision: CallPrecision,
    ) -> RefinedCallEdgeFact {
        RefinedCallEdgeFact {
            id: RefinedCallEdgeId(0),
            site: SITE,
            base_target: None,
            caller: FunctionId::from_raw(1),
            target_function: target.map(FunctionId::from_raw),
            target_symbol: None,
            synthetic_target: None,
            language: Language::TypeScript,
            edge_kind: CallEdgeKind::Method,
            algorithm: CallAlgorithm::PointsTo,
            tier,
            status,
            reason: (status != CallTargetStatus::Resolved)
                .then_some(UnresolvedCallReason::UnknownCallee),
            provenance: CallProvenance::Native,
            precision,
            assumptions: if precision == CallPrecision::Exact {
                vec![Assumption::WithinBudget]
            } else {
                Vec::new()
            },
            validation: RefinedCallValidation::ReferentiallyValidated,
            confidence: RefinedCallConfidence::High,
            evidence: Vec::new(),
            input_stable_keys: vec!["site".to_string()],
            stable_key: interner.intern(key),
        }
    }

    fn solver_exact(interner: &StableKeyInterner, target: u64) -> RefinedCallEdgeFact {
        edge(
            interner,
            &format!("solver:{target}"),
            Some(target),
            RefinedCallTier::PointsToAssisted,
            CallTargetStatus::Resolved,
            CallPrecision::Exact,
        )
    }

    fn solver_open(interner: &StableKeyInterner) -> RefinedCallEdgeFact {
        edge(
            interner,
            "solver:open",
            None,
            RefinedCallTier::PointsToAssisted,
            CallTargetStatus::Unresolved,
            CallPrecision::Unknown,
        )
    }

    fn typed_row(interner: &StableKeyInterner, target: u64, seeds: bool) -> TypedRow {
        TypedRow {
            edge: edge(
                interner,
                &format!("typed:{target}"),
                Some(target),
                RefinedCallTier::TypeDirected,
                CallTargetStatus::Resolved,
                CallPrecision::Exact,
            ),
            seeds,
        }
    }

    fn typed(
        rows: Vec<TypedRow>,
        site: TypedSite,
        functions: &[(u64, TypedFunction)],
    ) -> TypedRows {
        TypedRows {
            rows,
            sites: [(SITE, site)].into_iter().collect(),
            functions: functions
                .iter()
                .map(|(id, function)| (FunctionId::from_raw(*id), *function))
                .collect(),
            report: Default::default(),
        }
    }

    fn call(signature_parameters: Option<u32>) -> TypedSite {
        TypedSite {
            status: TsTypeSiteStatus::Resolved,
            signature_parameters,
            constructs: false,
        }
    }

    fn method(required_parameters: Option<u32>) -> TypedFunction {
        TypedFunction {
            kind: TsTypeDeclarationKind::Method,
            required_parameters,
        }
    }

    fn site_is_complete(edges: &[RefinedCallEdgeFact]) -> bool {
        edges
            .iter()
            .all(|edge| edge.status == CallTargetStatus::Resolved)
            && edges
                .iter()
                .any(|edge| edge.precision.tier() <= ClaimTier::Conservative)
    }

    #[test]
    fn a_function_needing_more_arguments_than_the_signature_moves_to_conservative() {
        let interner = interner();
        let mut edges = vec![solver_exact(&interner, 10), solver_exact(&interner, 11)];
        let effects = assign_type_tiers(
            &interner,
            &mut edges,
            typed(
                Vec::new(),
                call(Some(1)),
                &[(10, method(Some(2))), (11, method(Some(1)))],
            ),
        );

        assert_eq!(effects.mismatched, 1);
        assert_eq!(edges.len(), 2, "a mismatch never deletes an edge");
        let demoted = &edges[0];
        assert_eq!(demoted.precision, CallPrecision::Conservative);
        assert!(demoted.assumptions.is_empty());
        assert!(demoted.evidence.contains(&TYPE_MISMATCH.to_string()));
        assert!(
            demoted
                .evidence
                .contains(&"type_mismatch=arity".to_string())
        );
        assert_eq!(edges[1].precision, CallPrecision::Exact);
        assert!(
            site_is_complete(&edges),
            "the site stays complete, at the conservative tier for the demoted edge"
        );
    }

    #[test]
    fn construction_and_calls_mismatch_by_kind() {
        let interner = interner();
        let arrow = TypedFunction {
            kind: TsTypeDeclarationKind::Arrow,
            required_parameters: None,
        };
        let class = TypedFunction {
            kind: TsTypeDeclarationKind::Class,
            required_parameters: None,
        };
        let constructs = TypedSite {
            constructs: true,
            ..call(None)
        };

        let mut constructed = vec![solver_exact(&interner, 10)];
        assign_type_tiers(
            &interner,
            &mut constructed,
            typed(Vec::new(), constructs, &[(10, arrow)]),
        );
        assert!(
            constructed[0]
                .evidence
                .contains(&"type_mismatch=constructs_a_non_constructor".to_string())
        );

        let mut called = vec![solver_exact(&interner, 10)];
        assign_type_tiers(
            &interner,
            &mut called,
            typed(Vec::new(), call(None), &[(10, class)]),
        );
        assert!(
            called[0]
                .evidence
                .contains(&"type_mismatch=calls_a_class".to_string())
        );

        let mut constructed_class = vec![solver_exact(&interner, 10)];
        let effects = assign_type_tiers(
            &interner,
            &mut constructed_class,
            typed(Vec::new(), constructs, &[(10, class)]),
        );
        assert_eq!(effects.mismatched, 0);
        assert_eq!(constructed_class[0].precision, CallPrecision::Exact);
    }

    #[test]
    fn arity_is_judged_only_where_the_checker_resolved_the_call_and_both_counts_are_known() {
        let interner = interner();
        let union = TypedSite {
            status: TsTypeSiteStatus::Union,
            ..call(Some(0))
        };
        for (site, function) in [
            (union, method(Some(2))),
            (call(None), method(Some(2))),
            (call(Some(0)), method(None)),
        ] {
            let mut edges = vec![solver_exact(&interner, 10)];
            let effects = assign_type_tiers(
                &interner,
                &mut edges,
                typed(Vec::new(), site, &[(10, function)]),
            );
            assert_eq!(effects.mismatched, 0, "{site:?} {function:?}");
        }
    }

    #[test]
    fn an_open_site_is_never_judged_and_gains_the_receiver_class_method_as_a_seed() {
        let interner = interner();
        let mut edges = vec![solver_exact(&interner, 10), solver_open(&interner)];
        let effects = assign_type_tiers(
            &interner,
            &mut edges,
            typed(
                vec![
                    typed_row(&interner, 10, true),
                    typed_row(&interner, 12, true),
                    typed_row(&interner, 13, false),
                ],
                call(Some(0)),
                &[(10, method(Some(2)))],
            ),
        );

        assert_eq!(
            effects.mismatched, 0,
            "an open site claims nothing exact to demote"
        );
        assert_eq!(effects.seeded, 1);
        let seeds = edges
            .iter()
            .filter(|edge| edge.evidence.contains(&TYPE_SEED.to_string()))
            .collect::<Vec<_>>();
        assert_eq!(seeds.len(), 1);
        assert_eq!(seeds[0].target_function, Some(FunctionId::from_raw(12)));
        assert_eq!(seeds[0].precision, CallPrecision::Conservative);
        assert!(
            edges
                .iter()
                .any(|edge| edge.status == CallTargetStatus::Unresolved),
            "a seed never closes the site"
        );
    }

    #[test]
    fn a_complete_site_takes_no_typed_rows() {
        let interner = interner();
        let mut edges = vec![solver_exact(&interner, 10)];
        let effects = assign_type_tiers(
            &interner,
            &mut edges,
            typed(vec![typed_row(&interner, 12, true)], call(None), &[]),
        );
        assert_eq!(effects, TypeEffects::default());
        assert_eq!(edges.len(), 1);
    }

    #[test]
    fn a_site_only_the_checker_answered_lists_conservative_targets_and_stays_open() {
        let interner = interner();
        let mut edges = Vec::new();
        let effects = assign_type_tiers(
            &interner,
            &mut edges,
            typed(
                vec![
                    typed_row(&interner, 12, false),
                    typed_row(&interner, 13, true),
                ],
                call(None),
                &[],
            ),
        );

        assert_eq!(effects.typed_only_sites, 1);
        let targets = edges
            .iter()
            .filter(|edge| edge.status == CallTargetStatus::Resolved)
            .collect::<Vec<_>>();
        assert_eq!(targets.len(), 2);
        assert!(targets.iter().all(
            |edge| edge.precision == CallPrecision::Conservative && edge.assumptions.is_empty()
        ));
        let open = edges
            .iter()
            .filter(|edge| edge.status == CallTargetStatus::Unresolved)
            .collect::<Vec<_>>();
        assert_eq!(open.len(), 1, "one open row however many targets");
        assert!(open[0].evidence.contains(&"ts_type_open".to_string()));
    }

    #[test]
    fn a_marked_site_no_tier_answered_takes_typed_targets_without_another_open_row() {
        let interner = interner();
        let mut edges = vec![solver_open(&interner)];
        assign_type_tiers(
            &interner,
            &mut edges,
            typed(vec![typed_row(&interner, 12, false)], call(None), &[]),
        );
        assert_eq!(
            edges
                .iter()
                .filter(|edge| edge.status == CallTargetStatus::Unresolved)
                .count(),
            1
        );
        assert_eq!(edges.len(), 2);
    }
}
