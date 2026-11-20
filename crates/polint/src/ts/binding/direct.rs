#![allow(dead_code, reason = "kept for private internal consumers")]

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis_api::{
    ImportFact, ModuleNode, ModuleNodeKind, ResolutionStatus, ResolvedImportFact,
};
use crate::internal_core::{FileId, ModuleNodeId, StableKeyId, StableKeyInterner};
use crate::ts::binding::facts::{
    TsDirectBindingFact, TsDirectBindingKind, TsDirectBindingReason, TsDirectBindingStatus,
};
use crate::ts::binding::store::TsDirectBindingOutput;
use crate::ts::ids::TsDirectBindingId;
use crate::ts::inventory::facts::{
    TsCallsiteInventoryKind, TsInventoryCallsiteFact, TsInventoryFunctionFact, TsInventoryStatus,
};
use crate::ts::inventory::store::TsInventoryOutput;
use crate::ts::scope::facts::{
    TsBindingFact, TsBindingKind, TsBindingStatus, TsScopeFact, TsScopeKind,
};
use crate::ts::scope::store::TsScopeOutput;

#[derive(Clone, Copy)]
pub struct TsDirectBindingModuleFile<'a> {
    pub module_node: ModuleNodeId,
    pub inventory: &'a TsInventoryOutput,
    pub scope: &'a TsScopeOutput,
}

#[derive(Clone, Copy)]
pub struct TsDirectBindingModuleInput<'a> {
    pub imports: &'a [ImportFact],
    pub resolved_imports: &'a [ResolvedImportFact],
    pub module_nodes: &'a [ModuleNode],
    pub module_files: &'a [TsDirectBindingModuleFile<'a>],
}

impl<'a> TsDirectBindingModuleInput<'a> {
    pub fn empty() -> Self {
        Self {
            imports: &[],
            resolved_imports: &[],
            module_nodes: &[],
            module_files: &[],
        }
    }
}

pub fn resolve_direct_bindings(
    interner: &StableKeyInterner,
    inventory: &TsInventoryOutput,
    scope: &TsScopeOutput,
) -> TsDirectBindingOutput {
    resolve_direct_bindings_with_modules(
        interner,
        inventory,
        scope,
        &TsDirectBindingModuleInput::empty(),
    )
}

pub fn resolve_direct_bindings_with_modules(
    interner: &StableKeyInterner,
    inventory: &TsInventoryOutput,
    scope: &TsScopeOutput,
    module_input: &TsDirectBindingModuleInput<'_>,
) -> TsDirectBindingOutput {
    crate::ts::with_frontend_stable_keys(interner, || {
        let index = DirectBindingIndex::new(inventory, scope, module_input);
        let mut rows = Vec::new();

        for callsite in &inventory.callsites {
            let callsite_scope = index.scope_key_for_callsite(callsite);
            if let Some(reason) = intrinsic_unresolved_reason_for_callsite(callsite) {
                rows.push(unresolved_row(callsite, reason));
                continue;
            }

            let Some(display_name) = callsite.display_name.as_deref() else {
                rows.push(unresolved_row(
                    callsite,
                    TsDirectBindingReason::LocalBindingMissing,
                ));
                continue;
            };
            let lookup = CalleeLookup::new(display_name);
            let Some(visible) = index.visible_binding_for_lookup(callsite_scope, lookup) else {
                rows.push(unresolved_row(
                    callsite,
                    TsDirectBindingReason::LocalBindingMissing,
                ));
                continue;
            };

            rows.push(index.row_for_visible_binding(callsite, visible.binding, lookup));
        }

        TsDirectBindingOutput { bindings: rows }.normalized(interner)
    })
}

struct DirectBindingIndex<'a> {
    function_by_name: BTreeMap<&'a str, Vec<&'a TsInventoryFunctionFact>>,
    alias_by_scope_name: BTreeMap<(StableKeyId, &'a str), &'a TsBindingFact>,
    import_by_scope_name: BTreeMap<(StableKeyId, &'a str), &'a TsBindingFact>,
    local_by_scope_name: BTreeMap<(StableKeyId, &'a str), &'a TsBindingFact>,
    unsupported_by_scope_name: BTreeMap<(StableKeyId, &'a str), &'a TsBindingFact>,
    scope_parent_by_key: BTreeMap<StableKeyId, Option<StableKeyId>>,
    scopes_by_file: BTreeMap<FileId, Vec<&'a TsScopeFact>>,
    module_scope_by_file: BTreeMap<FileId, StableKeyId>,
    module_index: ModuleBindingIndex<'a>,
}

impl<'a> DirectBindingIndex<'a> {
    fn new(
        inventory: &'a TsInventoryOutput,
        scope: &'a TsScopeOutput,
        module_input: &'a TsDirectBindingModuleInput<'a>,
    ) -> Self {
        let mut function_by_name: BTreeMap<&str, Vec<&TsInventoryFunctionFact>> = BTreeMap::new();
        for function in &inventory.functions {
            if let Some(name) = function.display_name.as_deref() {
                function_by_name.entry(name).or_default().push(function);
            }
        }

        let mut scopes_by_file: BTreeMap<FileId, Vec<&TsScopeFact>> = BTreeMap::new();
        let mut module_scope_by_file = BTreeMap::new();
        for scope_row in &scope.scopes {
            scopes_by_file
                .entry(scope_row.file)
                .or_default()
                .push(scope_row);
            if scope_row.kind == TsScopeKind::Module {
                module_scope_by_file.insert(scope_row.file, scope_row.stable_key);
            }
        }

        Self {
            function_by_name,
            alias_by_scope_name: scope
                .bindings
                .iter()
                .filter(|binding| binding.module_source.is_none())
                .filter(|binding| binding.imported_name.is_some())
                .filter(|binding| {
                    matches!(
                        binding.binding_kind,
                        TsBindingKind::Alias | TsBindingKind::Destructuring
                    )
                })
                .map(|binding| ((binding.scope_key, binding.name.as_str()), binding))
                .collect(),
            import_by_scope_name: scope
                .bindings
                .iter()
                .filter(|binding| binding.module_source.is_some())
                .filter(|binding| {
                    matches!(
                        binding.binding_kind,
                        TsBindingKind::Import
                            | TsBindingKind::NamedImport
                            | TsBindingKind::DefaultImport
                            | TsBindingKind::NamespaceImport
                            | TsBindingKind::ReExport
                    )
                })
                .map(|binding| ((binding.scope_key, binding.name.as_str()), binding))
                .collect(),
            local_by_scope_name: scope
                .bindings
                .iter()
                .filter(|binding| {
                    matches!(
                        binding.binding_kind,
                        TsBindingKind::FunctionDeclaration
                            | TsBindingKind::FunctionExpression
                            | TsBindingKind::Var
                            | TsBindingKind::Let
                            | TsBindingKind::Const
                            | TsBindingKind::Parameter
                    )
                })
                .map(|binding| ((binding.scope_key, binding.name.as_str()), binding))
                .collect(),
            unsupported_by_scope_name: scope
                .bindings
                .iter()
                .filter(|binding| binding.binding_kind == TsBindingKind::UnsupportedDynamic)
                .map(|binding| ((binding.scope_key, binding.name.as_str()), binding))
                .collect(),
            scope_parent_by_key: scope
                .scopes
                .iter()
                .map(|scope| (scope.stable_key, scope.parent_scope_key))
                .collect(),
            scopes_by_file,
            module_scope_by_file,
            module_index: ModuleBindingIndex::new(module_input),
        }
    }

    fn scope_key_for_callsite(&self, callsite: &TsInventoryCallsiteFact) -> Option<StableKeyId> {
        self.scopes_by_file
            .get(&callsite.file)
            .and_then(|scopes| {
                scopes
                    .iter()
                    .filter(|scope| scope.kind != TsScopeKind::Module)
                    .filter(|scope| span_contains(&scope.span, &callsite.span))
                    .min_by_key(|scope| scope.span.end_byte - scope.span.start_byte)
                    .map(|scope| scope.stable_key)
            })
            .or_else(|| self.module_scope_by_file.get(&callsite.file).copied())
    }

    fn visible_binding_for_lookup(
        &self,
        scope_key: Option<StableKeyId>,
        lookup: CalleeLookup<'_>,
    ) -> Option<VisibleBinding<'a>> {
        let local = self.visible_any_binding(scope_key, lookup.local_name, false);
        if lookup.member_name.is_none() {
            return local;
        }

        let exact_member = self.visible_any_binding(scope_key, lookup.alias_name(), true);
        choose_visible_binding(local, exact_member)
    }

    fn row_for_visible_binding(
        &self,
        callsite: &TsInventoryCallsiteFact,
        binding: &'a TsBindingFact,
        lookup: CalleeLookup<'_>,
    ) -> TsDirectBindingFact {
        if let Some(reason) = unsupported_binding_reason(binding) {
            return unresolved_row(callsite, reason);
        }

        if is_import_binding(binding) {
            return self
                .module_row(callsite, binding, lookup)
                .unwrap_or_else(|| {
                    unresolved_import_row(
                        callsite,
                        binding,
                        None,
                        None,
                        lookup.local_kind,
                        TsDirectBindingStatus::Unresolved,
                        TsDirectBindingReason::ImportedBindingUnresolved,
                    )
                });
        }

        if matches!(
            binding.binding_kind,
            TsBindingKind::Alias | TsBindingKind::Destructuring
        ) {
            if let Some(target) = self.local_alias_target(binding) {
                return resolved_row(
                    callsite,
                    target,
                    Some(binding),
                    TsDirectBindingKind::LocalAlias,
                );
            }
            return unresolved_row(callsite, TsDirectBindingReason::LocalBindingMissing);
        }

        if matches!(binding.binding_kind, TsBindingKind::Parameter) && lookup.member_name.is_none()
        {
            return unresolved_row(callsite, TsDirectBindingReason::TokenFlowRequired);
        }

        if lookup.member_name.is_none()
            && matches!(
                binding.binding_kind,
                TsBindingKind::FunctionDeclaration | TsBindingKind::FunctionExpression
            )
            && let Some(target) = self.function_by_name(lookup.local_name, Some(binding))
        {
            return resolved_row(callsite, target, None, lookup.local_kind);
        }

        unresolved_row(callsite, TsDirectBindingReason::LocalBindingMissing)
    }

    fn local_alias_target(&self, alias: &'a TsBindingFact) -> Option<&'a TsInventoryFunctionFact> {
        let target_name = alias.imported_name.as_deref()?;
        let target_binding = self.visible_binding_in_map(
            Some(alias.scope_key),
            target_name,
            &self.local_by_scope_name,
        )?;
        if !matches!(
            target_binding.binding_kind,
            TsBindingKind::FunctionDeclaration | TsBindingKind::FunctionExpression
        ) {
            return None;
        }
        self.function_by_name(target_name, Some(target_binding))
    }

    fn function_by_name(
        &self,
        name: &str,
        binding: Option<&TsBindingFact>,
    ) -> Option<&'a TsInventoryFunctionFact> {
        let functions = self.function_by_name.get(name)?;
        match binding {
            Some(binding) => callable_for_binding(functions, binding),
            None => functions.iter().copied().find(|function| function.has_body),
        }
    }

    fn visible_any_binding(
        &self,
        scope_key: Option<StableKeyId>,
        name: &str,
        exact_member: bool,
    ) -> Option<VisibleBinding<'a>> {
        let mut current = scope_key;
        let mut distance = 0;
        while let Some(scope) = current {
            if let Some(binding) = self.binding_in_scope(scope, name) {
                return Some(VisibleBinding {
                    binding,
                    distance,
                    exact_member,
                });
            }
            current = self
                .scope_parent_by_key
                .get(&scope)
                .and_then(|parent| *parent);
            distance += 1;
        }
        None
    }

    fn binding_in_scope(&self, scope: StableKeyId, name: &str) -> Option<&'a TsBindingFact> {
        self.unsupported_by_scope_name
            .get(&(scope, name))
            .copied()
            .or_else(|| self.alias_by_scope_name.get(&(scope, name)).copied())
            .or_else(|| self.import_by_scope_name.get(&(scope, name)).copied())
            .or_else(|| self.local_by_scope_name.get(&(scope, name)).copied())
    }

    fn visible_binding_in_map(
        &self,
        scope_key: Option<StableKeyId>,
        name: &str,
        bindings: &BTreeMap<(StableKeyId, &'a str), &'a TsBindingFact>,
    ) -> Option<&'a TsBindingFact> {
        let mut current = scope_key;
        while let Some(scope) = current {
            if let Some(binding) = bindings.get(&(scope, name)) {
                return Some(*binding);
            }
            current = self
                .scope_parent_by_key
                .get(&scope)
                .and_then(|parent| *parent);
        }
        None
    }

    fn module_row(
        &self,
        callsite: &TsInventoryCallsiteFact,
        binding: &'a TsBindingFact,
        lookup: CalleeLookup<'_>,
    ) -> Option<TsDirectBindingFact> {
        let (export_name, kind) = imported_export_target(binding, lookup)?;
        let Some(resolved_import) = self.module_index.resolved_import(binding) else {
            return Some(unresolved_import_row(
                callsite,
                binding,
                None,
                None,
                kind,
                TsDirectBindingStatus::Unresolved,
                TsDirectBindingReason::ImportedBindingUnresolved,
            ));
        };
        let target_node = resolved_import.target_node;
        if resolved_import.status != ResolutionStatus::Resolved {
            let reason = if resolved_import.status == ResolutionStatus::External {
                TsDirectBindingReason::ExternalPackageUnresolved
            } else {
                TsDirectBindingReason::ImportedBindingUnresolved
            };
            let status = if resolved_import.status == ResolutionStatus::External {
                TsDirectBindingStatus::External
            } else {
                TsDirectBindingStatus::Unresolved
            };
            return Some(unresolved_import_row(
                callsite,
                binding,
                Some(resolved_import),
                target_node,
                kind,
                status,
                reason,
            ));
        }

        let Some(target_node) = target_node else {
            return Some(unresolved_import_row(
                callsite,
                binding,
                Some(resolved_import),
                None,
                kind,
                TsDirectBindingStatus::Unresolved,
                TsDirectBindingReason::ImportedBindingUnresolved,
            ));
        };
        if self.module_index.is_external_node(target_node) {
            return Some(unresolved_import_row(
                callsite,
                binding,
                Some(resolved_import),
                Some(target_node),
                kind,
                TsDirectBindingStatus::External,
                TsDirectBindingReason::ExternalPackageUnresolved,
            ));
        }

        let Some(target) = self
            .module_index
            .exported_function(target_node, export_name.as_str())
        else {
            return Some(unresolved_import_row(
                callsite,
                binding,
                Some(resolved_import),
                Some(target_node),
                kind,
                TsDirectBindingStatus::Unresolved,
                TsDirectBindingReason::ImportedBindingUnresolved,
            ));
        };

        Some(resolved_import_row(
            callsite,
            target.function,
            binding,
            resolved_import,
            target.module_node,
            if target.via_reexport {
                TsDirectBindingKind::ReExportedAlias
            } else {
                kind
            },
        ))
    }
}

#[derive(Clone, Copy)]
struct VisibleBinding<'a> {
    binding: &'a TsBindingFact,
    distance: usize,
    exact_member: bool,
}

fn choose_visible_binding<'a>(
    left: Option<VisibleBinding<'a>>,
    right: Option<VisibleBinding<'a>>,
) -> Option<VisibleBinding<'a>> {
    match (left, right) {
        (Some(left), Some(right)) => {
            if right.distance < left.distance
                || (right.distance == left.distance && right.exact_member && !left.exact_member)
            {
                Some(right)
            } else {
                Some(left)
            }
        }
        (Some(binding), None) | (None, Some(binding)) => Some(binding),
        (None, None) => None,
    }
}

struct ModuleBindingIndex<'a> {
    resolved_by_file_source: BTreeMap<(FileId, String), &'a ResolvedImportFact>,
    module_kind_by_id: BTreeMap<ModuleNodeId, ModuleNodeKind>,
    module_file_by_node: BTreeMap<ModuleNodeId, &'a TsDirectBindingModuleFile<'a>>,
}

impl<'a> ModuleBindingIndex<'a> {
    fn new(input: &'a TsDirectBindingModuleInput<'a>) -> Self {
        let import_path_by_id = input
            .imports
            .iter()
            .map(|import| (import.id, (import.file, import.path.as_str())))
            .collect::<BTreeMap<_, _>>();
        let resolved_by_file_source = input
            .resolved_imports
            .iter()
            .filter_map(|resolved| {
                let (file, path) = import_path_by_id.get(&resolved.import)?;
                Some(((*file, (*path).to_string()), resolved))
            })
            .collect();

        Self {
            resolved_by_file_source,
            module_kind_by_id: input
                .module_nodes
                .iter()
                .map(|node| (node.id, node.kind))
                .collect(),
            module_file_by_node: input
                .module_files
                .iter()
                .map(|module| (module.module_node, module))
                .collect(),
        }
    }

    fn resolved_import(&self, binding: &TsBindingFact) -> Option<&'a ResolvedImportFact> {
        let module_source = binding.module_source.as_ref()?;
        self.resolved_by_file_source
            .get(&(binding.file, module_source.clone()))
            .copied()
    }

    fn is_external_node(&self, node: ModuleNodeId) -> bool {
        self.module_kind_by_id
            .get(&node)
            .is_some_and(|kind| *kind == ModuleNodeKind::External)
    }

    fn exported_function(
        &self,
        module_node: ModuleNodeId,
        export_name: &str,
    ) -> Option<ModuleFunctionTarget<'a>> {
        let mut search = ExportSearch::default();
        let found = self.exported_function_inner(module_node, export_name, 0, &mut search);
        found.filter(|_| !search.exhausted)
    }

    /// The function `module_node` exports as `export_name`, following its
    /// re-exports. Each module and name is searched once: a second search
    /// finds what the first did not. Past nine hops or [`MAX_EXPORT_LOOKUPS`]
    /// lookups the search stops with no answer, as a later match would stand
    /// in for one the cut branch might have found first.
    fn exported_function_inner(
        &self,
        module_node: ModuleNodeId,
        export_name: &str,
        depth: usize,
        search: &mut ExportSearch,
    ) -> Option<ModuleFunctionTarget<'a>> {
        if search.exhausted {
            return None;
        }
        if depth > 8 || search.seen.len() >= MAX_EXPORT_LOOKUPS {
            search.exhausted = true;
            return None;
        }
        if !search.seen.insert((module_node, export_name.to_string())) {
            return None;
        }

        let module = self.module_file_by_node.get(&module_node)?;
        let mut explicit = false;
        for binding in &module.scope.bindings {
            if binding.exported_name.as_deref() != Some(export_name) {
                continue;
            }
            if let Some(function_key) = binding.inventory_function_key
                && let Some(declaration) = module
                    .inventory
                    .functions
                    .iter()
                    .find(|function| function.stable_key == function_key)
                && let Some(function) = implementation_of(
                    &module.inventory.functions.iter().collect::<Vec<_>>(),
                    declaration,
                )
            {
                return Some(ModuleFunctionTarget {
                    module_node,
                    function,
                    via_reexport: false,
                });
            }
            if binding.module_source.is_none()
                && let Some(imported_name) = binding.imported_name.as_deref()
                && let Some(function) = function_for_local_export(
                    module.inventory,
                    module.scope,
                    binding,
                    imported_name,
                )
            {
                return Some(ModuleFunctionTarget {
                    module_node,
                    function,
                    via_reexport: false,
                });
            }
            if let Some(function) = self.reexported_function(binding, export_name, depth, search) {
                return Some(function);
            }
            explicit = true;
        }
        // A name the module exports itself hides the `export *` ones, and two
        // `export *` that give one name different bindings give it none.
        if explicit {
            return None;
        }
        let mut found: Option<ModuleFunctionTarget<'a>> = None;
        for binding in &module.scope.bindings {
            if binding.exported_name.as_deref() != Some("*") {
                continue;
            }
            let Some(function) = self.reexported_function(binding, export_name, depth, search)
            else {
                continue;
            };
            match &found {
                Some(first)
                    if first.module_node != function.module_node
                        || first.function.stable_key != function.function.stable_key =>
                {
                    search.exhausted = true;
                    return None;
                }
                Some(_) => {}
                None => found = Some(function),
            }
        }
        found.filter(|_| !search.exhausted)
    }

    /// The function the re-export `binding` gives `export_name`, where it is
    /// a re-export of a resolved module.
    fn reexported_function(
        &self,
        binding: &TsBindingFact,
        export_name: &str,
        depth: usize,
        search: &mut ExportSearch,
    ) -> Option<ModuleFunctionTarget<'a>> {
        if !matches!(binding.binding_kind, TsBindingKind::ReExport) {
            return None;
        }
        let source = binding.module_source.as_ref()?;
        let reexport = self
            .resolved_by_file_source
            .get(&(binding.file, source.clone()))?;
        if reexport.status != ResolutionStatus::Resolved {
            return None;
        }
        let target_node = reexport.target_node?;
        let target_name = binding
            .imported_name
            .as_deref()
            .filter(|name| *name != "*")
            .unwrap_or(export_name);
        let mut function =
            self.exported_function_inner(target_node, target_name, depth + 1, search)?;
        function.via_reexport = true;
        Some(function)
    }
}

/// Bound on the module and name pairs one export search looks up.
const MAX_EXPORT_LOOKUPS: usize = 512;

#[derive(Default)]
struct ExportSearch {
    seen: BTreeSet<(ModuleNodeId, String)>,
    exhausted: bool,
}

struct ModuleFunctionTarget<'a> {
    module_node: ModuleNodeId,
    function: &'a TsInventoryFunctionFact,
    via_reexport: bool,
}

#[derive(Clone, Copy)]
struct CalleeLookup<'a> {
    display_name: &'a str,
    local_name: &'a str,
    member_name: Option<&'a str>,
    local_kind: TsDirectBindingKind,
}

impl<'a> CalleeLookup<'a> {
    fn new(display_name: &'a str) -> Self {
        display_name
            .rsplit_once('.')
            .map(|(object, property)| Self {
                display_name,
                local_name: object,
                member_name: Some(property),
                local_kind: TsDirectBindingKind::NamespaceMember,
            })
            .unwrap_or(Self {
                display_name,
                local_name: display_name,
                member_name: None,
                local_kind: TsDirectBindingKind::LocalFunction,
            })
    }

    fn target_name(self) -> &'a str {
        self.member_name.unwrap_or(self.local_name)
    }

    fn alias_name(self) -> &'a str {
        self.member_name
            .map(|_| self.display_name)
            .unwrap_or(self.local_name)
    }
}

fn intrinsic_unresolved_reason_for_callsite(
    callsite: &TsInventoryCallsiteFact,
) -> Option<TsDirectBindingReason> {
    if matches!(callsite.kind, TsCallsiteInventoryKind::DynamicImport)
        && matches!(callsite.status, TsInventoryStatus::Unresolved { .. })
    {
        return Some(TsDirectBindingReason::NonStringDynamicImport);
    }
    if callsite.display_name.is_none() {
        return Some(TsDirectBindingReason::ComputedProperty);
    }
    None
}

fn unsupported_binding_reason(binding: &TsBindingFact) -> Option<TsDirectBindingReason> {
    match &binding.status {
        TsBindingStatus::UnsupportedDynamic { reason } if reason.contains("function-token") => {
            Some(TsDirectBindingReason::TokenFlowRequired)
        }
        TsBindingStatus::UnsupportedDynamic { reason } if reason.contains("property-flow") => {
            Some(TsDirectBindingReason::PropertyFlowRequired)
        }
        TsBindingStatus::UnsupportedDynamic { reason } if reason.contains("prototype") => {
            Some(TsDirectBindingReason::PrototypeModelRequired)
        }
        TsBindingStatus::UnsupportedDynamic { reason } if reason.contains("receiver") => {
            Some(TsDirectBindingReason::ThisModelRequired)
        }
        _ => None,
    }
}

fn is_import_binding(binding: &TsBindingFact) -> bool {
    binding.module_source.is_some()
        && matches!(
            binding.binding_kind,
            TsBindingKind::Import
                | TsBindingKind::NamedImport
                | TsBindingKind::DefaultImport
                | TsBindingKind::NamespaceImport
                | TsBindingKind::ReExport
        )
}

fn span_contains(
    container: &crate::internal_core::Span,
    span: &crate::internal_core::Span,
) -> bool {
    container.file == span.file
        && container.start_byte <= span.start_byte
        && container.end_byte >= span.end_byte
}

fn spans_overlap(left: &crate::internal_core::Span, right: &crate::internal_core::Span) -> bool {
    left.file == right.file
        && left.start_byte <= right.end_byte
        && right.start_byte <= left.end_byte
}

fn imported_export_target(
    binding: &TsBindingFact,
    lookup: CalleeLookup<'_>,
) -> Option<(String, TsDirectBindingKind)> {
    match binding.binding_kind {
        TsBindingKind::NamedImport => binding
            .imported_name
            .as_deref()
            .map(|name| (name.to_string(), TsDirectBindingKind::ImportedNamed)),
        TsBindingKind::DefaultImport => {
            Some(("default".to_string(), TsDirectBindingKind::ImportedDefault))
        }
        TsBindingKind::NamespaceImport => lookup.member_name.map(|member| {
            (
                member.to_string(),
                TsDirectBindingKind::ImportedNamespaceMember,
            )
        }),
        TsBindingKind::ReExport => binding
            .imported_name
            .as_deref()
            .map(|name| (name.to_string(), TsDirectBindingKind::ReExportedAlias)),
        TsBindingKind::Import
            if binding.imported_name.as_deref() == Some("module.exports")
                && lookup.member_name.is_some() =>
        {
            lookup.member_name.map(|member| {
                (
                    member.to_string(),
                    TsDirectBindingKind::CommonJsRequireMember,
                )
            })
        }
        TsBindingKind::Import => binding
            .imported_name
            .as_deref()
            .map(|name| (name.to_string(), TsDirectBindingKind::CommonJsRequireMember)),
        _ => None,
    }
}

fn function_for_local_export<'a>(
    inventory: &'a TsInventoryOutput,
    scope: &'a TsScopeOutput,
    export: &TsBindingFact,
    local_name: &str,
) -> Option<&'a TsInventoryFunctionFact> {
    if let Some(binding) = visible_function_binding(scope, export.scope_key, local_name)
        && let Some(function) = function_by_binding(inventory, local_name, binding)
    {
        return Some(function);
    }

    let named = inventory
        .functions
        .iter()
        .filter(|function| function.display_name.as_deref() == Some(local_name))
        .collect::<Vec<_>>();
    let mut overlapping = named
        .iter()
        .copied()
        .filter(|function| spans_overlap(&function.span, &export.span));
    let first = overlapping.next()?;
    if overlapping.next().is_none() {
        implementation_of(&named, first)
    } else {
        None
    }
}

fn visible_function_binding<'a>(
    scope: &'a TsScopeOutput,
    scope_key: StableKeyId,
    name: &str,
) -> Option<&'a TsBindingFact> {
    let parent_by_key = scope
        .scopes
        .iter()
        .map(|scope| (scope.stable_key, scope.parent_scope_key))
        .collect::<BTreeMap<_, _>>();
    let mut current = Some(scope_key);
    while let Some(scope_key) = current {
        if let Some(binding) = scope.bindings.iter().find(|binding| {
            binding.scope_key == scope_key
                && binding.name == name
                && matches!(
                    binding.binding_kind,
                    TsBindingKind::FunctionDeclaration | TsBindingKind::FunctionExpression
                )
        }) {
            return Some(binding);
        }
        current = parent_by_key.get(&scope_key).and_then(|parent| *parent);
    }
    None
}

fn function_by_binding<'a>(
    inventory: &'a TsInventoryOutput,
    name: &str,
    binding: &TsBindingFact,
) -> Option<&'a TsInventoryFunctionFact> {
    let functions = inventory
        .functions
        .iter()
        .filter(|function| function.display_name.as_deref() == Some(name))
        .collect::<Vec<_>>();
    callable_for_binding(&functions, binding)
}

/// The function a call through `binding` runs, among the same-named
/// `functions`: the one whose declaration the binding names, else the only
/// callable one in the binding's file.
fn callable_for_binding<'a>(
    functions: &[&'a TsInventoryFunctionFact],
    binding: &TsBindingFact,
) -> Option<&'a TsInventoryFunctionFact> {
    if let Some(function) = functions.iter().copied().find(|function| {
        function.file == binding.file && spans_overlap(&function.span, &binding.span)
    }) {
        return implementation_of(functions, function);
    }

    let mut same_file = functions
        .iter()
        .copied()
        .filter(|function| function.file == binding.file && function.has_body);
    let first = same_file.next()?;
    if same_file.next().is_none() {
        Some(first)
    } else {
        None
    }
}

/// The function that runs when `declaration` is called. A declaration with a
/// body runs itself. A body-less one is an overload signature, whose
/// implementation is the next same-named declaration with a body in the same
/// file and lexical parent, or an ambient declaration with no implementation in
/// the analysed code, which no call is claimed to reach.
fn implementation_of<'a>(
    functions: &[&'a TsInventoryFunctionFact],
    declaration: &'a TsInventoryFunctionFact,
) -> Option<&'a TsInventoryFunctionFact> {
    if declaration.has_body {
        return Some(declaration);
    }
    functions
        .iter()
        .copied()
        .filter(|function| {
            function.has_body
                && function.file == declaration.file
                && function.lexical_parent_key == declaration.lexical_parent_key
                && function.display_name == declaration.display_name
                && function.span.start_byte >= declaration.span.end_byte
        })
        .min_by_key(|function| (function.span.start_byte, function.span.end_byte))
}

fn resolved_row(
    callsite: &TsInventoryCallsiteFact,
    target: &TsInventoryFunctionFact,
    binding: Option<&TsBindingFact>,
    kind: TsDirectBindingKind,
) -> TsDirectBindingFact {
    TsDirectBindingFact {
        id: TsDirectBindingId(0),
        callsite: callsite.id,
        callsite_stable_key: callsite.stable_key,
        target_function: Some(target.id),
        target_function_stable_key: Some(target.stable_key),
        scope_binding: binding.map(|binding| binding.id),
        scope_binding_stable_key: binding.map(|binding| binding.stable_key),
        resolved_import: None,
        module_node: None,
        kind,
        status: TsDirectBindingStatus::Resolved,
        reason: None,
        stable_key: direct_binding_key(
            callsite.stable_key,
            Some(target.stable_key),
            kind,
            TsDirectBindingStatus::Resolved,
            None,
        ),
    }
}

fn resolved_import_row(
    callsite: &TsInventoryCallsiteFact,
    target: &TsInventoryFunctionFact,
    binding: &TsBindingFact,
    resolved_import: &ResolvedImportFact,
    module_node: ModuleNodeId,
    kind: TsDirectBindingKind,
) -> TsDirectBindingFact {
    TsDirectBindingFact {
        id: TsDirectBindingId(0),
        callsite: callsite.id,
        callsite_stable_key: callsite.stable_key,
        target_function: Some(target.id),
        target_function_stable_key: Some(target.stable_key),
        scope_binding: Some(binding.id),
        scope_binding_stable_key: Some(binding.stable_key),
        resolved_import: Some(resolved_import.id),
        module_node: Some(module_node),
        kind,
        status: TsDirectBindingStatus::Resolved,
        reason: None,
        stable_key: direct_binding_key(
            callsite.stable_key,
            Some(target.stable_key),
            kind,
            TsDirectBindingStatus::Resolved,
            None,
        ),
    }
}

fn unresolved_row(
    callsite: &TsInventoryCallsiteFact,
    reason: TsDirectBindingReason,
) -> TsDirectBindingFact {
    TsDirectBindingFact {
        id: TsDirectBindingId(0),
        callsite: callsite.id,
        callsite_stable_key: callsite.stable_key,
        target_function: None,
        target_function_stable_key: None,
        scope_binding: None,
        scope_binding_stable_key: None,
        resolved_import: None,
        module_node: None,
        kind: TsDirectBindingKind::LocalFunction,
        status: TsDirectBindingStatus::Unresolved,
        reason: Some(reason),
        stable_key: direct_binding_key(
            callsite.stable_key,
            None,
            TsDirectBindingKind::LocalFunction,
            TsDirectBindingStatus::Unresolved,
            Some(reason),
        ),
    }
}

fn unresolved_import_row(
    callsite: &TsInventoryCallsiteFact,
    binding: &TsBindingFact,
    resolved_import: Option<&ResolvedImportFact>,
    module_node: Option<ModuleNodeId>,
    kind: TsDirectBindingKind,
    status: TsDirectBindingStatus,
    reason: TsDirectBindingReason,
) -> TsDirectBindingFact {
    TsDirectBindingFact {
        id: TsDirectBindingId(0),
        callsite: callsite.id,
        callsite_stable_key: callsite.stable_key,
        target_function: None,
        target_function_stable_key: None,
        scope_binding: Some(binding.id),
        scope_binding_stable_key: Some(binding.stable_key),
        resolved_import: resolved_import.map(|resolved| resolved.id),
        module_node,
        kind,
        status,
        reason: Some(reason),
        stable_key: direct_binding_key(
            callsite.stable_key,
            Some(binding.stable_key),
            kind,
            status,
            Some(reason),
        ),
    }
}

fn direct_binding_key(
    callsite_key: StableKeyId,
    target_key: Option<StableKeyId>,
    kind: TsDirectBindingKind,
    status: TsDirectBindingStatus,
    reason: Option<TsDirectBindingReason>,
) -> StableKeyId {
    let callsite_key = crate::ts::resolve_frontend_stable_key(callsite_key);
    let target_key = target_key
        .map(crate::ts::resolve_frontend_stable_key)
        .unwrap_or_else(|| "<unresolved>".into());
    crate::ts::intern_frontend_stable_key(format!(
        "ts_direct_binding|callsite={callsite_key}|target={target_key}|kind={kind:?}|status={}|reason={}",
        status.as_str(),
        reason.map(TsDirectBindingReason::as_str).unwrap_or("none")
    ))
}
