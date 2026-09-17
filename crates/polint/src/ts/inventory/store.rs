#![allow(dead_code, reason = "kept for private internal consumers")]

use std::collections::BTreeMap;

use crate::internal_core::{FileId, StableKeyId, StableKeyInterner};
use crate::ts::ids::{TsInventoryCallsiteId, TsInventoryFunctionId};
use crate::ts::inventory::facts::{
    TsCallsiteInventoryKind, TsFunctionInventoryKind, TsInventoryCallsiteFact,
    TsInventoryFunctionFact,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TsInventoryOutput {
    pub functions: Vec<TsInventoryFunctionFact>,
    pub callsites: Vec<TsInventoryCallsiteFact>,
}

impl TsInventoryOutput {
    pub fn normalized(mut self, interner: &StableKeyInterner) -> Self {
        self.functions.sort_by(|function, other| {
            interner
                .compare_canonical(function.stable_key, other.stable_key)
                .then_with(|| function.span.start_byte.cmp(&other.span.start_byte))
                .then_with(|| function.span.end_byte.cmp(&other.span.end_byte))
        });
        for (index, function) in self.functions.iter_mut().enumerate() {
            function.id = TsInventoryFunctionId(index as u64);
        }

        self.callsites.sort_by(|callsite, other| {
            interner
                .compare_canonical(callsite.stable_key, other.stable_key)
                .then_with(|| callsite.span.start_byte.cmp(&other.span.start_byte))
                .then_with(|| callsite.span.end_byte.cmp(&other.span.end_byte))
        });
        for (index, callsite) in self.callsites.iter_mut().enumerate() {
            callsite.id = TsInventoryCallsiteId(index as u64);
        }

        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct TsInventoryStore {
    output: TsInventoryOutput,
    functions_by_file: BTreeMap<FileId, Vec<usize>>,
    callsites_by_file: BTreeMap<FileId, Vec<usize>>,
    functions_by_stable_key: BTreeMap<StableKeyId, usize>,
    callsites_by_stable_key: BTreeMap<StableKeyId, usize>,
    functions_by_kind: BTreeMap<TsFunctionInventoryKind, Vec<usize>>,
    callsites_by_kind: BTreeMap<TsCallsiteInventoryKind, Vec<usize>>,
}

impl TsInventoryStore {
    pub fn from_output(output: TsInventoryOutput, interner: &StableKeyInterner) -> Self {
        let output = output.normalized(interner);
        let mut store = Self {
            output,
            ..Self::default()
        };

        for (index, function) in store.output.functions.iter().enumerate() {
            store
                .functions_by_file
                .entry(function.file)
                .or_default()
                .push(index);
            store
                .functions_by_stable_key
                .insert(function.stable_key, index);
            store
                .functions_by_kind
                .entry(function.kind)
                .or_default()
                .push(index);
        }

        for (index, callsite) in store.output.callsites.iter().enumerate() {
            store
                .callsites_by_file
                .entry(callsite.file)
                .or_default()
                .push(index);
            store
                .callsites_by_stable_key
                .insert(callsite.stable_key, index);
            store
                .callsites_by_kind
                .entry(callsite.kind)
                .or_default()
                .push(index);
        }

        store
    }

    pub fn functions(&self) -> &[TsInventoryFunctionFact] {
        &self.output.functions
    }

    pub fn callsites(&self) -> &[TsInventoryCallsiteFact] {
        &self.output.callsites
    }

    pub fn functions_for_file(&self, file: FileId) -> Vec<&TsInventoryFunctionFact> {
        self.function_refs(self.functions_by_file.get(&file))
    }

    pub fn callsites_for_file(&self, file: FileId) -> Vec<&TsInventoryCallsiteFact> {
        self.callsite_refs(self.callsites_by_file.get(&file))
    }

    pub fn function_by_stable_key(
        &self,
        stable_key: StableKeyId,
    ) -> Option<&TsInventoryFunctionFact> {
        self.functions_by_stable_key
            .get(&stable_key)
            .map(|index| &self.output.functions[*index])
    }

    pub fn callsite_by_stable_key(
        &self,
        stable_key: StableKeyId,
    ) -> Option<&TsInventoryCallsiteFact> {
        self.callsites_by_stable_key
            .get(&stable_key)
            .map(|index| &self.output.callsites[*index])
    }

    pub fn functions_by_kind(
        &self,
        kind: TsFunctionInventoryKind,
    ) -> Vec<&TsInventoryFunctionFact> {
        self.function_refs(self.functions_by_kind.get(&kind))
    }

    pub fn callsites_by_kind(
        &self,
        kind: TsCallsiteInventoryKind,
    ) -> Vec<&TsInventoryCallsiteFact> {
        self.callsite_refs(self.callsites_by_kind.get(&kind))
    }

    fn function_refs(&self, indexes: Option<&Vec<usize>>) -> Vec<&TsInventoryFunctionFact> {
        indexes.map_or_else(Vec::new, |indexes| {
            indexes
                .iter()
                .map(|&index| &self.output.functions[index])
                .collect()
        })
    }

    fn callsite_refs(&self, indexes: Option<&Vec<usize>>) -> Vec<&TsInventoryCallsiteFact> {
        indexes.map_or_else(Vec::new, |indexes| {
            indexes
                .iter()
                .map(|&index| &self.output.callsites[index])
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::internal_core::{FileId, Span};
    use crate::ts::ids::{TsInventoryCallsiteId, TsInventoryFunctionId};
    use crate::ts::inventory::facts::{
        TsCallsiteInventoryKind, TsFunctionInventoryKind, TsInventoryCallsiteFact,
        TsInventoryFunctionFact, TsInventoryStatus,
    };

    use super::*;

    #[test]
    fn normalized_assigns_dense_ids_after_stable_key_sort() {
        let interner = StableKeyInterner::default();
        let output = TsInventoryOutput {
            functions: vec![
                function(
                    &interner,
                    "function:b",
                    TsInventoryFunctionId(40),
                    FileId::from_raw(1),
                ),
                function(
                    &interner,
                    "function:a",
                    TsInventoryFunctionId(20),
                    FileId::from_raw(1),
                ),
            ],
            callsites: vec![
                callsite(
                    &interner,
                    "call:b",
                    TsInventoryCallsiteId(8),
                    FileId::from_raw(1),
                ),
                callsite(
                    &interner,
                    "call:a",
                    TsInventoryCallsiteId(4),
                    FileId::from_raw(1),
                ),
            ],
        }
        .normalized(&interner);

        assert_eq!(
            output
                .functions
                .iter()
                .map(|function| (interner.resolve(function.stable_key), function.id.0))
                .collect::<Vec<_>>(),
            vec![
                (std::sync::Arc::from("function:a"), 0),
                (std::sync::Arc::from("function:b"), 1)
            ]
        );
        assert_eq!(
            output
                .callsites
                .iter()
                .map(|callsite| (interner.resolve(callsite.stable_key), callsite.id.0))
                .collect::<Vec<_>>(),
            vec![
                (std::sync::Arc::from("call:a"), 0),
                (std::sync::Arc::from("call:b"), 1)
            ]
        );
    }

    #[test]
    fn store_indexes_support_file_stable_key_and_kind_lookup() {
        let interner = StableKeyInterner::default();
        let store = TsInventoryStore::from_output(
            TsInventoryOutput {
                functions: vec![
                    function(
                        &interner,
                        "function:a",
                        TsInventoryFunctionId(10),
                        FileId::from_raw(1),
                    ),
                    function(
                        &interner,
                        "function:b",
                        TsInventoryFunctionId(11),
                        FileId::from_raw(2),
                    ),
                ],
                callsites: vec![
                    callsite(
                        &interner,
                        "call:a",
                        TsInventoryCallsiteId(10),
                        FileId::from_raw(1),
                    ),
                    callsite(
                        &interner,
                        "call:b",
                        TsInventoryCallsiteId(11),
                        FileId::from_raw(2),
                    ),
                ],
            },
            &interner,
        );

        assert_eq!(store.functions().len(), 2);
        assert_eq!(store.callsites().len(), 2);
        assert_eq!(store.functions_for_file(FileId::from_raw(1)).len(), 1);
        assert_eq!(store.callsites_for_file(FileId::from_raw(2)).len(), 1);
        assert_eq!(
            store
                .function_by_stable_key(interner.intern("function:a"))
                .map(|function| function.file),
            Some(FileId::from_raw(1))
        );
        assert_eq!(
            store
                .callsite_by_stable_key(interner.intern("call:b"))
                .map(|callsite| callsite.file),
            Some(FileId::from_raw(2))
        );
        assert_eq!(
            store
                .functions_by_kind(TsFunctionInventoryKind::Arrow)
                .len(),
            2
        );
        assert_eq!(
            store.callsites_by_kind(TsCallsiteInventoryKind::Call).len(),
            2
        );
    }

    fn function(
        interner: &StableKeyInterner,
        stable_key: &str,
        id: TsInventoryFunctionId,
        file: FileId,
    ) -> TsInventoryFunctionFact {
        TsInventoryFunctionFact {
            id,
            file,
            span: span(file),
            stable_key: interner.intern(stable_key),
            lexical_parent_key: Some(interner.intern("module")),
            display_name: Some(stable_key.to_string()),
            kind: TsFunctionInventoryKind::Arrow,
            status: TsInventoryStatus::Resolved,
        }
    }

    fn callsite(
        interner: &StableKeyInterner,
        stable_key: &str,
        id: TsInventoryCallsiteId,
        file: FileId,
    ) -> TsInventoryCallsiteFact {
        TsInventoryCallsiteFact {
            id,
            file,
            span: span(file),
            stable_key: interner.intern(stable_key),
            lexical_parent_key: Some(interner.intern("module")),
            display_name: Some(stable_key.to_string()),
            kind: TsCallsiteInventoryKind::Call,
            status: TsInventoryStatus::Resolved,
        }
    }

    fn span(file: FileId) -> Span {
        Span::new(file, 1, 5, 1, 1, 1, 5)
    }
}
