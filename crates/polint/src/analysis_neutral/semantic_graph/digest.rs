//! Stable semantic-graph output digesting.
//!
//! The composition root supplies the upstream/frontend component labels; this
//! module owns the language-neutral graph-row digest and the lifecycle component
//! folding shared by all hosts.

use crate::analysis_api::{
    Digest, DigestKind, DigestParts, InputComponent, InputSnapshot, ProviderManifest,
};
use crate::internal_core::StableKeyInterner;

use super::cache_key::semantic_graph_provider_parameter_digest;
use super::store::SemanticGraphOutput;

pub fn semantic_graph_output_digest(
    manifest: &ProviderManifest,
    input_snapshot: &InputSnapshot,
    mut parts: Vec<String>,
    interner: &StableKeyInterner,
    output: &SemanticGraphOutput,
) -> Digest {
    use std::fmt::Write as _;
    parts.extend_component_parts("go_lifecycle", &input_snapshot.go_lifecycle.components);
    parts.extend_component_parts(
        "ts_js_lifecycle",
        &input_snapshot.ts_js_lifecycle.components,
    );

    let node_key_by_id: Vec<_> = output
        .nodes
        .iter()
        .map(|node| interner.resolve(node.stable_key))
        .collect();
    let header = parts;
    let mut parts = DigestParts::default();
    parts.extend(header);
    parts.extend_in_order(&output.nodes, |text, node| {
        let _ = write!(
            text,
            "node={}|prec={:?}",
            interner.resolve(node.stable_key),
            node.precision
        );
    });
    parts.extend_in_order(&output.edges, |text, edge| {
        let _ = write!(
            text,
            "edge={}|prec={:?}",
            interner.resolve(edge.stable_key),
            edge.precision
        );
    });
    parts.extend_in_order(&output.constraints, |text, constraint| {
        let mut refs: Vec<_> = constraint
            .kind
            .referenced_nodes()
            .into_iter()
            .map(|node| {
                node_key_by_id
                    .get(node.0 as usize)
                    .cloned()
                    .unwrap_or_else(|| "<unresolved>".into())
            })
            .collect();
        refs.sort_unstable();
        let _ = write!(
            text,
            "constraint={}|status={:?}|prec={:?}|refs=[{}]",
            interner.resolve(constraint.stable_key),
            constraint.status,
            constraint.precision,
            refs.join(","),
        );
    });
    if output.nodes.is_empty() && output.edges.is_empty() && output.constraints.is_empty() {
        parts.push("semantic_graph_output=empty");
    }

    parts.push(&format!("provider_id={}", manifest.id));
    parts.push(&format!("provider_version={}", manifest.provider_version()));
    parts.push(&format!("schema={}", manifest.primary_schema_label()));
    parts.push(&format!(
        "parameters={}",
        semantic_graph_provider_parameter_digest()
    ));
    parts.digest(DigestKind::ProviderOutput, "semantic_graph_output")
}

trait ComponentParts {
    fn extend_component_parts(&mut self, prefix: &str, components: &[InputComponent]);
}

impl ComponentParts for Vec<String> {
    fn extend_component_parts(&mut self, prefix: &str, components: &[InputComponent]) {
        self.extend(components.iter().map(|component| {
            format!(
                "{prefix}:{}:{:?}:{}",
                component.name, component.status, component.digest
            )
        }));
    }
}
