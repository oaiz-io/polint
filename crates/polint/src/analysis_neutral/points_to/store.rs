use std::collections::BTreeMap;

use super::facts::{
    PointsToBudgetStatus, PointsToConstraintFact, PointsToPrecision, PointsToSetFact,
    PointsToStatus,
};
use crate::analysis_neutral::ids::{ObjectTokenId, PointsToConstraintId, PointsToSetId, PtVarId};
use crate::internal_core::StableKeyInterner;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PointsToOutput {
    pub constraints: Vec<PointsToConstraintFact>,
    pub sets: Vec<PointsToSetFact>,
}

impl PointsToOutput {
    pub fn normalized(mut self, interner: &StableKeyInterner) -> Self {
        self.constraints.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.id.cmp(&right.id))
        });
        self.sets.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.id.cmp(&right.id))
        });
        for (index, row) in self.constraints.iter_mut().enumerate() {
            row.id = PointsToConstraintId(index as u64);
        }
        for (index, row) in self.sets.iter_mut().enumerate() {
            row.id = PointsToSetId(index as u64);
            row.objects.sort();
            row.objects.dedup();
        }
        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct PointsToStore {
    output: PointsToOutput,
    constraints_by_status: BTreeMap<PointsToStatus, Vec<usize>>,
    constraints_by_precision: BTreeMap<PointsToPrecision, Vec<usize>>,
    sets_by_variable: BTreeMap<PtVarId, Vec<usize>>,
    sets_by_object: BTreeMap<ObjectTokenId, Vec<usize>>,
    sets_by_budget: BTreeMap<PointsToBudgetStatus, Vec<usize>>,
}

impl PointsToStore {
    pub fn from_output(output: PointsToOutput, interner: &StableKeyInterner) -> Self {
        Self::from_normalized_output(output.normalized(interner))
    }

    pub fn from_normalized_output(output: PointsToOutput) -> Self {
        let mut store = Self {
            output,
            ..Self::default()
        };
        for (index, row) in store.output.constraints.iter().enumerate() {
            store
                .constraints_by_status
                .entry(row.status)
                .or_default()
                .push(index);
            store
                .constraints_by_precision
                .entry(row.precision)
                .or_default()
                .push(index);
        }
        for (index, row) in store.output.sets.iter().enumerate() {
            store
                .sets_by_variable
                .entry(row.variable)
                .or_default()
                .push(index);
            for object in &row.objects {
                store.sets_by_object.entry(*object).or_default().push(index);
            }
            store
                .sets_by_budget
                .entry(row.budget)
                .or_default()
                .push(index);
        }
        store
    }

    pub fn constraints(&self) -> &[PointsToConstraintFact] {
        &self.output.constraints
    }

    pub fn sets(&self) -> &[PointsToSetFact] {
        &self.output.sets
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::points_to::facts::{
        PointsToConstraintKind, PointsToPrecision, PointsToStatus,
    };

    fn constraint(id: u64, stable_key: &str) -> PointsToConstraintFact {
        PointsToConstraintFact {
            id: PointsToConstraintId(id),
            kind: PointsToConstraintKind::Copy {
                dst: PtVarId(1),
                src: PtVarId(2),
            },
            status: PointsToStatus::Present,
            precision: PointsToPrecision::FlowInsensitive,
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
        }
    }

    #[test]
    fn points_to_output_sorts_by_stable_key_and_reassigns_ids() {
        let interner = crate::internal_core::test_stable_key_interner();
        let output = PointsToOutput {
            constraints: vec![constraint(9, "pt:z"), constraint(2, "pt:a")],
            sets: Vec::new(),
        }
        .normalized(&interner);

        assert_eq!(
            interner.resolve(output.constraints[0].stable_key).as_ref(),
            "pt:a"
        );
        assert_eq!(output.constraints[0].id, PointsToConstraintId(0));
        assert_eq!(output.constraints[1].id, PointsToConstraintId(1));
    }
}
