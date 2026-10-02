use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::analysis_api::{
    CacheStats, Digest, DigestKind, InputComponent, InputSnapshot, ProviderExecution,
    ProviderFailureReason, ProviderFailureStage,
};
use crate::analysis_api::{FactFamily, FactRef, ProviderManifest};
use crate::analysis_api::{FunctionFact, SymbolFact};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::calls::cache_key::calls_provider_parameter_digest;
use crate::analysis_neutral::calls::direct::resolve_direct_call_targets;
use crate::analysis_neutral::calls::extract::extract_call_sites;
use crate::analysis_neutral::calls::store::CallOutput;
use crate::analysis_neutral::calls::typed::{TypedCallInputs, typed_call_targets};
use crate::analysis_neutral::calls::unresolved::derive_unresolved_calls;
use crate::analysis_neutral::ids::CallSiteId;
use crate::internal_core::{Diagnostic, DiagnosticRange};
use crate::internal_core::{FunctionId, SymbolId};

#[derive(Debug, Clone, Default)]
pub struct CallsProviderOutput {
    pub diagnostics: Vec<Diagnostic>,
    pub cache_stats: CacheStats,
    pub output_digest: Option<Digest>,
    pub execution: ProviderExecution,
}

/// The calls a typed frontend resolved, and the digest of the provider that
/// produced them, for [`derive_calls_with_cache_stats`].
pub struct TypedCalls<'a> {
    pub inputs: &'a TypedCallInputs,
    pub output_digest: Digest,
}

#[allow(clippy::too_many_arguments)]
pub fn derive_calls_with_cache_stats(
    db: &mut impl AnalysisHost,
    input_snapshot: &InputSnapshot,
    manifest: &ProviderManifest,
    semantic_mir_output_digest: Digest,
    cfg_output_digest: Digest,
    symbol_graph_output_digest: Digest,
    module_topology_output_digest: Digest,
    upstream_syntax_output_digests: Vec<Digest>,
    typed: TypedCalls<'_>,
) -> CallsProviderOutput {
    let mut started = std::time::Instant::now();
    let mut checkpoint = |step: &'static str| {
        tracing::debug!(target: "polint::kernel::stage", provider = "polint.calls", step, elapsed_ms = started.elapsed().as_millis() as u64, "provider step");
        started = std::time::Instant::now();
    };
    let mut sites = extract_call_sites(db);
    checkpoint("extract_sites");
    let mut targets = resolve_direct_call_targets(db, &sites);
    checkpoint("direct_targets");
    let interner = db.stable_key_interner();
    let (typed_targets, typed_sites, report) = typed_call_targets(&interner, &sites, typed.inputs);
    checkpoint("typed_join");
    if report.sites > 0 {
        tracing::debug!(
            target: "polint::kernel::stage",
            provider = "polint.calls",
            sites = report.sites,
            joined = report.joined,
            resolved = typed_sites.len(),
            targets = report.targets,
            "typed call facts joined"
        );
    }
    // Typed facts decide what a covered site calls; a same-named symbol does not.
    targets.retain(|target| !typed_sites.contains(&target.site));
    targets.extend(typed_targets);
    targets.sort_by(|left, right| {
        (interner.resolve(left.stable_key), left.site)
            .cmp(&(interner.resolve(right.stable_key), right.site))
    });
    targets.dedup_by(|left, right| left.stable_key == right.stable_key);
    for (index, target) in targets.iter_mut().enumerate() {
        target.id = crate::analysis_neutral::ids::CallTargetId(index as u64);
    }
    let resolved_sites = targets
        .iter()
        .filter(|target| {
            target.status == crate::analysis_neutral::calls::facts::CallTargetStatus::Resolved
        })
        .map(|target| target.site)
        .collect::<BTreeSet<_>>();
    // A typed site is as precise as its least precise listed candidate: an exact
    // static callee, a variable-type-analysis candidate set, or a class-hierarchy
    // one. A budget stop for candidates past the limit lists none; it makes the
    // analysis incomplete rather than the listed candidates less precise.
    let mut typed_precision =
        BTreeMap::<CallSiteId, crate::analysis_neutral::calls::facts::CallPrecision>::new();
    for target in targets.iter().filter(|target| {
        typed_sites.contains(&target.site)
            && target.status == crate::analysis_neutral::calls::facts::CallTargetStatus::Resolved
    }) {
        typed_precision
            .entry(target.site)
            .and_modify(|precision| *precision = (*precision).max(target.precision))
            .or_insert(target.precision);
    }
    for site in &mut sites {
        if resolved_sites.contains(&site.id) {
            site.status = crate::analysis_neutral::calls::facts::CallTargetStatus::Resolved;
            site.precision = typed_precision
                .get(&site.id)
                .copied()
                .unwrap_or(crate::analysis_neutral::calls::facts::CallPrecision::SetupAware);
        }
    }
    let unresolved = derive_unresolved_calls(db, &sites)
        .into_iter()
        .filter(|row| !resolved_sites.contains(&row.site))
        .collect();
    let output = CallOutput {
        sites,
        targets,
        unresolved,
    }
    .normalized(&db.stable_key_interner());
    checkpoint("unresolved_normalize");
    let output_digest = calls_output_digest(
        db,
        manifest,
        input_snapshot,
        &semantic_mir_output_digest,
        &cfg_output_digest,
        &symbol_graph_output_digest,
        &module_topology_output_digest,
        &upstream_syntax_output_digests,
        &typed.output_digest,
        &output,
    );
    checkpoint("digest");
    let mut cache_stats = CacheStats::default();
    cache_stats.record_recompute();

    let replaced = db.replace_call_facts(output);
    checkpoint("store_metadata");
    match replaced {
        Ok(()) => CallsProviderOutput {
            diagnostics: Vec::new(),
            cache_stats,
            output_digest: Some(output_digest),
            execution: Default::default(),
        },
        Err(error) => CallsProviderOutput {
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

#[allow(clippy::too_many_arguments)]
fn calls_output_digest(
    db: &impl AnalysisHost,
    manifest: &ProviderManifest,
    input_snapshot: &InputSnapshot,
    semantic_mir_output_digest: &Digest,
    cfg_output_digest: &Digest,
    symbol_graph_output_digest: &Digest,
    module_topology_output_digest: &Digest,
    upstream_syntax_output_digests: &[Digest],
    typed_output_digest: &Digest,
    output: &CallOutput,
) -> Digest {
    let mut parts = vec![
        format!("provider_id={}", manifest.id),
        format!("provider_version={}", manifest.provider_version()),
        format!("schema={}", manifest.primary_schema_label()),
        format!("parameters={}", calls_provider_parameter_digest()),
        format!("config={}", input_snapshot.config.digest),
        format!("semantic_mir={semantic_mir_output_digest}"),
        format!("cfg={cfg_output_digest}"),
        format!("symbol_graph={symbol_graph_output_digest}"),
        format!("module_topology={module_topology_output_digest}"),
        format!("typed_calls={typed_output_digest}"),
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

    let site_keys = call_site_key_map(db, output);
    let function_keys = function_key_map(db);
    let symbol_keys = symbol_key_map(db);
    parts.extend(output.sites.iter().map(|site| {
        format!(
            "call_site={} language={:?} span={} kind={:?} status={:?} precision={:?} callee={}",
            db.resolve_stable_key(site.stable_key),
            site.language,
            span_part(&site.span),
            site.kind,
            site.status,
            site.precision,
            callee_part(&site.callee),
        )
    }));
    parts.extend(output.targets.iter().map(|target| {
        format!(
            "call_target={} site={} edge={:?} algorithm={:?} status={:?} reason={} provenance={:?} precision={:?} target_function={} target_symbol={}",
            db.resolve_stable_key(target.stable_key),
            stable_site_key(&site_keys, target.site),
            target.edge_kind,
            target.algorithm,
            target.status,
            target.reason.map(|reason| format!("{reason:?}")).unwrap_or_else(|| "none".to_string()),
            target.provenance,
            target.precision,
            stable_function_key(&function_keys, target.target_function),
            stable_symbol_key(&symbol_keys, target.target_symbol),
        )
    }));
    parts.extend(output.targets.iter().filter_map(|target| {
        target.synthetic_target.as_ref().map(|synthetic| {
            format!(
                "call_target_synthetic={} target={synthetic}",
                db.resolve_stable_key(target.stable_key)
            )
        })
    }));
    parts.extend(output.unresolved.iter().map(|unresolved| {
        format!(
            "unresolved_call={} site={} algorithm={:?} status={:?} reason={:?} provenance={:?} precision={:?}",
            db.resolve_stable_key(unresolved.stable_key),
            stable_site_key(&site_keys, unresolved.site),
            unresolved.algorithm,
            unresolved.status,
            unresolved.reason,
            unresolved.provenance,
            unresolved.precision,
        )
    }));
    if output.sites.is_empty() && output.targets.is_empty() && output.unresolved.is_empty() {
        parts.push("call_output=empty".to_string());
    }

    parts.sort();
    let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    Digest::from_parts(DigestKind::ProviderOutput, "calls_output", &refs)
}

fn extend_component_parts(parts: &mut Vec<String>, prefix: &str, components: &[InputComponent]) {
    parts.extend(components.iter().map(|component| {
        format!(
            "{prefix}:{}:{:?}:{}",
            component.name, component.status, component.digest
        )
    }));
}

fn call_site_key_map(db: &impl AnalysisHost, output: &CallOutput) -> BTreeMap<CallSiteId, String> {
    output
        .sites
        .iter()
        .map(|site| (site.id, db.resolve_stable_key(site.stable_key).to_string()))
        .collect()
}

fn function_key_map(db: &impl AnalysisHost) -> BTreeMap<FunctionId, String> {
    db.functions()
        .iter()
        .map(|function| (function.id, function_key(db, function)))
        .collect()
}

fn function_key(db: &impl AnalysisHost, function: &FunctionFact) -> String {
    db.metadata_for(FactRef::new(FactFamily::Function, function.id.0))
        .map(|metadata| db.resolve_stable_key(metadata.stable_key).to_string())
        .unwrap_or_else(|| {
            format!(
                "{}:{}:{}",
                db.path_for(function.file),
                function.name,
                span_part(&function.span)
            )
        })
}

fn symbol_key_map(db: &impl AnalysisHost) -> BTreeMap<SymbolId, String> {
    db.symbols()
        .iter()
        .map(|symbol| (symbol.id, symbol_key(db, symbol)))
        .collect()
}

fn symbol_key(db: &impl AnalysisHost, symbol: &SymbolFact) -> String {
    db.metadata_for(FactRef::new(FactFamily::Symbol, symbol.id.0))
        .map(|metadata| db.resolve_stable_key(metadata.stable_key).to_string())
        .unwrap_or_else(|| db.resolve_stable_key(symbol.stable_key).to_string())
}

fn stable_site_key(keys: &BTreeMap<CallSiteId, String>, site: CallSiteId) -> String {
    keys.get(&site)
        .cloned()
        .unwrap_or_else(|| "<missing-site>".to_string())
}

fn stable_function_key(
    keys: &BTreeMap<FunctionId, String>,
    function: Option<FunctionId>,
) -> String {
    function
        .map(|function| {
            keys.get(&function)
                .cloned()
                .unwrap_or_else(|| format!("<missing-function:{}>", function.0))
        })
        .unwrap_or_else(|| "none".to_string())
}

fn stable_symbol_key(keys: &BTreeMap<SymbolId, String>, symbol: Option<SymbolId>) -> String {
    symbol
        .map(|symbol| {
            keys.get(&symbol)
                .cloned()
                .unwrap_or_else(|| format!("<missing-symbol:{}>", symbol.0))
        })
        .unwrap_or_else(|| "none".to_string())
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

fn callee_part(callee: &crate::analysis_neutral::calls::facts::CallCallee) -> String {
    match callee {
        crate::analysis_neutral::calls::facts::CallCallee::Identifier { name, .. } => {
            format!("identifier:{name}")
        }
        crate::analysis_neutral::calls::facts::CallCallee::Member { property, .. } => {
            format!("member:{property}")
        }
        crate::analysis_neutral::calls::facts::CallCallee::Index { .. } => "index".to_string(),
        crate::analysis_neutral::calls::facts::CallCallee::Super => "super".to_string(),
        crate::analysis_neutral::calls::facts::CallCallee::Import => "import".to_string(),
        crate::analysis_neutral::calls::facts::CallCallee::FunctionValue { .. } => {
            "function_value".to_string()
        }
        crate::analysis_neutral::calls::facts::CallCallee::Constructor { name, .. } => {
            format!("constructor:{}", name.as_deref().unwrap_or("unknown"))
        }
        crate::analysis_neutral::calls::facts::CallCallee::Unknown { reason } => {
            format!("unknown:{reason:?}")
        }
    }
}

fn provider_error_diagnostic(message: String) -> Diagnostic {
    Diagnostic::error(
        "polint/internal",
        "<workspace>",
        DiagnosticRange::point(1, 1),
        format!("Calls provider failed: {message}"),
    )
}
