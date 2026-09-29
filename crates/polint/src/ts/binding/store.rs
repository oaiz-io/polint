#![allow(dead_code, reason = "kept for private internal consumers")]

use std::collections::BTreeMap;

use crate::analysis_api::{Digest, DigestKind};
use crate::internal_core::{ModuleNodeId, StableKeyId, StableKeyInterner};
use crate::ts::binding::facts::{TsDirectBindingFact, TsDirectBindingReason};
use crate::ts::ids::TsDirectBindingId;

pub const TS_DIRECT_BINDING_SCHEMA_LABEL: &str = "ts-direct-binding-facts-1";

pub fn ts_direct_binding_provider_parameter_digest() -> Digest {
    Digest::from_parts(
        DigestKind::ProviderParameters,
        "ts_direct_binding_provider_parameters",
        &[
            TS_DIRECT_BINDING_SCHEMA_LABEL,
            "output=ts_direct_bindings",
            "output=direct_binding_statuses",
            "output=direct_binding_unresolved_reasons",
            "input=ts_syntax_output",
            "input=ts_inventory_output",
            "input=ts_scope_output",
            "input=module_graph_output",
            "lifecycle=tsconfig",
            "lifecycle=jsconfig",
            "lifecycle=tsconfig_extends",
            "lifecycle=package_json",
            "lifecycle=workspace_packages",
            "lifecycle=lockfile",
            "config=polint_config",
            "provider=ts_direct_binding_v1",
            "schema=ts_direct_binding_schema_v1",
        ],
    )
}

pub fn ts_direct_binding_output_digest(
    output: &TsDirectBindingOutput,
    interner: &StableKeyInterner,
) -> Digest {
    let mut parts = vec![format!(
        "parameters={}",
        ts_direct_binding_provider_parameter_digest()
    )];
    parts.extend(output.bindings.iter().map(|binding| {
        format!(
            "binding={} callsite={} target={} scope={} import={} module={} kind={:?} status={} reason={}",
            interner.resolve(binding.stable_key),
            interner.resolve(binding.callsite_stable_key),
            binding
                .target_function_stable_key
                .map(|key| interner.resolve(key))
                .unwrap_or_else(|| "none".into()),
            binding
                .scope_binding_stable_key
                .map(|key| interner.resolve(key))
                .unwrap_or_else(|| "none".into()),
            binding
                .resolved_import
                .map(|id| id.0.to_string())
                .unwrap_or_else(|| "none".to_string()),
            binding
                .module_node
                .map(|id| id.0.to_string())
                .unwrap_or_else(|| "none".to_string()),
            binding.kind,
            binding.status.as_str(),
            binding
                .reason
                .map(TsDirectBindingReason::as_str)
                .unwrap_or("none"),
        )
    }));
    if output.bindings.is_empty() {
        parts.push("ts_direct_binding_output=empty".to_string());
    }
    parts.sort();
    let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    Digest::from_parts(
        DigestKind::ProviderOutput,
        "ts_direct_binding_output",
        &refs,
    )
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TsDirectBindingOutput {
    pub bindings: Vec<TsDirectBindingFact>,
}

impl TsDirectBindingOutput {
    pub fn normalized(mut self, interner: &StableKeyInterner) -> Self {
        self.bindings.sort_by(|binding, other| {
            interner
                .compare_canonical(binding.stable_key, other.stable_key)
                .then_with(|| {
                    interner
                        .compare_canonical(binding.callsite_stable_key, other.callsite_stable_key)
                })
        });
        for (index, binding) in self.bindings.iter_mut().enumerate() {
            binding.id = TsDirectBindingId(index as u64);
        }
        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct TsDirectBindingStore {
    output: TsDirectBindingOutput,
    bindings_by_callsite_stable_key: BTreeMap<StableKeyId, Vec<usize>>,
    bindings_by_target_function_stable_key: BTreeMap<StableKeyId, Vec<usize>>,
    bindings_by_unresolved_reason: BTreeMap<TsDirectBindingReason, Vec<usize>>,
    bindings_by_module_node: BTreeMap<ModuleNodeId, Vec<usize>>,
    bindings_by_stable_key: BTreeMap<StableKeyId, usize>,
}

impl TsDirectBindingStore {
    pub fn from_output(output: TsDirectBindingOutput, interner: &StableKeyInterner) -> Self {
        let output = output.normalized(interner);
        let mut store = Self {
            output,
            ..Self::default()
        };

        for (index, binding) in store.output.bindings.iter().enumerate() {
            store
                .bindings_by_callsite_stable_key
                .entry(binding.callsite_stable_key)
                .or_default()
                .push(index);
            if let Some(target_function) = binding.target_function_stable_key {
                store
                    .bindings_by_target_function_stable_key
                    .entry(target_function)
                    .or_default()
                    .push(index);
            }
            if let Some(reason) = binding.reason {
                store
                    .bindings_by_unresolved_reason
                    .entry(reason)
                    .or_default()
                    .push(index);
            }
            if let Some(module_node) = binding.module_node {
                store
                    .bindings_by_module_node
                    .entry(module_node)
                    .or_default()
                    .push(index);
            }
            store
                .bindings_by_stable_key
                .insert(binding.stable_key, index);
        }

        store
    }

    pub fn bindings(&self) -> &[TsDirectBindingFact] {
        &self.output.bindings
    }

    pub fn bindings_by_callsite_stable_key(
        &self,
        callsite_stable_key: StableKeyId,
    ) -> Vec<&TsDirectBindingFact> {
        self.binding_refs(
            self.bindings_by_callsite_stable_key
                .get(&callsite_stable_key),
        )
    }

    pub fn bindings_by_target_function_stable_key(
        &self,
        target_function_stable_key: StableKeyId,
    ) -> Vec<&TsDirectBindingFact> {
        self.binding_refs(
            self.bindings_by_target_function_stable_key
                .get(&target_function_stable_key),
        )
    }

    pub fn bindings_by_unresolved_reason(
        &self,
        reason: TsDirectBindingReason,
    ) -> Vec<&TsDirectBindingFact> {
        self.binding_refs(self.bindings_by_unresolved_reason.get(&reason))
    }

    pub fn bindings_by_module_node(&self, module_node: ModuleNodeId) -> Vec<&TsDirectBindingFact> {
        self.binding_refs(self.bindings_by_module_node.get(&module_node))
    }

    pub fn binding_by_stable_key(&self, stable_key: StableKeyId) -> Option<&TsDirectBindingFact> {
        self.bindings_by_stable_key
            .get(&stable_key)
            .map(|index| &self.output.bindings[*index])
    }

    fn binding_refs(&self, indexes: Option<&Vec<usize>>) -> Vec<&TsDirectBindingFact> {
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
    use crate::analysis_api::{Digest, DigestKind};
    use crate::internal_core::{ModuleNodeId, ResolvedImportId};
    use crate::ts::binding::facts::{
        TsDirectBindingKind, TsDirectBindingReason, TsDirectBindingStatus,
    };
    use crate::ts::ids::{
        TsBindingId, TsDirectBindingId, TsInventoryCallsiteId, TsInventoryFunctionId,
    };

    use super::*;

    #[test]
    fn normalized_assigns_dense_ids_after_stable_key_sort() {
        let interner = StableKeyInterner::default();
        let output = TsDirectBindingOutput {
            bindings: vec![
                binding(
                    &interner,
                    "binding:b",
                    TsDirectBindingId(22),
                    TsInventoryCallsiteId(2),
                ),
                binding(
                    &interner,
                    "binding:a",
                    TsDirectBindingId(11),
                    TsInventoryCallsiteId(1),
                ),
            ],
        }
        .normalized(&interner);

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
    fn store_indexes_callsite_target_reason_module_and_stable_key() {
        let interner = StableKeyInterner::default();
        let mut resolved = binding(
            &interner,
            "binding:resolved",
            TsDirectBindingId(10),
            TsInventoryCallsiteId(3),
        );
        resolved.target_function = Some(TsInventoryFunctionId(7));
        resolved.target_function_stable_key = Some(interner.intern("function:7"));
        resolved.module_node = Some(ModuleNodeId::from_raw(9));

        let mut unresolved = binding(
            &interner,
            "binding:unresolved",
            TsDirectBindingId(11),
            TsInventoryCallsiteId(4),
        );
        unresolved.status = TsDirectBindingStatus::Unresolved;
        unresolved.reason = Some(TsDirectBindingReason::ImportedBindingUnresolved);

        let store = TsDirectBindingStore::from_output(
            TsDirectBindingOutput {
                bindings: vec![unresolved, resolved],
            },
            &interner,
        );

        assert_eq!(store.bindings().len(), 2);
        assert_eq!(
            store.bindings_by_callsite_stable_key(interner.intern("callsite:3"))[0].stable_key,
            interner.intern("binding:resolved")
        );
        assert_eq!(
            store.bindings_by_target_function_stable_key(interner.intern("function:7"))[0]
                .stable_key,
            interner.intern("binding:resolved")
        );
        assert_eq!(
            store.bindings_by_module_node(ModuleNodeId::from_raw(9))[0].stable_key,
            interner.intern("binding:resolved")
        );
        assert_eq!(
            store.bindings_by_unresolved_reason(TsDirectBindingReason::ImportedBindingUnresolved)
                [0]
            .stable_key,
            interner.intern("binding:unresolved")
        );
        assert_eq!(
            store
                .binding_by_stable_key(interner.intern("binding:resolved"))
                .map(|binding| binding.callsite),
            Some(TsInventoryCallsiteId(3))
        );
    }

    #[test]
    fn store_indexes_global_output_by_stable_keys_not_dense_ids() {
        let interner = StableKeyInterner::default();
        let mut first = binding(
            &interner,
            "binding:first",
            TsDirectBindingId(10),
            TsInventoryCallsiteId(0),
        );
        first.callsite_stable_key = interner.intern("file:a:callsite:0");
        first.target_function = Some(TsInventoryFunctionId(0));
        first.target_function_stable_key = Some(interner.intern("file:a:function:0"));

        let mut second = binding(
            &interner,
            "binding:second",
            TsDirectBindingId(11),
            TsInventoryCallsiteId(0),
        );
        second.callsite_stable_key = interner.intern("file:b:callsite:0");
        second.target_function = Some(TsInventoryFunctionId(0));
        second.target_function_stable_key = Some(interner.intern("file:b:function:0"));

        let store = TsDirectBindingStore::from_output(
            TsDirectBindingOutput {
                bindings: vec![first, second],
            },
            &interner,
        );

        assert_eq!(
            store.bindings_by_callsite_stable_key(interner.intern("file:a:callsite:0"))[0]
                .stable_key,
            interner.intern("binding:first")
        );
        assert_eq!(
            store.bindings_by_callsite_stable_key(interner.intern("file:b:callsite:0"))[0]
                .stable_key,
            interner.intern("binding:second")
        );
        assert_eq!(
            store.bindings_by_target_function_stable_key(interner.intern("file:a:function:0"))[0]
                .stable_key,
            interner.intern("binding:first")
        );
        assert_eq!(
            store.bindings_by_target_function_stable_key(interner.intern("file:b:function:0"))[0]
                .stable_key,
            interner.intern("binding:second")
        );
    }

    #[test]
    fn provider_parameter_digest_names_phase_45_d12_inputs() {
        assert_eq!(
            ts_direct_binding_provider_parameter_digest(),
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "ts_direct_binding_provider_parameters",
                &[
                    "ts-direct-binding-facts-1",
                    "output=ts_direct_bindings",
                    "output=direct_binding_statuses",
                    "output=direct_binding_unresolved_reasons",
                    "input=ts_syntax_output",
                    "input=ts_inventory_output",
                    "input=ts_scope_output",
                    "input=module_graph_output",
                    "lifecycle=tsconfig",
                    "lifecycle=jsconfig",
                    "lifecycle=tsconfig_extends",
                    "lifecycle=package_json",
                    "lifecycle=workspace_packages",
                    "lifecycle=lockfile",
                    "config=polint_config",
                    "provider=ts_direct_binding_v1",
                    "schema=ts_direct_binding_schema_v1",
                ],
            )
        );
    }

    #[test]
    fn output_digest_changes_for_binding_rows() {
        let interner = StableKeyInterner::default();
        let empty = TsDirectBindingOutput::default();
        let populated = TsDirectBindingOutput {
            bindings: vec![binding(
                &interner,
                "binding:resolved",
                TsDirectBindingId(10),
                TsInventoryCallsiteId(3),
            )],
        }
        .normalized(&interner);

        assert_ne!(
            ts_direct_binding_output_digest(&empty, &interner),
            ts_direct_binding_output_digest(&populated, &interner)
        );
    }

    #[test]
    fn output_digest_changes_for_source_scope_module_and_status_inputs() {
        let interner = StableKeyInterner::default();
        let base = TsDirectBindingOutput {
            bindings: vec![binding(
                &interner,
                "binding:resolved",
                TsDirectBindingId(10),
                TsInventoryCallsiteId(3),
            )],
        };
        let base_digest = ts_direct_binding_output_digest(&base, &interner);

        let mut changed_source = base.clone();
        changed_source.bindings[0].callsite_stable_key = interner.intern("callsite:changed-source");
        assert_ne!(
            base_digest,
            ts_direct_binding_output_digest(&changed_source, &interner)
        );

        let mut changed_scope = base.clone();
        changed_scope.bindings[0].scope_binding_stable_key =
            Some(interner.intern("scope:changed-binding"));
        assert_ne!(
            base_digest,
            ts_direct_binding_output_digest(&changed_scope, &interner)
        );

        let mut changed_module = base.clone();
        changed_module.bindings[0].resolved_import = Some(ResolvedImportId::from_raw(44));
        changed_module.bindings[0].module_node = Some(ModuleNodeId::from_raw(45));
        assert_ne!(
            base_digest,
            ts_direct_binding_output_digest(&changed_module, &interner)
        );

        let mut changed_status = base;
        changed_status.bindings[0].status = TsDirectBindingStatus::Unresolved;
        changed_status.bindings[0].reason = Some(TsDirectBindingReason::ImportedBindingUnresolved);
        assert_ne!(
            base_digest,
            ts_direct_binding_output_digest(&changed_status, &interner)
        );
    }

    #[test]
    fn output_digest_preserves_hit_for_noop_row_order_change() {
        let interner = StableKeyInterner::default();
        let first = TsDirectBindingOutput {
            bindings: vec![
                binding(
                    &interner,
                    "binding:a",
                    TsDirectBindingId(10),
                    TsInventoryCallsiteId(3),
                ),
                binding(
                    &interner,
                    "binding:b",
                    TsDirectBindingId(11),
                    TsInventoryCallsiteId(4),
                ),
            ],
        };
        let second = TsDirectBindingOutput {
            bindings: vec![
                binding(
                    &interner,
                    "binding:b",
                    TsDirectBindingId(11),
                    TsInventoryCallsiteId(4),
                ),
                binding(
                    &interner,
                    "binding:a",
                    TsDirectBindingId(10),
                    TsInventoryCallsiteId(3),
                ),
            ],
        };

        assert_eq!(
            ts_direct_binding_output_digest(&first, &interner),
            ts_direct_binding_output_digest(&second, &interner)
        );
    }

    fn binding(
        interner: &StableKeyInterner,
        stable_key: &str,
        id: TsDirectBindingId,
        callsite: TsInventoryCallsiteId,
    ) -> TsDirectBindingFact {
        TsDirectBindingFact {
            id,
            callsite,
            callsite_stable_key: interner.intern(format!("callsite:{}", callsite.0)),
            target_function: None,
            target_function_stable_key: None,
            scope_binding: Some(TsBindingId(1)),
            scope_binding_stable_key: Some(interner.intern("scope:binding")),
            resolved_import: Some(ResolvedImportId::from_raw(2)),
            module_node: None,
            kind: TsDirectBindingKind::ImportedNamed,
            status: TsDirectBindingStatus::Resolved,
            reason: None,
            stable_key: interner.intern(stable_key),
        }
    }
}
