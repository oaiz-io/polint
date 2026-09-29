use serde_json::{Value, json};

use crate::analysis_neutral::evidence::facts::EvidenceExpansion;
use crate::analysis_neutral::evidence::store::EvidenceStore;
use crate::analysis_neutral::ids::EvidenceBundleId;
use crate::internal_core::{DiagnosticRange, Evidence};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvidenceRenderLimit {
    pub max_paths: usize,
    pub max_edges_per_path: usize,
    pub max_unknowns: usize,
    pub max_omitted_regions: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceSarifStep {
    pub message: String,
    pub location: Option<EvidenceSarifLocation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceSarifLocation {
    pub uri: String,
    pub range: DiagnosticRange,
}

impl Default for EvidenceRenderLimit {
    fn default() -> Self {
        Self {
            max_paths: 5,
            max_edges_per_path: 96,
            max_unknowns: 32,
            max_omitted_regions: 32,
        }
    }
}

pub fn render_bundle_json_v1(
    store: &EvidenceStore,
    interner: &crate::internal_core::StableKeyInterner,
    bundle_id: EvidenceBundleId,
    limit: EvidenceRenderLimit,
) -> Option<Value> {
    let bundle = store
        .bundles()
        .iter()
        .find(|bundle| bundle.id == bundle_id)?;
    let mut paths = bundle
        .selected_paths
        .iter()
        .filter_map(|path_id| store.paths().iter().find(|path| path.id == *path_id))
        .collect::<Vec<_>>();
    paths.sort_by(|left, right| {
        left.rank
            .cmp(&right.rank)
            .then_with(|| interner.compare_canonical(left.stable_key, right.stable_key))
    });

    let mut unknowns = store
        .unknowns()
        .iter()
        .filter(|unknown| unknown.bundle == Some(bundle.id))
        .collect::<Vec<_>>();
    unknowns
        .sort_by(|unknown, other| interner.compare_canonical(unknown.stable_key, other.stable_key));

    let mut omitted = store
        .omitted_regions()
        .iter()
        .filter(|region| region.bundle == Some(bundle.id))
        .collect::<Vec<_>>();
    omitted
        .sort_by(|region, other| interner.compare_canonical(region.stable_key, other.stable_key));

    let replay_key = store
        .replay_keys()
        .iter()
        .find(|key| key.bundle == bundle.id)
        .map(|key| interner.resolve(key.stable_key).to_string())
        .or_else(|| bundle.replay_key.clone());

    let total_paths = paths.len();
    let total_unknowns = unknowns.len();
    let total_omitted_regions = omitted.len();
    let rendered_paths = total_paths.min(limit.max_paths);
    let rendered_unknowns = total_unknowns.min(limit.max_unknowns);
    let rendered_omitted_regions = total_omitted_regions.min(limit.max_omitted_regions);

    Some(json!({
        "version": 1,
        "bundle": {
            "id": bundle.id.0,
            "stable_key": interner.resolve(bundle.stable_key),
            "diagnostic_stable_key": interner.resolve(bundle.diagnostic_stable_key),
            "status": label(bundle.status),
            "precision": label(bundle.precision),
            "provenance": label(bundle.provenance),
            "validation": label(bundle.validation),
            "confidence": label(bundle.confidence),
            "replay_key": replay_key,
        },
        "paths": paths.into_iter().take(limit.max_paths).map(|path| {
            let total_edges = path.edges.len();
            let edge_values = path
                .edges
                .iter()
                .take(limit.max_edges_per_path)
                .filter_map(|edge_id| store.edge(*edge_id))
                .map(|edge| {
                    let mut value = json!({
                        "id": edge.id.0,
                        "stable_key": interner.resolve(edge.stable_key),
                        "kind": label(edge.kind),
                        "status": label(edge.status),
                        "precision": label(edge.precision),
                        "provenance": label(edge.provenance),
                        "validation": label(edge.validation),
                        "confidence": label(edge.confidence),
                        "summary_stable_key": edge.summary_stable_key,
                        "expansion": expansion_json(&edge.expansion),
                    });
                    if let Some(location) = edge_location_json(store, edge)
                        && let Some(object) = value.as_object_mut()
                    {
                        object.insert("location".to_string(), location);
                    }
                    value
                })
                .collect::<Vec<_>>();
            json!({
                "id": path.id.0,
                "stable_key": interner.resolve(path.stable_key),
                "rank": path.rank,
                "status": label(path.status),
                "hidden_node_count": path.hidden_node_count,
                "nodes": path.nodes.iter().map(|node| node.0).collect::<Vec<_>>(),
                "edges": edge_values,
                "omitted_regions": path.omitted_regions.iter().map(|region| region.0).collect::<Vec<_>>(),
                "total_edges": total_edges,
                "rendered_edges": total_edges.min(limit.max_edges_per_path),
                "edges_truncated": total_edges > limit.max_edges_per_path,
            })
        }).collect::<Vec<_>>(),
        "unknowns": unknowns.into_iter().take(limit.max_unknowns).map(|unknown| json!({
            "stable_key": interner.resolve(unknown.stable_key),
            "reason": label(unknown.reason),
            "message": unknown.message,
            "edge": unknown.edge.map(|edge| edge.0),
        })).collect::<Vec<_>>(),
        "omitted_regions": omitted.into_iter().take(limit.max_omitted_regions).map(|region| json!({
            "id": region.id.0,
            "stable_key": interner.resolve(region.stable_key),
            "reason": label(region.reason),
            "hidden_node_count": region.hidden_node_count,
            "hidden_edge_count": region.hidden_edge_count,
            "budget_label": region.budget_label,
        })).collect::<Vec<_>>(),
        "limits": {
            "max_paths": limit.max_paths,
            "max_edges_per_path": limit.max_edges_per_path,
            "max_unknowns": limit.max_unknowns,
            "max_omitted_regions": limit.max_omitted_regions,
            "total_paths": total_paths,
            "rendered_paths": rendered_paths,
            "paths_truncated": total_paths > limit.max_paths,
            "total_unknowns": total_unknowns,
            "rendered_unknowns": rendered_unknowns,
            "unknowns_truncated": total_unknowns > limit.max_unknowns,
            "total_omitted_regions": total_omitted_regions,
            "rendered_omitted_regions": rendered_omitted_regions,
            "omitted_regions_truncated": total_omitted_regions > limit.max_omitted_regions,
        },
    }))
}

pub fn scalar_evidence_for_bundle_json(value: &Value) -> Vec<Evidence> {
    let Some(bundle) = value.get("bundle") else {
        return Vec::new();
    };
    ["stable_key", "status", "precision", "provenance"]
        .into_iter()
        .filter_map(|label| {
            bundle
                .get(label)
                .and_then(Value::as_str)
                .map(|value| Evidence::new(format!("evidence_{label}"), value.to_string()))
        })
        .collect()
}

pub fn sarif_thread_flow_messages(value: &Value) -> Vec<String> {
    sarif_thread_flow_steps(value)
        .into_iter()
        .map(|step| step.message)
        .collect()
}

pub fn sarif_thread_flow_steps(value: &Value) -> Vec<EvidenceSarifStep> {
    let mut steps = value
        .get("paths")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|path| {
            path.get("edges")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|edge| {
                    let kind = edge.get("kind").and_then(Value::as_str).unwrap_or("edge");
                    let status = edge.get("status").and_then(Value::as_str).unwrap_or("unknown");
                    let precision = edge
                        .get("precision")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    let expansion = edge
                        .get("expansion")
                        .and_then(Value::as_object)
                        .and_then(|object| object.get("state"))
                        .and_then(Value::as_str)
                        .unwrap_or("none");
                    let provenance = edge
                        .get("provenance")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    let validation = edge
                        .get("validation")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    let confidence = edge
                        .get("confidence")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    EvidenceSarifStep {
                        message: format!(
                            "evidence {kind}: status={status}; precision={precision}; provenance={provenance}; validation={validation}; confidence={confidence}; expansion={expansion}"
                        ),
                        location: sarif_location_from_edge(edge),
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let fallback_location = steps.iter().find_map(|step| step.location.clone());
    if let Some(unknowns) = value.get("unknowns").and_then(Value::as_array)
        && !unknowns.is_empty()
    {
        steps.push(EvidenceSarifStep {
            message: "evidence unknown: path contains unknown evidence".to_string(),
            location: fallback_location.clone(),
        });
    }
    if let Some(omitted) = value.get("omitted_regions").and_then(Value::as_array)
        && !omitted.is_empty()
    {
        steps.push(EvidenceSarifStep {
            message: "evidence omitted: budget or compact rendering hid details".to_string(),
            location: fallback_location,
        });
    }
    steps
}

fn edge_location_json(
    _store: &EvidenceStore,
    _edge: &crate::analysis_neutral::evidence::facts::EvidenceEdgeFact,
) -> Option<Value> {
    None
}

fn sarif_location_from_edge(edge: &Value) -> Option<EvidenceSarifLocation> {
    let location = edge.get("location")?.as_object()?;
    let uri = location.get("uri")?.as_str()?.to_string();
    let range = location.get("range")?.as_object()?;
    Some(EvidenceSarifLocation {
        uri,
        range: DiagnosticRange::new(
            range.get("start_line")?.as_u64()?.try_into().ok()?,
            range.get("start_col")?.as_u64()?.try_into().ok()?,
            range.get("end_line")?.as_u64()?.try_into().ok()?,
            range.get("end_col")?.as_u64()?.try_into().ok()?,
        ),
    })
}

fn expansion_json(expansion: &EvidenceExpansion) -> Value {
    match expansion {
        EvidenceExpansion::None => json!({"state": "none"}),
        EvidenceExpansion::Expandable { key } => json!({"state": "expandable", "key": key}),
        EvidenceExpansion::Opaque { reason } => json!({"state": "opaque", "reason": reason}),
        EvidenceExpansion::ExternalModel { model } => {
            json!({"state": "external_model", "model": model})
        }
    }
}

fn label<T: std::fmt::Debug>(value: T) -> String {
    format!("{value:?}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::evidence::facts::{
        EvidenceBundleFact, EvidenceConfidence, EvidenceEdgeFact, EvidenceEdgeKind,
        EvidenceNodeFact, EvidenceNodeKind, EvidenceOmittedReason, EvidenceOmittedRegionFact,
        EvidencePathFact, EvidencePrecision, EvidenceProvenance, EvidenceQueryMode,
        EvidenceRankScore, EvidenceReplayKeyFact, EvidenceStatus, EvidenceUnknownFact,
        EvidenceUnknownReason, EvidenceValidation,
    };
    use crate::analysis_neutral::evidence::store::EvidenceOutput;
    use crate::analysis_neutral::ids::{
        EvidenceEdgeId, EvidenceNodeId, EvidenceOmittedRegionId, EvidencePathId,
    };

    #[test]
    fn bundle_json_is_deterministic_and_bounded() {
        let store = render_store();

        let first = render_bundle_json_v1(
            &store,
            &crate::internal_core::test_stable_key_interner(),
            EvidenceBundleId(0),
            EvidenceRenderLimit {
                max_paths: 1,
                max_edges_per_path: 1,
                max_unknowns: 1,
                max_omitted_regions: 1,
            },
        )
        .expect("bundle");
        let second = render_bundle_json_v1(
            &store,
            &crate::internal_core::test_stable_key_interner(),
            EvidenceBundleId(0),
            EvidenceRenderLimit {
                max_paths: 1,
                max_edges_per_path: 1,
                max_unknowns: 1,
                max_omitted_regions: 1,
            },
        )
        .expect("bundle");

        assert_eq!(first, second);
        assert_eq!(first["paths"].as_array().expect("paths").len(), 1);
        assert_eq!(first["paths"][0]["hidden_node_count"], 2);
        assert_eq!(first["bundle"]["replay_key"], "replay:bundle");
        assert_eq!(
            first["paths"][0]["edges"][0]["expansion"]["key"],
            "expand:summary"
        );
        let rendered = serde_json::to_string(&first).expect("json");
        assert!(!rendered.contains("/Users/"));
        assert!(!rendered.contains("raw source body"));
        assert!(!rendered.contains("timestamp"));
    }

    #[test]
    fn bundle_json_reports_totals_and_truncation_flags() {
        let store = render_store();

        let value = render_bundle_json_v1(
            &store,
            &crate::internal_core::test_stable_key_interner(),
            EvidenceBundleId(0),
            EvidenceRenderLimit {
                max_paths: 0,
                max_edges_per_path: 0,
                max_unknowns: 0,
                max_omitted_regions: 0,
            },
        )
        .expect("bundle");

        assert_eq!(value["limits"]["total_paths"], 1);
        assert_eq!(value["limits"]["rendered_paths"], 0);
        assert_eq!(value["limits"]["paths_truncated"], true);
        assert_eq!(value["limits"]["total_unknowns"], 1);
        assert_eq!(value["limits"]["unknowns_truncated"], true);
        assert_eq!(value["limits"]["total_omitted_regions"], 1);
        assert_eq!(value["limits"]["omitted_regions_truncated"], true);
    }

    #[test]
    fn bundle_json_round_trips_through_structured_evidence_validator() {
        let store = render_store();

        let value = render_bundle_json_v1(
            &store,
            &crate::internal_core::test_stable_key_interner(),
            EvidenceBundleId(0),
            EvidenceRenderLimit::default(),
        )
        .expect("bundle");

        crate::internal_core::StructuredEvidenceV1::try_from_value(value.clone())
            .expect("rendered evidence should satisfy public schema contract");
        assert!(
            value["paths"][0]["edges"][0].get("location").is_none(),
            "absent edge locations should be omitted instead of serialized as null"
        );
    }

    #[test]
    fn scalar_bridge_preserves_existing_evidence_shape() {
        let store = render_store();
        let value = render_bundle_json_v1(
            &store,
            &crate::internal_core::test_stable_key_interner(),
            EvidenceBundleId(0),
            EvidenceRenderLimit::default(),
        )
        .expect("bundle");

        let scalar = scalar_evidence_for_bundle_json(&value);

        assert!(scalar.iter().any(|item| item.label == "evidence_status"));
        assert!(scalar.iter().all(|item| !item.value.is_empty()));
    }

    #[test]
    fn sarif_messages_include_lossy_unknown_and_budget_wording() {
        let store = render_store();
        let value = render_bundle_json_v1(
            &store,
            &crate::internal_core::test_stable_key_interner(),
            EvidenceBundleId(0),
            EvidenceRenderLimit::default(),
        )
        .expect("bundle");

        let messages = sarif_thread_flow_messages(&value);

        assert!(messages.iter().any(|message| message.contains("unknown")));
        assert!(messages.iter().any(|message| message.contains("omitted")));
    }

    #[test]
    fn sarif_messages_include_provenance_validation_and_confidence() {
        let store = render_store();
        let value = render_bundle_json_v1(
            &store,
            &crate::internal_core::test_stable_key_interner(),
            EvidenceBundleId(0),
            EvidenceRenderLimit::default(),
        )
        .expect("bundle");

        let messages = sarif_thread_flow_messages(&value);

        assert!(
            messages
                .iter()
                .any(|message| message.contains("provenance=Summary"))
        );
        assert!(
            messages
                .iter()
                .any(|message| { message.contains("validation=ReferentiallyValidated") })
        );
        assert!(
            messages
                .iter()
                .any(|message| message.contains("confidence=Medium"))
        );
    }

    fn render_store() -> EvidenceStore {
        EvidenceStore::from_output(
            EvidenceOutput {
                nodes: vec![node(0), node(1)],
                edges: vec![EvidenceEdgeFact {
                    id: EvidenceEdgeId(0),
                    from: EvidenceNodeId(0),
                    to: EvidenceNodeId(1),
                    kind: EvidenceEdgeKind::Summary,
                    query_mode: EvidenceQueryMode::Path,
                    status: EvidenceStatus::Present,
                    precision: EvidencePrecision::SetupAware,
                    provenance: EvidenceProvenance::Summary,
                    validation: EvidenceValidation::ReferentiallyValidated,
                    confidence: EvidenceConfidence::Medium,
                    call_site: None,
                    summary_stable_key: Some(std::sync::Arc::from("summary:tito")),
                    expansion: EvidenceExpansion::Expandable {
                        key: "expand:summary".to_string(),
                    },
                    compact_label: None,
                    source_fact_stable_keys: Vec::new(),
                    stable_key: crate::internal_core::stable_key_for_test("edge:summary"),
                }],
                bundles: vec![EvidenceBundleFact {
                    id: EvidenceBundleId(0),
                    diagnostic_stable_key: crate::internal_core::stable_key_for_test("diag:1"),
                    query_mode: EvidenceQueryMode::Path,
                    status: EvidenceStatus::Partial,
                    precision: EvidencePrecision::SetupAware,
                    provenance: EvidenceProvenance::Query,
                    validation: EvidenceValidation::RendererValidated,
                    confidence: EvidenceConfidence::Medium,
                    entry_node: None,
                    selected_paths: vec![EvidencePathId(0)],
                    selected_slices: Vec::new(),
                    replay_key: Some("replay:bundle".to_string()),
                    stable_key: crate::internal_core::stable_key_for_test("bundle:diag"),
                }],
                paths: vec![EvidencePathFact {
                    id: EvidencePathId(0),
                    bundle: Some(EvidenceBundleId(0)),
                    query_mode: EvidenceQueryMode::Path,
                    nodes: vec![EvidenceNodeId(0), EvidenceNodeId(1)],
                    edges: vec![EvidenceEdgeId(0)],
                    rank: 0,
                    score: EvidenceRankScore::default(),
                    status: EvidenceStatus::Partial,
                    hidden_node_count: 2,
                    omitted_regions: vec![EvidenceOmittedRegionId(0)],
                    stable_key: crate::internal_core::stable_key_for_test("path:summary"),
                }],
                slices: Vec::new(),
                unknowns: vec![EvidenceUnknownFact {
                    bundle: Some(EvidenceBundleId(0)),
                    path: Some(EvidencePathId(0)),
                    slice: None,
                    edge: Some(EvidenceEdgeId(0)),
                    reason: EvidenceUnknownReason::OpaqueSummary,
                    message: "opaque summary".to_string(),
                    source_fact_stable_keys: Vec::new(),
                    stable_key: crate::internal_core::stable_key_for_test("unknown:summary"),
                }],
                omitted_regions: vec![EvidenceOmittedRegionFact {
                    id: EvidenceOmittedRegionId(0),
                    bundle: Some(EvidenceBundleId(0)),
                    path: Some(EvidencePathId(0)),
                    slice: None,
                    reason: EvidenceOmittedReason::CompactRendering,
                    hidden_node_count: 2,
                    hidden_edge_count: 1,
                    budget_label: Some("compact".to_string()),
                    stable_key: crate::internal_core::stable_key_for_test("omitted:summary"),
                }],
                replay_keys: vec![EvidenceReplayKeyFact {
                    bundle: EvidenceBundleId(0),
                    query_mode: EvidenceQueryMode::Path,
                    graph_schema: "evidence.graph.v1".to_string(),
                    query_budget: Default::default(),
                    ranking: crate::analysis_neutral::evidence::facts::EvidenceRankingMode::DeterministicDisplay,
                    renderer: crate::analysis_neutral::evidence::facts::EvidenceRendererMode::Json,
                    upstream_digest_keys: Vec::new(),
                    stable_key: crate::internal_core::stable_key_for_test("replay:bundle"),
                }],
            },
            &crate::internal_core::test_stable_key_interner(),
        )
        .expect("valid evidence")
    }

    fn node(id: u64) -> EvidenceNodeFact {
        EvidenceNodeFact {
            id: EvidenceNodeId(id),
            kind: EvidenceNodeKind::Summary,
            language: crate::internal_core::Language::Go,
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
            validation: EvidenceValidation::Native,
            confidence: EvidenceConfidence::High,
            compact_label: None,
            source_fact_stable_keys: Vec::new(),
            stable_key: crate::internal_core::stable_key_for_test(&format!("node:{id}")),
        }
    }
}
