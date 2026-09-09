use crate::analysis_api::{Digest, DigestKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct V13CacheDependency {
    pub provider_id: &'static str,
    pub manifest_inputs: &'static [&'static str],
    pub upstream_output_digests: &'static [&'static str],
}

pub fn v13_cache_dependency_ledger() -> &'static [V13CacheDependency] {
    &[
        V13CacheDependency {
            provider_id: "polint.semantic_graph",
            manifest_inputs: &[
                "source_files",
                "functions",
                "packages",
                "imports",
                "resolved_imports",
                "module_nodes",
                "scopes",
                "call_sites",
                "value_facts",
                "places",
                "go_semantic_functions",
                "go_semantic_callsites",
                "ts_object_allocations",
                "ts_property_writes",
                "ts_property_reads",
                "ts_receiver_bindings",
                "ts_prototype_links",
                "adaptation_model_files",
                "adaptation_model_budget",
            ],
            upstream_output_digests: &[
                "polint.calls",
                "polint.identity",
                "polint.abstract_domains",
                "polint.entrypoints",
                "polint.reachability",
                "polint.type_value_alias",
                "polint.symbol_graph",
                "polint.module_topology",
                "polint.go.syntax",
                "polint.ts.syntax",
                "polint.semantic_mir",
                "ts_direct_binding_output",
                "adaptation_model_input",
                "polint.go.semantic",
            ],
        },
        V13CacheDependency {
            provider_id: "polint.go.semantic",
            manifest_inputs: &[
                "source_files",
                "packages",
                "functions",
                "go.module_roots",
                "go.package_patterns",
                "go.build_tags",
                "go.include_tests",
                "go.offline",
            ],
            upstream_output_digests: &[],
        },
        V13CacheDependency {
            provider_id: "polint.solver",
            manifest_inputs: &[
                "source_files",
                "functions",
                "call_sites",
                "imports",
                "resolved_imports",
                "module_nodes",
                "reachability_roots",
                "semantic_constraints",
                "semantic_nodes",
                "go_semantic_functions",
                "go_semantic_callsites",
                "go_semantic_method_sets",
                "go_semantic_address_taken",
                "go_semantic_instantiated_types",
                "go_semantic_dynamic_dispatch",
                "ts_object_allocations",
                "ts_property_writes",
                "ts_property_reads",
                "ts_receiver_bindings",
                "ts_prototype_links",
            ],
            upstream_output_digests: &[
                "polint.semantic_graph",
                "polint.type_value_alias",
                "polint.go.semantic",
            ],
        },
        V13CacheDependency {
            provider_id: "polint.refined_calls",
            manifest_inputs: &[
                "call_sites",
                "call_targets",
                "unresolved_calls",
                "functions",
                "symbols",
                "entrypoints",
                "dispatch_edges",
                "summary_call",
                "summary_events",
                "type_facts",
                "value_facts",
                "allocation_tokens",
                "points_to_sets",
                "alias_answers",
                "extension_facts",
                "semantic_nodes",
                "semantic_constraints",
                "go_semantic_functions",
                "go_semantic_callsites",
                "solver_derived_edges",
            ],
            upstream_output_digests: &[
                "polint.calls",
                "polint.entrypoints",
                "polint.direct_summaries",
                "polint.type_value_alias",
                "polint.extensions",
                "polint.solver",
            ],
        },
    ]
}

pub fn semantic_mir_provider_parameter_digest() -> Digest {
    Digest::from_parts(
        DigestKind::ProviderParameters,
        "semantic_mir_provider_parameters",
        &[
            "mir_bodies",
            "mir_operations",
            "places",
            "unsupported_semantics",
            "go_lowering",
            "ts_js_lowering",
            // A class's constructor body is lowered under the class function and
            // an object shorthand method's span follows its key: both change which
            // body owns a call site, so stale MIR must not be reused.
            "ts-class-constructor-owner-1",
            "semantic-mir-facts-5",
        ],
    )
}

#[cfg(test)]
mod semantic_mir_layer_key {
    use super::*;

    #[test]
    fn semantic_mir_provider_parameter_digest_uses_exact_output_and_lowering_terms() {
        assert_eq!(
            super::semantic_mir_provider_parameter_digest(),
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "semantic_mir_provider_parameters",
                &[
                    "mir_bodies",
                    "mir_operations",
                    "places",
                    "unsupported_semantics",
                    "go_lowering",
                    "ts_js_lowering",
                    "ts-class-constructor-owner-1",
                    "semantic-mir-facts-5",
                ],
            )
        );
    }

    #[test]
    fn v13_cache_dependency_ledger_names_cache_sensitive_inputs() {
        let ledger = v13_cache_dependency_ledger();

        assert_eq!(
            ledger
                .iter()
                .map(|entry| entry.provider_id)
                .collect::<Vec<_>>(),
            vec![
                "polint.semantic_graph",
                "polint.go.semantic",
                "polint.solver",
                "polint.refined_calls",
            ]
        );
        assert!(
            ledger
                .iter()
                .find(|entry| entry.provider_id == "polint.semantic_graph")
                .unwrap()
                .manifest_inputs
                .contains(&"adaptation_model_files")
        );
        assert!(
            ledger
                .iter()
                .find(|entry| entry.provider_id == "polint.semantic_graph")
                .unwrap()
                .manifest_inputs
                .contains(&"adaptation_model_budget")
        );
        assert!(
            ledger
                .iter()
                .find(|entry| entry.provider_id == "polint.solver")
                .unwrap()
                .manifest_inputs
                .contains(&"semantic_constraints")
        );
        assert!(
            ledger
                .iter()
                .find(|entry| entry.provider_id == "polint.refined_calls")
                .unwrap()
                .manifest_inputs
                .contains(&"solver_derived_edges")
        );
        assert!(
            ledger
                .iter()
                .find(|entry| entry.provider_id == "polint.solver")
                .unwrap()
                .upstream_output_digests
                .contains(&"polint.type_value_alias")
        );
    }
}
