use std::collections::BTreeMap;

use super::facts::{AliasAnswerFact, AliasOperand, AliasPrecision, AliasStatus};
use crate::analysis_neutral::ids::{AccessPathId, AliasAnswerId, PlaceId};
use crate::internal_core::StableKeyInterner;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AliasOutput {
    pub answers: Vec<AliasAnswerFact>,
}

impl AliasOutput {
    pub fn normalized(mut self, interner: &StableKeyInterner) -> Self {
        self.answers.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.id.cmp(&right.id))
        });
        for (index, row) in self.answers.iter_mut().enumerate() {
            row.id = AliasAnswerId(index as u64);
        }
        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct AliasStore {
    output: AliasOutput,
    by_status: BTreeMap<AliasStatus, Vec<usize>>,
    by_precision: BTreeMap<AliasPrecision, Vec<usize>>,
    by_place: BTreeMap<PlaceId, Vec<usize>>,
    by_access_path: BTreeMap<AccessPathId, Vec<usize>>,
}

impl AliasStore {
    pub fn from_output(output: AliasOutput, interner: &StableKeyInterner) -> Self {
        Self::from_normalized_output(output.normalized(interner))
    }

    pub fn from_normalized_output(output: AliasOutput) -> Self {
        let mut store = Self {
            output,
            ..Self::default()
        };
        for (index, row) in store.output.answers.iter().enumerate() {
            store.by_status.entry(row.status).or_default().push(index);
            store
                .by_precision
                .entry(row.precision)
                .or_default()
                .push(index);
            match row.left {
                AliasOperand::Place(place) => store.by_place.entry(place).or_default().push(index),
                AliasOperand::AccessPath(path) => {
                    store.by_access_path.entry(path).or_default().push(index);
                }
            }
            match row.right {
                AliasOperand::Place(place) => store.by_place.entry(place).or_default().push(index),
                AliasOperand::AccessPath(path) => {
                    store.by_access_path.entry(path).or_default().push(index);
                }
            }
        }
        store
    }

    pub fn answers(&self) -> &[AliasAnswerFact] {
        &self.output.answers
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::aliases::facts::{AliasReason, AliasStatus};

    fn answer(id: u64, stable_key: &str) -> AliasAnswerFact {
        AliasAnswerFact {
            id: AliasAnswerId(id),
            left: AliasOperand::Place(PlaceId(1)),
            right: AliasOperand::Place(PlaceId(2)),
            status: AliasStatus::Unknown,
            reason: AliasReason::MissingPointsTo,
            evidence: vec!["missing-points-to".to_string()],
            precision: AliasPrecision::Unknown,
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
        }
    }

    #[test]
    fn alias_output_sorts_by_stable_key_and_reassigns_ids() {
        let interner = crate::internal_core::test_stable_key_interner();
        let output = AliasOutput {
            answers: vec![answer(10, "alias:z"), answer(4, "alias:a")],
        }
        .normalized(&interner);

        assert_eq!(
            interner.resolve(output.answers[0].stable_key).as_ref(),
            "alias:a"
        );
        assert_eq!(output.answers[0].id, AliasAnswerId(0));
        assert_eq!(output.answers[1].id, AliasAnswerId(1));
    }
}
