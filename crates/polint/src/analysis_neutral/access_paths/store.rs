use std::collections::BTreeMap;

use super::facts::{AccessPathFact, AccessPathStatus};
use crate::analysis_neutral::ids::{AccessPathId, PlaceId};
use crate::internal_core::{FunctionId, Language, StableKeyInterner};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccessPathOutput {
    pub access_paths: Vec<AccessPathFact>,
}

impl AccessPathOutput {
    pub fn normalized(mut self, interner: &StableKeyInterner) -> Self {
        self.access_paths.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.id.cmp(&right.id))
        });
        for (index, row) in self.access_paths.iter_mut().enumerate() {
            row.id = AccessPathId(index as u64);
        }
        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct AccessPathStore {
    output: AccessPathOutput,
    by_language: BTreeMap<Language, Vec<usize>>,
    by_function: BTreeMap<FunctionId, Vec<usize>>,
    by_base: BTreeMap<PlaceId, Vec<usize>>,
    by_status: BTreeMap<AccessPathStatus, Vec<usize>>,
}

impl AccessPathStore {
    pub fn from_output(output: AccessPathOutput, interner: &StableKeyInterner) -> Self {
        Self::from_normalized_output(output.normalized(interner))
    }

    pub fn from_normalized_output(output: AccessPathOutput) -> Self {
        let mut store = Self {
            output,
            ..Self::default()
        };
        for (index, row) in store.output.access_paths.iter().enumerate() {
            store
                .by_language
                .entry(row.language)
                .or_default()
                .push(index);
            if let Some(function) = row.function {
                store.by_function.entry(function).or_default().push(index);
            }
            store.by_base.entry(row.base).or_default().push(index);
            store.by_status.entry(row.status).or_default().push(index);
        }
        store
    }

    pub fn access_paths(&self) -> &[AccessPathFact] {
        &self.output.access_paths
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::access_paths::facts::AccessPathProjection;

    fn access_path(id: u64, stable_key: &str) -> AccessPathFact {
        AccessPathFact {
            id: AccessPathId(id),
            base: PlaceId(1),
            projections: vec![AccessPathProjection::Property(stable_key.to_string())],
            depth: 1,
            language: Language::JavaScript,
            file: None,
            function: None,
            body: None,
            status: AccessPathStatus::Resolved,
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
        }
    }

    #[test]
    fn access_path_output_sorts_by_stable_key_and_reassigns_ids() {
        let interner = crate::internal_core::test_stable_key_interner();
        let output = AccessPathOutput {
            access_paths: vec![access_path(8, "path:z"), access_path(2, "path:a")],
        }
        .normalized(&interner);

        assert_eq!(
            interner.resolve(output.access_paths[0].stable_key).as_ref(),
            "path:a"
        );
        assert_eq!(output.access_paths[0].id, AccessPathId(0));
        assert_eq!(output.access_paths[1].id, AccessPathId(1));
    }
}
