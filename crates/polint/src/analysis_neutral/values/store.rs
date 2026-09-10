use std::collections::BTreeMap;

use super::facts::{
    AllocationKind, AllocationTokenFact, ValueFact, ValueKind, ValueStatus, ValueSubject,
};
use crate::analysis_neutral::ids::{AbstractValueId, AllocationTokenId, PlaceId, ValueFactId};
use crate::internal_core::{FunctionId, Language, StableKeyInterner};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValueOutput {
    pub values: Vec<ValueFact>,
    pub allocations: Vec<AllocationTokenFact>,
}

impl ValueOutput {
    pub fn normalized(mut self, interner: &StableKeyInterner) -> Self {
        self.allocations.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.id.cmp(&right.id))
        });
        let allocation_remap = self
            .allocations
            .iter()
            .enumerate()
            .map(|(index, row)| (row.id, AllocationTokenId(index as u64)))
            .collect::<BTreeMap<_, _>>();
        for (index, row) in self.allocations.iter_mut().enumerate() {
            row.id = AllocationTokenId(index as u64);
        }
        self.values.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.id.cmp(&right.id))
        });
        for (index, row) in self.values.iter_mut().enumerate() {
            row.id = ValueFactId(index as u64);
            row.value = AbstractValueId(index as u64);
            remap_value_allocation_refs(row, &allocation_remap);
        }
        self
    }
}

fn remap_value_allocation_refs(
    row: &mut ValueFact,
    allocation_remap: &BTreeMap<AllocationTokenId, AllocationTokenId>,
) {
    if let ValueSubject::Allocation(token) = &mut row.subject
        && let Some(remapped) = allocation_remap.get(token)
    {
        *token = *remapped;
    }

    match &mut row.kind {
        ValueKind::Object(token) | ValueKind::Array(token) | ValueKind::CompositeLiteral(token) => {
            if let Some(remapped) = allocation_remap.get(token) {
                *token = *remapped;
            }
        }
        _ => {}
    }
}

#[derive(Debug, Clone, Default)]
pub struct ValueStore {
    output: ValueOutput,
    by_language: BTreeMap<Language, Vec<usize>>,
    by_function: BTreeMap<FunctionId, Vec<usize>>,
    by_status: BTreeMap<ValueStatus, Vec<usize>>,
    allocations_by_kind: BTreeMap<AllocationKind, Vec<usize>>,
    by_place: BTreeMap<PlaceId, Vec<usize>>,
}

impl ValueStore {
    pub fn from_output(output: ValueOutput, interner: &StableKeyInterner) -> Self {
        Self::from_normalized_output(output.normalized(interner))
    }

    pub fn from_normalized_output(output: ValueOutput) -> Self {
        let mut store = Self {
            output,
            ..Self::default()
        };
        for (index, row) in store.output.values.iter().enumerate() {
            store
                .by_language
                .entry(row.language)
                .or_default()
                .push(index);
            if let Some(function) = row.function {
                store.by_function.entry(function).or_default().push(index);
            }
            if let super::facts::ValueSubject::Place(place) = row.subject {
                store.by_place.entry(place).or_default().push(index);
            }
            store.by_status.entry(row.status).or_default().push(index);
        }
        for (index, row) in store.output.allocations.iter().enumerate() {
            store
                .allocations_by_kind
                .entry(row.kind)
                .or_default()
                .push(index);
        }
        store
    }

    pub fn values(&self) -> &[ValueFact] {
        &self.output.values
    }

    pub fn allocations(&self) -> &[AllocationTokenFact] {
        &self.output.allocations
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::values::facts::{
        ValueKind, ValuePrecision, ValueProvenance, ValueSubject,
    };

    fn value(id: u64, stable_key: &str) -> ValueFact {
        ValueFact {
            id: ValueFactId(id),
            subject: ValueSubject::Synthetic(stable_key.to_string()),
            value: crate::analysis_neutral::ids::AbstractValueId(id),
            kind: ValueKind::Null,
            language: Language::TypeScript,
            file: None,
            function: None,
            body: None,
            precision: ValuePrecision::ExactLocal,
            status: ValueStatus::Present,
            provenance: ValueProvenance::Native,
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
        }
    }

    fn allocation(id: u64, stable_key: &str) -> AllocationTokenFact {
        AllocationTokenFact {
            id: AllocationTokenId(id),
            kind: AllocationKind::CompositeLiteral,
            language: Language::Go,
            file: None,
            function: None,
            body: None,
            source_place: None,
            source_operation: None,
            span: None,
            provenance: ValueProvenance::Native,
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
        }
    }

    #[test]
    fn value_output_sorts_by_stable_key_and_reassigns_ids() {
        let interner = crate::internal_core::test_stable_key_interner();
        let output = ValueOutput {
            values: vec![value(9, "value:z"), value(3, "value:a")],
            allocations: Vec::new(),
        }
        .normalized(&interner);

        assert_eq!(
            interner.resolve(output.values[0].stable_key).as_ref(),
            "value:a"
        );
        assert_eq!(output.values[0].id, ValueFactId(0));
        assert_eq!(output.values[0].value, AbstractValueId(0));
        assert_eq!(output.values[1].id, ValueFactId(1));
        assert_eq!(output.values[1].value, AbstractValueId(1));
    }

    #[test]
    fn value_output_remaps_allocation_references_after_sorting_allocations() {
        let interner = crate::internal_core::test_stable_key_interner();
        let output = ValueOutput {
            values: vec![
                ValueFact {
                    kind: ValueKind::CompositeLiteral(AllocationTokenId(0)),
                    ..value(0, "value:z")
                },
                ValueFact {
                    subject: ValueSubject::Allocation(AllocationTokenId(1)),
                    kind: ValueKind::Object(AllocationTokenId(1)),
                    ..value(1, "value:a")
                },
            ],
            allocations: vec![allocation(0, "alloc:z"), allocation(1, "alloc:a")],
        }
        .normalized(&interner);

        assert_eq!(
            interner.resolve(output.allocations[0].stable_key).as_ref(),
            "alloc:a"
        );
        assert_eq!(output.allocations[0].id, AllocationTokenId(0));
        assert_eq!(
            interner.resolve(output.allocations[1].stable_key).as_ref(),
            "alloc:z"
        );
        assert_eq!(output.allocations[1].id, AllocationTokenId(1));
        assert_eq!(
            output.values[0].subject,
            ValueSubject::Allocation(AllocationTokenId(0))
        );
        assert_eq!(
            output.values[0].kind,
            ValueKind::Object(AllocationTokenId(0))
        );
        assert_eq!(
            output.values[1].kind,
            ValueKind::CompositeLiteral(AllocationTokenId(1))
        );
    }
}
