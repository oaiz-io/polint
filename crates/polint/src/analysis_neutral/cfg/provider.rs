use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::analysis_api::ProviderManifest;
use crate::analysis_api::{
    CacheStats, Digest, DigestBuilder, DigestKind, InputComponent, InputComponentStatus,
    InputSnapshot, ProviderExecution, ProviderFailureReason, ProviderFailureStage,
};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::cfg::budget::{max_dominance_pairs, worst_case_dominance_pairs};
use crate::analysis_neutral::cfg::derived::{
    DominanceMaterialization, derive_control_dependence, derive_dominators, derive_postdominators,
    derive_reachability,
};
use crate::analysis_neutral::cfg::facts::CfgView;
use crate::analysis_neutral::cfg::ids::{BasicBlockId, CfgEdgeId, CfgFunctionId, CfgNodeId};
use crate::analysis_neutral::cfg::lower::lower_cfg;
use crate::analysis_neutral::cfg::store::CfgOutput;
use crate::internal_core::{Diagnostic, DiagnosticRange};

#[derive(Debug, Clone, Default)]
pub struct CfgProviderOutput {
    pub diagnostics: Vec<Diagnostic>,
    pub cache_stats: CacheStats,
    pub output_digest: Option<Digest>,
    pub execution: ProviderExecution,
}

pub fn derive_cfg_with_cache_stats(
    db: &mut impl AnalysisHost,
    input_snapshot: &InputSnapshot,
    manifest: &ProviderManifest,
    semantic_mir_output_digest: Digest,
    upstream_syntax_output_digests: Vec<Digest>,
) -> CfgProviderOutput {
    let mut started = std::time::Instant::now();
    let mut checkpoint = |step: &'static str| {
        tracing::debug!(target: "polint::kernel::stage", provider = "polint.cfg", step, elapsed_ms = started.elapsed().as_millis() as u64, "provider step");
        started = std::time::Instant::now();
    };
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let mut output = derive_cfg_output(db).normalized(interner);
    checkpoint("lower_normalize");
    let bounded = append_derived_rows(interner, &mut output, CfgView::NormalControl);
    let output = output.normalized(interner);
    checkpoint("derived_normalize");
    let output_digest = cfg_output_digest(
        manifest,
        input_snapshot,
        &semantic_mir_output_digest,
        &upstream_syntax_output_digests,
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

fn derive_cfg_output(db: &impl AnalysisHost) -> CfgOutput {
    lower_cfg(db)
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
    output.reachability = derive_reachability(interner, output, view);
    checkpoint("reachability");
    output.dominators = derive_dominators(interner, output, view, materialization);
    checkpoint("dominators");
    output.postdominators = derive_postdominators(interner, output, view, materialization);
    checkpoint("postdominators");
    output.control_dependence = derive_control_dependence(interner, output, view);
    checkpoint("control_dependence");
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
}

fn cfg_output_digest(
    manifest: &ProviderManifest,
    input_snapshot: &InputSnapshot,
    semantic_mir_output_digest: &Digest,
    upstream_syntax_output_digests: &[Digest],
    output: &CfgOutput,
    interner: &crate::internal_core::StableKeyInterner,
) -> Digest {
    let mut digest = Digest::builder(DigestKind::ProviderOutput, "cfg_output");
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

    let function_keys = function_key_map(output);
    let node_keys = node_key_map(output);
    let block_keys = block_key_map(output);
    let edge_keys = edge_key_map(output);

    for row in sorted_refs_by_stable_key(interner, &output.functions) {
        digest.part("cfg_function");
        digest.part(interner.resolve(row.stable_key).as_ref());
        digest.debug_part(row.language);
        digest.part(&span_part(&row.span));
        digest.part(stable_node_key(interner, &node_keys, row.entry_node).as_ref());
        digest.part(stable_node_key(interner, &node_keys, row.normal_exit_node).as_ref());
        digest.part(optional_node_key(interner, &node_keys, row.exceptional_exit_node).as_ref());
        digest.debug_part(row.status);
        digest.debug_part(row.precision);
    }

    for row in sorted_refs_by_stable_key(interner, &output.nodes) {
        digest.part("cfg_node");
        digest.part(interner.resolve(row.stable_key).as_ref());
        digest.part(stable_function_key(interner, &function_keys, row.cfg_function).as_ref());
        digest.part(stable_block_key(interner, &block_keys, row.block).as_ref());
        digest.debug_part(row.kind);
        digest.part(optional_span_part(row.span.as_ref()).as_ref());
        digest.bool_part(row.generated);
        digest.part(&row.operation_ordinal.to_string());
        digest.debug_part(row.status);
        digest.debug_part(row.precision);
    }

    for row in sorted_refs_by_stable_key(interner, &output.blocks) {
        digest.part("basic_block");
        digest.part(interner.resolve(row.stable_key).as_ref());
        digest.part(stable_function_key(interner, &function_keys, row.cfg_function).as_ref());
        digest.debug_part(row.kind);
        digest.part(optional_node_key(interner, &node_keys, row.first_node).as_ref());
        digest.part(optional_node_key(interner, &node_keys, row.last_node).as_ref());
        digest.bool_part(row.reachable);
        digest.part(&row.reverse_postorder.to_string());
        digest.debug_part(row.status);
        digest.debug_part(row.precision);
    }

    for row in sorted_refs_by_stable_key(interner, &output.edges) {
        digest.part("cfg_edge");
        digest.part(interner.resolve(row.stable_key).as_ref());
        digest.part(stable_function_key(interner, &function_keys, row.cfg_function).as_ref());
        digest.debug_part(row.view);
        digest.part(stable_node_key(interner, &node_keys, row.from).as_ref());
        digest.part(stable_node_key(interner, &node_keys, row.to).as_ref());
        digest.part(stable_block_key(interner, &block_keys, row.from_block).as_ref());
        digest.part(stable_block_key(interner, &block_keys, row.to_block).as_ref());
        digest.debug_part(row.kind);
        digest.part(row.label.as_deref().unwrap_or("none"));
        digest.debug_part(row.status);
        digest.debug_part(row.precision);
    }

    for row in sorted_refs_by_stable_key(interner, &output.reachability) {
        digest.part("cfg_reachability");
        digest.part(interner.resolve(row.stable_key).as_ref());
        digest.part(stable_function_key(interner, &function_keys, row.cfg_function).as_ref());
        digest.debug_part(row.view);
        digest.part(stable_block_key(interner, &block_keys, row.block).as_ref());
        digest.bool_part(row.reachable);
        digest.debug_part(row.status);
        digest.debug_part(row.precision);
    }

    for row in sorted_refs_by_stable_key(interner, &output.dominators) {
        digest.part("cfg_dominator");
        digest.part(interner.resolve(row.stable_key).as_ref());
        digest.part(stable_function_key(interner, &function_keys, row.cfg_function).as_ref());
        digest.debug_part(row.view);
        digest.part(stable_block_key(interner, &block_keys, row.dominator).as_ref());
        digest.part(stable_block_key(interner, &block_keys, row.dominated).as_ref());
        digest.bool_part(row.immediate);
        digest.debug_part(row.status);
        digest.debug_part(row.precision);
    }

    for row in sorted_refs_by_stable_key(interner, &output.postdominators) {
        digest.part("cfg_postdominator");
        digest.part(interner.resolve(row.stable_key).as_ref());
        digest.part(stable_function_key(interner, &function_keys, row.cfg_function).as_ref());
        digest.debug_part(row.view);
        digest.part(stable_block_key(interner, &block_keys, row.postdominator).as_ref());
        digest.part(stable_block_key(interner, &block_keys, row.postdominated).as_ref());
        digest.bool_part(row.immediate);
        digest.debug_part(row.status);
        digest.debug_part(row.precision);
    }

    for row in sorted_refs_by_stable_key(interner, &output.control_dependence) {
        digest.part("cfg_control_dependence");
        digest.part(interner.resolve(row.stable_key).as_ref());
        digest.part(stable_function_key(interner, &function_keys, row.cfg_function).as_ref());
        digest.debug_part(row.view);
        digest.part(stable_edge_key(interner, &edge_keys, row.controlling_edge).as_ref());
        digest.debug_part(row.controlling_edge_kind);
        digest.part(stable_block_key(interner, &block_keys, row.controlled_block).as_ref());
        digest.debug_part(row.status);
        digest.debug_part(row.precision);
    }

    for row in sorted_refs_by_stable_key(interner, &output.unsupported) {
        digest.part("unsupported_control_flow");
        digest.part(interner.resolve(row.stable_key).as_ref());
        digest.part(optional_function_key(interner, &function_keys, row.cfg_function).as_ref());
        digest.debug_part(row.language);
        digest.part(&span_part(&row.span));
        digest.part(&row.construct);
        digest.part(&row.source_evidence);
        digest.debug_part(row.conservative_action);
        digest.debug_part(row.status);
        digest.debug_part(row.precision);
    }

    digest.finish()
}

trait StableKeyed {
    fn stable_key(&self) -> crate::internal_core::StableKeyId;
}

macro_rules! impl_stable_keyed {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl StableKeyed for $ty {
                fn stable_key(&self) -> crate::internal_core::StableKeyId {
                    self.stable_key
                }
            }
        )+
    };
}

impl_stable_keyed!(
    crate::analysis_neutral::cfg::facts::CfgFunctionFact,
    crate::analysis_neutral::cfg::facts::CfgNodeFact,
    crate::analysis_neutral::cfg::facts::BasicBlockFact,
    crate::analysis_neutral::cfg::facts::CfgEdgeFact,
    crate::analysis_neutral::cfg::facts::ReachabilityFact,
    crate::analysis_neutral::cfg::facts::DominatorFact,
    crate::analysis_neutral::cfg::facts::PostDominatorFact,
    crate::analysis_neutral::cfg::facts::ControlDependenceFact,
    crate::analysis_neutral::cfg::facts::UnsupportedControlFlowFact,
);

fn sorted_refs_by_stable_key<'a, T: StableKeyed>(
    interner: &crate::internal_core::StableKeyInterner,
    rows: &'a [T],
) -> Vec<&'a T> {
    let mut refs = rows.iter().collect::<Vec<_>>();
    refs.sort_by(|row, other| interner.compare_canonical(row.stable_key(), other.stable_key()));
    refs
}

fn function_key_map(
    output: &CfgOutput,
) -> BTreeMap<CfgFunctionId, crate::internal_core::StableKeyId> {
    output
        .functions
        .iter()
        .map(|row| (row.id, row.stable_key))
        .collect()
}

fn node_key_map(output: &CfgOutput) -> BTreeMap<CfgNodeId, crate::internal_core::StableKeyId> {
    output
        .nodes
        .iter()
        .map(|row| (row.id, row.stable_key))
        .collect()
}

fn block_key_map(output: &CfgOutput) -> BTreeMap<BasicBlockId, crate::internal_core::StableKeyId> {
    output
        .blocks
        .iter()
        .map(|row| (row.id, row.stable_key))
        .collect()
}

fn edge_key_map(output: &CfgOutput) -> BTreeMap<CfgEdgeId, crate::internal_core::StableKeyId> {
    output
        .edges
        .iter()
        .map(|row| (row.id, row.stable_key))
        .collect()
}

fn stable_function_key<'a>(
    interner: &crate::internal_core::StableKeyInterner,
    keys: &BTreeMap<CfgFunctionId, crate::internal_core::StableKeyId>,
    id: CfgFunctionId,
) -> Cow<'a, str> {
    keys.get(&id)
        .map(|key| Cow::Owned(interner.resolve(*key).to_string()))
        .unwrap_or_else(|| Cow::Owned(format!("<missing-function:{}>", id.0)))
}

fn stable_node_key<'a>(
    interner: &crate::internal_core::StableKeyInterner,
    keys: &BTreeMap<CfgNodeId, crate::internal_core::StableKeyId>,
    id: CfgNodeId,
) -> Cow<'a, str> {
    keys.get(&id)
        .map(|key| Cow::Owned(interner.resolve(*key).to_string()))
        .unwrap_or_else(|| Cow::Owned(format!("<missing-node:{}>", id.0)))
}

fn stable_block_key<'a>(
    interner: &crate::internal_core::StableKeyInterner,
    keys: &BTreeMap<BasicBlockId, crate::internal_core::StableKeyId>,
    id: BasicBlockId,
) -> Cow<'a, str> {
    keys.get(&id)
        .map(|key| Cow::Owned(interner.resolve(*key).to_string()))
        .unwrap_or_else(|| Cow::Owned(format!("<missing-block:{}>", id.0)))
}

fn stable_edge_key<'a>(
    interner: &crate::internal_core::StableKeyInterner,
    keys: &BTreeMap<CfgEdgeId, crate::internal_core::StableKeyId>,
    id: CfgEdgeId,
) -> Cow<'a, str> {
    keys.get(&id)
        .map(|key| Cow::Owned(interner.resolve(*key).to_string()))
        .unwrap_or_else(|| Cow::Owned(format!("<missing-edge:{}>", id.0)))
}

fn optional_function_key<'a>(
    interner: &crate::internal_core::StableKeyInterner,
    keys: &BTreeMap<CfgFunctionId, crate::internal_core::StableKeyId>,
    id: Option<CfgFunctionId>,
) -> Cow<'a, str> {
    id.map(|id| stable_function_key(interner, keys, id))
        .unwrap_or(Cow::Borrowed("none"))
}

fn optional_node_key<'a>(
    interner: &crate::internal_core::StableKeyInterner,
    keys: &BTreeMap<CfgNodeId, crate::internal_core::StableKeyId>,
    id: Option<CfgNodeId>,
) -> Cow<'a, str> {
    id.map(|id| stable_node_key(interner, keys, id))
        .unwrap_or(Cow::Borrowed("none"))
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
