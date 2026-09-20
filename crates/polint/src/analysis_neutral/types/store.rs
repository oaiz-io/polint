use std::collections::BTreeMap;

use super::facts::{NarrowedTypeFact, TypeFact, TypePrecision, TypeShape, TypeStatus};
use crate::analysis_neutral::access_paths::store::AccessPathOutput;
use crate::analysis_neutral::aliases::store::AliasOutput;
use crate::analysis_neutral::ids::{NarrowedTypeId, PlaceId, TypeFactId, TypeSetId};
use crate::analysis_neutral::points_to::store::PointsToOutput;
use crate::analysis_neutral::values::store::ValueOutput;
use crate::internal_core::{FunctionId, Language, StableKeyInterner};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypeValueAliasOutput {
    pub types: TypeOutput,
    pub values: ValueOutput,
    pub access_paths: AccessPathOutput,
    pub points_to: PointsToOutput,
    pub aliases: AliasOutput,
}

impl TypeValueAliasOutput {
    pub fn normalized(self, interner: &StableKeyInterner) -> Self {
        Self {
            types: self.types.normalized(interner),
            values: self.values.normalized(interner),
            access_paths: self.access_paths.normalized(interner),
            points_to: self.points_to.normalized(interner),
            aliases: self.aliases.normalized(interner),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypeOutput {
    pub types: Vec<TypeFact>,
    pub narrowed: Vec<NarrowedTypeFact>,
}

impl TypeOutput {
    pub fn normalized(mut self, interner: &StableKeyInterner) -> Self {
        self.types.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.id.cmp(&right.id))
        });
        self.narrowed.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.id.cmp(&right.id))
        });
        let mut type_set_remap = BTreeMap::new();
        for (index, row) in self.types.iter().enumerate() {
            type_set_remap
                .entry(row.type_set)
                .or_insert(TypeSetId(index as u64));
        }
        for (index, row) in self.types.iter_mut().enumerate() {
            row.id = TypeFactId(index as u64);
            if let Some(remapped_set) = type_set_remap.get(&row.type_set) {
                row.type_set = *remapped_set;
            }
            remap_type_shape_sets(&mut row.shape, &type_set_remap);
        }
        for (index, row) in self.narrowed.iter_mut().enumerate() {
            row.id = NarrowedTypeId(index as u64);
            if let Some(remapped_set) = type_set_remap.get(&row.type_set) {
                row.type_set = *remapped_set;
            }
        }
        self
    }
}

fn remap_type_shape_sets(shape: &mut TypeShape, type_set_remap: &BTreeMap<TypeSetId, TypeSetId>) {
    match shape {
        TypeShape::Union(sets) | TypeShape::Intersection(sets) => {
            for set in sets {
                if let Some(remapped) = type_set_remap.get(set) {
                    *set = *remapped;
                }
            }
        }
        _ => {}
    }
}

#[derive(Debug, Clone, Default)]
pub struct TypeStore {
    output: TypeOutput,
    by_language: BTreeMap<Language, Vec<usize>>,
    by_function: BTreeMap<FunctionId, Vec<usize>>,
    by_place: BTreeMap<PlaceId, Vec<usize>>,
    by_status: BTreeMap<TypeStatus, Vec<usize>>,
    by_precision: BTreeMap<TypePrecision, Vec<usize>>,
}

impl TypeStore {
    pub fn from_output(output: TypeOutput, interner: &StableKeyInterner) -> Self {
        Self::from_normalized_output(output.normalized(interner))
    }

    pub fn from_normalized_output(output: TypeOutput) -> Self {
        let mut store = Self {
            output,
            ..Self::default()
        };
        for (index, row) in store.output.types.iter().enumerate() {
            store
                .by_language
                .entry(row.language)
                .or_default()
                .push(index);
            if let Some(function) = row.function {
                store.by_function.entry(function).or_default().push(index);
            }
            if let Some(place) = row.place {
                store.by_place.entry(place).or_default().push(index);
            }
            store.by_status.entry(row.status).or_default().push(index);
            store
                .by_precision
                .entry(row.precision)
                .or_default()
                .push(index);
        }
        store
    }

    pub fn types(&self) -> &[TypeFact] {
        &self.output.types
    }

    pub fn narrowed(&self) -> &[NarrowedTypeFact] {
        &self.output.narrowed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::types::facts::{
        TypeConfidence, TypePhase, TypeProvenance, TypeShape, TypeSubject,
    };
    use crate::internal_core::Language;

    fn type_fact(id: u64, stable_key: &str) -> TypeFact {
        TypeFact {
            id: TypeFactId(id),
            subject: TypeSubject::Synthetic(stable_key.to_string()),
            type_set: TypeSetId(id),
            shape: TypeShape::Any,
            phase: TypePhase::Declared,
            language: Language::Go,
            file: None,
            function: None,
            body: None,
            place: None,
            cfg_block: None,
            operation: None,
            precision: TypePrecision::Conservative,
            confidence: TypeConfidence::Medium,
            status: TypeStatus::Present,
            provenance: TypeProvenance::Native,
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
        }
    }

    #[test]
    fn type_output_sorts_by_stable_key_and_reassigns_ids() {
        let interner = crate::internal_core::test_stable_key_interner();
        let output = TypeOutput {
            types: vec![type_fact(7, "type:z"), type_fact(3, "type:a")],
            narrowed: Vec::new(),
        }
        .normalized(&interner);

        assert_eq!(
            interner.resolve(output.types[0].stable_key).as_ref(),
            "type:a"
        );
        assert_eq!(output.types[0].id, TypeFactId(0));
        assert_eq!(output.types[0].type_set, TypeSetId(0));
        assert_eq!(output.types[1].id, TypeFactId(1));
        assert_eq!(output.types[1].type_set, TypeSetId(1));
    }

    #[test]
    fn type_output_remaps_type_set_references_after_sorting() {
        let interner = crate::internal_core::test_stable_key_interner();
        let output = TypeOutput {
            types: vec![
                TypeFact {
                    shape: TypeShape::Union(vec![TypeSetId(7), TypeSetId(3)]),
                    ..type_fact(7, "type:z")
                },
                type_fact(3, "type:a"),
            ],
            narrowed: vec![NarrowedTypeFact {
                id: NarrowedTypeId(9),
                place: PlaceId(1),
                type_set: TypeSetId(7),
                cfg_block: None,
                operation: None,
                predicate: None,
                evidence: "narrowed".to_string(),
                language: Language::Go,
                file: None,
                function: None,
                body: None,
                precision: TypePrecision::Conservative,
                status: TypeStatus::Present,
                stable_key: crate::internal_core::stable_key_for_test("narrowed:z"),
            }],
        }
        .normalized(&interner);

        assert_eq!(output.narrowed[0].type_set, TypeSetId(1));
        assert_eq!(
            output.types[1].shape,
            TypeShape::Union(vec![TypeSetId(1), TypeSetId(0)])
        );
    }
}
