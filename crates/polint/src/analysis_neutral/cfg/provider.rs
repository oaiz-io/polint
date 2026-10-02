use std::borrow::Cow;

use crate::analysis_api::ProviderManifest;
use crate::analysis_api::{
    CacheStats, Digest, DigestBuilder, DigestKind, InputComponent, InputComponentStatus,
    InputSnapshot, ProviderExecution, ProviderFailureReason, ProviderFailureStage,
};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::cfg::budget::{max_dominance_pairs, worst_case_dominance_pairs};
use crate::analysis_neutral::cfg::derived::{
    DominanceMaterialization, derive_control_dependence_for, derive_dominators_for,
    derive_postdominators_for, derive_reachability_for,
};
use crate::analysis_neutral::cfg::facts::CfgView;
use crate::analysis_neutral::cfg::graph::CfgGraphIndex;
use crate::analysis_neutral::cfg::ids::{BasicBlockId, CfgEdgeId, CfgFunctionId, CfgNodeId};
use crate::analysis_neutral::cfg::lower::lower_cfg;
use crate::analysis_neutral::cfg::store::CfgOutput;
use crate::internal_core::{Diagnostic, DiagnosticRange, Language};

#[derive(Debug, Clone, Default)]
pub struct CfgProviderOutput {
    pub diagnostics: Vec<Diagnostic>,
    pub cache_stats: CacheStats,
    pub output_digest: Option<Digest>,
    pub execution: ProviderExecution,
}

pub fn derive_cfg_with_cache_stats(
    db: &mut (impl AnalysisHost + Sync),
    input_snapshot: &InputSnapshot,
    manifest: &ProviderManifest,
    semantic_mir_output_digest: Digest,
    upstream_syntax_output_digests: Vec<Digest>,
    derived_relations: bool,
    lower_go: bool,
) -> CfgProviderOutput {
    let mut started = std::time::Instant::now();
    let mut checkpoint = |step: &'static str| {
        tracing::debug!(target: "polint::kernel::stage", provider = "polint.cfg", step, elapsed_ms = started.elapsed().as_millis() as u64, "provider step");
        started = std::time::Instant::now();
    };
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    // A run whose Go calls are answered by the typed call layer, and that asks
    // for nothing beyond call resolution, reads no Go control flow.
    let mut output = lower_cfg(db, |body| lower_go || body.language != Language::Go);
    checkpoint("lower_normalize");
    // Reachability, dominance and control dependence are read only by the
    // control-flow queries and the data-flow evidence, so a run that asks for
    // neither does not materialise them (nor the dominance budget they report).
    let (output, bounded) = if derived_relations {
        let bounded = append_derived_rows(interner, &mut output, CfgView::NormalControl);
        (output.normalized(interner), bounded)
    } else {
        (output, None)
    };
    checkpoint("derived_normalize");
    let output_digest = cfg_output_digest(
        manifest,
        input_snapshot,
        &semantic_mir_output_digest,
        &upstream_syntax_output_digests,
        derived_relations,
        lower_go,
        &output,
        interner,
    );
    checkpoint("digest");
    let mut cache_stats = CacheStats::default();
    cache_stats.record_recompute();

    match db.replace_cfg_facts(output) {
        Ok(()) => {
            checkpoint("store_metadata");
            CfgProviderOutput {
                diagnostics: bounded
                    .map(dominance_budget_diagnostic)
                    .into_iter()
                    .collect(),
                cache_stats,
                output_digest: Some(output_digest),
                execution: Default::default(),
            }
        }
        Err(error) => CfgProviderOutput {
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

/// The dominance relation a run declined to materialise in full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DominanceBudgetTrip {
    estimated_pairs: usize,
    limit: usize,
}

fn append_derived_rows(
    interner: &crate::internal_core::StableKeyInterner,
    output: &mut CfgOutput,
    view: CfgView,
) -> Option<DominanceBudgetTrip> {
    let mut started = std::time::Instant::now();
    let mut checkpoint = |step: &'static str| {
        tracing::debug!(target: "polint::kernel::stage", provider = "polint.cfg", step, elapsed_ms = started.elapsed().as_millis() as u64, "provider step");
        started = std::time::Instant::now();
    };
    if output.functions.is_empty() {
        return None;
    }
    let limit = max_dominance_pairs();
    let estimated_pairs = worst_case_dominance_pairs(output);
    let bounded = estimated_pairs > limit;
    let materialization = if bounded {
        DominanceMaterialization::ImmediateOnly
    } else {
        DominanceMaterialization::Full
    };
    let (reachability, dominators, postdominators, control_dependence) = {
        let index = CfgGraphIndex::new(interner, output);
        let graphs = index.graphs(view);
        checkpoint("graph_index");
        let reachability = derive_reachability_for(interner, &graphs, view);
        checkpoint("reachability");
        let dominators = derive_dominators_for(interner, &graphs, view, materialization);
        checkpoint("dominators");
        let postdominators = derive_postdominators_for(interner, &graphs, view, materialization);
        checkpoint("postdominators");
        let control_dependence = derive_control_dependence_for(interner, &graphs, view);
        checkpoint("control_dependence");
        (reachability, dominators, postdominators, control_dependence)
    };
    output.reachability = reachability;
    output.dominators = dominators;
    output.postdominators = postdominators;
    output.control_dependence = control_dependence;
    bounded.then_some(DominanceBudgetTrip {
        estimated_pairs,
        limit,
    })
}

/// Reports a bounded dominance relation on the same rule id the kernel's
/// resource envelope uses, so `polint unknowns` shows one `budget_exceeded`
/// vocabulary for "polint bounded itself to fit a resource envelope".
fn dominance_budget_diagnostic(trip: DominanceBudgetTrip) -> Diagnostic {
    Diagnostic::warning(
        crate::analysis_kernel::resource::RESOURCE_BUDGET_RULE_ID,
        "<workspace>",
        DiagnosticRange::point(1, 1),
        format!(
            "control-flow dominance materialisation bounded: worst-case {} pairs against a \
             {} pair ceiling. `cfg_dominators` and `cfg_postdominators` carry the immediate \
             (tree) edges only; the full relation is their reflexive transitive closure.",
            trip.estimated_pairs, trip.limit,
        ),
    )
    .with_evidence(
        crate::diagnostics::BUDGET_EVIDENCE_LABEL,
        "cfg_dominance_pairs",
    )
    .with_evidence(crate::diagnostics::BUDGET_STATUS_EVIDENCE_LABEL, "exceeded")
    .with_evidence("estimated_pairs", trip.estimated_pairs.to_string())
    .with_evidence("limit", trip.limit.to_string())
}

fn cfg_output_digest(
    manifest: &ProviderManifest,
    input_snapshot: &InputSnapshot,
    semantic_mir_output_digest: &Digest,
    upstream_syntax_output_digests: &[Digest],
    derived_relations: bool,
    lower_go: bool,
    output: &CfgOutput,
    interner: &crate::internal_core::StableKeyInterner,
) -> Digest {
    let mut digest = Digest::builder(DigestKind::ProviderOutput, "cfg_output");
    digest.field(
        "derived_relations",
        if derived_relations { "true" } else { "false" },
    );
    digest.field("lower_go", if lower_go { "true" } else { "false" });
    digest.part("provider_id");
    digest.part(manifest.id);
    digest.part("provider_version");
    digest.part(manifest.provider_version());
    digest.part("schema");
    digest.part(&manifest.primary_schema_label());
    digest.part("config");
    digest.part(&input_snapshot.config.digest.to_string());
    digest.part("semantic_mir");
    digest.part(&semantic_mir_output_digest.to_string());
    append_component_digest_parts(
        &mut digest,
        "go_lifecycle",
        &input_snapshot.go_lifecycle.components,
    );
    append_component_digest_parts(
        &mut digest,
        "ts_js_lifecycle",
        &input_snapshot.ts_js_lifecycle.components,
    );
    append_component_digest_parts(&mut digest, "model", &input_snapshot.models);
    append_component_digest_parts(&mut digest, "extension", &input_snapshot.extensions);
    append_component_digest_parts(&mut digest, "tool", &input_snapshot.tool_invocations);

    let mut upstream_syntax_output_digests =
        upstream_syntax_output_digests.iter().collect::<Vec<_>>();
    upstream_syntax_output_digests.sort();
    for upstream in upstream_syntax_output_digests {
        digest.part("upstream_syntax");
        digest.part(&upstream.to_string());
    }

    // Rows are hashed in storage order. `CfgOutput::normalized` fixes that
    // order from the rows' stable keys and the deterministic ids the lowering
    // assigns, so re-sorting by key text here would only repeat it.
    let function_keys = dense_keys(
        output
            .functions
            .iter()
            .map(|row| (row.id.0, row.stable_key)),
    );
    let node_keys = dense_keys(output.nodes.iter().map(|row| (row.id.0, row.stable_key)));
    let block_keys = dense_keys(output.blocks.iter().map(|row| (row.id.0, row.stable_key)));
    let edge_keys = dense_keys(output.edges.iter().map(|row| (row.id.0, row.stable_key)));
    let kind = DigestKind::ProviderOutput;
    // One read view per hashing task: see `StableKeyInterner::read_view`.
    let task = || interner.read_view();

    let functions = Digest::of_rows(
        kind,
        "cfg_function",
        &output.functions,
        task,
        |digest, keys, row| {
            digest.part("cfg_function");
            digest.part(keys.text(row.stable_key));
            digest.debug_part(row.language);
            digest.part(&span_part(&row.span));
            digest.part(&key_text(keys, &node_keys, row.entry_node.0, "node"));
            digest.part(&key_text(keys, &node_keys, row.normal_exit_node.0, "node"));
            digest.part(
                &row.exceptional_exit_node
                    .map_or(Cow::Borrowed("none"), |id| {
                        key_text(keys, &node_keys, id.0, "node")
                    }),
            );
            digest.debug_part(row.status);
            digest.debug_part(row.precision);
        },
    );
    let nodes = Digest::of_rows(
        kind,
        "cfg_node",
        &output.nodes,
        task,
        |digest, keys, row| {
            digest.part("cfg_node");
            digest.part(keys.text(row.stable_key));
            digest.part(&key_text(
                keys,
                &function_keys,
                row.cfg_function.0,
                "function",
            ));
            digest.part(&key_text(keys, &block_keys, row.block.0, "block"));
            digest.debug_part(row.kind);
            digest.part(optional_span_part(row.span.as_ref()).as_ref());
            digest.bool_part(row.generated);
            digest.part(&row.operation_ordinal.to_string());
            digest.debug_part(row.status);
            digest.debug_part(row.precision);
        },
    );
    let blocks = Digest::of_rows(
        kind,
        "basic_block",
        &output.blocks,
        task,
        |digest, keys, row| {
            digest.part("basic_block");
            digest.part(keys.text(row.stable_key));
            digest.part(&key_text(
                keys,
                &function_keys,
                row.cfg_function.0,
                "function",
            ));
            digest.debug_part(row.kind);
            digest.part(&row.first_node.map_or(Cow::Borrowed("none"), |id| {
                key_text(keys, &node_keys, id.0, "node")
            }));
            digest.part(&row.last_node.map_or(Cow::Borrowed("none"), |id| {
                key_text(keys, &node_keys, id.0, "node")
            }));
            digest.bool_part(row.reachable);
            digest.part(&row.reverse_postorder.to_string());
            digest.debug_part(row.status);
            digest.debug_part(row.precision);
        },
    );
    let edges = Digest::of_rows(
        kind,
        "cfg_edge",
        &output.edges,
        task,
        |digest, keys, row| {
            digest.part("cfg_edge");
            digest.part(keys.text(row.stable_key));
            digest.part(&key_text(
                keys,
                &function_keys,
                row.cfg_function.0,
                "function",
            ));
            digest.debug_part(row.view);
            digest.part(&key_text(keys, &node_keys, row.from.0, "node"));
            digest.part(&key_text(keys, &node_keys, row.to.0, "node"));
            digest.part(&key_text(keys, &block_keys, row.from_block.0, "block"));
            digest.part(&key_text(keys, &block_keys, row.to_block.0, "block"));
            digest.debug_part(row.kind);
            digest.part(row.label.as_deref().unwrap_or("none"));
            digest.debug_part(row.status);
            digest.debug_part(row.precision);
        },
    );
    let reachability = Digest::of_rows(
        kind,
        "cfg_reachability",
        &output.reachability,
        task,
        |digest, keys, row| {
            digest.part("cfg_reachability");
            digest.part(keys.text(row.stable_key));
            digest.part(&key_text(
                keys,
                &function_keys,
                row.cfg_function.0,
                "function",
            ));
            digest.debug_part(row.view);
            digest.part(&key_text(keys, &block_keys, row.block.0, "block"));
            digest.bool_part(row.reachable);
            digest.debug_part(row.status);
            digest.debug_part(row.precision);
        },
    );
    let dominators = Digest::of_rows(
        kind,
        "cfg_dominator",
        &output.dominators,
        task,
        |digest, keys, row| {
            digest.part("cfg_dominator");
            digest.part(keys.text(row.stable_key));
            digest.part(&key_text(
                keys,
                &function_keys,
                row.cfg_function.0,
                "function",
            ));
            digest.debug_part(row.view);
            digest.part(&key_text(keys, &block_keys, row.dominator.0, "block"));
            digest.part(&key_text(keys, &block_keys, row.dominated.0, "block"));
            digest.bool_part(row.immediate);
            digest.debug_part(row.status);
            digest.debug_part(row.precision);
        },
    );
    let postdominators = Digest::of_rows(
        kind,
        "cfg_postdominator",
        &output.postdominators,
        task,
        |digest, keys, row| {
            digest.part("cfg_postdominator");
            digest.part(keys.text(row.stable_key));
            digest.part(&key_text(
                keys,
                &function_keys,
                row.cfg_function.0,
                "function",
            ));
            digest.debug_part(row.view);
            digest.part(&key_text(keys, &block_keys, row.postdominator.0, "block"));
            digest.part(&key_text(keys, &block_keys, row.postdominated.0, "block"));
            digest.bool_part(row.immediate);
            digest.debug_part(row.status);
            digest.debug_part(row.precision);
        },
    );
    let control_dependence = Digest::of_rows(
        kind,
        "cfg_control_dependence",
        &output.control_dependence,
        task,
        |digest, keys, row| {
            digest.part("cfg_control_dependence");
            digest.part(keys.text(row.stable_key));
            digest.part(&key_text(
                keys,
                &function_keys,
                row.cfg_function.0,
                "function",
            ));
            digest.debug_part(row.view);
            digest.part(&key_text(keys, &edge_keys, row.controlling_edge.0, "edge"));
            digest.debug_part(row.controlling_edge_kind);
            digest.part(&key_text(
                keys,
                &block_keys,
                row.controlled_block.0,
                "block",
            ));
            digest.debug_part(row.status);
            digest.debug_part(row.precision);
        },
    );
    let unsupported = Digest::of_rows(
        kind,
        "unsupported_control_flow",
        &output.unsupported,
        task,
        |digest, keys, row| {
            digest.part("unsupported_control_flow");
            digest.part(keys.text(row.stable_key));
            digest.part(&row.cfg_function.map_or(Cow::Borrowed("none"), |id| {
                key_text(keys, &function_keys, id.0, "function")
            }));
            digest.debug_part(row.language);
            digest.part(&span_part(&row.span));
            digest.part(&row.construct);
            digest.part(&row.source_evidence);
            digest.debug_part(row.conservative_action);
            digest.debug_part(row.status);
            digest.debug_part(row.precision);
        },
    );
    for family in [
        functions,
        nodes,
        blocks,
        edges,
        reachability,
        dominators,
        postdominators,
        control_dependence,
        unsupported,
    ] {
        digest.part(&family.value);
    }

    digest.finish()
}

/// Stable keys indexed by the dense id of the row that carries them.
fn dense_keys(
    rows: impl Iterator<Item = (u64, crate::internal_core::StableKeyId)>,
) -> Vec<Option<crate::internal_core::StableKeyId>> {
    let mut keys = Vec::new();
    for (id, key) in rows {
        let index = usize::try_from(id).expect("CFG ids index their rows");
        if keys.len() <= index {
            keys.resize(index + 1, None);
        }
        keys[index] = Some(key);
    }
    keys
}

/// The key text of the row with id `id`, or a placeholder naming the missing row.
fn key_text<'a>(
    keys: &'a crate::internal_core::StableKeyReadView<'_>,
    table: &[Option<crate::internal_core::StableKeyId>],
    id: u64,
    family: &str,
) -> Cow<'a, str> {
    usize::try_from(id)
        .ok()
        .and_then(|index| table.get(index).copied().flatten())
        .map(|key| Cow::Borrowed(keys.text(key)))
        .unwrap_or_else(|| Cow::Owned(format!("<missing-{family}:{id}>")))
}

fn span_part(span: &crate::internal_core::Span) -> String {
    format!(
        "{}:{}..{}:{}@{}..{}",
        span.start_line,
        span.start_col,
        span.end_line,
        span.end_col,
        span.start_byte,
        span.end_byte
    )
}

fn optional_span_part(span: Option<&crate::internal_core::Span>) -> Cow<'_, str> {
    span.map(|span| Cow::Owned(span_part(span)))
        .unwrap_or(Cow::Borrowed("none"))
}

fn append_component_digest_parts(
    digest: &mut DigestBuilder,
    prefix: &str,
    components: &[InputComponent],
) {
    let mut components = components.iter().collect::<Vec<_>>();
    components.sort_by(|left, right| {
        (
            left.name.as_str(),
            component_status_rank(left.status),
            &left.digest,
        )
            .cmp(&(
                right.name.as_str(),
                component_status_rank(right.status),
                &right.digest,
            ))
    });
    for component in components {
        digest.part(prefix);
        digest.part(&component.name);
        digest.debug_part(component.status);
        digest.part(&component.digest.to_string());
    }
}

fn component_status_rank(status: InputComponentStatus) -> u8 {
    match status {
        InputComponentStatus::Present => 0,
        InputComponentStatus::Absent => 1,
        InputComponentStatus::Unsupported => 2,
        InputComponentStatus::SetupMissing => 3,
    }
}

fn provider_error_diagnostic(message: String) -> Diagnostic {
    Diagnostic::error(
        "polint/internal",
        "<workspace>",
        DiagnosticRange::point(1, 1),
        format!("CFG provider failed: {message}"),
    )
}
