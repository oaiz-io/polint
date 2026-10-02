use super::cache_key::abstract_domains_provider_parameter_digest;
use super::solver::{IdeDomainSolver, SolverInput, SolverPolicy};
use super::store::{DomainMaterialization, DomainOutput};
use crate::analysis_api::ProviderManifest;
use crate::analysis_api::{CacheStats, Digest, DigestKind, InputComponent, InputSnapshot};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::cfg::ids::BasicBlockId;
use crate::analysis_neutral::ids::{MirBodyId, MirOpId, PlaceId};
use crate::internal_core::Diagnostic;

#[derive(Debug, Clone, Default)]
pub struct AbstractDomainsProviderOutput {
    pub diagnostics: Vec<Diagnostic>,
    pub cache_stats: CacheStats,
    pub output_digest: Option<Digest>,
}

#[allow(clippy::too_many_arguments)]
pub fn derive_abstract_domains_with_cache_stats(
    db: &mut impl AnalysisHost,
    input_snapshot: &InputSnapshot,
    manifest: &ProviderManifest,
    semantic_mir_output_digest: Digest,
    cfg_output_digest: Digest,
    calls_output_digest: Digest,
    symbol_graph_output_digest: Digest,
    module_topology_output_digest: Digest,
    upstream_syntax_output_digests: Vec<Digest>,
) -> AbstractDomainsProviderOutput {
    derive_abstract_domains_with_materialization(
        db,
        input_snapshot,
        manifest,
        semantic_mir_output_digest,
        cfg_output_digest,
        calls_output_digest,
        symbol_graph_output_digest,
        module_topology_output_digest,
        upstream_syntax_output_digests,
        DomainMaterialization::Full,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn derive_summary_input_abstract_domains_with_cache_stats(
    db: &mut impl AnalysisHost,
    input_snapshot: &InputSnapshot,
    manifest: &ProviderManifest,
    semantic_mir_output_digest: Digest,
    cfg_output_digest: Digest,
    calls_output_digest: Digest,
    symbol_graph_output_digest: Digest,
    module_topology_output_digest: Digest,
    upstream_syntax_output_digests: Vec<Digest>,
) -> AbstractDomainsProviderOutput {
    derive_abstract_domains_with_materialization(
        db,
        input_snapshot,
        manifest,
        semantic_mir_output_digest,
        cfg_output_digest,
        calls_output_digest,
        symbol_graph_output_digest,
        module_topology_output_digest,
        upstream_syntax_output_digests,
        DomainMaterialization::SummaryInputs,
    )
}

#[allow(clippy::too_many_arguments)]
fn derive_abstract_domains_with_materialization(
    db: &mut impl AnalysisHost,
    input_snapshot: &InputSnapshot,
    manifest: &ProviderManifest,
    semantic_mir_output_digest: Digest,
    cfg_output_digest: Digest,
    calls_output_digest: Digest,
    symbol_graph_output_digest: Digest,
    module_topology_output_digest: Digest,
    upstream_syntax_output_digests: Vec<Digest>,
    materialization: DomainMaterialization,
) -> AbstractDomainsProviderOutput {
    let mut started = std::time::Instant::now();
    let mut checkpoint = |step: &'static str| {
        tracing::debug!(target: "polint::kernel::stage", provider = "polint.abstract_domains", step, elapsed_ms = started.elapsed().as_millis() as u64, "provider step");
        started = std::time::Instant::now();
    };
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let solver = IdeDomainSolver::new(SolverPolicy::deterministic());
    let result = match materialization {
        DomainMaterialization::Full => solver.solve(SolverInput::from(&*db)),
        DomainMaterialization::SummaryInputs => {
            solver.solve_summary_inputs(SolverInput::from(&*db))
        }
    };
    checkpoint("solve");
    let body_keys = body_stable_key_map(db);
    let block_keys = block_stable_key_map(db);
    let operation_keys = operation_stable_key_map(db);
    let place_keys = place_stable_key_map(db);
    checkpoint("key_maps");
    let output = DomainOutput::from_results_with_materialization(
        interner,
        result.results(),
        Some(&place_keys),
        materialization,
    );
    checkpoint("materialize");
    let output_digest = abstract_domains_output_digest(
        manifest,
        input_snapshot,
        &semantic_mir_output_digest,
        &cfg_output_digest,
        &calls_output_digest,
        &symbol_graph_output_digest,
        &module_topology_output_digest,
        &upstream_syntax_output_digests,
        &body_keys,
        &block_keys,
        &operation_keys,
        &place_keys,
        interner,
        &output,
        materialization,
    );
    checkpoint("digest");
    let mut cache_stats = CacheStats::default();
    cache_stats.record_recompute();
    db.replace_abstract_domain_facts(output);
    checkpoint("store_metadata");

    AbstractDomainsProviderOutput {
        diagnostics: Vec::new(),
        cache_stats,
        output_digest: Some(output_digest),
    }
}

#[allow(clippy::too_many_arguments)]
fn abstract_domains_output_digest(
    manifest: &ProviderManifest,
    input_snapshot: &InputSnapshot,
    semantic_mir_output_digest: &Digest,
    cfg_output_digest: &Digest,
    calls_output_digest: &Digest,
    symbol_graph_output_digest: &Digest,
    module_topology_output_digest: &Digest,
    upstream_syntax_output_digests: &[Digest],
    body_keys: &std::collections::BTreeMap<MirBodyId, String>,
    block_keys: &std::collections::BTreeMap<BasicBlockId, String>,
    operation_keys: &std::collections::BTreeMap<MirOpId, String>,
    place_keys: &std::collections::BTreeMap<PlaceId, String>,
    interner: &crate::internal_core::StableKeyInterner,
    output: &DomainOutput,
    materialization: DomainMaterialization,
) -> Digest {
    let mut parts = vec![
        format!("provider_id={}", manifest.id),
        format!("provider_version={}", manifest.provider_version()),
        format!("schema={}", manifest.primary_schema_label()),
        format!("materialization={}", materialization_label(materialization)),
        format!(
            "parameters={}",
            abstract_domains_provider_parameter_digest()
        ),
        format!("config={}", input_snapshot.config.digest),
        format!("semantic_mir={semantic_mir_output_digest}"),
        format!("cfg={cfg_output_digest}"),
        format!("calls={calls_output_digest}"),
        format!("symbol_graph={symbol_graph_output_digest}"),
        format!("module_topology={module_topology_output_digest}"),
    ];
    extend_component_parts(
        &mut parts,
        "go_lifecycle",
        &input_snapshot.go_lifecycle.components,
    );
    extend_component_parts(
        &mut parts,
        "ts_js_lifecycle",
        &input_snapshot.ts_js_lifecycle.components,
    );
    extend_component_parts(&mut parts, "model", &input_snapshot.models);
    extend_component_parts(&mut parts, "extension", &input_snapshot.extensions);
    extend_component_parts(&mut parts, "tool", &input_snapshot.tool_invocations);
    parts.extend(
        upstream_syntax_output_digests
            .iter()
            .map(|digest| format!("upstream_syntax={digest}")),
    );
    parts.extend(output.observations.iter().map(|row| {
        format!(
            "observation={} body={} block={} operation={} place={} slot={:?} location={:?} status={:?} precision={:?} value={:?}",
            interner.resolve(row.stable_key),
            stable_body_key(body_keys, row.body),
            row.block
                .and_then(|block| block_keys.get(&block).cloned())
                .unwrap_or_else(|| "none".to_string()),
            row.operation
                .and_then(|operation| operation_keys.get(&operation).cloned())
                .unwrap_or_else(|| "none".to_string()),
            row.place
                .and_then(|place| place_keys.get(&place).cloned())
                .unwrap_or_else(|| "none".to_string()),
            row.slot,
            row.location,
            row.status,
            row.precision,
            row.value,
        )
    }));
    parts.extend(output.events.iter().map(|row| {
        format!(
            "event={} body={} block={} operation={} slot={:?} status={:?} precision={:?} reason={}",
            interner.resolve(row.stable_key),
            stable_body_key(body_keys, row.body),
            row.block
                .and_then(|block| block_keys.get(&block).cloned())
                .unwrap_or_else(|| "none".to_string()),
            row.operation
                .and_then(|operation| operation_keys.get(&operation).cloned())
                .unwrap_or_else(|| "none".to_string()),
            row.slot,
            row.status,
            row.precision,
            row.reason,
        )
    }));
    if output.observations.is_empty() && output.events.is_empty() {
        parts.push("domain_output=empty".to_string());
    }

    parts.sort();
    let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    Digest::from_parts(DigestKind::ProviderOutput, "abstract_domains_output", &refs)
}

fn materialization_label(materialization: DomainMaterialization) -> &'static str {
    match materialization {
        DomainMaterialization::Full => "full",
        DomainMaterialization::SummaryInputs => "summary_inputs",
    }
}

fn extend_component_parts(parts: &mut Vec<String>, prefix: &str, components: &[InputComponent]) {
    parts.extend(components.iter().map(|component| {
        format!(
            "{prefix}:{}:{:?}:{}",
            component.name, component.status, component.digest
        )
    }));
}

fn body_stable_key_map(db: &impl AnalysisHost) -> std::collections::BTreeMap<MirBodyId, String> {
    db.mir_bodies()
        .iter()
        .map(|body| (body.id, db.resolve_stable_key(body.stable_key).to_string()))
        .collect()
}

fn block_stable_key_map(
    db: &impl AnalysisHost,
) -> std::collections::BTreeMap<BasicBlockId, String> {
    db.cfg_blocks()
        .iter()
        .map(|block| {
            (
                block.id,
                db.resolve_stable_key(block.stable_key).to_string(),
            )
        })
        .collect()
}

fn operation_stable_key_map(db: &impl AnalysisHost) -> std::collections::BTreeMap<MirOpId, String> {
    db.mir_operations()
        .iter()
        .map(|operation| {
            (
                operation.id,
                db.resolve_stable_key(operation.stable_key).to_string(),
            )
        })
        .collect()
}

fn place_stable_key_map(db: &impl AnalysisHost) -> std::collections::BTreeMap<PlaceId, String> {
    db.mir_places()
        .iter()
        .map(|place| {
            (
                place.id,
                db.resolve_stable_key(place.stable_key).to_string(),
            )
        })
        .collect()
}

fn stable_body_key(
    body_keys: &std::collections::BTreeMap<MirBodyId, String>,
    body: MirBodyId,
) -> String {
    body_keys
        .get(&body)
        .cloned()
        .unwrap_or_else(|| "missing-body".to_string())
}
