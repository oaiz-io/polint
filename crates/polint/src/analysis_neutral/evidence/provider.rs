use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug;
use std::iter::Peekable;
use std::sync::Arc;

use super::cache_key::{
    evidence_provider_parameter_digest, evidence_provider_parameter_digest_for_snapshot,
};
use super::facts::{
    EvidenceConfidence, EvidenceEdgeFact, EvidenceEdgeKind, EvidenceExpansion, EvidenceNodeFact,
    EvidenceNodeKind, EvidencePrecision, EvidenceProvenance, EvidenceQueryMode, EvidenceStatus,
    EvidenceValidation,
};
use super::store::EvidenceOutput;

use crate::analysis_api::ProviderManifest;
use crate::analysis_api::{
    CacheStats, Digest, DigestBuilder, DigestKind, InputComponent, InputSnapshot,
    ProviderExecution, ProviderFailureReason, ProviderFailureStage,
};
use crate::analysis_api::{FactFamily, stable_key_from_parts};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::cfg::facts::{CfgPrecision, CfgStatus};
use crate::analysis_neutral::data_flow::facts::{
    DataFlowConfidence, DataFlowEdgeFact, DataFlowEdgeKind, DataFlowNodeFact, DataFlowNodeKind,
    DataFlowPrecision, DataFlowProvenance, DataFlowStatus, DataFlowValidation,
};
use crate::analysis_neutral::ids::{EvidenceEdgeId, EvidenceNodeId};
use crate::internal_core::Diagnostic;

pub const EVIDENCE_PROVIDER_ID: &str = "polint.evidence";

#[derive(Debug, Clone, Default)]
pub struct EvidenceProviderOutput {
    pub diagnostics: Vec<Diagnostic>,
    pub cache_stats: CacheStats,
    pub output_digest: Option<Digest>,
    pub execution: ProviderExecution,
}

#[allow(clippy::too_many_arguments)]
pub fn derive_evidence_with_cache_stats(
    db: &mut impl AnalysisHost,
    input_snapshot: &InputSnapshot,
    manifest: &ProviderManifest,
    semantic_mir_output_digest: Digest,
    cfg_output_digest: Digest,
    calls_output_digest: Digest,
    refined_calls_output_digest: Digest,
    direct_summaries_output_digest: Digest,
    type_value_alias_output_digest: Digest,
    entrypoints_output_digest: Digest,
    extensions_output_digest: Digest,
    data_flow_output_digest: Digest,
) -> EvidenceProviderOutput {
    debug_assert_eq!(manifest.id, EVIDENCE_PROVIDER_ID);
    let mut checkpoint = std::time::Instant::now();
    let mut record = |step| {
        let elapsed = checkpoint.elapsed();
        tracing::debug!(target: "polint::kernel::stage", provider = EVIDENCE_PROVIDER_ID, step, elapsed_ms = elapsed.as_secs_f64() * 1000.0, "provider step");
        checkpoint = std::time::Instant::now();
    };
    let mut output = EvidenceOutput::empty();
    derive_data_flow_evidence(db, &mut output);
    record("data_flow");
    derive_control_dependence_evidence(db, &mut output);
    record("control_dependence");
    let interner = db.stable_key_interner();
    let output = output.normalized(&interner);
    record("normalize");
    let output_digest = evidence_output_digest(
        manifest,
        input_snapshot,
        &semantic_mir_output_digest,
        &cfg_output_digest,
        &calls_output_digest,
        &refined_calls_output_digest,
        &direct_summaries_output_digest,
        &type_value_alias_output_digest,
        &entrypoints_output_digest,
        &extensions_output_digest,
        &data_flow_output_digest,
        &output,
        &interner,
    );
    record("digest");
    let mut cache_stats = CacheStats::default();
    cache_stats.record_recompute();

    let replaced = db.replace_evidence_facts(output);
    record("store_metadata");
    match replaced {
        Ok(()) => EvidenceProviderOutput {
            diagnostics: Vec::new(),
            cache_stats,
            output_digest: Some(output_digest),
            execution: Default::default(),
        },
        Err(error) => EvidenceProviderOutput {
            diagnostics: vec![provider_error_diagnostic(error.to_string())],
            cache_stats,
            output_digest: None,
            execution: ProviderExecution::Failed {
                stage: ProviderFailureStage::Validation,
                reason: ProviderFailureReason::ValidationRejected,
            },
        },
    }
}

fn derive_data_flow_evidence(db: &impl AnalysisHost, output: &mut EvidenceOutput) {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let mut node_map = BTreeMap::new();
    for node in db.data_flow_nodes() {
        let evidence_id = EvidenceNodeId(output.nodes.len() as u64);
        node_map.insert(node.id, evidence_id);
        output
            .nodes
            .push(evidence_node_from_data_flow(interner, node, evidence_id));
    }

    for edge in db.data_flow_edges() {
        let (Some(from), Some(to)) = (node_map.get(&edge.from), node_map.get(&edge.to)) else {
            continue;
        };
        output.edges.push(evidence_edge_from_data_flow(
            interner,
            edge,
            EvidenceEdgeId(output.edges.len() as u64),
            *from,
            *to,
        ));
    }
}

fn evidence_node_from_data_flow(
    interner: &crate::internal_core::StableKeyInterner,
    node: &DataFlowNodeFact,
    id: EvidenceNodeId,
) -> EvidenceNodeFact {
    EvidenceNodeFact {
        id,
        kind: data_flow_node_kind(node),
        language: node.language,
        file: node.file,
        function: node.function,
        body: node.body,
        operation: node.operation,
        cfg_node: node.cfg_node,
        place: node.place,
        symbol: node.symbol,
        reference: node.reference,
        call_site: node.call_site,
        span: node.span.clone(),
        status: EvidenceStatus::Present,
        precision: EvidencePrecision::Syntax,
        provenance: EvidenceProvenance::Native,
        validation: EvidenceValidation::ReferentiallyValidated,
        confidence: EvidenceConfidence::High,
        compact_label: Some(format!("{:?}", node.kind)),
        source_fact_stable_keys: vec![interner.resolve(node.stable_key)],
        stable_key: stable_key_from_parts(
            interner,
            FactFamily::EvidenceNode,
            &[(
                "data_flow_node",
                interner.resolve(node.stable_key).to_string(),
            )],
        ),
    }
}

fn evidence_edge_from_data_flow(
    interner: &crate::internal_core::StableKeyInterner,
    edge: &DataFlowEdgeFact,
    id: EvidenceEdgeId,
    from: EvidenceNodeId,
    to: EvidenceNodeId,
) -> EvidenceEdgeFact {
    let summary_stable_key = summary_source_key(edge);
    EvidenceEdgeFact {
        id,
        from,
        to,
        kind: data_flow_edge_kind(edge.kind),
        query_mode: EvidenceQueryMode::ThinBackward,
        status: data_flow_status(edge.status),
        precision: data_flow_precision(edge.precision),
        provenance: data_flow_provenance(edge.provenance),
        validation: data_flow_validation(edge.validation),
        confidence: data_flow_confidence(edge.confidence),
        call_site: edge.call_site,
        summary_stable_key: summary_stable_key.clone(),
        expansion: evidence_expansion(edge, summary_stable_key.as_deref()),
        compact_label: Some(format!("{:?}", edge.kind)),
        source_fact_stable_keys: std::iter::once(interner.resolve(edge.stable_key))
            .chain(edge.input_stable_keys.iter().cloned())
            .collect(),
        stable_key: stable_key_from_parts(
            interner,
            FactFamily::EvidenceEdge,
            &[(
                "data_flow_edge",
                interner.resolve(edge.stable_key).to_string(),
            )],
        ),
    }
}

fn derive_control_dependence_evidence(db: &impl AnalysisHost, output: &mut EvidenceOutput) {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let mut edges = HashMap::with_capacity(db.cfg_edges().len());
    let mut blocks = HashMap::with_capacity(db.cfg_blocks().len());
    let mut functions = HashMap::with_capacity(db.cfg_functions().len());
    for edge in db.cfg_edges() {
        edges.entry(edge.id).or_insert(edge);
    }
    for block in db.cfg_blocks() {
        blocks.entry(block.id).or_insert(block);
    }
    for function in db.cfg_functions() {
        functions.entry(function.id).or_insert(function);
    }
    for dependence in db.cfg_control_dependence() {
        let Some(controlling_edge) = edges.get(&dependence.controlling_edge) else {
            continue;
        };
        let from = EvidenceNodeId(output.nodes.len() as u64);
        let controlled_node = blocks
            .get(&dependence.controlled_block)
            .and_then(|block| block.first_node.or(block.last_node));
        let function = functions.get(&dependence.cfg_function);
        output.nodes.push(EvidenceNodeFact {
            id: from,
            kind: EvidenceNodeKind::Statement,
            language: function
                .map(|function| function.language)
                .unwrap_or(crate::internal_core::Language::Unknown),
            file: function.map(|function| function.file),
            function: function.map(|function| function.function),
            body: None,
            operation: None,
            cfg_node: Some(controlling_edge.from),
            place: None,
            symbol: None,
            reference: None,
            call_site: None,
            span: None,
            status: cfg_status(dependence.status),
            precision: cfg_precision(dependence.precision),
            provenance: EvidenceProvenance::Native,
            validation: EvidenceValidation::ReferentiallyValidated,
            confidence: EvidenceConfidence::High,
            compact_label: Some(format!("control:{:?}", dependence.controlling_edge_kind)),
            source_fact_stable_keys: vec![
                interner.resolve(dependence.stable_key),
                interner.resolve(controlling_edge.stable_key),
            ],
            stable_key: stable_key_from_parts(
                interner,
                FactFamily::EvidenceNode,
                &[
                    (
                        "control_dependence",
                        interner.resolve(dependence.stable_key).to_string(),
                    ),
                    ("role", "controller".to_string()),
                ],
            ),
        });
        let to = EvidenceNodeId(output.nodes.len() as u64);
        output.nodes.push(EvidenceNodeFact {
            id: to,
            kind: EvidenceNodeKind::Synthetic,
            language: crate::internal_core::Language::Unknown,
            file: None,
            function: None,
            body: None,
            operation: None,
            cfg_node: controlled_node,
            place: None,
            symbol: None,
            reference: None,
            call_site: None,
            span: None,
            status: cfg_status(dependence.status),
            precision: cfg_precision(dependence.precision),
            provenance: EvidenceProvenance::Native,
            validation: EvidenceValidation::ReferentiallyValidated,
            confidence: EvidenceConfidence::High,
            compact_label: Some("controlled_block".to_string()),
            source_fact_stable_keys: vec![interner.resolve(dependence.stable_key)],
            stable_key: stable_key_from_parts(
                interner,
                FactFamily::EvidenceNode,
                &[
                    (
                        "control_dependence",
                        interner.resolve(dependence.stable_key).to_string(),
                    ),
                    ("role", "controlled".to_string()),
                ],
            ),
        });
        output.edges.push(EvidenceEdgeFact {
            id: EvidenceEdgeId(output.edges.len() as u64),
            from,
            to,
            kind: EvidenceEdgeKind::Control,
            query_mode: EvidenceQueryMode::FullBackward,
            status: cfg_status(dependence.status),
            precision: cfg_precision(dependence.precision),
            provenance: EvidenceProvenance::Native,
            validation: EvidenceValidation::ReferentiallyValidated,
            confidence: EvidenceConfidence::High,
            call_site: None,
            summary_stable_key: None,
            expansion: EvidenceExpansion::None,
            compact_label: Some(format!("{:?}", dependence.controlling_edge_kind)),
            source_fact_stable_keys: vec![
                interner.resolve(dependence.stable_key),
                interner.resolve(controlling_edge.stable_key),
            ],
            stable_key: stable_key_from_parts(
                interner,
                FactFamily::EvidenceEdge,
                &[(
                    "control_dependence",
                    interner.resolve(dependence.stable_key).to_string(),
                )],
            ),
        });
    }
}

fn data_flow_node_kind(node: &DataFlowNodeFact) -> EvidenceNodeKind {
    if node.operation.is_some() {
        EvidenceNodeKind::Operation
    } else if node.place.is_some() {
        EvidenceNodeKind::Place
    } else if node.call_site.is_some() {
        EvidenceNodeKind::CallSite
    } else if node.model.is_some()
        || matches!(
            node.kind,
            DataFlowNodeKind::Source
                | DataFlowNodeKind::Sink
                | DataFlowNodeKind::Sanitizer
                | DataFlowNodeKind::Barrier
        )
    {
        EvidenceNodeKind::Model
    } else {
        EvidenceNodeKind::Synthetic
    }
}

fn data_flow_edge_kind(kind: DataFlowEdgeKind) -> EvidenceEdgeKind {
    match kind {
        DataFlowEdgeKind::LocalRead
        | DataFlowEdgeKind::LocalBinding
        | DataFlowEdgeKind::LocalAssignment
        | DataFlowEdgeKind::LocalUse
        | DataFlowEdgeKind::LocalWrite
        | DataFlowEdgeKind::ReturnValue
        | DataFlowEdgeKind::CallArgumentToReturn
        | DataFlowEdgeKind::SourceIntroduction => EvidenceEdgeKind::DataValue,
        DataFlowEdgeKind::CallArgumentToParameter => EvidenceEdgeKind::ParameterIn,
        DataFlowEdgeKind::CallReturnToUse => EvidenceEdgeKind::ParameterOut,
        DataFlowEdgeKind::ReceiverToMethod => EvidenceEdgeKind::Call,
        DataFlowEdgeKind::FieldProjection
        | DataFlowEdgeKind::IndexProjection
        | DataFlowEdgeKind::Dereference
        | DataFlowEdgeKind::AddressOf => EvidenceEdgeKind::DataAddress,
        DataFlowEdgeKind::SummaryTito | DataFlowEdgeKind::SummaryProjected => {
            EvidenceEdgeKind::Summary
        }
        DataFlowEdgeKind::Model => EvidenceEdgeKind::Model,
        DataFlowEdgeKind::Sanitizer | DataFlowEdgeKind::Barrier => {
            EvidenceEdgeKind::ExplanationOnly
        }
        DataFlowEdgeKind::UnknownFlow
        | DataFlowEdgeKind::HavocFlow
        | DataFlowEdgeKind::BudgetTruncated => EvidenceEdgeKind::Unknown,
    }
}

fn summary_source_key(edge: &DataFlowEdgeFact) -> Option<Arc<str>> {
    edge.input_stable_keys
        .iter()
        .find(|key| is_summary_key(key))
        .cloned()
}

fn is_summary_key(key: &str) -> bool {
    key.starts_with("summary:")
        || key.contains("SummaryTito")
        || key.contains("SummaryControl")
        || key.contains("SummaryCall")
        || key.contains("SummaryMemory")
        || key.contains("SummaryEvent")
}

fn evidence_expansion(
    edge: &DataFlowEdgeFact,
    summary_stable_key: Option<&str>,
) -> EvidenceExpansion {
    let Some(summary_stable_key) = summary_stable_key else {
        return EvidenceExpansion::None;
    };
    if edge.status == DataFlowStatus::Present
        && matches!(
            edge.kind,
            DataFlowEdgeKind::SummaryTito | DataFlowEdgeKind::SummaryProjected
        )
    {
        EvidenceExpansion::Expandable {
            key: format!("evidence:expand:{summary_stable_key}"),
        }
    } else if edge.provenance == DataFlowProvenance::Model {
        EvidenceExpansion::ExternalModel {
            model: edge
                .model
                .map(|model| format!("data_flow_model:{}", model.0))
                .unwrap_or_else(|| "external_model".to_string()),
        }
    } else {
        EvidenceExpansion::Opaque {
            reason: summary_opaque_reason(edge),
        }
    }
}

fn summary_opaque_reason(edge: &DataFlowEdgeFact) -> String {
    edge.evidence
        .iter()
        .find_map(|entry| entry.strip_prefix("reason=").map(str::to_string))
        .unwrap_or_else(|| format!("summary_status={:?}", edge.status))
}

fn data_flow_status(status: DataFlowStatus) -> EvidenceStatus {
    match status {
        DataFlowStatus::Present => EvidenceStatus::Present,
        DataFlowStatus::Unknown => EvidenceStatus::Unknown,
        DataFlowStatus::Unsupported => EvidenceStatus::Unsupported,
        DataFlowStatus::SetupMissing => EvidenceStatus::SetupMissing,
        DataFlowStatus::BudgetExceeded => EvidenceStatus::BudgetExceeded,
        DataFlowStatus::Rejected => EvidenceStatus::Rejected,
    }
}

fn data_flow_precision(precision: DataFlowPrecision) -> EvidencePrecision {
    match precision {
        DataFlowPrecision::Exact => EvidencePrecision::Exact,
        DataFlowPrecision::SetupAware => EvidencePrecision::SetupAware,
        DataFlowPrecision::Syntax => EvidencePrecision::Syntax,
        DataFlowPrecision::Conservative => EvidencePrecision::Conservative,
        DataFlowPrecision::Heuristic => EvidencePrecision::Heuristic,
        DataFlowPrecision::Unknown => EvidencePrecision::Unknown,
    }
}

fn data_flow_provenance(provenance: DataFlowProvenance) -> EvidenceProvenance {
    match provenance {
        DataFlowProvenance::Native => EvidenceProvenance::Native,
        DataFlowProvenance::Summary => EvidenceProvenance::Summary,
        DataFlowProvenance::Extension => EvidenceProvenance::Extension,
        DataFlowProvenance::Model => EvidenceProvenance::Model,
        DataFlowProvenance::Query => EvidenceProvenance::Query,
    }
}

fn data_flow_validation(validation: DataFlowValidation) -> EvidenceValidation {
    match validation {
        DataFlowValidation::Native => EvidenceValidation::Native,
        DataFlowValidation::ReferentiallyValidated => EvidenceValidation::ReferentiallyValidated,
        DataFlowValidation::ExtensionValidated => EvidenceValidation::ExtensionValidated,
        DataFlowValidation::BudgetValidated => EvidenceValidation::BudgetValidated,
        DataFlowValidation::Rejected => EvidenceValidation::Rejected,
    }
}

fn data_flow_confidence(confidence: DataFlowConfidence) -> EvidenceConfidence {
    match confidence {
        DataFlowConfidence::High => EvidenceConfidence::High,
        DataFlowConfidence::Medium => EvidenceConfidence::Medium,
        DataFlowConfidence::Low => EvidenceConfidence::Low,
    }
}

fn cfg_status(status: CfgStatus) -> EvidenceStatus {
    match status {
        CfgStatus::Resolved => EvidenceStatus::Present,
        CfgStatus::Partial => EvidenceStatus::Partial,
        CfgStatus::Unknown => EvidenceStatus::Unknown,
        CfgStatus::Unsupported => EvidenceStatus::Unsupported,
    }
}

fn cfg_precision(precision: CfgPrecision) -> EvidencePrecision {
    match precision {
        CfgPrecision::ExactSyntax | CfgPrecision::ExactLowered => EvidencePrecision::Exact,
        CfgPrecision::SetupAware => EvidencePrecision::SetupAware,
        CfgPrecision::Conservative => EvidencePrecision::Conservative,
        CfgPrecision::Heuristic => EvidencePrecision::Heuristic,
        CfgPrecision::Unknown => EvidencePrecision::Unknown,
        CfgPrecision::Unsupported => EvidencePrecision::Unknown,
    }
}

#[allow(clippy::too_many_arguments)]
/// Fact-family part prefixes, in the order a sorted part list puts them.
///
/// The streaming digest below relies on three properties of this list, all
/// asserted by `family_prefixes_partition_the_sorted_order`: it is strictly
/// ascending, no entry is a prefix of another, and no header label starts with
/// any entry.
const EVIDENCE_FAMILY_PREFIXES: [&str; 8] = [
    "evidence_bundle=",
    "evidence_edge=",
    "evidence_node=",
    "evidence_omitted_region=",
    "evidence_path=",
    "evidence_replay_key=",
    "evidence_slice=",
    "evidence_unknown=",
];

/// Every label a header part can carry (the `<label>=` or `<label>:` stem).
const EVIDENCE_HEADER_LABELS: [&str; 20] = [
    "provider_id",
    "provider_version",
    "schema",
    "parameters",
    "input_parameters",
    "semantic_mir",
    "cfg",
    "calls",
    "refined_calls",
    "direct_summaries",
    "type_value_alias",
    "entrypoints",
    "extensions",
    "data_flow",
    "go_lifecycle",
    "ts_js_lifecycle",
    "model",
    "extension",
    "tool",
    "evidence_output",
];

/// Provider-output digest for the evidence layer, hashed in sorted part order.
///
/// The digest is the FNV fold over the run's parts **in sorted order**. Building
/// that order the obvious way — collect every part into one `Vec<String>`, sort,
/// hash — materialises one ~4 KB payload per evidence fact simultaneously. On
/// excalidraw that is the single largest allocation in the whole pipeline:
/// measured at **+2.3 GB of peak RSS** for this one call, more than the facts it
/// describes.
///
/// Every fact family's
/// parts share a `<family>=` prefix, and no family name is a prefix of another,
/// so in sorted order the families form contiguous blocks ordered by prefix.
/// A header part (`provider_id=…`, `cfg=…`, `extensions=…`, …) never starts with
/// a family prefix, so it sorts before *every* part of a family exactly when it
/// sorts before that family's prefix. Emitting the (few, small) header parts and
/// the family blocks in that merged order therefore reproduces `parts.sort()`
/// exactly. Nodes and edges have their integer ID first in the debug payload,
/// so their order can be established without expanding the rest of each row.
/// Only one expanded node or edge payload is live at a time; smaller families
/// retain the general full-payload sort.
#[allow(clippy::too_many_arguments)]
fn evidence_output_digest(
    manifest: &ProviderManifest,
    input_snapshot: &InputSnapshot,
    semantic_mir_output_digest: &Digest,
    cfg_output_digest: &Digest,
    calls_output_digest: &Digest,
    refined_calls_output_digest: &Digest,
    direct_summaries_output_digest: &Digest,
    type_value_alias_output_digest: &Digest,
    entrypoints_output_digest: &Digest,
    extensions_output_digest: &Digest,
    data_flow_output_digest: &Digest,
    output: &EvidenceOutput,
    interner: &crate::internal_core::StableKeyInterner,
) -> Digest {
    let upstream = vec![
        semantic_mir_output_digest.clone(),
        cfg_output_digest.clone(),
        calls_output_digest.clone(),
        refined_calls_output_digest.clone(),
        direct_summaries_output_digest.clone(),
        type_value_alias_output_digest.clone(),
        entrypoints_output_digest.clone(),
        extensions_output_digest.clone(),
        data_flow_output_digest.clone(),
    ];
    let mut header = vec![
        format!("provider_id={}", manifest.id),
        format!("provider_version={}", manifest.provider_version()),
        format!("schema={}", manifest.primary_schema_label()),
        format!("parameters={}", evidence_provider_parameter_digest()),
        format!(
            "input_parameters={}",
            evidence_provider_parameter_digest_for_snapshot(input_snapshot, &upstream)
        ),
        format!("semantic_mir={semantic_mir_output_digest}"),
        format!("cfg={cfg_output_digest}"),
        format!("calls={calls_output_digest}"),
        format!("refined_calls={refined_calls_output_digest}"),
        format!("direct_summaries={direct_summaries_output_digest}"),
        format!("type_value_alias={type_value_alias_output_digest}"),
        format!("entrypoints={entrypoints_output_digest}"),
        format!("extensions={extensions_output_digest}"),
        format!("data_flow={data_flow_output_digest}"),
    ];
    extend_component_parts(
        &mut header,
        "go_lifecycle",
        &input_snapshot.go_lifecycle.components,
    );
    extend_component_parts(
        &mut header,
        "ts_js_lifecycle",
        &input_snapshot.ts_js_lifecycle.components,
    );
    extend_component_parts(&mut header, "model", &input_snapshot.models);
    extend_component_parts(&mut header, "extension", &input_snapshot.extensions);
    extend_component_parts(&mut header, "tool", &input_snapshot.tool_invocations);
    if output.nodes.is_empty()
        && output.edges.is_empty()
        && output.bundles.is_empty()
        && output.paths.is_empty()
        && output.slices.is_empty()
        && output.unknowns.is_empty()
        && output.omitted_regions.is_empty()
        && output.replay_keys.is_empty()
    {
        header.push("evidence_output=empty".to_string());
    }
    header.sort();

    let mut digest = Digest::builder(DigestKind::ProviderOutput, "evidence_output");
    let mut header = header.into_iter().peekable();
    // Families in ascending prefix order — the order `parts.sort()` produced.
    emit_family(
        &mut digest,
        &mut header,
        EVIDENCE_FAMILY_PREFIXES[0],
        family_parts(interner, EVIDENCE_FAMILY_PREFIXES[0], &output.bundles),
    );
    emit_family(
        &mut digest,
        &mut header,
        EVIDENCE_FAMILY_PREFIXES[1],
        indexed_family_parts(
            interner,
            EVIDENCE_FAMILY_PREFIXES[1],
            &output.edges,
            |fact| fact.id.0,
        ),
    );
    emit_family(
        &mut digest,
        &mut header,
        EVIDENCE_FAMILY_PREFIXES[2],
        indexed_family_parts(
            interner,
            EVIDENCE_FAMILY_PREFIXES[2],
            &output.nodes,
            |fact| fact.id.0,
        ),
    );
    emit_family(
        &mut digest,
        &mut header,
        EVIDENCE_FAMILY_PREFIXES[3],
        family_parts(
            interner,
            EVIDENCE_FAMILY_PREFIXES[3],
            &output.omitted_regions,
        ),
    );
    emit_family(
        &mut digest,
        &mut header,
        EVIDENCE_FAMILY_PREFIXES[4],
        family_parts(interner, EVIDENCE_FAMILY_PREFIXES[4], &output.paths),
    );
    emit_family(
        &mut digest,
        &mut header,
        EVIDENCE_FAMILY_PREFIXES[5],
        family_parts(interner, EVIDENCE_FAMILY_PREFIXES[5], &output.replay_keys),
    );
    emit_family(
        &mut digest,
        &mut header,
        EVIDENCE_FAMILY_PREFIXES[6],
        family_parts(interner, EVIDENCE_FAMILY_PREFIXES[6], &output.slices),
    );
    emit_family(
        &mut digest,
        &mut header,
        EVIDENCE_FAMILY_PREFIXES[7],
        family_parts(interner, EVIDENCE_FAMILY_PREFIXES[7], &output.unknowns),
    );
    for part in header {
        digest.part(&part);
    }
    digest.finish()
}

/// `<prefix><payload>` for one fact family, materialised on its own.
///
/// Taking the same `prefix` the emission order is keyed on keeps the two in
/// lockstep: a family cannot be hashed under one label and ordered under
/// another.
fn family_parts<T: Debug>(
    interner: &crate::internal_core::StableKeyInterner,
    prefix: &str,
    facts: &[T],
) -> Vec<String> {
    let mut parts = facts
        .iter()
        .map(|fact| format!("{prefix}{}", stable_fact_payload(interner, fact)))
        .collect::<Vec<_>>();
    parts.sort();
    parts
}

fn emit_family(
    digest: &mut DigestBuilder,
    header: &mut Peekable<std::vec::IntoIter<String>>,
    prefix: &str,
    parts: impl IntoIterator<Item = String>,
) {
    while header.peek().is_some_and(|part| part.as_str() < prefix) {
        digest.part(&header.next().expect("peeked part is present"));
    }
    for part in parts {
        digest.part(&part);
    }
}

/// These row types start their Debug payload with an integer ID. Decimal IDs
/// order the payloads because the closing ')' sorts before every digit. Keep
/// references in that order and materialize only the current row. Equal IDs
/// still compare their full payload, preserving the order of unnormalized input.
fn indexed_family_parts<'a, T: Debug>(
    interner: &'a crate::internal_core::StableKeyInterner,
    prefix: &'a str,
    facts: &'a [T],
    id: impl Fn(&T) -> u64,
) -> impl Iterator<Item = String> + 'a {
    let mut indexed = facts
        .iter()
        .map(|fact| (id(fact).to_string(), fact))
        .collect::<Vec<_>>();
    indexed.sort_by(|(left_id, left), (right_id, right)| {
        left_id.cmp(right_id).then_with(|| {
            stable_fact_payload(interner, left).cmp(&stable_fact_payload(interner, right))
        })
    });
    indexed
        .into_iter()
        .map(move |(_, fact)| format!("{prefix}{}", stable_fact_payload(interner, fact)))
}

fn extend_component_parts(parts: &mut Vec<String>, prefix: &str, components: &[InputComponent]) {
    if components.is_empty() {
        parts.push(format!("{prefix}=absent"));
        return;
    }
    parts.extend(components.iter().map(|component| {
        format!(
            "{prefix}:{}:{:?}:{}",
            component.name, component.status, component.digest
        )
    }));
}

fn stable_fact_payload<T>(interner: &crate::internal_core::StableKeyInterner, fact: &T) -> String
where
    T: Debug,
{
    resolve_stable_key_ids(interner, &format!("{fact:?}"))
}

fn resolve_stable_key_ids(
    interner: &crate::internal_core::StableKeyInterner,
    payload: &str,
) -> String {
    let mut resolved = String::with_capacity(payload.len());
    let mut remaining = payload;
    while let Some(start) = remaining.find("StableKeyId(") {
        resolved.push_str(&remaining[..start]);
        let id_start = start + "StableKeyId(".len();
        let Some(relative_end) = remaining[id_start..].find(')') else {
            resolved.push_str(&remaining[start..]);
            return resolved;
        };
        let id_end = id_start + relative_end;
        let Ok(id) = remaining[id_start..id_end].parse::<u32>() else {
            resolved.push_str(&remaining[start..=id_end]);
            remaining = &remaining[id_end + 1..];
            continue;
        };
        resolved.push_str(&format!(
            "{:?}",
            interner.resolve(crate::internal_core::StableKeyId(id))
        ));
        remaining = &remaining[id_end + 1..];
    }
    resolved.push_str(remaining);
    resolved
}

fn provider_error_diagnostic(message: String) -> Diagnostic {
    Diagnostic::error(
        "polint/internal",
        "<workspace>",
        crate::internal_core::DiagnosticRange::point(1, 1),
        format!("Evidence provider failed: {message}"),
    )
}

#[cfg(test)]
mod digest_order_tests {
    use super::*;

    #[test]
    fn id_ordered_payloads_match_full_sort_for_sparse_and_repeated_ids() {
        let interner = crate::internal_core::StableKeyInterner::default();
        let stable_key = interner.intern("source: unicode λ and escaped \"quote\"\\newline\n");
        let ids = [100, 1, 10, 2, 0, 99, 9, 10, 11, u64::MAX, 101, 1];
        let nodes = ids
            .iter()
            .enumerate()
            .map(|(index, &id)| EvidenceNodeFact {
                id: EvidenceNodeId(id),
                kind: EvidenceNodeKind::Statement,
                language: crate::internal_core::Language::TypeScript,
                file: None,
                function: None,
                body: None,
                operation: None,
                cfg_node: None,
                place: None,
                symbol: None,
                reference: None,
                call_site: None,
                span: None,
                status: EvidenceStatus::Present,
                precision: EvidencePrecision::Syntax,
                provenance: EvidenceProvenance::Native,
                validation: EvidenceValidation::ReferentiallyValidated,
                confidence: EvidenceConfidence::High,
                compact_label: Some(format!("label:{}", ids.len() - index)),
                source_fact_stable_keys: vec![interner.resolve(stable_key)],
                stable_key,
            })
            .collect::<Vec<_>>();
        let edges = nodes
            .iter()
            .map(|node| EvidenceEdgeFact {
                id: EvidenceEdgeId(node.id.0),
                from: EvidenceNodeId(10),
                to: EvidenceNodeId(2),
                kind: EvidenceEdgeKind::DataValue,
                query_mode: EvidenceQueryMode::ThinBackward,
                status: node.status,
                precision: node.precision,
                provenance: node.provenance,
                validation: node.validation,
                confidence: node.confidence,
                call_site: None,
                summary_stable_key: Some(interner.resolve(stable_key)),
                expansion: EvidenceExpansion::None,
                compact_label: node.compact_label.clone(),
                source_fact_stable_keys: node.source_fact_stable_keys.clone(),
                stable_key,
            })
            .collect::<Vec<_>>();
        for count in 0..=ids.len() {
            assert_eq!(
                indexed_family_parts(&interner, "evidence_node=", &nodes[..count], |node| node
                    .id
                    .0)
                .collect::<Vec<_>>(),
                family_parts(&interner, "evidence_node=", &nodes[..count]),
            );
            assert_eq!(
                indexed_family_parts(&interner, "evidence_edge=", &edges[..count], |edge| edge
                    .id
                    .0)
                .collect::<Vec<_>>(),
                family_parts(&interner, "evidence_edge=", &edges[..count]),
            );
        }
    }

    /// The streaming digest emits header parts and family blocks in a merged
    /// order and claims it equals sorting every part together. That holds only
    /// while the three properties below do, so assert them rather than trust
    /// them: a new fact family or a new header label must not silently change
    /// the digest of every evidence layer ever produced.
    #[test]
    fn family_prefixes_partition_the_sorted_order() {
        for window in EVIDENCE_FAMILY_PREFIXES.windows(2) {
            assert!(window[0] < window[1], "prefixes must ascend: {window:?}");
            assert!(
                !window[1].starts_with(window[0]),
                "no family prefix may prefix another: {window:?}"
            );
        }

        for label in EVIDENCE_HEADER_LABELS {
            for separator in ['=', ':'] {
                let header = format!("{label}{separator}value");
                for prefix in EVIDENCE_FAMILY_PREFIXES {
                    assert!(
                        !header.starts_with(prefix),
                        "header `{header}` must not fall inside family `{prefix}`"
                    );
                    // A header sorts before every part of a family exactly when
                    // it sorts before that family's prefix — the merge rule.
                    for payload in ["", "\u{0}", "zzz", "~"] {
                        let part = format!("{prefix}{payload}");
                        assert_eq!(
                            header.as_str() < prefix,
                            header < part,
                            "header `{header}` straddles family part `{part}`"
                        );
                    }
                }
            }
        }
    }
}
