use std::fmt::Debug;

use super::cache_key::{
    data_flow_provider_parameter_digest, data_flow_provider_parameter_digest_for_snapshot,
};
use super::facts::{
    DataFlowAlgorithm, DataFlowConfidence, DataFlowEdgeFact, DataFlowEdgeKind, DataFlowModelFact,
    DataFlowModelKind, DataFlowNodeFact, DataFlowNodeKind, DataFlowPrecision, DataFlowProvenance,
    DataFlowStatus, DataFlowValidation,
};
use super::store::{
    DataFlowOutput, next_data_flow_edge_id, next_data_flow_model_id, next_data_flow_node_id,
};
use crate::analysis_api::{
    CacheStats, Digest, DigestKind, InputComponent, InputSnapshot, ProviderExecution,
    ProviderFailureReason, ProviderFailureStage, stable_key_from_key_parts,
};
use crate::analysis_api::{FactFamily, ProviderManifest, stable_key_from_parts};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::entrypoints::facts::TrustBoundaryFact;
use crate::analysis_neutral::ids::{DataFlowModelId, DataFlowNodeId};
use crate::analysis_neutral::places::{PlaceFact, PlaceRoot};
use crate::internal_core::{Diagnostic, KeyPart};

pub const DATA_FLOW_PROVIDER_ID: &str = "polint.data_flow";

#[derive(Debug, Clone, Default)]
pub struct DataFlowProviderOutput {
    pub diagnostics: Vec<Diagnostic>,
    pub cache_stats: CacheStats,
    pub output_digest: Option<Digest>,
    pub execution: ProviderExecution,
}

#[allow(clippy::too_many_arguments)]
pub fn derive_data_flow_with_cache_stats(
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
) -> DataFlowProviderOutput {
    let mut started = std::time::Instant::now();
    let mut checkpoint = |step: &'static str| {
        tracing::debug!(target: "polint::kernel::stage", provider = DATA_FLOW_PROVIDER_ID, step, elapsed_ms = started.elapsed().as_millis() as u64, "provider step");
        started = std::time::Instant::now();
    };
    debug_assert_eq!(manifest.id, DATA_FLOW_PROVIDER_ID);
    let mut output = DataFlowOutput::empty();
    derive_local_place_nodes(db, &mut output);
    checkpoint("place_nodes");
    super::local::derive_local_value_flow(db, &mut output);
    checkpoint("local_flow");
    super::direct_calls::derive_direct_call_edges(db, &mut output);
    checkpoint("direct_calls");
    super::summary_edges::derive_summary_projected_edges(db, &mut output);
    checkpoint("summary_edges");
    derive_source_models(db, &mut output);
    derive_extension_models(db, &mut output);
    checkpoint("models");
    let interner = db.stable_key_interner();
    output = output.normalized(&interner);
    checkpoint("normalize");

    let output_digest = data_flow_output_digest(
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
        &output,
        &interner,
    );
    checkpoint("digest");
    let mut cache_stats = CacheStats::default();
    cache_stats.record_recompute();

    match db.replace_data_flow_facts(output) {
        Ok(()) => {
            checkpoint("store_metadata");
            DataFlowProviderOutput {
                diagnostics: Vec::new(),
                cache_stats,
                output_digest: Some(output_digest),
                execution: Default::default(),
            }
        }
        Err(error) => DataFlowProviderOutput {
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

fn derive_local_place_nodes(db: &impl AnalysisHost, output: &mut DataFlowOutput) {
    for place in db.mir_places() {
        output
            .nodes
            .push(super::local::node_from_place(output, place, db));
    }
}

fn derive_source_models(db: &impl AnalysisHost, output: &mut DataFlowOutput) {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    for boundary in db.trust_boundary_facts() {
        let model_id = next_data_flow_model_id(&output.models);
        let stable_key = stable_key_from_key_parts(
            interner,
            FactFamily::DataFlowModel,
            [
                ("kind", KeyPart::Text("source")),
                ("trust_boundary", KeyPart::Key(boundary.stable_key)),
            ],
        );
        output.models.push(DataFlowModelFact {
            id: model_id,
            kind: DataFlowModelKind::Source,
            language: boundary.language,
            provider_id: boundary.provider_id.clone(),
            model_id: Some(format!("{:?}", boundary.source_kind)),
            source_stable_key: Some(interner.resolve(boundary.stable_key).to_string()),
            status: DataFlowStatus::Present,
            precision: DataFlowPrecision::SetupAware,
            validation: DataFlowValidation::ReferentiallyValidated,
            confidence: DataFlowConfidence::High,
            provenance: DataFlowProvenance::Native,
            evidence: vec!["trust_boundary".to_string()],
            payload_labels: vec![
                format!("source_kind={:?}", boundary.source_kind),
                boundary.access_path.clone().unwrap_or_default(),
            ],
            stable_key,
        });
        let source_node = next_data_flow_node_id(&output.nodes);
        output.nodes.push(DataFlowNodeFact {
            id: source_node,
            kind: DataFlowNodeKind::Source,
            language: boundary.language,
            file: Some(boundary.file),
            function: boundary.target_parameter,
            body: None,
            operation: None,
            cfg_node: None,
            place: None,
            symbol: None,
            reference: None,
            call_site: None,
            model: Some(model_id),
            span: Some(boundary.span.clone()),
            stable_key: stable_key_from_key_parts(
                interner,
                FactFamily::DataFlowNode,
                [("source_model", KeyPart::Key(stable_key))],
            ),
        });
        derive_source_introduction_edges(db, output, boundary, source_node, model_id);
    }
}

fn derive_source_introduction_edges(
    db: &impl AnalysisHost,
    output: &mut DataFlowOutput,
    boundary: &TrustBoundaryFact,
    source_node: DataFlowNodeId,
    model_id: DataFlowModelId,
) {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let Some(target_function) = boundary.target_parameter else {
        return;
    };
    let targets = db
        .mir_places()
        .iter()
        .filter(|place| parameter_matches_boundary(place, target_function, boundary))
        .filter_map(|place| {
            let node = output
                .nodes
                .iter()
                .find(|node| node.place == Some(place.id))
                .map(|node| node.id)?;
            Some((place, node))
        })
        .collect::<Vec<_>>();

    for (place, target_node) in targets {
        push_source_introduction_edge(
            interner,
            output,
            boundary,
            place,
            source_node,
            target_node,
            model_id,
        );
    }
}

fn parameter_matches_boundary(
    place: &PlaceFact,
    target_function: crate::internal_core::FunctionId,
    boundary: &TrustBoundaryFact,
) -> bool {
    let PlaceRoot::Parameter {
        function, index, ..
    } = &place.root
    else {
        return false;
    };
    if *function != target_function {
        return false;
    }
    match boundary.target_parameter_index {
        Some(target_index) => *index as usize == target_index,
        None => true,
    }
}

fn push_source_introduction_edge(
    interner: &crate::internal_core::StableKeyInterner,
    output: &mut DataFlowOutput,
    boundary: &TrustBoundaryFact,
    place: &PlaceFact,
    source_node: DataFlowNodeId,
    target_node: DataFlowNodeId,
    model_id: DataFlowModelId,
) {
    let stable_key = stable_key_from_key_parts(
        interner,
        FactFamily::DataFlowEdge,
        [
            ("kind", KeyPart::Text("SourceIntroduction")),
            ("trust_boundary", KeyPart::Key(boundary.stable_key)),
            ("place", KeyPart::Key(place.stable_key)),
        ],
    );
    if output
        .edges
        .iter()
        .any(|edge| edge.stable_key == stable_key)
    {
        return;
    }
    output.edges.push(DataFlowEdgeFact {
        id: next_data_flow_edge_id(&output.edges),
        from: source_node,
        to: target_node,
        kind: DataFlowEdgeKind::SourceIntroduction,
        algorithm: DataFlowAlgorithm::ExtensionModel,
        status: if boundary.target_parameter_index.is_some() {
            DataFlowStatus::Present
        } else {
            DataFlowStatus::Unknown
        },
        precision: if boundary.target_parameter_index.is_some() {
            DataFlowPrecision::SetupAware
        } else {
            DataFlowPrecision::Unknown
        },
        validation: DataFlowValidation::ReferentiallyValidated,
        confidence: if boundary.target_parameter_index.is_some() {
            DataFlowConfidence::High
        } else {
            DataFlowConfidence::Low
        },
        provenance: DataFlowProvenance::Native,
        call_site: None,
        call_target: None,
        refined_call: None,
        model: Some(model_id),
        budget: None,
        evidence: vec![
            "trust_boundary_source_introduction".to_string(),
            format!("source_kind={:?}", boundary.source_kind),
            boundary
                .target_parameter_index
                .map(|index| format!("target_parameter_index={index}"))
                .unwrap_or_else(|| "target_parameter_index=unknown".to_string()),
        ],
        input_stable_keys: vec![
            interner.resolve(boundary.stable_key),
            interner.resolve(place.stable_key),
        ],
        stable_key,
    });
}

fn derive_extension_models(db: &impl AnalysisHost, output: &mut DataFlowOutput) {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    for fact in db.extension_facts() {
        let kind = match fact.fact_family.as_str() {
            "data_flow.source" | "source" => DataFlowModelKind::Source,
            "data_flow.sink" | "sink" => DataFlowModelKind::Sink,
            "data_flow.sanitizer" | "sanitizer" => DataFlowModelKind::Sanitizer,
            "data_flow.barrier" | "barrier" => DataFlowModelKind::Barrier,
            "data_flow.tito" | "tito" => DataFlowModelKind::Tito,
            _ => continue,
        };
        output.models.push(DataFlowModelFact {
            id: next_data_flow_model_id(&output.models),
            kind,
            language: crate::internal_core::Language::Unknown,
            provider_id: fact.provider_id.clone(),
            model_id: Some(fact.extension_id.clone()),
            source_stable_key: Some(interner.resolve(fact.stable_key).to_string()),
            status: DataFlowStatus::Present,
            precision: extension_precision(fact.precision),
            validation: DataFlowValidation::ExtensionValidated,
            confidence: extension_confidence(fact.confidence),
            provenance: DataFlowProvenance::Extension,
            evidence: fact.evidence.clone(),
            payload_labels: fact.payload_labels.clone(),
            stable_key: stable_key_from_key_parts(
                interner,
                FactFamily::DataFlowModel,
                [
                    ("kind", KeyPart::Text(&format!("{kind:?}"))),
                    ("extension_fact", KeyPart::Key(fact.stable_key)),
                ],
            ),
        });
    }
}

fn extension_precision(
    precision: crate::analysis_neutral::extensions::sinks::ExtensionFactPrecision,
) -> DataFlowPrecision {
    match precision {
        crate::analysis_neutral::extensions::sinks::ExtensionFactPrecision::Exact => DataFlowPrecision::Exact,
        crate::analysis_neutral::extensions::sinks::ExtensionFactPrecision::SetupAware => {
            DataFlowPrecision::SetupAware
        }
        crate::analysis_neutral::extensions::sinks::ExtensionFactPrecision::Heuristic => DataFlowPrecision::Heuristic,
        crate::analysis_neutral::extensions::sinks::ExtensionFactPrecision::GeneratedUnvalidated => {
            DataFlowPrecision::Heuristic
        }
    }
}

fn extension_confidence(
    confidence: crate::analysis_neutral::extensions::sinks::ExtensionFactConfidence,
) -> DataFlowConfidence {
    match confidence {
        crate::analysis_neutral::extensions::sinks::ExtensionFactConfidence::High => {
            DataFlowConfidence::High
        }
        crate::analysis_neutral::extensions::sinks::ExtensionFactConfidence::Medium => {
            DataFlowConfidence::Medium
        }
        crate::analysis_neutral::extensions::sinks::ExtensionFactConfidence::Low => {
            DataFlowConfidence::Low
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn data_flow_output_digest(
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
    output: &DataFlowOutput,
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
    ];
    let mut parts = vec![
        format!("provider_id={}", manifest.id),
        format!("provider_version={}", manifest.provider_version()),
        format!("schema={}", manifest.primary_schema_label()),
        format!("parameters={}", data_flow_provider_parameter_digest()),
        format!(
            "input_parameters={}",
            data_flow_provider_parameter_digest_for_snapshot(input_snapshot, &upstream)
        ),
        format!("semantic_mir={semantic_mir_output_digest}"),
        format!("cfg={cfg_output_digest}"),
        format!("calls={calls_output_digest}"),
        format!("refined_calls={refined_calls_output_digest}"),
        format!("direct_summaries={direct_summaries_output_digest}"),
        format!("type_value_alias={type_value_alias_output_digest}"),
        format!("entrypoints={entrypoints_output_digest}"),
        format!("extensions={extensions_output_digest}"),
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
    parts.extend(output.nodes.iter().map(|node| {
        format!(
            "data_flow_node={}",
            stable_fact_payload(interner, node.stable_key, node)
        )
    }));
    parts.extend(output.edges.iter().map(|edge| {
        format!(
            "data_flow_edge={}",
            stable_fact_payload(interner, edge.stable_key, edge)
        )
    }));
    parts.extend(output.models.iter().map(|model| {
        format!(
            "data_flow_model={}",
            stable_fact_payload(interner, model.stable_key, model)
        )
    }));
    parts.extend(output.budgets.iter().map(|budget| {
        format!(
            "data_flow_budget={}",
            stable_fact_payload(interner, budget.stable_key, budget)
        )
    }));
    if output.nodes.is_empty() && output.edges.is_empty() && output.models.is_empty() {
        parts.push("data_flow_output=empty".to_string());
    }
    parts.sort();
    let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    Digest::from_parts(DigestKind::ProviderOutput, "data_flow_output", &refs)
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

fn stable_fact_payload<T>(
    interner: &crate::internal_core::StableKeyInterner,
    _stable_key: crate::internal_core::StableKeyId,
    fact: &T,
) -> String
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
        format!("Data-flow provider failed: {message}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_api::FunctionFact;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::entrypoints::facts::{
        EntrypointConfidence, EntrypointFact, EntrypointKind, EntrypointPrecision,
        EntrypointProvenance, EntrypointStatus, TriggerMetadata, TrustBoundarySourceKind,
    };
    use crate::analysis_neutral::entrypoints::store::EntrypointOutput;
    use crate::analysis_neutral::ids::{EntrypointId, MirBodyId, PlaceId, TrustBoundaryId};
    use crate::analysis_neutral::mir_body::{MirBody, MirOutput, MirStatus};
    use crate::analysis_neutral::places::{PlaceProjection, PlaceStatus};
    use crate::internal_core::{FileId, FunctionId, Language, Span};
    use std::path::PathBuf;

    #[test]
    fn source_models_create_source_introduction_edges_to_matching_parameters() {
        let mut db = LocalAnalysisDb::new();
        let interner = db.stable_key_interner();
        let file = db.add_file(
            PathBuf::from("src/main.ts"),
            "src/main.ts".to_string(),
            "export function handler(req: Request) {}\n".to_string(),
        );
        let function = db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "handler".to_string(),
            Span::point(file, 1, 1),
            Language::TypeScript,
            false,
            true,
            1,
            Vec::new(),
        ));
        db.replace_semantic_mir(MirOutput {
            bodies: vec![mir_body(&interner, file, function)],
            places: vec![parameter_place(file, function)],
            operations: Vec::new(),
            unsupported: Vec::new(),
            ..MirOutput::default()
        })
        .expect("valid MIR");
        db.replace_entrypoint_facts(EntrypointOutput {
            entrypoints: vec![entrypoint(file, function)],
            trust_boundaries: vec![trust_boundary(file, function)],
            dispatch_edges: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("valid entrypoint facts");
        let mut output = DataFlowOutput::empty();
        derive_local_place_nodes(&db, &mut output);

        derive_source_models(&db, &mut output);

        assert!(output.nodes.iter().any(|node| {
            node.kind == DataFlowNodeKind::Source && node.model == Some(DataFlowModelId(0))
        }));
        let edge = output
            .edges
            .iter()
            .find(|edge| edge.kind == DataFlowEdgeKind::SourceIntroduction)
            .unwrap_or_else(|| panic!("missing source introduction edge: {output:#?}"));
        assert_eq!(edge.status, DataFlowStatus::Present);
        assert_eq!(edge.model, Some(DataFlowModelId(0)));
        assert!(
            edge.evidence
                .iter()
                .any(|value| value == "source_kind=QueryString")
        );
    }

    #[test]
    fn source_models_downgrade_unknown_parameter_index_edges() {
        let mut db = LocalAnalysisDb::new();
        let interner = db.stable_key_interner();
        let file = db.add_file(
            PathBuf::from("src/main.ts"),
            "src/main.ts".to_string(),
            "export function handler(req: Request, res: Response) {}\n".to_string(),
        );
        let function = db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "handler".to_string(),
            Span::point(file, 1, 1),
            Language::TypeScript,
            false,
            true,
            1,
            Vec::new(),
        ));
        db.replace_semantic_mir(MirOutput {
            bodies: vec![mir_body(&interner, file, function)],
            places: vec![
                parameter_place_with_index(&interner, file, function, 0, "req"),
                parameter_place_with_index(&interner, file, function, 1, "res"),
            ],
            operations: Vec::new(),
            unsupported: Vec::new(),
            ..MirOutput::default()
        })
        .expect("valid MIR");
        let mut boundary = trust_boundary(file, function);
        boundary.target_parameter_index = None;
        db.replace_entrypoint_facts(EntrypointOutput {
            entrypoints: vec![entrypoint(file, function)],
            trust_boundaries: vec![boundary],
            dispatch_edges: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("valid entrypoint facts");
        let mut output = DataFlowOutput::empty();
        derive_local_place_nodes(&db, &mut output);

        derive_source_models(&db, &mut output);

        let source_edges = output
            .edges
            .iter()
            .filter(|edge| edge.kind == DataFlowEdgeKind::SourceIntroduction)
            .collect::<Vec<_>>();
        assert_eq!(source_edges.len(), 2);
        assert!(source_edges.iter().all(|edge| {
            edge.status == DataFlowStatus::Unknown
                && edge.precision == DataFlowPrecision::Unknown
                && edge.confidence == DataFlowConfidence::Low
                && edge
                    .evidence
                    .iter()
                    .any(|value| value == "target_parameter_index=unknown")
        }));
    }

    fn mir_body(
        interner: &crate::internal_core::StableKeyInterner,
        file: FileId,
        function: FunctionId,
    ) -> MirBody {
        MirBody {
            id: MirBodyId(0),
            language: Language::TypeScript,
            file,
            function,
            package: None,
            module: None,
            owner_stable_key: interner.intern("function:handler".to_string()),
            span: Span::point(file, 1, 1),
            stable_key: interner.intern("body:handler".to_string()),
            status: MirStatus::Resolved,
        }
    }

    fn parameter_place(file: FileId, function: FunctionId) -> PlaceFact {
        let interner = crate::internal_core::StableKeyInterner::default();
        parameter_place_with_index(&interner, file, function, 0, "req")
    }

    fn parameter_place_with_index(
        interner: &crate::internal_core::StableKeyInterner,
        file: FileId,
        function: FunctionId,
        index: u32,
        name: &str,
    ) -> PlaceFact {
        PlaceFact {
            id: PlaceId(index as u64),
            language: Language::TypeScript,
            file: Some(file),
            function: Some(function),
            root: PlaceRoot::Parameter {
                function,
                index,
                name: Some(name.to_string()),
            },
            projections: Vec::<PlaceProjection>::new(),
            stable_key: interner.intern(format!("place:{name}")),
            status: PlaceStatus::Resolved,
        }
    }

    fn entrypoint(file: FileId, function: FunctionId) -> EntrypointFact {
        EntrypointFact {
            id: EntrypointId(0),
            language: Language::TypeScript,
            framework_id: "express".to_string(),
            kind: EntrypointKind::HttpRoute,
            target_function: function,
            target_symbol: None,
            registration_span: Span::point(file, 1, 1),
            registration_file: file,
            trigger_metadata: TriggerMetadata::empty(),
            trust_boundary_link: None,
            precision: EntrypointPrecision::ResolvedStatic,
            provenance: EntrypointProvenance::NativeRecognizer,
            confidence: EntrypointConfidence::High,
            status: EntrypointStatus::Resolved,
            provider_id: "polint.entrypoints".to_string(),
            stable_key: crate::internal_core::stable_key_for_test("entrypoint:handler"),
        }
    }

    fn trust_boundary(file: FileId, function: FunctionId) -> TrustBoundaryFact {
        TrustBoundaryFact {
            id: TrustBoundaryId(0),
            entrypoint_stable_key: crate::internal_core::stable_key_for_test("entrypoint:handler"),
            source_kind: TrustBoundarySourceKind::QueryString,
            target_parameter: Some(function),
            target_parameter_index: Some(0),
            access_path: None,
            protocol: None,
            language: Language::TypeScript,
            file,
            span: Span::point(file, 1, 1),
            precision: EntrypointPrecision::ResolvedStatic,
            provider_id: "polint.entrypoints".to_string(),
            stable_key: crate::internal_core::stable_key_for_test("trust-boundary:query"),
        }
    }
}
