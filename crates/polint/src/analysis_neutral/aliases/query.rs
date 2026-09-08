use std::collections::{BTreeMap, BTreeSet};

use super::facts::{AliasAnswerFact, AliasOperand, AliasPrecision, AliasReason, AliasStatus};
use crate::analysis_api::{FactFamily, stable_key_from_parts};
use crate::analysis_neutral::access_paths::facts::AccessPathFact;
use crate::analysis_neutral::ids::{AliasAnswerId, ObjectTokenId, PtVarId};
use crate::analysis_neutral::points_to::facts::{
    PointsToBudgetStatus, PointsToSetFact, PointsToStatus,
};
use crate::analysis_neutral::points_to::vars;

#[derive(Debug, Default)]
pub struct AliasQueryIndex<'a> {
    access_paths: BTreeMap<crate::analysis_neutral::ids::AccessPathId, &'a AccessPathFact>,
    paths_by_base: BTreeMap<crate::analysis_neutral::ids::PlaceId, Vec<&'a AccessPathFact>>,
    points_to: BTreeMap<PtVarId, &'a PointsToSetFact>,
    budget_exceeded: bool,
}

impl<'a> AliasQueryIndex<'a> {
    pub fn new(access_paths: &'a [AccessPathFact], points_to: &'a [PointsToSetFact]) -> Self {
        let access_paths = access_paths
            .iter()
            .map(|path| (path.id, path))
            .collect::<BTreeMap<_, _>>();
        let mut paths_by_base: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for &path in access_paths.values() {
            paths_by_base.entry(path.base).or_default().push(path);
        }
        Self {
            access_paths,
            paths_by_base,
            points_to: points_to.iter().map(|set| (set.variable, set)).collect(),
            budget_exceeded: points_to
                .iter()
                .any(|set| set.budget == PointsToBudgetStatus::BudgetExceeded),
        }
    }

    pub fn answer(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
        left: AliasOperand,
        right: AliasOperand,
    ) -> AliasAnswerFact {
        let (status, reason, precision, evidence) = self.classify(left, right);
        let stable_key = self.answer_stable_key(interner, left, right, status, &reason);
        AliasAnswerFact {
            id: AliasAnswerId(0),
            left,
            right,
            status,
            reason,
            evidence,
            precision,
            stable_key,
        }
    }

    pub(super) fn operand_stable_identity(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
        operand: AliasOperand,
    ) -> String {
        match operand {
            AliasOperand::AccessPath(path) => self
                .access_paths
                .get(&path)
                .map(|fact| interner.resolve(fact.stable_key).to_string())
                .expect("alias access-path operand must reference an indexed path"),
            AliasOperand::Place(place) => {
                let mut path_keys = self
                    .paths_by_base
                    .get(&place)
                    .into_iter()
                    .flatten()
                    .map(|path| interner.resolve(path.stable_key).to_string())
                    .collect::<BTreeSet<_>>();
                if path_keys.is_empty()
                    && let Some(set) = self.points_to.get(&vars::place_var(place))
                {
                    path_keys.insert(interner.resolve(set.stable_key).to_string());
                }
                semantic_relation_identity(&path_keys)
            }
        }
    }

    pub(super) fn answer_stable_key(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
        left: AliasOperand,
        right: AliasOperand,
        status: AliasStatus,
        reason: &AliasReason,
    ) -> crate::internal_core::StableKeyId {
        stable_key_from_parts(
            interner,
            FactFamily::AliasAnswer,
            &[
                ("left", self.operand_stable_identity(interner, left)),
                ("right", self.operand_stable_identity(interner, right)),
                ("status", alias_status_label(status).to_string()),
                ("reason", alias_reason_label(reason).to_string()),
            ],
        )
    }

    fn classify(
        &self,
        left: AliasOperand,
        right: AliasOperand,
    ) -> (AliasStatus, AliasReason, AliasPrecision, Vec<String>) {
        if left == right || self.same_access_path(left, right) {
            return (
                AliasStatus::MustAlias,
                AliasReason::SameStablePlace,
                AliasPrecision::ExactLocal,
                vec!["same stable place/access path".to_string()],
            );
        }
        if self.common_base_different_projection(left, right) {
            return (
                AliasStatus::PartialAlias,
                AliasReason::CommonBaseDifferentProjection,
                AliasPrecision::Conservative,
                vec!["common base with different known projections".to_string()],
            );
        }
        if self.budget_exceeded {
            return (
                AliasStatus::Unknown,
                AliasReason::BudgetExceeded,
                AliasPrecision::Unknown,
                vec!["points-to budget exceeded".to_string()],
            );
        }
        let Some(left_objects) = self.objects_for(left) else {
            return (
                AliasStatus::Unknown,
                AliasReason::MissingPointsTo,
                AliasPrecision::Unknown,
                vec!["missing left points-to set".to_string()],
            );
        };
        let Some(right_objects) = self.objects_for(right) else {
            return (
                AliasStatus::Unknown,
                AliasReason::MissingPointsTo,
                AliasPrecision::Unknown,
                vec!["missing right points-to set".to_string()],
            );
        };
        if left_objects.is_empty() || right_objects.is_empty() {
            return (
                AliasStatus::Unknown,
                AliasReason::MissingPointsTo,
                AliasPrecision::Unknown,
                vec!["empty points-to set".to_string()],
            );
        }
        if left_objects.is_disjoint(&right_objects) {
            return (
                AliasStatus::NoAlias,
                AliasReason::DisjointPointsToSets,
                AliasPrecision::FlowInsensitive,
                vec!["disjoint points-to sets".to_string()],
            );
        }
        if left_objects.len() == 1 && left_objects == right_objects {
            return (
                AliasStatus::MustAlias,
                AliasReason::SingletonEqualObject,
                AliasPrecision::FlowInsensitive,
                vec!["singleton-equal object token".to_string()],
            );
        }
        (
            AliasStatus::MayAlias,
            AliasReason::OverlappingPointsToSets,
            AliasPrecision::Conservative,
            vec!["overlapping points-to sets".to_string()],
        )
    }

    fn same_access_path(&self, left: AliasOperand, right: AliasOperand) -> bool {
        match (left, right) {
            (AliasOperand::Place(place), AliasOperand::AccessPath(path))
            | (AliasOperand::AccessPath(path), AliasOperand::Place(place)) => self
                .access_paths
                .get(&path)
                .is_some_and(|path| path.base == place && path.projections.is_empty()),
            (AliasOperand::AccessPath(left), AliasOperand::AccessPath(right)) => self
                .access_paths
                .get(&left)
                .zip(self.access_paths.get(&right))
                .is_some_and(|(left, right)| {
                    left.base == right.base && left.projections == right.projections
                }),
            _ => false,
        }
    }

    fn common_base_different_projection(&self, left: AliasOperand, right: AliasOperand) -> bool {
        match (left, right) {
            (AliasOperand::AccessPath(left), AliasOperand::AccessPath(right)) => self
                .access_paths
                .get(&left)
                .zip(self.access_paths.get(&right))
                .is_some_and(|(left, right)| {
                    left.base == right.base && left.projections != right.projections
                }),
            _ => false,
        }
    }

    fn objects_for(&self, operand: AliasOperand) -> Option<BTreeSet<ObjectTokenId>> {
        let variable = match operand {
            AliasOperand::Place(place) => vars::place_var(place),
            AliasOperand::AccessPath(path) => vars::access_path_var(path),
        };
        let set = self.points_to.get(&variable)?;
        if set.status == PointsToStatus::BudgetExceeded {
            return None;
        }
        Some(set.objects.iter().copied().collect())
    }
}

fn semantic_relation_identity(fragments: &BTreeSet<String>) -> String {
    assert!(
        !fragments.is_empty(),
        "alias place operand must be referenced by at least one access path"
    );
    fragments
        .iter()
        .map(|fragment| format!("{}:{fragment}", fragment.len()))
        .collect()
}

fn alias_status_label(status: AliasStatus) -> &'static str {
    match status {
        AliasStatus::NoAlias => "no_alias",
        AliasStatus::MayAlias => "may_alias",
        AliasStatus::MustAlias => "must_alias",
        AliasStatus::PartialAlias => "partial_alias",
        AliasStatus::Unknown => "unknown",
    }
}

fn alias_reason_label(reason: &AliasReason) -> &'static str {
    match reason {
        AliasReason::SameStablePlace => "same_stable_place",
        AliasReason::DisjointLocals => "disjoint_locals",
        AliasReason::DisjointAllocations => "disjoint_allocations",
        AliasReason::DisjointPointsToSets => "disjoint_points_to_sets",
        AliasReason::OverlappingPointsToSets => "overlapping_points_to_sets",
        AliasReason::SingletonEqualObject => "singleton_equal_object",
        AliasReason::CommonBaseDifferentProjection => "common_base_different_projection",
        AliasReason::UnsupportedDynamicConstruct => "unsupported_dynamic_construct",
        AliasReason::SetupMissing => "setup_missing",
        AliasReason::BudgetExceeded => "budget_exceeded",
        AliasReason::ExtensionProvided => "extension_provided",
        AliasReason::MissingPointsTo => "missing_points_to",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::access_paths::facts::{AccessPathProjection, AccessPathStatus};
    use crate::analysis_neutral::ids::{AccessPathId, PlaceId, PointsToSetId};
    use crate::analysis_neutral::points_to::facts::{PointsToPrecision, PointsToStatus};
    use crate::analysis_neutral::points_to::vars;
    use crate::internal_core::Language;

    #[test]
    fn base_path_index_matches_full_scan_after_duplicate_id_resolution() {
        let interner = crate::internal_core::test_stable_key_interner();
        let paths = (0..100)
            .map(|id| path(AccessPathId(id % 31), PlaceId(id % 7), "field"))
            .collect::<Vec<_>>();
        let index = AliasQueryIndex::new(&paths, &[]);
        let indexed_paths = paths
            .iter()
            .map(|path| (path.id, path))
            .collect::<BTreeMap<_, _>>();
        for base in 0..7 {
            let fragments = indexed_paths
                .values()
                .filter(|path| path.base == PlaceId(base))
                .map(|path| interner.resolve(path.stable_key).to_string())
                .collect::<BTreeSet<_>>();
            assert_eq!(
                index.operand_stable_identity(&interner, AliasOperand::Place(PlaceId(base))),
                semantic_relation_identity(&fragments),
            );
        }
    }

    #[test]
    fn alias_query_returns_all_statuses_with_evidence() {
        let paths = vec![
            path(AccessPathId(0), PlaceId(1), "a"),
            path(AccessPathId(1), PlaceId(1), "b"),
            path(AccessPathId(2), PlaceId(9), "missing"),
        ];
        let sets = vec![
            set(vars::place_var(PlaceId(1)), &[ObjectTokenId(1)]),
            set(vars::place_var(PlaceId(2)), &[ObjectTokenId(2)]),
            set(
                vars::place_var(PlaceId(3)),
                &[ObjectTokenId(1), ObjectTokenId(2)],
            ),
            set(
                vars::place_var(PlaceId(4)),
                &[ObjectTokenId(1), ObjectTokenId(3)],
            ),
        ];
        let index = AliasQueryIndex::new(&paths, &sets);

        let answers = [
            index.answer(
                &crate::internal_core::test_stable_key_interner(),
                AliasOperand::Place(PlaceId(1)),
                AliasOperand::Place(PlaceId(1)),
            ),
            index.answer(
                &crate::internal_core::test_stable_key_interner(),
                AliasOperand::Place(PlaceId(1)),
                AliasOperand::Place(PlaceId(2)),
            ),
            index.answer(
                &crate::internal_core::test_stable_key_interner(),
                AliasOperand::Place(PlaceId(3)),
                AliasOperand::Place(PlaceId(4)),
            ),
            index.answer(
                &crate::internal_core::test_stable_key_interner(),
                AliasOperand::AccessPath(AccessPathId(0)),
                AliasOperand::AccessPath(AccessPathId(1)),
            ),
            index.answer(
                &crate::internal_core::test_stable_key_interner(),
                AliasOperand::Place(PlaceId(1)),
                AliasOperand::Place(PlaceId(9)),
            ),
        ];
        for status in [
            AliasStatus::MustAlias,
            AliasStatus::NoAlias,
            AliasStatus::MayAlias,
            AliasStatus::PartialAlias,
            AliasStatus::Unknown,
        ] {
            assert!(answers.iter().any(|answer| answer.status == status));
        }
        assert!(answers.iter().all(|answer| !answer.evidence.is_empty()));
    }

    #[test]
    fn zero_projection_access_path_aliases_its_base_place() {
        let paths = vec![AccessPathFact {
            id: AccessPathId(7),
            base: PlaceId(3),
            projections: Vec::new(),
            depth: 0,
            language: Language::TypeScript,
            file: None,
            function: None,
            body: None,
            status: AccessPathStatus::Resolved,
            stable_key: crate::internal_core::stable_key_for_test("path:root"),
        }];
        let index = AliasQueryIndex::new(&paths, &[]);

        let answer = index.answer(
            &crate::internal_core::test_stable_key_interner(),
            AliasOperand::Place(PlaceId(3)),
            AliasOperand::AccessPath(AccessPathId(7)),
        );

        assert_eq!(answer.status, AliasStatus::MustAlias);
        assert_eq!(answer.reason, AliasReason::SameStablePlace);
    }

    fn path(id: AccessPathId, base: PlaceId, field: &str) -> AccessPathFact {
        AccessPathFact {
            id,
            base,
            projections: vec![AccessPathProjection::Property(field.to_string())],
            depth: 1,
            language: Language::TypeScript,
            file: None,
            function: None,
            body: None,
            status: AccessPathStatus::Partial,
            stable_key: crate::internal_core::stable_key_for_test(&format!("path:{}", id.0)),
        }
    }

    fn set(variable: PtVarId, objects: &[ObjectTokenId]) -> PointsToSetFact {
        PointsToSetFact {
            id: PointsToSetId(0),
            variable,
            objects: objects.to_vec(),
            status: PointsToStatus::Present,
            precision: PointsToPrecision::FlowInsensitive,
            budget: PointsToBudgetStatus::WithinBudget,
            stable_key: crate::internal_core::stable_key_for_test(&format!("set:{}", variable.0)),
        }
    }
}
