#![allow(dead_code, reason = "kept for private internal consumers")]

use crate::internal_core::{FileId, StableKeyId, StableKeyInterner};
use crate::ts::ids::{TsBindingId, TsScopeId};
use crate::ts::scope::facts::{TsBindingFact, TsBindingKind, TsScopeFact};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TsScopeOutput {
    pub scopes: Vec<TsScopeFact>,
    pub bindings: Vec<TsBindingFact>,
}

impl TsScopeOutput {
    pub fn normalized(mut self, interner: &StableKeyInterner) -> Self {
        self.scopes.sort_by(|scope, other| {
            interner
                .compare_canonical(scope.stable_key, other.stable_key)
                .then_with(|| scope.span.start_byte.cmp(&other.span.start_byte))
                .then_with(|| scope.span.end_byte.cmp(&other.span.end_byte))
        });
        for (index, scope) in self.scopes.iter_mut().enumerate() {
            scope.id = TsScopeId(index as u64);
        }

        self.bindings.sort_by(|binding, other| {
            interner
                .compare_canonical(binding.stable_key, other.stable_key)
                .then_with(|| binding.span.start_byte.cmp(&other.span.start_byte))
                .then_with(|| binding.span.end_byte.cmp(&other.span.end_byte))
        });
        for (index, binding) in self.bindings.iter_mut().enumerate() {
            binding.id = TsBindingId(index as u64);
        }

        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct TsScopeStore {
    output: TsScopeOutput,
    scopes_by_file: std::collections::BTreeMap<FileId, Vec<usize>>,
    scopes_by_stable_key: std::collections::BTreeMap<StableKeyId, usize>,
    bindings_by_file: std::collections::BTreeMap<FileId, Vec<usize>>,
    bindings_by_name: std::collections::BTreeMap<String, Vec<usize>>,
    bindings_by_scope_name: std::collections::BTreeMap<(StableKeyId, String), Vec<usize>>,
    bindings_by_kind: std::collections::BTreeMap<TsBindingKind, Vec<usize>>,
    imports_by_module_imported: std::collections::BTreeMap<(String, String), Vec<usize>>,
    exports_by_name: std::collections::BTreeMap<String, Vec<usize>>,
}

impl TsScopeStore {
    pub fn from_output(output: TsScopeOutput, interner: &StableKeyInterner) -> Self {
        let output = output.normalized(interner);
        let mut store = Self {
            output,
            ..Self::default()
        };

        for (index, scope) in store.output.scopes.iter().enumerate() {
            store
                .scopes_by_file
                .entry(scope.file)
                .or_default()
                .push(index);
            store.scopes_by_stable_key.insert(scope.stable_key, index);
        }

        for (index, binding) in store.output.bindings.iter().enumerate() {
            store
                .bindings_by_file
                .entry(binding.file)
                .or_default()
                .push(index);
            store
                .bindings_by_name
                .entry(binding.name.clone())
                .or_default()
                .push(index);
            store
                .bindings_by_scope_name
                .entry((binding.scope_key, binding.name.clone()))
                .or_default()
                .push(index);
            store
                .bindings_by_kind
                .entry(binding.binding_kind)
                .or_default()
                .push(index);
            if let (Some(module), Some(imported)) = (&binding.module_source, &binding.imported_name)
            {
                store
                    .imports_by_module_imported
                    .entry((module.clone(), imported.clone()))
                    .or_default()
                    .push(index);
            }
            if let Some(exported) = &binding.exported_name {
                store
                    .exports_by_name
                    .entry(exported.clone())
                    .or_default()
                    .push(index);
            }
        }

        store
    }

    pub fn scopes(&self) -> &[TsScopeFact] {
        &self.output.scopes
    }

    pub fn bindings(&self) -> &[TsBindingFact] {
        &self.output.bindings
    }

    pub fn scopes_for_file(&self, file: FileId) -> Vec<&TsScopeFact> {
        self.scope_refs(self.scopes_by_file.get(&file))
    }

    pub fn bindings_for_file(&self, file: FileId) -> Vec<&TsBindingFact> {
        self.binding_refs(self.bindings_by_file.get(&file))
    }

    pub fn scope_by_stable_key(&self, stable_key: StableKeyId) -> Option<&TsScopeFact> {
        self.scopes_by_stable_key
            .get(&stable_key)
            .map(|index| &self.output.scopes[*index])
    }

    pub fn bindings_by_name(&self, name: &str) -> Vec<&TsBindingFact> {
        self.binding_refs(self.bindings_by_name.get(name))
    }

    pub fn lookup_binding_in_scope(
        &self,
        scope_key: StableKeyId,
        name: &str,
    ) -> Vec<&TsBindingFact> {
        self.binding_refs(
            self.bindings_by_scope_name
                .get(&(scope_key, name.to_string())),
        )
    }

    pub fn bindings_by_kind(&self, kind: TsBindingKind) -> Vec<&TsBindingFact> {
        self.binding_refs(self.bindings_by_kind.get(&kind))
    }

    pub fn import_aliases(&self, module_source: &str, imported_name: &str) -> Vec<&TsBindingFact> {
        self.binding_refs(
            self.imports_by_module_imported
                .get(&(module_source.to_string(), imported_name.to_string())),
        )
    }

    pub fn exports_by_name(&self, exported_name: &str) -> Vec<&TsBindingFact> {
        self.binding_refs(self.exports_by_name.get(exported_name))
    }

    fn scope_refs(&self, indexes: Option<&Vec<usize>>) -> Vec<&TsScopeFact> {
        indexes.map_or_else(Vec::new, |indexes| {
            indexes
                .iter()
                .map(|&index| &self.output.scopes[index])
                .collect()
        })
    }

    fn binding_refs(&self, indexes: Option<&Vec<usize>>) -> Vec<&TsBindingFact> {
        indexes.map_or_else(Vec::new, |indexes| {
            indexes
                .iter()
                .map(|&index| &self.output.bindings[index])
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::internal_core::{FileId, Span};
    use crate::ts::scope::facts::{
        TsBindingStatus, TsDeclarationKind, TsImportExportKind, TsScopeKind,
    };

    use super::*;

    #[test]
    fn normalized_assigns_dense_ids_after_stable_key_sort() {
        let interner = StableKeyInterner::default();
        let output = TsScopeOutput {
            scopes: vec![
                scope(&interner, "scope:b", TsScopeId(22)),
                scope(&interner, "scope:a", TsScopeId(11)),
            ],
            bindings: vec![
                binding(&interner, "binding:b", TsBindingId(22), "second", "scope:b"),
                binding(&interner, "binding:a", TsBindingId(11), "first", "scope:a"),
            ],
        }
        .normalized(&interner);

        assert_eq!(
            output
                .scopes
                .iter()
                .map(|scope| (interner.resolve(scope.stable_key), scope.id.0))
                .collect::<Vec<_>>(),
            vec![
                (std::sync::Arc::from("scope:a"), 0),
                (std::sync::Arc::from("scope:b"), 1)
            ]
        );
        assert_eq!(
            output
                .bindings
                .iter()
                .map(|binding| (interner.resolve(binding.stable_key), binding.id.0))
                .collect::<Vec<_>>(),
            vec![
                (std::sync::Arc::from("binding:a"), 0),
                (std::sync::Arc::from("binding:b"), 1)
            ]
        );
    }

    #[test]
    fn store_indexes_file_scope_name_kind_import_and_export_lookups() {
        let interner = StableKeyInterner::default();
        let mut import = binding(
            &interner,
            "binding:import",
            TsBindingId(1),
            "local",
            "scope:a",
        );
        import.binding_kind = TsBindingKind::NamedImport;
        import.module_source = Some("./dep".to_string());
        import.imported_name = Some("remote".to_string());

        let mut export = binding(
            &interner,
            "binding:export",
            TsBindingId(2),
            "reExport",
            "scope:a",
        );
        export.binding_kind = TsBindingKind::ReExport;
        export.exported_name = Some("reExport".to_string());

        let store = TsScopeStore::from_output(
            TsScopeOutput {
                scopes: vec![scope(&interner, "scope:a", TsScopeId(1))],
                bindings: vec![
                    binding(
                        &interner,
                        "binding:local",
                        TsBindingId(3),
                        "local",
                        "scope:a",
                    ),
                    import,
                    export,
                ],
            },
            &interner,
        );

        assert_eq!(store.scopes().len(), 1);
        assert_eq!(store.bindings().len(), 3);
        assert_eq!(store.scopes_for_file(FileId::from_raw(1)).len(), 1);
        assert_eq!(store.bindings_for_file(FileId::from_raw(1)).len(), 3);
        assert!(
            store
                .scope_by_stable_key(interner.intern("scope:a"))
                .is_some()
        );
        assert_eq!(store.bindings_by_name("local").len(), 2);
        assert_eq!(
            store
                .lookup_binding_in_scope(interner.intern("scope:a"), "local")
                .len(),
            2
        );
        assert_eq!(store.bindings_by_kind(TsBindingKind::NamedImport).len(), 1);
        assert_eq!(store.import_aliases("./dep", "remote").len(), 1);
        assert_eq!(store.exports_by_name("reExport").len(), 1);
    }

    fn scope(interner: &StableKeyInterner, stable_key: &str, id: TsScopeId) -> TsScopeFact {
        TsScopeFact {
            id,
            file: FileId::from_raw(1),
            span: span(),
            stable_key: interner.intern(stable_key),
            parent_scope_key: None,
            kind: TsScopeKind::Module,
        }
    }

    fn binding(
        interner: &StableKeyInterner,
        stable_key: &str,
        id: TsBindingId,
        name: &str,
        scope_key: &str,
    ) -> TsBindingFact {
        TsBindingFact {
            id,
            file: FileId::from_raw(1),
            span: span(),
            stable_key: interner.intern(stable_key),
            scope_key: interner.intern(scope_key),
            parent_scope_key: None,
            name: name.to_string(),
            declaration_kind: TsDeclarationKind::Const,
            binding_kind: TsBindingKind::Const,
            import_export_kind: TsImportExportKind::None,
            module_source: None,
            imported_name: None,
            exported_name: None,
            inventory_function_key: None,
            inventory_callsite_key: None,
            status: TsBindingStatus::present(),
        }
    }

    fn span() -> Span {
        Span::new(FileId::from_raw(1), 1, 4, 1, 1, 1, 4)
    }
}
