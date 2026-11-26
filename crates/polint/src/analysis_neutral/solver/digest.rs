//! Stable output digesting for the unified solver.

use std::fmt::Write as _;

use crate::analysis_api::{Digest, DigestKind, DigestParts, ProviderManifest};
use crate::internal_core::StableKeyInterner;

use super::budget::SolverBudget;
use super::cache_key::{solver_budget_digest_parts, solver_provider_parameter_digest};
use super::store::SolverOutput;

#[allow(clippy::too_many_arguments)]
pub fn solver_output_digest(
    interner: &StableKeyInterner,
    manifest: &ProviderManifest,
    budget: &SolverBudget,
    semantic_graph_output_digest: &Digest,
    type_value_alias_output_digest: &Digest,
    go_semantic_output_digest: &Digest,
    output: &SolverOutput,
) -> Digest {
    let mut parts = DigestParts::default();
    parts.extend([
        format!("provider_id={}", manifest.id),
        format!("provider_version={}", manifest.provider_version()),
        format!("schema={}", manifest.primary_schema_label()),
        format!("parameters={}", solver_provider_parameter_digest(budget)),
        format!("semantic_graph_output={semantic_graph_output_digest}"),
        format!("type_value_alias_output={type_value_alias_output_digest}"),
        format!("go_semantic_output={go_semantic_output_digest}"),
        format!("budget_status={}", output.budget_status.as_str()),
        format!("incomplete_sites={}", output.incomplete_sites),
    ]);
    parts.extend(solver_budget_digest_parts(budget));
    parts.extend_in_order(&output.derived_edges, |text, edge| {
        let _ = write!(
            text,
            "edge={}|status={:?}|prec={:?}|prov={}",
            interner.resolve(edge.stable_key),
            edge.status,
            edge.precision,
            edge.provenance.stable_key_fragment(interner),
        );
    });
    if output.derived_edges.is_empty() {
        parts.push("solver_output=empty");
    }
    parts.extend(
        output
            .budget_reasons
            .iter()
            .map(|reason| format!("budget_exceeded_reason={reason}")),
    );
    parts.digest(DigestKind::ProviderOutput, "solver_output")
}
