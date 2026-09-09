use crate::analysis_api::{Digest, DigestKind};

/// Schema label for the `polint.semantic_graph` provider manifest.
pub const SEMANTIC_GRAPH_SCHEMA_LABEL: &str = "semantic-graph-facts-1";

/// Provider parameter digest for `polint.semantic_graph`.
///
/// The algorithm-version strings are part of the parameter digest so any bump to a
/// node/edge/constraint emission algorithm deterministically invalidates the
/// semantic-graph cache. The locked test below is the intended trip-wire: adding or
/// bumping an algorithm version requires extending this list.
///
/// # SC3 dependency-index inputs — PRESENT-NOW vs DEFERRED-AND-WHY (D-17)
///
/// The dependency-index contract enumerates the inputs that the unified semantic
/// graph will eventually digest. Producers exist for only a
/// subset; the rest have no producer yet and are documented here as RESERVED rather
/// than silently omitted, so the partial coverage reads as intentional. Each
/// deferred input enters this digest (and the manifest `inputs` slice) only when its
/// producer lands — invalidating the cache at that point by construction.
///
/// READ-AND-FOLDED (the projection reads these families today; the producer output
/// digest of each is folded into the provider output digest in
/// `provider::semantic_graph_output_digest`, and the families appear in the manifest
/// `inputs` slice):
/// - functions / packages (`polint.go.syntax` / `polint.ts.syntax`)
/// - scopes (`polint.symbol_graph`)
/// - call sites (`polint.calls`)
/// - value facts (`polint.type_value_alias`)
/// - MIR places (`polint.semantic_mir`)
///
/// ALSO-FOLDED, NOT-YET-READ (digests folded so the keystone over-invalidates rather
/// than risks a stale graph as later stages begin consuming them, but the projection
/// does NOT read these families yet, so they are NOT in the manifest `inputs` slice):
/// `polint.identity`, `polint.abstract_domains`, `polint.entrypoints`,
/// `polint.reachability`, `polint.module_topology`.
///
/// DEFERRED-AND-WHY (no producer exists yet, so these inputs are not silently
/// treated as present):
/// - CFG — reserved for the budgeted solver consumption.
/// - summaries — reserved for interprocedural summary folding.
/// - accepted adaptation models — require the private model producer and
///   digest recipe; semantic-graph lowering starts once accepted facts feed
///   `ConstraintKind::ModelEdge`.
/// - solver budgets — require adaptation-model budget threading and a complete
///   cache/budget sweep.
///
/// This is a comment/doc addition only: ZERO deferred inputs are digested here, and
/// the runtime behavior is unchanged.
pub fn semantic_graph_provider_parameter_digest() -> Digest {
    Digest::from_parts(
        DigestKind::ProviderParameters,
        "semantic_graph_provider_parameters",
        &[
            SEMANTIC_GRAPH_SCHEMA_LABEL,
            "semantic_nodes",
            "semantic_edges",
            "semantic_constraints",
            "node_projection_v1",
            "edge_projection_v1",
            "constraint_projection_v1",
            "ts_direct_binding_output",
            "ts_direct_binding_projection_v1",
            "ts_token_source_flow_projection_v1",
            "ts_callable_flow_projection_v1",
            "go_semantic_output_digest",
            "go_semantic_projection_v1",
            "ts_object_model_projection_v4",
            "adaptation_model_v1",
        ],
    )
}

#[cfg(test)]
mod tests {
    use crate::analysis_api::{Digest, DigestKind};

    #[test]
    fn semantic_graph_provider_parameter_digest_locks_parts_list() {
        assert_eq!(
            super::semantic_graph_provider_parameter_digest(),
            Digest::from_parts(
                DigestKind::ProviderParameters,
                "semantic_graph_provider_parameters",
                &[
                    "semantic-graph-facts-1",
                    "semantic_nodes",
                    "semantic_edges",
                    "semantic_constraints",
                    "node_projection_v1",
                    "edge_projection_v1",
                    "constraint_projection_v1",
                    "ts_direct_binding_output",
                    "ts_direct_binding_projection_v1",
                    "ts_token_source_flow_projection_v1",
                    "ts_callable_flow_projection_v1",
                    "go_semantic_output_digest",
                    "go_semantic_projection_v1",
                    "ts_object_model_projection_v4",
                    "adaptation_model_v1",
                ],
            )
        );
    }

    #[test]
    fn algorithm_version_bump_invalidates_the_pre_bump_digest() {
        // Bumping any frozen algorithm version must deterministically invalidate the
        // semantic-graph cache: the live digest differs from a pre-bump parts list,
        // so stale nodes/edges/constraints are not silently reused.
        let pre_bump = Digest::from_parts(
            DigestKind::ProviderParameters,
            "semantic_graph_provider_parameters",
            &[
                "semantic-graph-facts-1",
                "semantic_nodes",
                "semantic_edges",
                "semantic_constraints",
                "node_projection_v0",
                "edge_projection_v1",
                "constraint_projection_v1",
            ],
        );
        assert_ne!(super::semantic_graph_provider_parameter_digest(), pre_bump);
    }

    #[test]
    fn ts_direct_binding_projection_invalidates_the_pre_phase_45_digest() {
        let pre_phase_45 = Digest::from_parts(
            DigestKind::ProviderParameters,
            "semantic_graph_provider_parameters",
            &[
                "semantic-graph-facts-1",
                "semantic_nodes",
                "semantic_edges",
                "semantic_constraints",
                "node_projection_v1",
                "edge_projection_v1",
                "constraint_projection_v1",
            ],
        );

        assert_ne!(
            super::semantic_graph_provider_parameter_digest(),
            pre_phase_45
        );
    }

    #[test]
    fn go_semantic_projection_invalidates_the_pre_phase_46_digest() {
        let pre_phase_46 = Digest::from_parts(
            DigestKind::ProviderParameters,
            "semantic_graph_provider_parameters",
            &[
                "semantic-graph-facts-1",
                "semantic_nodes",
                "semantic_edges",
                "semantic_constraints",
                "node_projection_v1",
                "edge_projection_v1",
                "constraint_projection_v1",
                "ts_direct_binding_output",
                "ts_direct_binding_projection_v1",
            ],
        );

        assert_ne!(
            super::semantic_graph_provider_parameter_digest(),
            pre_phase_46
        );
    }

    #[test]
    fn semantic_graph_schema_label_is_semantic_graph_facts_1() {
        assert_eq!(super::SEMANTIC_GRAPH_SCHEMA_LABEL, "semantic-graph-facts-1");
    }
}
