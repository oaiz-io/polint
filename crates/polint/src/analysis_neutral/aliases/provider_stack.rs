use super::facts::{AliasAnswerFact, AliasOperand, AliasPrecision, AliasReason, AliasStatus};
use super::query::AliasQueryIndex;
use super::store::AliasOutput;
use crate::analysis_neutral::access_paths::facts::AccessPathFact;
use crate::analysis_neutral::ids::AliasAnswerId;
use crate::analysis_neutral::points_to::facts::PointsToSetFact;

pub const MAX_PROVIDER_STACK_PAIRS: usize = 64;

pub fn derive_alias_answers(
    interner: &crate::internal_core::StableKeyInterner,
    access_paths: &[AccessPathFact],
    points_to_sets: &[PointsToSetFact],
) -> AliasOutput {
    let index = AliasQueryIndex::new(access_paths, points_to_sets);
    let mut operands = access_paths
        .iter()
        .flat_map(|path| {
            [
                AliasOperand::Place(path.base),
                AliasOperand::AccessPath(path.id),
            ]
        })
        .collect::<Vec<_>>();
    sort_operands(&index, interner, &mut operands);
    operands.dedup();

    let mut answers = Vec::new();
    let mut budget_reported = false;
    for (left_index, left) in operands.iter().enumerate() {
        for right in operands.iter().skip(left_index) {
            if answers.len() >= MAX_PROVIDER_STACK_PAIRS {
                if !budget_reported {
                    answers.push(budget_exceeded_answer(&index, interner, *left, *right));
                    budget_reported = true;
                }
                break;
            }
            answers.push(index.answer(interner, *left, *right));
        }
    }
    AliasOutput { answers }.normalized(interner)
}

fn sort_operands(
    index: &AliasQueryIndex<'_>,
    interner: &crate::internal_core::StableKeyInterner,
    operands: &mut [AliasOperand],
) {
    // A base appears once per projection in the input. Build its potentially
    // large semantic identity once, then sort the original sequence by compact
    // lexical ranks. Equal text gets equal ranks, preserving stable-sort ties
    // and the subsequent adjacent-only deduplication exactly.
    let mut identities = std::collections::BTreeMap::new();
    for &operand in operands.iter() {
        identities
            .entry(operand)
            .or_insert_with(|| index.operand_stable_identity(interner, operand));
    }
    let mut identities = identities.into_iter().collect::<Vec<_>>();
    identities.sort_by(|left, right| left.1.cmp(&right.1));
    let mut ranks = std::collections::BTreeMap::new();
    let mut previous = None;
    let mut rank = 0usize;
    for (operand, identity) in &identities {
        if previous.is_some_and(|previous| previous != identity.as_str()) {
            rank += 1;
        }
        ranks.insert(*operand, rank);
        previous = Some(identity.as_str());
    }
    operands.sort_by_cached_key(|operand| ranks[operand]);
}

fn budget_exceeded_answer(
    index: &AliasQueryIndex<'_>,
    interner: &crate::internal_core::StableKeyInterner,
    left: AliasOperand,
    right: AliasOperand,
) -> AliasAnswerFact {
    AliasAnswerFact {
        id: AliasAnswerId(0),
        left,
        right,
        status: AliasStatus::Unknown,
        reason: AliasReason::BudgetExceeded,
        evidence: vec!["provider-stack alias pair budget exceeded".to_string()],
        precision: AliasPrecision::Unknown,
        stable_key: index.answer_stable_key(
            interner,
            left,
            right,
            AliasStatus::Unknown,
            &AliasReason::BudgetExceeded,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::access_paths::facts::{AccessPathProjection, AccessPathStatus};
    use crate::analysis_neutral::ids::{
        AccessPathId, ObjectTokenId, PlaceId, PointsToSetId, PtVarId,
    };
    use crate::analysis_neutral::points_to::facts::{
        PointsToBudgetStatus, PointsToPrecision, PointsToStatus,
    };
    use crate::analysis_neutral::points_to::vars;
    use crate::internal_core::Language;

    #[test]
    fn compact_operand_ranks_preserve_text_sort_ties_and_duplicates() {
        let interner = crate::internal_core::test_stable_key_interner();
        for count in [0, 1, 8, 65, 100] {
            let paths = (0..count)
                .map(|id| {
                    let mut fact = path(AccessPathId(id), PlaceId(id % 7), "field");
                    // Different paths can share identity text; stable-sort ties
                    // must not reorder their interleaved base operands.
                    fact.stable_key = interner.intern(format!("path:{}", id % 11));
                    fact
                })
                .collect::<Vec<_>>();
            let index = AliasQueryIndex::new(&paths, &[]);
            for reverse in [false, true] {
                let mut expected = paths
                    .iter()
                    .flat_map(|path| {
                        [
                            AliasOperand::Place(path.base),
                            AliasOperand::AccessPath(path.id),
                        ]
                    })
                    .collect::<Vec<_>>();
                if reverse {
                    expected.reverse();
                }
                let mut actual = expected.clone();
                expected.sort_by_cached_key(|operand| {
                    index.operand_stable_identity(&interner, *operand)
                });
                expected.dedup();
                sort_operands(&index, &interner, &mut actual);
                actual.dedup();
                assert_eq!(actual, expected);
            }
        }
    }

    #[test]
    fn provider_stack_derives_evidence_backed_answers() {
        let paths = vec![
            path(AccessPathId(0), PlaceId(1), "a"),
            path(AccessPathId(1), PlaceId(1), "b"),
            path(AccessPathId(2), PlaceId(2), "a"),
        ];
        let sets = vec![
            set(vars::place_var(PlaceId(1)), &[ObjectTokenId(1)]),
            set(vars::place_var(PlaceId(2)), &[ObjectTokenId(2)]),
            set(
                vars::access_path_var(AccessPathId(0)),
                &[ObjectTokenId(1), ObjectTokenId(2)],
            ),
            set(
                vars::access_path_var(AccessPathId(1)),
                &[ObjectTokenId(1), ObjectTokenId(3)],
            ),
        ];
        let output = derive_alias_answers(
            &crate::internal_core::test_stable_key_interner(),
            &paths,
            &sets,
        );

        assert!(!output.answers.is_empty());
        assert!(
            output
                .answers
                .iter()
                .all(|answer| !answer.evidence.is_empty())
        );
        assert!(output.answers.iter().any(|answer| {
            answer.status == crate::analysis_neutral::aliases::facts::AliasStatus::PartialAlias
        }));
    }

    #[test]
    fn provider_stack_reports_budget_exhaustion_instead_of_silent_truncation() {
        let paths = (0..20)
            .map(|id| path(AccessPathId(id), PlaceId(id + 1), "field"))
            .collect::<Vec<_>>();

        let output = derive_alias_answers(
            &crate::internal_core::test_stable_key_interner(),
            &paths,
            &[],
        );

        assert!(output.answers.iter().any(|answer| {
            answer.status == AliasStatus::Unknown && answer.reason == AliasReason::BudgetExceeded
        }));
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
