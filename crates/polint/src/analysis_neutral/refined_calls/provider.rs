use std::collections::BTreeMap;

use crate::analysis_api::{
    CacheStats, Digest, DigestKind, InputComponent, InputSnapshot, ProviderExecution,
    ProviderFailureReason, ProviderFailureStage, stable_key_from_key_parts,
};
use crate::analysis_api::{
    FactFamily, FactRef, ProviderManifest, stable_key_from_parts, stable_key_text_from_parts,
};
use crate::analysis_neutral::calls::facts::{
    CallAlgorithm, CallEdgeKind, CallPrecision, CallProvenance, CallSiteFact, CallSyntaxKind,
    CallTargetFact, CallTargetStatus, UnresolvedCallReason,
};
use crate::analysis_neutral::points_to::facts::{PointsToPrecision, PointsToStatus};
use crate::analysis_neutral::refined_calls::cache_key::{
    refined_calls_provider_parameter_digest, refined_calls_provider_parameter_digest_for_snapshot,
};
use crate::analysis_neutral::refined_calls::facts::{
    RefinedCallConfidence, RefinedCallEdgeFact, RefinedCallTier, RefinedCallValidation,
};
use crate::analysis_neutral::refined_calls::store::RefinedCallOutput;
use crate::analysis_neutral::semantic_graph::constraints::ConstraintKind;
use crate::analysis_neutral::semantic_graph::facts::NodeKind;
use crate::analysis_neutral::solver::facts::DerivedEdgeFact;
use crate::internal_core::{
    Diagnostic, DiagnosticRange, FileId, FunctionId, KeyPart, Language, Span, StableKeyId,
    StableKeyInterner, SymbolId,
};

use crate::analysis_neutral::AnalysisHost;

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoSemanticFunctionInput {
    pub qualified: String,
    pub name: String,
    pub file: Option<FileId>,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoSemanticCallsiteInput {
    pub stable_key: StableKeyId,
    pub caller: String,
    pub file: Option<FileId>,
    pub span: Option<Span>,
}

pub const REFINED_CALLS_PROVIDER_ID: &str = "polint.refined_calls";

#[derive(Debug, Clone, Default)]
pub struct RefinedCallsProviderOutput {
    pub diagnostics: Vec<Diagnostic>,
    pub cache_stats: CacheStats,
    pub output_digest: Option<Digest>,
    pub execution: ProviderExecution,
}

#[allow(clippy::too_many_arguments)]
pub fn derive_refined_calls_with_cache_stats(
    db: &mut impl AnalysisHost,
    input_snapshot: &InputSnapshot,
    manifest: &ProviderManifest,
    calls_output_digest: Digest,
    entrypoints_output_digest: Digest,
    direct_summaries_output_digest: Digest,
    type_value_alias_output_digest: Digest,
    extensions_output_digest: Digest,
    solver_output_digest: Digest,
    go_semantic_functions: &[GoSemanticFunctionInput],
    go_semantic_callsites: &[GoSemanticCallsiteInput],
) -> RefinedCallsProviderOutput {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    debug_assert_eq!(manifest.id, REFINED_CALLS_PROVIDER_ID);
    let mut output = RefinedCallOutput::empty();
    let call_sites = db
        .call_sites()
        .iter()
        .map(|site| {
            (
                site.id,
                (
                    site.language,
                    db.resolve_stable_key(site.stable_key).to_string(),
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    for target in db.call_targets() {
        let Some((language, site_key)) = call_sites.get(&target.site) else {
            return failed_provider_output(format!(
                "dangling call site {:?} for call target {:?}",
                target.site, target.id
            ));
        };
        let base_target_key = db
            .metadata_for(FactRef::new(FactFamily::CallTarget, target.id.0))
            .map(|metadata| db.resolve_stable_key(metadata.stable_key).to_string())
            .unwrap_or_else(|| interner.resolve(target.stable_key).to_string());
        output.edges.push(refined_edge_from_base_target(
            interner,
            target,
            crate::analysis_neutral::ids::RefinedCallEdgeId(0),
            RefinedCallTier::DirectOnly,
            *language,
            base_target_key,
            site_key,
        ));
    }
    output.edges.extend(
        crate::analysis_neutral::refined_calls::framework::derive_framework_refinements(db).edges,
    );
    output
        .edges
        .extend(crate::analysis_neutral::refined_calls::go::derive_go_refinements(db).edges);
    output
        .edges
        .extend(crate::analysis_neutral::refined_calls::ts_js::derive_ts_js_refinements(db).edges);
    output.edges.extend(
        crate::analysis_neutral::refined_calls::summaries::derive_summary_assisted_refinements(db)
            .edges,
    );
    output.edges.extend(
        crate::analysis_neutral::refined_calls::extensions::derive_extension_refinements(db).edges,
    );
    output.edges.extend(
        derive_solver_refinements_with_inputs(db, go_semantic_functions, go_semantic_callsites)
            .edges,
    );
    output = finalized_output(interner, output);

    let output_digest = match refined_calls_output_digest(
        db,
        interner,
        manifest,
        input_snapshot,
        &calls_output_digest,
        &entrypoints_output_digest,
        &direct_summaries_output_digest,
        &type_value_alias_output_digest,
        &extensions_output_digest,
        &solver_output_digest,
        &output,
    ) {
        Ok(digest) => digest,
        Err(error) => return failed_provider_output(error.to_string()),
    };
    let mut cache_stats = CacheStats::default();
    cache_stats.record_recompute();

    match db.replace_refined_call_facts(output) {
        Ok(()) => RefinedCallsProviderOutput {
            diagnostics: Vec::new(),
            cache_stats,
            output_digest: Some(output_digest),
            execution: Default::default(),
        },
        Err(error) => RefinedCallsProviderOutput {
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

pub fn finalized_output(
    interner: &StableKeyInterner,
    mut output: RefinedCallOutput,
) -> RefinedCallOutput {
    output = output.normalized(interner);
    for (index, edge) in output.edges.iter_mut().enumerate() {
        edge.id = crate::analysis_neutral::ids::RefinedCallEdgeId(index as u64);
    }
    output
}

fn refined_edge_from_base_target(
    interner: &StableKeyInterner,
    target: &CallTargetFact,
    id: crate::analysis_neutral::ids::RefinedCallEdgeId,
    tier: RefinedCallTier,
    language: Language,
    base_target_key: String,
    site_key: &str,
) -> RefinedCallEdgeFact {
    RefinedCallEdgeFact {
        id,
        site: target.site,
        base_target: Some(target.id),
        caller: target.caller,
        target_function: target.target_function,
        target_symbol: target.target_symbol,
        synthetic_target: None,
        language,
        edge_kind: target.edge_kind,
        algorithm: target.algorithm,
        tier,
        status: target.status,
        reason: target.reason,
        provenance: target.provenance,
        precision: target.precision,
        validation: validation_for_target(target),
        confidence: confidence_for_target(target),
        evidence: vec!["base_call_target".to_string()],
        input_stable_keys: vec![base_target_key.clone()],
        stable_key: stable_refined_call_key(interner, tier, &base_target_key, site_key),
    }
}

pub fn derive_solver_refinements(db: &impl AnalysisHost) -> RefinedCallOutput {
    derive_solver_refinements_with_inputs(db, &[], &[])
}

pub fn derive_solver_refinements_with_inputs(
    db: &impl AnalysisHost,
    go_semantic_functions: &[GoSemanticFunctionInput],
    go_semantic_callsites: &[GoSemanticCallsiteInput],
) -> RefinedCallOutput {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let index = SolverProjectionIndex::new(db, go_semantic_functions, go_semantic_callsites);
    let mut output = RefinedCallOutput::empty();
    output.edges.extend(
        db.solver_derived_edges()
            .iter()
            .filter_map(|edge| refined_edge_from_solver_edge(interner, &index, edge)),
    );
    output
}

fn refined_edge_from_solver_edge(
    interner: &StableKeyInterner,
    index: &SolverProjectionIndex<'_>,
    edge: &DerivedEdgeFact,
) -> Option<RefinedCallEdgeFact> {
    if edge.provenance.constraint_kind != "call_constraint" {
        return None;
    }
    let caller = index.function_by_node.get(&edge.source).copied()?;
    let target_function = index.function_by_node.get(&edge.target).copied()?;
    let site = index.callsite_for_solver_edge(interner, edge)?;
    if site.caller != caller {
        return None;
    }

    Some(RefinedCallEdgeFact {
        id: crate::analysis_neutral::ids::RefinedCallEdgeId(0),
        site: site.id,
        base_target: None,
        caller,
        target_function: Some(target_function),
        target_symbol: None,
        synthetic_target: None,
        language: site.language,
        edge_kind: solver_edge_kind_for_site(site),
        algorithm: solver_algorithm_for_site(site),
        tier: RefinedCallTier::PointsToAssisted,
        status: call_status_for_solver_edge(edge.status),
        reason: unresolved_reason_for_solver_edge(edge.status),
        provenance: CallProvenance::Model,
        precision: call_precision_for_solver_edge(edge.precision),
        validation: RefinedCallValidation::ReferentiallyValidated,
        confidence: confidence_for_solver_edge(edge.status),
        evidence: solver_evidence(edge),
        input_stable_keys: solver_input_stable_keys(interner, edge),
        stable_key: stable_refined_call_key_from_solver_edge(interner, site, edge),
    })
}

struct SolverProjectionIndex<'a> {
    function_by_node: BTreeMap<crate::analysis_neutral::ids::SemanticNodeId, FunctionId>,
    callsite_by_stable_key: BTreeMap<String, &'a CallSiteFact>,
}

impl<'a> SolverProjectionIndex<'a> {
    fn new(
        db: &'a impl AnalysisHost,
        go_semantic_functions: &'a [GoSemanticFunctionInput],
        go_semantic_callsites: &'a [GoSemanticCallsiteInput],
    ) -> Self {
        let function_by_node = db
            .semantic_nodes()
            .iter()
            .filter_map(|node| {
                let NodeKind::Function(function) = node.kind else {
                    return None;
                };
                Some((node.id, function))
            })
            .collect();
        let callsite_by_id = db
            .call_sites()
            .iter()
            .map(|site| (site.id, site))
            .collect::<BTreeMap<_, _>>();
        let callsite_id_by_node = db
            .semantic_nodes()
            .iter()
            .filter_map(|node| {
                let NodeKind::Callsite(callsite) = node.kind else {
                    return None;
                };
                Some((node.id, callsite))
            })
            .collect::<BTreeMap<_, _>>();
        let mut callsite_by_stable_key = db
            .call_sites()
            .iter()
            .map(|site| (db.resolve_stable_key(site.stable_key).to_string(), site))
            .collect::<BTreeMap<_, _>>();
        for constraint in db.semantic_constraints() {
            let ConstraintKind::CallConstraint { callsite } = &constraint.kind else {
                continue;
            };
            let Some(site) = callsite_id_by_node
                .get(callsite)
                .and_then(|callsite| callsite_by_id.get(callsite))
                .copied()
            else {
                continue;
            };
            callsite_by_stable_key.insert(
                db.resolve_stable_key(constraint.stable_key).to_string(),
                site,
            );
        }
        for callsite in go_semantic_callsites {
            let Some(site) =
                core_callsite_for_go_semantic_callsite(db, callsite, go_semantic_functions)
            else {
                continue;
            };
            callsite_by_stable_key
                .insert(db.resolve_stable_key(callsite.stable_key).to_string(), site);
        }
        Self {
            function_by_node,
            callsite_by_stable_key,
        }
    }

    fn callsite_for_solver_edge(
        &self,
        interner: &StableKeyInterner,
        edge: &DerivedEdgeFact,
    ) -> Option<&'a CallSiteFact> {
        edge.provenance.contributing_facts.iter().find_map(|fact| {
            self.callsite_by_stable_key
                .get(interner.resolve(fact.stable_key).as_ref())
                .copied()
        })
    }
}

fn core_callsite_for_go_semantic_callsite<'a>(
    db: &'a impl AnalysisHost,
    callsite: &GoSemanticCallsiteInput,
    go_semantic_functions: &[GoSemanticFunctionInput],
) -> Option<&'a CallSiteFact> {
    let file = callsite.file?;
    let span = callsite.span.as_ref()?;
    let mut candidates = db
        .call_sites()
        .iter()
        .filter(|site| {
            site.language == Language::Go && site.file == file && same_byte_span(&site.span, span)
        })
        .collect::<Vec<_>>();
    let caller =
        core_function_for_go_semantic_function(db, &callsite.caller, go_semantic_functions);
    if let Some(caller) = caller {
        let caller_matches = candidates
            .iter()
            .copied()
            .filter(|site| site.caller == caller)
            .collect::<Vec<_>>();
        if !caller_matches.is_empty() {
            candidates = caller_matches;
        }
    }
    let dynamic_matches = candidates
        .iter()
        .copied()
        .filter(|site| {
            matches!(
                site.status,
                CallTargetStatus::Unresolved | CallTargetStatus::Ambiguous
            )
        })
        .collect::<Vec<_>>();
    if !dynamic_matches.is_empty() {
        candidates = dynamic_matches;
    }
    candidates
        .into_iter()
        .min_by_key(|site| db.resolve_stable_key(site.stable_key))
}

fn core_function_for_go_semantic_function(
    db: &impl AnalysisHost,
    qualified: &str,
    go_semantic_functions: &[GoSemanticFunctionInput],
) -> Option<FunctionId> {
    go_semantic_functions
        .iter()
        .filter(|function| function.qualified == qualified)
        .filter_map(|function| core_function_for_go_semantic_fact(db, function))
        .next()
}

fn core_function_for_go_semantic_fact(
    db: &impl AnalysisHost,
    function: &GoSemanticFunctionInput,
) -> Option<FunctionId> {
    let file = function.file?;
    let span = function.span.as_ref()?;
    matching_core_function_for_go_semantic_span(db, file, &function.name, span).map(|core| core.id)
}

fn matching_core_function_for_go_semantic_span<'a>(
    db: &'a impl AnalysisHost,
    file: FileId,
    name: &str,
    span: &Span,
) -> Option<&'a crate::analysis_api::FunctionFact> {
    let bucket = db
        .functions()
        .iter()
        .filter(|core| core.language == Language::Go && core.file == file && core.name == name)
        .collect::<Vec<_>>();
    if let Some(exact) = bucket
        .iter()
        .copied()
        .find(|core| same_byte_span(&core.span, span))
    {
        return Some(exact);
    }
    if span.start_byte != span.end_byte {
        return None;
    }
    bucket
        .iter()
        .copied()
        .filter(|core| {
            core.span.start_byte <= span.start_byte && span.start_byte <= core.span.end_byte
        })
        .min_by_key(|core| core.span.end_byte.saturating_sub(core.span.start_byte))
}

fn same_byte_span(left: &Span, right: &Span) -> bool {
    left.file == right.file
        && left.start_byte == right.start_byte
        && left.end_byte == right.end_byte
}

fn solver_edge_kind_for_site(site: &CallSiteFact) -> CallEdgeKind {
    match site.kind {
        CallSyntaxKind::Method | CallSyntaxKind::Member => CallEdgeKind::Method,
        CallSyntaxKind::Constructor | CallSyntaxKind::New => CallEdgeKind::Constructor,
        CallSyntaxKind::GoRoutine => CallEdgeKind::Spawn,
        CallSyntaxKind::Deferred => CallEdgeKind::Deferred,
        CallSyntaxKind::FunctionValue => CallEdgeKind::FunctionValue,
        CallSyntaxKind::Function => CallEdgeKind::FunctionValue,
        _ => CallEdgeKind::Unknown,
    }
}

fn solver_algorithm_for_site(site: &CallSiteFact) -> CallAlgorithm {
    match site.language {
        Language::Go => CallAlgorithm::GoRta,
        language if language.is_ts_family() => CallAlgorithm::PointsTo,
        _ => CallAlgorithm::PointsTo,
    }
}

fn call_status_for_solver_edge(status: PointsToStatus) -> CallTargetStatus {
    match status {
        PointsToStatus::Present => CallTargetStatus::Resolved,
        PointsToStatus::Unknown => CallTargetStatus::Unresolved,
        PointsToStatus::Unsupported => CallTargetStatus::Unsupported,
        PointsToStatus::SetupMissing => CallTargetStatus::SetupMissing,
        PointsToStatus::BudgetExceeded => CallTargetStatus::BudgetExceeded,
    }
}

fn unresolved_reason_for_solver_edge(status: PointsToStatus) -> Option<UnresolvedCallReason> {
    match status {
        PointsToStatus::BudgetExceeded => Some(UnresolvedCallReason::BudgetExceeded),
        PointsToStatus::SetupMissing => Some(UnresolvedCallReason::SetupMissing),
        PointsToStatus::Unsupported => Some(UnresolvedCallReason::UnsupportedSyntax),
        PointsToStatus::Unknown => Some(UnresolvedCallReason::Unknown),
        PointsToStatus::Present => None,
    }
}

fn call_precision_for_solver_edge(precision: PointsToPrecision) -> CallPrecision {
    match precision {
        PointsToPrecision::FlowInsensitive | PointsToPrecision::LocalFlowSensitive => {
            CallPrecision::SetupAware
        }
        PointsToPrecision::SummaryProjected | PointsToPrecision::Heuristic => {
            CallPrecision::Heuristic
        }
        PointsToPrecision::Unknown => CallPrecision::Unknown,
        PointsToPrecision::Unsupported => CallPrecision::Unsupported,
    }
}

fn confidence_for_solver_edge(status: PointsToStatus) -> RefinedCallConfidence {
    match status {
        PointsToStatus::Present => RefinedCallConfidence::Medium,
        PointsToStatus::BudgetExceeded => RefinedCallConfidence::Low,
        PointsToStatus::Unknown | PointsToStatus::Unsupported | PointsToStatus::SetupMissing => {
            RefinedCallConfidence::Low
        }
    }
}

fn solver_evidence(edge: &DerivedEdgeFact) -> Vec<String> {
    vec![
        "solver_derived_edge".to_string(),
        format!("constraint:{}", edge.provenance.constraint_kind),
    ]
}

fn solver_input_stable_keys(interner: &StableKeyInterner, edge: &DerivedEdgeFact) -> Vec<String> {
    let mut keys = vec![interner.resolve(edge.stable_key).to_string()];
    keys.extend(
        edge.provenance
            .contributing_facts
            .iter()
            .map(|fact| interner.resolve(fact.stable_key).to_string()),
    );
    keys
}

fn stable_refined_call_key_from_solver_edge(
    interner: &StableKeyInterner,
    site: &CallSiteFact,
    edge: &DerivedEdgeFact,
) -> StableKeyId {
    stable_key_from_key_parts(
        interner,
        FactFamily::RefinedCallEdge,
        [
            (
                "tier",
                KeyPart::Text(&format!("{:?}", RefinedCallTier::PointsToAssisted)),
            ),
            ("solver_edge", KeyPart::Key(edge.stable_key)),
            ("site", KeyPart::Key(site.stable_key)),
        ],
    )
}

fn validation_for_target(target: &CallTargetFact) -> RefinedCallValidation {
    match target.status {
        CallTargetStatus::Rejected => RefinedCallValidation::Rejected,
        _ => RefinedCallValidation::Native,
    }
}

fn confidence_for_target(target: &CallTargetFact) -> RefinedCallConfidence {
    match target.status {
        CallTargetStatus::Resolved => RefinedCallConfidence::High,
        CallTargetStatus::Ambiguous => RefinedCallConfidence::Medium,
        CallTargetStatus::Unresolved
        | CallTargetStatus::Unsupported
        | CallTargetStatus::SetupMissing
        | CallTargetStatus::BudgetExceeded
        | CallTargetStatus::Rejected => RefinedCallConfidence::Low,
    }
}

pub fn stable_refined_call_key(
    interner: &StableKeyInterner,
    tier: RefinedCallTier,
    base_target_key: &str,
    site_key: &str,
) -> StableKeyId {
    stable_key_from_parts(
        interner,
        FactFamily::RefinedCallEdge,
        &[
            ("tier", format!("{tier:?}")),
            ("base_target", base_target_key.to_string()),
            ("site", site_key.to_string()),
        ],
    )
}

#[allow(clippy::too_many_arguments)]
pub fn refined_calls_output_digest(
    db: &impl AnalysisHost,
    interner: &StableKeyInterner,
    manifest: &ProviderManifest,
    input_snapshot: &InputSnapshot,
    calls_output_digest: &Digest,
    entrypoints_output_digest: &Digest,
    direct_summaries_output_digest: &Digest,
    type_value_alias_output_digest: &Digest,
    extensions_output_digest: &Digest,
    solver_output_digest: &Digest,
    output: &RefinedCallOutput,
) -> Result<Digest, crate::analysis_neutral::error::AnalysisError> {
    let upstream = vec![
        calls_output_digest.clone(),
        entrypoints_output_digest.clone(),
        direct_summaries_output_digest.clone(),
        type_value_alias_output_digest.clone(),
        extensions_output_digest.clone(),
        solver_output_digest.clone(),
    ];
    let mut parts = vec![
        format!("provider_id={}", manifest.id),
        format!("provider_version={}", manifest.provider_version()),
        format!("schema={}", manifest.primary_schema_label()),
        format!("parameters={}", refined_calls_provider_parameter_digest()),
        format!(
            "input_parameters={}",
            refined_calls_provider_parameter_digest_for_snapshot(input_snapshot, &upstream)
        ),
        format!("config={}", input_snapshot.config.digest),
        format!("calls={calls_output_digest}"),
        format!("entrypoints={entrypoints_output_digest}"),
        format!("direct_summaries={direct_summaries_output_digest}"),
        format!("type_value_alias={type_value_alias_output_digest}"),
        format!("extensions={extensions_output_digest}"),
        format!("solver={solver_output_digest}"),
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
    for edge in &output.edges {
        parts.push(format!(
            "refined_call_edge={}",
            refined_call_edge_payload(db, interner, edge)?
        ));
    }
    if output.edges.is_empty() {
        parts.push("refined_calls_output=empty".to_string());
    }

    parts.sort();
    let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    Ok(Digest::from_parts(
        DigestKind::ProviderOutput,
        "refined_calls_output",
        &refs,
    ))
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

fn refined_call_edge_payload(
    db: &impl AnalysisHost,
    interner: &StableKeyInterner,
    edge: &RefinedCallEdgeFact,
) -> Result<String, crate::analysis_neutral::error::AnalysisError> {
    let stable_key = interner.resolve(edge.stable_key);
    let payload = RefinedCallEdgeDigest {
        site_key: relation_stable_key(db, interner, FactFamily::CallSite, edge.site.0)?,
        base_target_key: edge
            .base_target
            .map(|target| relation_stable_key(db, interner, FactFamily::CallTarget, target.0))
            .transpose()?,
        caller_key: relation_stable_key(db, interner, FactFamily::Function, edge.caller.0)?,
        target_function_key: edge
            .target_function
            .map(|function| relation_stable_key(db, interner, FactFamily::Function, function.0))
            .transpose()?,
        target_symbol_key: edge
            .target_symbol
            .map(|symbol| relation_stable_key(db, interner, FactFamily::Symbol, symbol.0))
            .transpose()?,
        synthetic_target: edge.synthetic_target.as_deref(),
        language: edge.language,
        edge_kind: edge.edge_kind,
        algorithm: edge.algorithm,
        tier: edge.tier,
        status: edge.status,
        reason: edge.reason,
        provenance: edge.provenance,
        precision: edge.precision,
        validation: edge.validation,
        confidence: edge.confidence,
        evidence: &edge.evidence,
        input_stable_keys: &edge.input_stable_keys,
        stable_key: stable_key.as_ref(),
    };
    serde_json::to_string(&payload).map_err(|error| {
        crate::analysis_neutral::error::AnalysisError::InvalidFact {
            provider: REFINED_CALLS_PROVIDER_ID,
            reason: format!("failed to serialize refined call digest payload: {error}"),
        }
    })
}

fn relation_stable_key(
    db: &impl AnalysisHost,
    interner: &StableKeyInterner,
    family: FactFamily,
    run_id: u64,
) -> Result<String, crate::analysis_neutral::error::AnalysisError> {
    let relation_exists = match family {
        FactFamily::CallSite => db.call_sites().iter().any(|site| site.id.0 == run_id),
        FactFamily::CallTarget => db.call_targets().iter().any(|target| target.id.0 == run_id),
        FactFamily::Function => db
            .functions()
            .iter()
            .any(|function| function.id.0 == run_id),
        FactFamily::Symbol => db.symbols().iter().any(|symbol| symbol.id.0 == run_id),
        _ => false,
    };
    if !relation_exists {
        return Err(crate::analysis_neutral::error::AnalysisError::InvalidFact {
            provider: REFINED_CALLS_PROVIDER_ID,
            reason: format!("dangling {} relation with run id {run_id}", family.label()),
        });
    }
    if let Some(metadata) = db.metadata_for(FactRef::new(family, run_id)) {
        return Ok(db.resolve_stable_key(metadata.stable_key).to_string());
    }

    let stable_key = match family {
        FactFamily::CallSite => db
            .call_sites()
            .iter()
            .find(|site| site.id.0 == run_id)
            .map(|site| interner.resolve(site.stable_key).to_string()),
        FactFamily::CallTarget => db
            .call_targets()
            .iter()
            .find(|target| target.id.0 == run_id)
            .map(|target| interner.resolve(target.stable_key).to_string()),
        FactFamily::Function => db
            .functions()
            .iter()
            .find(|function| function.id.0 == run_id)
            .map(|function| {
                stable_key_text_from_parts(
                    FactFamily::Function,
                    &[
                        ("path", db.path_for(function.file)),
                        ("language", format!("{:?}", function.language)),
                        ("name", function.name.clone()),
                        ("span", stable_span(&function.span)),
                    ],
                )
            }),
        FactFamily::Symbol => db
            .symbols()
            .iter()
            .find(|symbol| symbol.id.0 == run_id)
            .map(|symbol| interner.resolve(symbol.stable_key).to_string()),
        _ => None,
    };
    stable_key.ok_or_else(
        || crate::analysis_neutral::error::AnalysisError::InvalidFact {
            provider: REFINED_CALLS_PROVIDER_ID,
            reason: format!("dangling {} relation with run id {run_id}", family.label()),
        },
    )
}

fn stable_span(span: &Span) -> String {
    format!(
        "{}-{}:{}:{}-{}:{}",
        span.start_byte,
        span.end_byte,
        span.start_line,
        span.start_col,
        span.end_line,
        span.end_col
    )
}

#[derive(Serialize)]
struct RefinedCallEdgeDigest<'a> {
    site_key: String,
    base_target_key: Option<String>,
    caller_key: String,
    target_function_key: Option<String>,
    target_symbol_key: Option<String>,
    synthetic_target: Option<&'a str>,
    language: Language,
    edge_kind: CallEdgeKind,
    algorithm: CallAlgorithm,
    tier: RefinedCallTier,
    status: CallTargetStatus,
    reason: Option<UnresolvedCallReason>,
    provenance: CallProvenance,
    precision: CallPrecision,
    validation: RefinedCallValidation,
    confidence: RefinedCallConfidence,
    evidence: &'a [String],
    input_stable_keys: &'a [String],
    stable_key: &'a str,
}

fn failed_provider_output(message: String) -> RefinedCallsProviderOutput {
    let mut cache_stats = CacheStats::default();
    cache_stats.record_recompute();
    RefinedCallsProviderOutput {
        diagnostics: vec![provider_error_diagnostic(message)],
        cache_stats,
        output_digest: None,
        execution: ProviderExecution::Failed {
            stage: ProviderFailureStage::Validation,
            reason: ProviderFailureReason::ValidationRejected,
        },
    }
}

fn provider_error_diagnostic(message: String) -> Diagnostic {
    Diagnostic::error(
        "polint/internal",
        "<workspace>",
        DiagnosticRange::point(1, 1),
        format!("Refined calls provider failed: {message}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_api::{
        CachePolicy, FunctionFact, GoLifecycleSnapshot, InputComponentStatus, PrecisionCeiling,
        ProviderKind, SchemaVersion, SymbolFact, SymbolKind, SymbolNamespace, SymbolPrecision,
        TsJsLifecycleSnapshot,
    };
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::calls::facts::{CallCallee, CallSiteFact};
    use crate::analysis_neutral::calls::store::CallOutput;
    use crate::analysis_neutral::ids::{CallSiteId, CallTargetId, MirBodyId, MirOpId};

    fn manifest() -> ProviderManifest {
        ProviderManifest {
            id: REFINED_CALLS_PROVIDER_ID,
            kind: ProviderKind::WholeRepoDerived,
            inputs: &[],
            outputs: &["refined_call_edges"],
            language_ids: &[],
            cache_policy: CachePolicy::InMemoryDerived,
            schema_versions: &[SchemaVersion {
                name: "refined-call-test",
                version: 1,
            }],
            precision_ceiling: PrecisionCeiling::SetupAware,
        }
    }

    fn snapshot() -> InputSnapshot {
        InputSnapshot {
            schema_version: "test".to_string(),
            files: Vec::new(),
            config: InputComponent {
                name: "config".to_string(),
                status: InputComponentStatus::Present,
                digest: Digest::absent(DigestKind::Config, "test"),
                detail: Vec::new(),
            },
            go_lifecycle: GoLifecycleSnapshot {
                components: Vec::new(),
            },
            ts_js_lifecycle: TsJsLifecycleSnapshot {
                components: Vec::new(),
            },
            rules: Vec::new(),
            models: Vec::new(),
            extensions: Vec::new(),
            tool_invocations: Vec::new(),
            provider_schemas: Vec::new(),
        }
    }

    fn provider_run(db: &mut LocalAnalysisDb) -> RefinedCallsProviderOutput {
        let upstream = Digest::absent(DigestKind::ProviderOutput, "test");
        derive_refined_calls_with_cache_stats(
            db,
            &snapshot(),
            &manifest(),
            upstream.clone(),
            upstream.clone(),
            upstream.clone(),
            upstream.clone(),
            upstream.clone(),
            upstream,
            &[],
            &[],
        )
    }

    fn complete_graph(dense_offset: u64, include_target_function: bool) -> LocalAnalysisDb {
        let mut db = LocalAnalysisDb::new();
        let file = db.add_file(
            "src/app.ts".into(),
            "src/app.ts".to_string(),
            "function caller() { callee(); }".to_string(),
        );
        for index in 0..dense_offset {
            db.push_function(FunctionFact::new(
                FunctionId::from_raw(0),
                file,
                format!("padding_{index}"),
                Span::point(file, 1, 1),
                Language::TypeScript,
                false,
                false,
                1,
                Vec::new(),
            ));
        }
        let caller = db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "caller".to_string(),
            Span::point(file, 1, 1),
            Language::TypeScript,
            false,
            true,
            1,
            Vec::new(),
        ));
        let target = if include_target_function {
            db.push_function(FunctionFact::new(
                FunctionId::from_raw(0),
                file,
                "callee".to_string(),
                Span::point(file, 1, 20),
                Language::TypeScript,
                false,
                true,
                1,
                Vec::new(),
            ))
        } else {
            FunctionId::from_raw(caller.0 + 1)
        };
        let site = CallSiteId(10 + dense_offset);
        let call_target = CallTargetId(20 + dense_offset);
        let symbol = SymbolId::from_raw(30 + dense_offset);
        let interner = db.stable_key_interner();
        db.replace_symbol_graph_facts(
            vec![SymbolFact::new(
                symbol,
                Language::TypeScript,
                "callee".to_string(),
                "callee".to_string(),
                SymbolKind::Function,
                SymbolNamespace::Value,
                Some(file),
                None,
                None,
                None,
                Some(Span::point(file, 1, 20)),
                false,
                interner.intern("symbol:callee"),
                SymbolPrecision::ExactLocal,
            )],
            Vec::new(),
            Vec::new(),
        );
        db.replace_call_facts(CallOutput {
            sites: vec![CallSiteFact {
                in_throw: false,
                id: site,
                language: Language::TypeScript,
                file,
                caller,
                owner_symbol: None,
                body: MirBodyId(dense_offset),
                operation: MirOpId(dense_offset),
                span: Span::point(file, 1, 21),
                kind: CallSyntaxKind::Function,
                callee: CallCallee::Identifier {
                    reference: None,
                    name: "callee".to_string(),
                },
                receiver: None,
                arguments: Vec::new(),
                result: None,
                status: CallTargetStatus::Resolved,
                precision: CallPrecision::Exact,
                stable_key: interner.intern("call-site:caller-to-callee"),
            }],
            targets: vec![CallTargetFact {
                id: call_target,
                site,
                caller,
                target_function: Some(target),
                target_symbol: Some(symbol),
                edge_kind: CallEdgeKind::Direct,
                algorithm: CallAlgorithm::DirectReference,
                status: CallTargetStatus::Resolved,
                reason: None,
                provenance: CallProvenance::Native,
                precision: CallPrecision::Exact,
                stable_key: interner.intern("call-target:caller-to-callee"),
            }],
            unresolved: Vec::new(),
        })
        .expect("complete call graph");
        db
    }

    #[test]
    fn provider_digest_is_invariant_to_production_graph_dense_id_remapping() {
        let mut first_db = complete_graph(0, true);
        let mut remapped_db = complete_graph(100, true);

        let first = provider_run(&mut first_db);
        let remapped = provider_run(&mut remapped_db);

        assert!(first.output_digest.is_some());
        assert!(remapped.output_digest.is_some());
        assert_ne!(
            first_db.refined_call_edges()[0].site,
            remapped_db.refined_call_edges()[0].site
        );
        assert_eq!(first.output_digest, remapped.output_digest);
        assert_eq!(
            first_db.resolve_stable_key(first_db.refined_call_edges()[0].stable_key),
            remapped_db.resolve_stable_key(remapped_db.refined_call_edges()[0].stable_key)
        );
    }

    #[test]
    fn provider_rejects_dangling_relation_without_publishing_digest() {
        let mut db = complete_graph(0, false);

        let output = provider_run(&mut db);

        assert!(output.output_digest.is_none());
        assert!(matches!(
            output.execution,
            ProviderExecution::Failed {
                stage: ProviderFailureStage::Validation,
                reason: ProviderFailureReason::ValidationRejected,
            }
        ));
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.message.contains("dangling Function relation") })
        );
        assert!(db.refined_call_edges().is_empty());
    }
}
