use super::cache_key::{
    type_value_alias_provider_parameter_digest,
    type_value_alias_provider_parameter_digest_for_snapshot,
};
use super::facts::{NarrowedTypeFact, TypeFact};
use super::store::TypeValueAliasOutput;
use crate::analysis_api::ProviderManifest;
use crate::analysis_api::{
    CacheStats, Digest, DigestKind, InputComponent, InputSnapshot, ProviderExecution,
    ProviderFailureReason, ProviderFailureStage,
};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::access_paths::facts::AccessPathFact;
use crate::analysis_neutral::aliases::facts::AliasAnswerFact;
use crate::analysis_neutral::points_to::facts::{PointsToConstraintFact, PointsToSetFact};
use crate::analysis_neutral::values::facts::{AllocationTokenFact, ValueFact};
use crate::internal_core::{Diagnostic, DiagnosticRange};
use serde::Serialize;

pub const TYPE_VALUE_ALIAS_PROVIDER_ID: &str = "polint.type_value_alias";

#[derive(Debug, thiserror::Error)]
enum DigestBuildError {
    #[error("dangling {relation} relation: {identity}")]
    DanglingRelation {
        relation: &'static str,
        identity: String,
    },
    #[error("failed to serialize {row_kind} digest payload: {source}")]
    Serialization {
        row_kind: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

type DigestBuildResult<T> = Result<T, DigestBuildError>;

#[derive(Debug, Clone, Default)]
pub struct TypeValueAliasProviderOutput {
    pub diagnostics: Vec<Diagnostic>,
    pub cache_stats: CacheStats,
    pub output_digest: Option<Digest>,
    pub execution: ProviderExecution,
}

#[allow(clippy::too_many_arguments)]
pub fn derive_type_value_alias_with_cache_stats(
    db: &mut impl AnalysisHost,
    input_snapshot: &InputSnapshot,
    manifest: &ProviderManifest,
    semantic_mir_output_digest: Digest,
    cfg_output_digest: Digest,
    calls_output_digest: Digest,
    abstract_domains_output_digest: Digest,
    direct_summaries_output_digest: Digest,
    entrypoints_output_digest: Digest,
    extensions_output_digest: Digest,
    symbol_graph_output_digest: Digest,
    module_topology_output_digest: Digest,
    upstream_syntax_output_digests: Vec<Digest>,
) -> TypeValueAliasProviderOutput {
    let mut started = std::time::Instant::now();
    let mut checkpoint = |step: &'static str| {
        tracing::debug!(target: "polint::kernel::stage", provider = TYPE_VALUE_ALIAS_PROVIDER_ID, step, elapsed_ms = started.elapsed().as_millis() as u64, "provider step");
        started = std::time::Instant::now();
    };
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    debug_assert_eq!(manifest.id, TYPE_VALUE_ALIAS_PROVIDER_ID);
    let mut diagnostics = Vec::new();
    let mut output = super::go::derive_go_type_value_alias(db);
    checkpoint("go_facts");
    let ts_js_output = super::ts_js::derive_ts_js_type_value_alias(db);
    checkpoint("ts_js_facts");
    output.types.types.extend(ts_js_output.types.types);
    output.types.narrowed.extend(ts_js_output.types.narrowed);
    output.values.values.extend(ts_js_output.values.values);
    output
        .values
        .allocations
        .extend(ts_js_output.values.allocations);
    output
        .access_paths
        .access_paths
        .extend(ts_js_output.access_paths.access_paths);
    output = output.normalized(interner);
    let base_merge = super::validate::merge_extension_type_value_alias_base_facts(
        interner,
        output,
        db.extension_facts(),
    );
    output = base_merge.output.normalized(interner);
    diagnostics.extend(base_merge.diagnostics);
    let relation_merge = super::validate::merge_extension_type_value_alias_relation_facts(
        interner,
        output,
        db.extension_facts(),
    );
    output = relation_merge.output;
    diagnostics.extend(relation_merge.diagnostics);
    checkpoint("merge_normalize");
    let mut points_to_constraints =
        crate::analysis_neutral::points_to::constraints::derive_points_to_constraints(
            interner, &output,
        );
    points_to_constraints.extend(std::mem::take(&mut output.points_to.constraints));
    checkpoint("points_to_constraints");
    let points_to_output = crate::analysis_neutral::points_to::solver::output_with_solved_sets(
        interner,
        points_to_constraints,
        crate::analysis_neutral::points_to::solver::PointsToBudget::default(),
    );
    checkpoint("points_to_solve");
    let mut alias_output = crate::analysis_neutral::aliases::provider_stack::derive_alias_answers(
        interner,
        &output.access_paths.access_paths,
        &points_to_output.sets,
    );
    checkpoint("alias_answers");
    alias_output
        .answers
        .extend(std::mem::take(&mut output.aliases.answers));
    output.points_to = points_to_output;
    output.aliases = alias_output;
    output = output.normalized(interner);
    checkpoint("normalize");
    let output_digest = type_value_alias_output_digest(
        db,
        manifest,
        input_snapshot,
        &semantic_mir_output_digest,
        &cfg_output_digest,
        &calls_output_digest,
        &abstract_domains_output_digest,
        &direct_summaries_output_digest,
        &entrypoints_output_digest,
        &extensions_output_digest,
        &symbol_graph_output_digest,
        &module_topology_output_digest,
        &upstream_syntax_output_digests,
        &output,
    );
    checkpoint("digest");
    let mut cache_stats = CacheStats::default();
    cache_stats.record_recompute();

    match output_digest {
        Ok(output_digest) => {
            db.replace_normalized_type_value_alias_facts(output);
            checkpoint("store_metadata");
            TypeValueAliasProviderOutput {
                diagnostics,
                cache_stats,
                output_digest: Some(output_digest),
                execution: Default::default(),
            }
        }
        Err(error) => {
            diagnostics.push(digest_error_diagnostic(&error));
            TypeValueAliasProviderOutput {
                diagnostics,
                cache_stats,
                output_digest: None,
                execution: ProviderExecution::Failed {
                    stage: ProviderFailureStage::Validation,
                    reason: ProviderFailureReason::ValidationRejected,
                },
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn type_value_alias_output_digest<H: AnalysisHost + ?Sized>(
    db: &H,
    manifest: &ProviderManifest,
    input_snapshot: &InputSnapshot,
    semantic_mir_output_digest: &Digest,
    cfg_output_digest: &Digest,
    calls_output_digest: &Digest,
    abstract_domains_output_digest: &Digest,
    direct_summaries_output_digest: &Digest,
    entrypoints_output_digest: &Digest,
    extensions_output_digest: &Digest,
    symbol_graph_output_digest: &Digest,
    module_topology_output_digest: &Digest,
    upstream_syntax_output_digests: &[Digest],
    output: &TypeValueAliasOutput,
) -> DigestBuildResult<Digest> {
    let keys = DigestKeys::new(db, output);
    let mut upstream = vec![
        semantic_mir_output_digest.clone(),
        cfg_output_digest.clone(),
        calls_output_digest.clone(),
        abstract_domains_output_digest.clone(),
        direct_summaries_output_digest.clone(),
        entrypoints_output_digest.clone(),
        extensions_output_digest.clone(),
        symbol_graph_output_digest.clone(),
        module_topology_output_digest.clone(),
    ];
    upstream.extend(upstream_syntax_output_digests.iter().cloned());

    let mut parts = vec![
        format!("provider_id={}", manifest.id),
        format!("provider_version={}", manifest.provider_version()),
        format!("schema={}", manifest.primary_schema_label()),
        format!(
            "parameters={}",
            type_value_alias_provider_parameter_digest()
        ),
        format!(
            "input_parameters={}",
            type_value_alias_provider_parameter_digest_for_snapshot(input_snapshot, &upstream)
        ),
        format!("config={}", input_snapshot.config.digest),
        format!("semantic_mir={semantic_mir_output_digest}"),
        format!("cfg={cfg_output_digest}"),
        format!("calls={calls_output_digest}"),
        format!("abstract_domains={abstract_domains_output_digest}"),
        format!("direct_summaries={direct_summaries_output_digest}"),
        format!("entrypoints={entrypoints_output_digest}"),
        format!("extensions={extensions_output_digest}"),
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
    for row in &output.types.types {
        parts.push(format!("type={}", type_fact_payload(&keys, row)?));
    }
    for row in &output.types.narrowed {
        parts.push(format!(
            "narrowed_type={}",
            narrowed_type_fact_payload(&keys, row)?
        ));
    }
    for row in &output.values.values {
        parts.push(format!("value={}", value_fact_payload(&keys, row)?));
    }
    for row in &output.values.allocations {
        parts.push(format!(
            "allocation={}",
            allocation_fact_payload(&keys, row)?
        ));
    }
    for row in &output.access_paths.access_paths {
        parts.push(format!(
            "access_path={}",
            access_path_fact_payload(&keys, row)?
        ));
    }
    for row in &output.points_to.constraints {
        parts.push(format!(
            "points_to_constraint={}",
            points_to_constraint_payload(&keys, row)?
        ));
    }
    for row in &output.points_to.sets {
        parts.push(format!(
            "points_to_set={}",
            points_to_set_payload(&keys, row)?
        ));
    }
    for row in &output.aliases.answers {
        parts.push(format!(
            "alias_answer={}",
            alias_answer_payload(&keys, row)?
        ));
    }
    if output.types.types.is_empty()
        && output.types.narrowed.is_empty()
        && output.values.values.is_empty()
        && output.values.allocations.is_empty()
        && output.access_paths.access_paths.is_empty()
        && output.points_to.constraints.is_empty()
        && output.points_to.sets.is_empty()
        && output.aliases.answers.is_empty()
    {
        parts.push("type_value_alias_output=empty".to_string());
    }

    parts.sort();
    let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    Ok(Digest::from_parts(
        DigestKind::ProviderOutput,
        "type_value_alias_output",
        &refs,
    ))
}

fn digest_error_diagnostic(error: &DigestBuildError) -> Diagnostic {
    Diagnostic::error(
        "polint/internal",
        "<workspace>",
        DiagnosticRange::point(1, 1),
        "Type/value/alias digest construction failed; facts were not stored.",
    )
    .with_evidence("provider", TYPE_VALUE_ALIAS_PROVIDER_ID)
    .with_evidence("reason", error.to_string())
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

struct DigestKeys<'a, H: AnalysisHost + ?Sized> {
    db: &'a H,
    type_sets: std::collections::BTreeMap<crate::analysis_neutral::ids::TypeSetId, String>,
    values: std::collections::BTreeMap<crate::analysis_neutral::ids::ValueFactId, String>,
    abstract_values:
        std::collections::BTreeMap<crate::analysis_neutral::ids::AbstractValueId, String>,
    allocations:
        std::collections::BTreeMap<crate::analysis_neutral::ids::AllocationTokenId, String>,
    access_paths: std::collections::BTreeMap<crate::analysis_neutral::ids::AccessPathId, String>,
    points_to_variables: std::collections::BTreeMap<crate::analysis_neutral::ids::PtVarId, String>,
}

impl<'a, H: AnalysisHost + ?Sized> DigestKeys<'a, H> {
    fn new(db: &'a H, output: &TypeValueAliasOutput) -> Self {
        let stable_key = |key| db.resolve_stable_key(key).to_string();
        let mut type_sets = std::collections::BTreeMap::new();
        let mut abstract_values = std::collections::BTreeMap::new();
        let mut points_to_fragments = std::collections::BTreeMap::new();
        for row in &output.types.types {
            type_sets
                .entry(row.type_set)
                .or_insert_with(|| stable_key(row.stable_key));
        }
        for row in &output.values.values {
            abstract_values
                .entry(row.value)
                .or_insert_with(|| stable_key(row.stable_key));
        }
        for row in &output.points_to.constraints {
            record_points_to_variable_fragments(
                &mut points_to_fragments,
                &stable_key(row.stable_key),
                &row.kind,
            );
        }
        for row in &output.points_to.sets {
            points_to_fragments
                .entry(row.variable)
                .or_insert_with(std::collections::BTreeSet::new)
                .insert(format!("set:{}", stable_key(row.stable_key)));
        }
        let points_to_variables = points_to_fragments
            .into_iter()
            .map(|(variable, fragments)| (variable, semantic_relation_identity(&fragments)))
            .collect();
        Self {
            db,
            type_sets,
            values: output
                .values
                .values
                .iter()
                .map(|row| (row.id, stable_key(row.stable_key)))
                .collect(),
            abstract_values,
            allocations: output
                .values
                .allocations
                .iter()
                .map(|row| (row.id, stable_key(row.stable_key)))
                .collect(),
            access_paths: output
                .access_paths
                .access_paths
                .iter()
                .map(|row| (row.id, stable_key(row.stable_key)))
                .collect(),
            points_to_variables,
        }
    }

    fn stable_key(&self, key: crate::internal_core::StableKeyId) -> String {
        self.db.resolve_stable_key(key).to_string()
    }

    fn dangling(&self, relation: &'static str, identity: impl std::fmt::Debug) -> DigestBuildError {
        DigestBuildError::DanglingRelation {
            relation,
            identity: format!("{identity:?}"),
        }
    }

    fn relation_metadata(
        &self,
        family: crate::analysis_api::FactFamily,
        run_id: u64,
    ) -> Option<String> {
        self.db
            .metadata_for(crate::analysis_api::FactRef::new(family, run_id))
            .map(|metadata| self.stable_key(metadata.stable_key))
    }

    fn file(&self, id: crate::internal_core::FileId) -> DigestBuildResult<String> {
        if let Some(key) =
            self.relation_metadata(crate::analysis_api::FactFamily::SourceFile, u64::from(id.0))
        {
            return Ok(key);
        }
        self.db
            .files()
            .iter()
            .any(|file| file.id == id)
            .then(|| self.db.path_for(id).replace('\\', "/"))
            .ok_or_else(|| self.dangling("file", id))
    }

    fn function(&self, id: crate::internal_core::FunctionId) -> DigestBuildResult<String> {
        if let Some(key) = self.relation_metadata(crate::analysis_api::FactFamily::Function, id.0) {
            return Ok(key);
        }
        if let Some(body) = self.db.mir_bodies().iter().find(|body| body.function == id) {
            return Ok(self.stable_key(body.owner_stable_key));
        }
        let function = self
            .db
            .functions()
            .iter()
            .find(|function| function.id == id)
            .ok_or_else(|| self.dangling("function", id))?;
        stable_json_payload(
            "function_identity",
            (
                "function",
                function.language,
                self.file(function.file)?,
                &function.name,
                function.span.start_line,
                function.span.start_col,
                function.span.end_line,
                function.span.end_col,
            ),
        )
    }

    fn symbol(&self, id: crate::internal_core::SymbolId) -> DigestBuildResult<String> {
        if let Some(key) = self.relation_metadata(crate::analysis_api::FactFamily::Symbol, id.0) {
            return Ok(key);
        }
        self.db
            .symbols()
            .iter()
            .find(|symbol| symbol.id == id)
            .map(|symbol| self.stable_key(symbol.stable_key))
            .ok_or_else(|| self.dangling("symbol", id))
    }

    fn body(&self, id: crate::analysis_neutral::ids::MirBodyId) -> DigestBuildResult<String> {
        if let Some(key) = self.relation_metadata(crate::analysis_api::FactFamily::MirBody, id.0) {
            return Ok(key);
        }
        self.db
            .mir_bodies()
            .iter()
            .find(|body| body.id == id)
            .map(|body| self.stable_key(body.stable_key))
            .ok_or_else(|| self.dangling("mir_body", id))
    }

    fn place(&self, id: crate::analysis_neutral::ids::PlaceId) -> DigestBuildResult<String> {
        if let Some(key) = self.relation_metadata(crate::analysis_api::FactFamily::Place, id.0) {
            return Ok(key);
        }
        self.db
            .mir_places()
            .iter()
            .find(|place| place.id == id)
            .map(|place| self.stable_key(place.stable_key))
            .ok_or_else(|| self.dangling("place", id))
    }

    fn operation(&self, id: crate::analysis_neutral::ids::MirOpId) -> DigestBuildResult<String> {
        if let Some(key) =
            self.relation_metadata(crate::analysis_api::FactFamily::MirOperation, id.0)
        {
            return Ok(key);
        }
        self.db
            .mir_operations()
            .iter()
            .find(|operation| operation.id == id)
            .map(|operation| self.stable_key(operation.stable_key))
            .ok_or_else(|| self.dangling("mir_operation", id))
    }

    fn cfg_block(
        &self,
        id: crate::analysis_neutral::cfg::ids::BasicBlockId,
    ) -> DigestBuildResult<String> {
        if let Some(key) = self.relation_metadata(crate::analysis_api::FactFamily::BasicBlock, id.0)
        {
            return Ok(key);
        }
        self.db
            .cfg_blocks()
            .iter()
            .find(|block| block.id == id)
            .map(|block| self.stable_key(block.stable_key))
            .ok_or_else(|| self.dangling("cfg_block", id))
    }

    fn call_site(&self, id: crate::analysis_neutral::ids::CallSiteId) -> DigestBuildResult<String> {
        if let Some(key) = self.relation_metadata(crate::analysis_api::FactFamily::CallSite, id.0) {
            return Ok(key);
        }
        self.db
            .call_sites()
            .iter()
            .find(|site| site.id == id)
            .map(|site| self.stable_key(site.stable_key))
            .ok_or_else(|| self.dangling("call_site", id))
    }

    fn type_set(&self, id: crate::analysis_neutral::ids::TypeSetId) -> DigestBuildResult<String> {
        self.type_sets
            .get(&id)
            .cloned()
            .ok_or_else(|| self.dangling("type_set", id))
    }

    fn value(&self, id: crate::analysis_neutral::ids::ValueFactId) -> DigestBuildResult<String> {
        self.values
            .get(&id)
            .cloned()
            .ok_or_else(|| self.dangling("value", id))
    }

    fn abstract_value(
        &self,
        id: crate::analysis_neutral::ids::AbstractValueId,
    ) -> DigestBuildResult<String> {
        self.abstract_values
            .get(&id)
            .cloned()
            .ok_or_else(|| self.dangling("abstract_value", id))
    }

    fn allocation(
        &self,
        id: crate::analysis_neutral::ids::AllocationTokenId,
    ) -> DigestBuildResult<String> {
        self.allocations
            .get(&id)
            .cloned()
            .ok_or_else(|| self.dangling("allocation", id))
    }

    fn access_path(
        &self,
        id: crate::analysis_neutral::ids::AccessPathId,
    ) -> DigestBuildResult<String> {
        self.access_paths
            .get(&id)
            .cloned()
            .ok_or_else(|| self.dangling("access_path", id))
    }

    fn points_to_variable(
        &self,
        id: crate::analysis_neutral::ids::PtVarId,
    ) -> DigestBuildResult<String> {
        self.points_to_variables
            .get(&id)
            .cloned()
            .ok_or_else(|| self.dangling("points_to_variable", id))
    }

    fn object(&self, id: crate::analysis_neutral::ids::ObjectTokenId) -> DigestBuildResult<String> {
        let tag = id.0 & 0xf;
        let payload = id.0 >> 4;
        match tag {
            1 => self.allocation(crate::analysis_neutral::ids::AllocationTokenId(payload)),
            2 => self.abstract_value(crate::analysis_neutral::ids::AbstractValueId(payload)),
            3 => self.value(crate::analysis_neutral::ids::ValueFactId(payload)),
            _ => Err(self.dangling("points_to_object", id)),
        }
    }
}

fn record_points_to_variable_fragments(
    fragments: &mut std::collections::BTreeMap<
        crate::analysis_neutral::ids::PtVarId,
        std::collections::BTreeSet<String>,
    >,
    constraint_key: &str,
    kind: &crate::analysis_neutral::points_to::facts::PointsToConstraintKind,
) {
    use crate::analysis_neutral::points_to::facts::PointsToConstraintKind as Kind;
    let mut record = |variable, role: &str| {
        fragments
            .entry(variable)
            .or_default()
            .insert(format!("{role}:{constraint_key}"));
    };
    match kind {
        Kind::AddressOf { dst, .. } => record(*dst, "address_of_destination"),
        Kind::Copy { dst, src } => {
            record(*dst, "copy_destination");
            record(*src, "copy_source");
        }
        Kind::Load { dst, pointer } => {
            record(*dst, "load_destination");
            record(*pointer, "load_pointer");
        }
        Kind::Store { pointer, src } => {
            record(*pointer, "store_pointer");
            record(*src, "store_source");
        }
        Kind::FieldLoad { dst, base, .. } | Kind::ElementLoad { dst, base, .. } => {
            record(*dst, "projection_load_destination");
            record(*base, "projection_load_base");
        }
        Kind::FieldStore { base, src, .. } | Kind::ElementStore { base, src, .. } => {
            record(*base, "projection_store_base");
            record(*src, "projection_store_source");
        }
        Kind::CallReturn { dst, .. } => record(*dst, "call_return_destination"),
        Kind::SummaryFlow { dst, src, .. } => {
            record(*dst, "summary_destination");
            record(*src, "summary_source");
        }
    }
}

fn semantic_relation_identity(fragments: &std::collections::BTreeSet<String>) -> String {
    fragments
        .iter()
        .map(|fragment| format!("{}:{fragment}", fragment.len()))
        .collect()
}

fn type_fact_payload<H: AnalysisHost + ?Sized>(
    keys: &DigestKeys<'_, H>,
    fact: &TypeFact,
) -> DigestBuildResult<String> {
    stable_json_payload(
        "type",
        TypeFactDigest {
            subject: type_subject_key(keys, &fact.subject)?,
            type_set: keys.type_set(fact.type_set)?,
            shape: type_shape_key(keys, &fact.shape)?,
            phase: fact.phase,
            language: fact.language,
            file: fact.file.map(|id| keys.file(id)).transpose()?,
            function: fact.function.map(|id| keys.function(id)).transpose()?,
            body: fact.body.map(|id| keys.body(id)).transpose()?,
            place: fact.place.map(|id| keys.place(id)).transpose()?,
            cfg_block: fact.cfg_block.map(|id| keys.cfg_block(id)).transpose()?,
            operation: fact.operation.map(|id| keys.operation(id)).transpose()?,
            precision: fact.precision,
            confidence: fact.confidence,
            status: fact.status,
            provenance: &fact.provenance,
            stable_key: keys.stable_key(fact.stable_key),
        },
    )
}

fn narrowed_type_fact_payload<H: AnalysisHost + ?Sized>(
    keys: &DigestKeys<'_, H>,
    fact: &NarrowedTypeFact,
) -> DigestBuildResult<String> {
    stable_json_payload(
        "narrowed_type",
        NarrowedTypeFactDigest {
            place: keys.place(fact.place)?,
            type_set: keys.type_set(fact.type_set)?,
            cfg_block: fact.cfg_block.map(|id| keys.cfg_block(id)).transpose()?,
            operation: fact.operation.map(|id| keys.operation(id)).transpose()?,
            predicate: fact.predicate.map(|id| keys.place(id)).transpose()?,
            evidence: fact.evidence.as_str(),
            language: fact.language,
            file: fact.file.map(|id| keys.file(id)).transpose()?,
            function: fact.function.map(|id| keys.function(id)).transpose()?,
            body: fact.body.map(|id| keys.body(id)).transpose()?,
            precision: fact.precision,
            status: fact.status,
            stable_key: keys.stable_key(fact.stable_key),
        },
    )
}

fn value_fact_payload<H: AnalysisHost + ?Sized>(
    keys: &DigestKeys<'_, H>,
    fact: &ValueFact,
) -> DigestBuildResult<String> {
    stable_json_payload(
        "value",
        ValueFactDigest {
            subject: value_subject_key(keys, &fact.subject)?,
            value: keys.abstract_value(fact.value)?,
            kind: value_kind_key(keys, &fact.kind)?,
            language: fact.language,
            file: fact.file.map(|id| keys.file(id)).transpose()?,
            function: fact.function.map(|id| keys.function(id)).transpose()?,
            body: fact.body.map(|id| keys.body(id)).transpose()?,
            precision: fact.precision,
            status: fact.status,
            provenance: &fact.provenance,
            stable_key: keys.stable_key(fact.stable_key),
        },
    )
}

fn allocation_fact_payload<H: AnalysisHost + ?Sized>(
    keys: &DigestKeys<'_, H>,
    fact: &AllocationTokenFact,
) -> DigestBuildResult<String> {
    stable_json_payload(
        "allocation",
        AllocationFactDigest {
            kind: fact.kind,
            language: fact.language,
            file: fact.file.map(|id| keys.file(id)).transpose()?,
            function: fact.function.map(|id| keys.function(id)).transpose()?,
            body: fact.body.map(|id| keys.body(id)).transpose()?,
            source_place: fact.source_place.map(|id| keys.place(id)).transpose()?,
            source_operation: fact
                .source_operation
                .map(|id| keys.operation(id))
                .transpose()?,
            span: fact
                .span
                .as_ref()
                .map(|span| stable_span(keys, span))
                .transpose()?,
            provenance: &fact.provenance,
            stable_key: keys.stable_key(fact.stable_key),
        },
    )
}

fn access_path_fact_payload<H: AnalysisHost + ?Sized>(
    keys: &DigestKeys<'_, H>,
    fact: &AccessPathFact,
) -> DigestBuildResult<String> {
    let projections = fact
        .projections
        .iter()
        .map(|projection| access_path_projection_key(keys, projection))
        .collect::<DigestBuildResult<Vec<_>>>()?;
    stable_json_payload(
        "access_path",
        AccessPathFactDigest {
            base: keys.place(fact.base)?,
            projections,
            depth: fact.depth,
            language: fact.language,
            file: fact.file.map(|id| keys.file(id)).transpose()?,
            function: fact.function.map(|id| keys.function(id)).transpose()?,
            body: fact.body.map(|id| keys.body(id)).transpose()?,
            status: fact.status,
            stable_key: keys.stable_key(fact.stable_key),
        },
    )
}

fn points_to_constraint_payload<H: AnalysisHost + ?Sized>(
    keys: &DigestKeys<'_, H>,
    fact: &PointsToConstraintFact,
) -> DigestBuildResult<String> {
    stable_json_payload(
        "points_to_constraint",
        PointsToConstraintDigest {
            kind: points_to_constraint_kind_key(keys, &fact.kind)?,
            status: fact.status,
            precision: fact.precision,
            stable_key: keys.stable_key(fact.stable_key),
        },
    )
}

fn points_to_set_payload<H: AnalysisHost + ?Sized>(
    keys: &DigestKeys<'_, H>,
    fact: &PointsToSetFact,
) -> DigestBuildResult<String> {
    let mut objects = fact
        .objects
        .iter()
        .map(|object| keys.object(*object))
        .collect::<DigestBuildResult<Vec<_>>>()?;
    objects.sort();
    stable_json_payload(
        "points_to_set",
        PointsToSetDigest {
            variable: keys.points_to_variable(fact.variable)?,
            objects,
            status: fact.status,
            precision: fact.precision,
            budget: fact.budget,
            stable_key: keys.stable_key(fact.stable_key),
        },
    )
}

fn alias_answer_payload<H: AnalysisHost + ?Sized>(
    keys: &DigestKeys<'_, H>,
    fact: &AliasAnswerFact,
) -> DigestBuildResult<String> {
    stable_json_payload(
        "alias_answer",
        AliasAnswerDigest {
            left: alias_operand_key(keys, fact.left)?,
            right: alias_operand_key(keys, fact.right)?,
            status: fact.status,
            reason: &fact.reason,
            evidence: &fact.evidence,
            precision: fact.precision,
            stable_key: keys.stable_key(fact.stable_key),
        },
    )
}

fn type_subject_key<H: AnalysisHost + ?Sized>(
    keys: &DigestKeys<'_, H>,
    subject: &super::facts::TypeSubject,
) -> DigestBuildResult<String> {
    use super::facts::TypeSubject;
    Ok(match subject {
        TypeSubject::Symbol(id) => format!("symbol|{}", keys.symbol(*id)?),
        TypeSubject::Place(id) => format!("place|{}", keys.place(*id)?),
        TypeSubject::Operation(id) => format!("operation|{}", keys.operation(*id)?),
        TypeSubject::Function(id) => format!("function|{}", keys.function(*id)?),
        TypeSubject::Synthetic(value) => format!("synthetic|{value}"),
        TypeSubject::Unknown(value) => format!("unknown|{value}"),
    })
}

fn type_shape_key<H: AnalysisHost + ?Sized>(
    keys: &DigestKeys<'_, H>,
    shape: &super::facts::TypeShape,
) -> DigestBuildResult<String> {
    use super::facts::TypeShape;
    Ok(match shape {
        TypeShape::Primitive(value) => format!("primitive|{value}"),
        TypeShape::Literal(value) => format!("literal|{value}"),
        TypeShape::Nullish(value) => format!("nullish|{value}"),
        TypeShape::Callable { signature } => format!("callable|{signature}"),
        TypeShape::Class { name } => format!("class|{}", stable_json_payload("class_shape", name)?),
        TypeShape::Object { shape_id } => {
            format!("object|{}", stable_json_payload("object_shape", shape_id)?)
        }
        TypeShape::Module { module_key } => format!("module|{module_key}"),
        TypeShape::Nominal { type_id } => format!("nominal|{type_id}"),
        TypeShape::Structural { shape_id } => format!("structural|{shape_id}"),
        TypeShape::Union(sets) => format!("union|{}", stable_type_sets(keys, sets)?),
        TypeShape::Intersection(sets) => {
            format!("intersection|{}", stable_type_sets(keys, sets)?)
        }
        TypeShape::GenericPlaceholder(value) => format!("generic|{value}"),
        TypeShape::Any => "any".to_string(),
        TypeShape::Unknown { reason } => format!("unknown|{reason}"),
        TypeShape::Unsupported { reason } => format!("unsupported|{reason}"),
    })
}

fn stable_type_sets<H: AnalysisHost + ?Sized>(
    keys: &DigestKeys<'_, H>,
    sets: &[crate::analysis_neutral::ids::TypeSetId],
) -> DigestBuildResult<String> {
    let mut sets = sets
        .iter()
        .map(|id| keys.type_set(*id))
        .collect::<DigestBuildResult<Vec<_>>>()?;
    sets.sort();
    stable_json_payload("type_sets", sets)
}

fn value_subject_key<H: AnalysisHost + ?Sized>(
    keys: &DigestKeys<'_, H>,
    subject: &crate::analysis_neutral::values::facts::ValueSubject,
) -> DigestBuildResult<String> {
    use crate::analysis_neutral::values::facts::ValueSubject;
    Ok(match subject {
        ValueSubject::Place(id) => format!("place|{}", keys.place(*id)?),
        ValueSubject::Operation(id) => format!("operation|{}", keys.operation(*id)?),
        ValueSubject::Allocation(id) => format!("allocation|{}", keys.allocation(*id)?),
        ValueSubject::Synthetic(value) => format!("synthetic|{value}"),
        ValueSubject::Unknown(value) => format!("unknown|{value}"),
    })
}

fn value_kind_key<H: AnalysisHost + ?Sized>(
    keys: &DigestKeys<'_, H>,
    kind: &crate::analysis_neutral::values::facts::ValueKind,
) -> DigestBuildResult<String> {
    use crate::analysis_neutral::values::facts::ValueKind;
    Ok(match kind {
        ValueKind::Null => "null".to_string(),
        ValueKind::Undefined => "undefined".to_string(),
        ValueKind::Nil => "nil".to_string(),
        ValueKind::Bool(value) => format!("bool|{value}"),
        ValueKind::Number(value) => format!("number|{value}"),
        ValueKind::String(value) => format!("string|{value}"),
        ValueKind::Literal(value) => format!("literal|{value}"),
        ValueKind::FunctionObject => "function_object".to_string(),
        ValueKind::ClassObject => "class_object".to_string(),
        ValueKind::ModuleObject => "module_object".to_string(),
        ValueKind::PlaceRef(id) => format!("place_ref|{}", keys.place(*id)?),
        ValueKind::Object(id) => format!("object|{}", keys.allocation(*id)?),
        ValueKind::Array(id) => format!("array|{}", keys.allocation(*id)?),
        ValueKind::CompositeLiteral(id) => format!("composite|{}", keys.allocation(*id)?),
        ValueKind::CallReturn(id) => format!("call_return|{}", keys.place(*id)?),
        ValueKind::Unknown { evidence } => format!("unknown|{evidence}"),
    })
}

fn access_path_projection_key<H: AnalysisHost + ?Sized>(
    keys: &DigestKeys<'_, H>,
    projection: &crate::analysis_neutral::access_paths::facts::AccessPathProjection,
) -> DigestBuildResult<String> {
    use crate::analysis_neutral::access_paths::facts::AccessPathProjection;
    Ok(match projection {
        AccessPathProjection::Field(value) => format!("field|{value}"),
        AccessPathProjection::Property(value) => format!("property|{value}"),
        AccessPathProjection::IndexKnown(value) => format!("index|{value}"),
        AccessPathProjection::IndexUnknown { evidence } => format!("index_unknown|{evidence}"),
        AccessPathProjection::Deref => "deref".to_string(),
        AccessPathProjection::AwaitResult => "await_result".to_string(),
        AccessPathProjection::CallReturn(id) => format!("call_return|{}", keys.call_site(*id)?),
        AccessPathProjection::Unknown { evidence } => format!("unknown|{evidence}"),
    })
}

fn points_to_constraint_kind_key<H: AnalysisHost + ?Sized>(
    keys: &DigestKeys<'_, H>,
    kind: &crate::analysis_neutral::points_to::facts::PointsToConstraintKind,
) -> DigestBuildResult<String> {
    use crate::analysis_neutral::points_to::facts::PointsToConstraintKind;
    match kind {
        PointsToConstraintKind::AddressOf { dst, object } => stable_json_payload(
            "points_to_address_of",
            (
                "address_of",
                keys.points_to_variable(*dst)?,
                keys.object(*object)?,
            ),
        ),
        PointsToConstraintKind::Copy { dst, src } => stable_json_payload(
            "points_to_copy",
            (
                "copy",
                keys.points_to_variable(*dst)?,
                keys.points_to_variable(*src)?,
            ),
        ),
        PointsToConstraintKind::Load { dst, pointer } => stable_json_payload(
            "points_to_load",
            (
                "load",
                keys.points_to_variable(*dst)?,
                keys.points_to_variable(*pointer)?,
            ),
        ),
        PointsToConstraintKind::Store { pointer, src } => stable_json_payload(
            "points_to_store",
            (
                "store",
                keys.points_to_variable(*pointer)?,
                keys.points_to_variable(*src)?,
            ),
        ),
        PointsToConstraintKind::FieldLoad { dst, base, field } => stable_json_payload(
            "points_to_field_load",
            (
                "field_load",
                keys.points_to_variable(*dst)?,
                keys.points_to_variable(*base)?,
                field,
            ),
        ),
        PointsToConstraintKind::FieldStore { base, field, src } => stable_json_payload(
            "points_to_field_store",
            (
                "field_store",
                keys.points_to_variable(*base)?,
                field,
                keys.points_to_variable(*src)?,
            ),
        ),
        PointsToConstraintKind::ElementLoad { dst, base, index } => stable_json_payload(
            "points_to_element_load",
            (
                "element_load",
                keys.points_to_variable(*dst)?,
                keys.points_to_variable(*base)?,
                index,
            ),
        ),
        PointsToConstraintKind::ElementStore { base, index, src } => stable_json_payload(
            "points_to_element_store",
            (
                "element_store",
                keys.points_to_variable(*base)?,
                index,
                keys.points_to_variable(*src)?,
            ),
        ),
        PointsToConstraintKind::CallReturn { dst, value } => stable_json_payload(
            "points_to_call_return",
            (
                "call_return",
                keys.points_to_variable(*dst)?,
                keys.value(*value)?,
            ),
        ),
        PointsToConstraintKind::SummaryFlow {
            dst,
            src,
            summary_key,
        } => stable_json_payload(
            "points_to_summary_flow",
            (
                "summary_flow",
                keys.points_to_variable(*dst)?,
                keys.points_to_variable(*src)?,
                summary_key,
            ),
        ),
    }
}

fn alias_operand_key<H: AnalysisHost + ?Sized>(
    keys: &DigestKeys<'_, H>,
    operand: crate::analysis_neutral::aliases::facts::AliasOperand,
) -> DigestBuildResult<String> {
    use crate::analysis_neutral::aliases::facts::AliasOperand;
    Ok(match operand {
        AliasOperand::Place(id) => format!("place|{}", keys.place(id)?),
        AliasOperand::AccessPath(id) => format!("access_path|{}", keys.access_path(id)?),
    })
}

fn stable_span<H: AnalysisHost + ?Sized>(
    keys: &DigestKeys<'_, H>,
    span: &crate::internal_core::Span,
) -> DigestBuildResult<String> {
    stable_json_payload(
        "span",
        (
            keys.file(span.file)?,
            span.start_byte,
            span.end_byte,
            span.start_line,
            span.start_col,
            span.end_line,
            span.end_col,
        ),
    )
}

fn stable_json_payload<T: Serialize>(
    row_kind: &'static str,
    payload: T,
) -> DigestBuildResult<String> {
    serde_json::to_string(&payload)
        .map_err(|source| DigestBuildError::Serialization { row_kind, source })
}

#[derive(Serialize)]
struct TypeFactDigest<'a> {
    subject: String,
    type_set: String,
    shape: String,
    phase: super::facts::TypePhase,
    language: crate::internal_core::Language,
    file: Option<String>,
    function: Option<String>,
    body: Option<String>,
    place: Option<String>,
    cfg_block: Option<String>,
    operation: Option<String>,
    precision: super::facts::TypePrecision,
    confidence: super::facts::TypeConfidence,
    status: super::facts::TypeStatus,
    provenance: &'a super::facts::TypeProvenance,
    stable_key: String,
}

#[derive(Serialize)]
struct NarrowedTypeFactDigest<'a> {
    place: String,
    type_set: String,
    cfg_block: Option<String>,
    operation: Option<String>,
    predicate: Option<String>,
    evidence: &'a str,
    language: crate::internal_core::Language,
    file: Option<String>,
    function: Option<String>,
    body: Option<String>,
    precision: super::facts::TypePrecision,
    status: super::facts::TypeStatus,
    stable_key: String,
}

#[derive(Serialize)]
struct ValueFactDigest<'a> {
    subject: String,
    value: String,
    kind: String,
    language: crate::internal_core::Language,
    file: Option<String>,
    function: Option<String>,
    body: Option<String>,
    precision: crate::analysis_neutral::values::facts::ValuePrecision,
    status: crate::analysis_neutral::values::facts::ValueStatus,
    provenance: &'a crate::analysis_neutral::values::facts::ValueProvenance,
    stable_key: String,
}

#[derive(Serialize)]
struct AllocationFactDigest<'a> {
    kind: crate::analysis_neutral::values::facts::AllocationKind,
    language: crate::internal_core::Language,
    file: Option<String>,
    function: Option<String>,
    body: Option<String>,
    source_place: Option<String>,
    source_operation: Option<String>,
    span: Option<String>,
    provenance: &'a crate::analysis_neutral::values::facts::ValueProvenance,
    stable_key: String,
}

#[derive(Serialize)]
struct AccessPathFactDigest {
    base: String,
    projections: Vec<String>,
    depth: u32,
    language: crate::internal_core::Language,
    file: Option<String>,
    function: Option<String>,
    body: Option<String>,
    status: crate::analysis_neutral::access_paths::facts::AccessPathStatus,
    stable_key: String,
}

#[derive(Serialize)]
struct PointsToConstraintDigest {
    kind: String,
    status: crate::analysis_neutral::points_to::facts::PointsToStatus,
    precision: crate::analysis_neutral::points_to::facts::PointsToPrecision,
    stable_key: String,
}

#[derive(Serialize)]
struct PointsToSetDigest {
    variable: String,
    objects: Vec<String>,
    status: crate::analysis_neutral::points_to::facts::PointsToStatus,
    precision: crate::analysis_neutral::points_to::facts::PointsToPrecision,
    budget: crate::analysis_neutral::points_to::facts::PointsToBudgetStatus,
    stable_key: String,
}

#[derive(Serialize)]
struct AliasAnswerDigest<'a> {
    left: String,
    right: String,
    status: crate::analysis_neutral::aliases::facts::AliasStatus,
    reason: &'a crate::analysis_neutral::aliases::facts::AliasReason,
    evidence: &'a [String],
    precision: crate::analysis_neutral::aliases::facts::AliasPrecision,
    stable_key: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_api::{
        CachePolicy, FactFamily, GoLifecycleSnapshot, InputComponentStatus, PrecisionCeiling,
        ProviderKind, SchemaVersion, TsJsLifecycleSnapshot, stable_key_from_parts,
    };
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::access_paths::facts::{AccessPathProjection, AccessPathStatus};
    use crate::analysis_neutral::access_paths::store::AccessPathOutput;
    use crate::analysis_neutral::aliases::facts::{
        AliasOperand, AliasPrecision, AliasReason, AliasStatus,
    };
    use crate::analysis_neutral::aliases::store::AliasOutput;
    use crate::analysis_neutral::ids::{
        AbstractValueId, AccessPathId, AliasAnswerId, AllocationTokenId, NarrowedTypeId,
        PointsToConstraintId, PointsToSetId, TypeFactId, TypeSetId, ValueFactId,
    };
    use crate::analysis_neutral::mir_body::MirOutput;
    use crate::analysis_neutral::places::{PlaceFact, PlaceRoot, PlaceStatus};
    use crate::analysis_neutral::points_to::facts::{
        PointsToBudgetStatus, PointsToConstraintKind, PointsToPrecision, PointsToStatus,
    };
    use crate::analysis_neutral::points_to::store::PointsToOutput;
    use crate::analysis_neutral::points_to::vars;
    use crate::analysis_neutral::types::facts::{
        TypeConfidence, TypePhase, TypePrecision, TypeProvenance, TypeShape, TypeStatus,
        TypeSubject,
    };
    use crate::analysis_neutral::types::store::TypeOutput;
    use crate::analysis_neutral::values::facts::{
        AllocationKind, ValueKind, ValuePrecision, ValueProvenance, ValueStatus, ValueSubject,
    };
    use crate::analysis_neutral::values::store::ValueOutput;
    use crate::internal_core::{Language, StableKeyInterner};

    fn manifest() -> ProviderManifest {
        ProviderManifest {
            id: TYPE_VALUE_ALIAS_PROVIDER_ID,
            kind: ProviderKind::WholeRepoDerived,
            inputs: &[],
            outputs: &["type_value_alias"],
            language_ids: &[],
            cache_policy: CachePolicy::InMemoryDerived,
            schema_versions: &[SchemaVersion {
                name: "type-value-alias-test",
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

    fn graph(
        db: &mut LocalAnalysisDb,
        base: u64,
        value_uses_second_allocation: bool,
    ) -> TypeValueAliasOutput {
        let interner = db.stable_key_interner();
        let source_place = crate::analysis_neutral::ids::PlaceId(base + 7);
        db.replace_semantic_mir(MirOutput {
            places: vec![PlaceFact {
                id: source_place,
                language: Language::TypeScript,
                file: None,
                function: None,
                root: PlaceRoot::Unknown {
                    evidence: "digest-test-place".to_string(),
                },
                projections: Vec::new(),
                stable_key: production_key(&interner, FactFamily::Place, "source-place"),
                status: PlaceStatus::Resolved,
            }],
            ..MirOutput::default()
        })
        .expect("test MIR should be valid");
        let source_place = db.mir_places()[0].id;

        let type_set = TypeSetId(base + 1);
        let value_id = ValueFactId(base + 2);
        let abstract_value = AbstractValueId(base + 3);
        let first_allocation = AllocationTokenId(base + 4);
        let second_allocation = AllocationTokenId(base + 5);
        let referenced_allocation = if value_uses_second_allocation {
            second_allocation
        } else {
            first_allocation
        };
        let access_path = AccessPathId(base + 6);

        TypeValueAliasOutput {
            types: TypeOutput {
                types: vec![TypeFact {
                    id: TypeFactId(base),
                    subject: TypeSubject::Synthetic("subject".to_string()),
                    type_set,
                    shape: TypeShape::Union(vec![type_set]),
                    phase: TypePhase::Inferred,
                    language: Language::TypeScript,
                    file: None,
                    function: None,
                    body: None,
                    place: Some(source_place),
                    cfg_block: None,
                    operation: None,
                    precision: TypePrecision::Conservative,
                    confidence: TypeConfidence::Medium,
                    status: TypeStatus::Present,
                    provenance: TypeProvenance::Native,
                    stable_key: production_key(&interner, FactFamily::Type, "type"),
                }],
                narrowed: vec![NarrowedTypeFact {
                    id: NarrowedTypeId(base + 11),
                    place: source_place,
                    type_set,
                    cfg_block: None,
                    operation: None,
                    predicate: Some(source_place),
                    evidence: "test narrowing".to_string(),
                    language: Language::TypeScript,
                    file: None,
                    function: None,
                    body: None,
                    precision: TypePrecision::Conservative,
                    status: TypeStatus::Present,
                    stable_key: production_key(
                        &interner,
                        FactFamily::NarrowedType,
                        "narrowed-type",
                    ),
                }],
            },
            values: ValueOutput {
                values: vec![ValueFact {
                    id: value_id,
                    subject: ValueSubject::Allocation(referenced_allocation),
                    value: abstract_value,
                    kind: ValueKind::Object(referenced_allocation),
                    language: Language::TypeScript,
                    file: None,
                    function: None,
                    body: None,
                    precision: ValuePrecision::Conservative,
                    status: ValueStatus::Present,
                    provenance: ValueProvenance::Native,
                    stable_key: production_key(&interner, FactFamily::Value, "value"),
                }],
                allocations: vec![
                    allocation(&interner, first_allocation, "first"),
                    allocation(&interner, second_allocation, "second"),
                ],
            },
            access_paths: AccessPathOutput {
                access_paths: vec![AccessPathFact {
                    id: access_path,
                    base: source_place,
                    projections: vec![AccessPathProjection::Field("field".to_string())],
                    depth: 1,
                    language: Language::TypeScript,
                    file: None,
                    function: None,
                    body: None,
                    status: AccessPathStatus::Resolved,
                    stable_key: production_key(&interner, FactFamily::AccessPath, "access-path"),
                }],
            },
            points_to: PointsToOutput {
                constraints: vec![PointsToConstraintFact {
                    id: PointsToConstraintId(base + 8),
                    kind: PointsToConstraintKind::AddressOf {
                        dst: vars::allocation_var(first_allocation),
                        object: vars::allocation_object(first_allocation),
                    },
                    status: PointsToStatus::Present,
                    precision: PointsToPrecision::FlowInsensitive,
                    stable_key: production_key(
                        &interner,
                        FactFamily::PointsToConstraint,
                        "constraint",
                    ),
                }],
                sets: vec![PointsToSetFact {
                    id: PointsToSetId(base + 9),
                    variable: vars::allocation_var(first_allocation),
                    objects: vec![vars::allocation_object(first_allocation)],
                    status: PointsToStatus::Present,
                    precision: PointsToPrecision::FlowInsensitive,
                    budget: PointsToBudgetStatus::WithinBudget,
                    stable_key: production_key(&interner, FactFamily::PointsToSet, "points-to-set"),
                }],
            },
            aliases: AliasOutput {
                answers: vec![AliasAnswerFact {
                    id: AliasAnswerId(base + 10),
                    left: AliasOperand::AccessPath(access_path),
                    right: AliasOperand::AccessPath(access_path),
                    status: AliasStatus::MustAlias,
                    reason: AliasReason::SameStablePlace,
                    evidence: vec!["same stable path".to_string()],
                    precision: AliasPrecision::ExactLocal,
                    stable_key: production_key(&interner, FactFamily::AliasAnswer, "alias-answer"),
                }],
            },
        }
    }

    fn production_key(
        interner: &StableKeyInterner,
        family: FactFamily,
        identity: &str,
    ) -> crate::internal_core::StableKeyId {
        stable_key_from_parts(interner, family, &[("identity", identity.to_string())])
    }

    fn allocation(
        interner: &StableKeyInterner,
        id: AllocationTokenId,
        identity: &str,
    ) -> AllocationTokenFact {
        AllocationTokenFact {
            id,
            kind: AllocationKind::ObjectLiteral,
            language: Language::TypeScript,
            file: None,
            function: None,
            body: None,
            source_place: None,
            source_operation: None,
            span: None,
            provenance: ValueProvenance::Native,
            stable_key: production_key(interner, FactFamily::AllocationToken, identity),
        }
    }

    fn digest_result(
        db: &LocalAnalysisDb,
        output: &TypeValueAliasOutput,
    ) -> DigestBuildResult<Digest> {
        let upstream = Digest::absent(DigestKind::ProviderOutput, "test");
        type_value_alias_output_digest(
            db,
            &manifest(),
            &snapshot(),
            &upstream,
            &upstream,
            &upstream,
            &upstream,
            &upstream,
            &upstream,
            &upstream,
            &upstream,
            &upstream,
            &[],
            output,
        )
    }

    fn digest(db: &LocalAnalysisDb, output: &TypeValueAliasOutput) -> Digest {
        digest_result(db, output).expect("complete test graph has canonical digest relations")
    }

    #[test]
    fn output_digest_is_invariant_to_equivalent_dense_graph_remapping() {
        let mut first_db = LocalAnalysisDb::new();
        let mut second_db = LocalAnalysisDb::new();
        let first = graph(&mut first_db, 10, false);
        let remapped = graph(&mut second_db, 100, false);

        assert_eq!(digest(&first_db, &first), digest(&second_db, &remapped));
    }

    #[test]
    fn output_digest_changes_when_a_stable_relation_changes() {
        let mut first_db = LocalAnalysisDb::new();
        let mut second_db = LocalAnalysisDb::new();
        let first = graph(&mut first_db, 10, false);
        let changed = graph(&mut second_db, 100, true);

        assert_ne!(digest(&first_db, &first), digest(&second_db, &changed));
    }

    #[test]
    fn output_digest_rejects_a_dangling_stable_relation() {
        let mut db = LocalAnalysisDb::new();
        let mut output = graph(&mut db, 10, false);
        output.access_paths.access_paths[0].base = crate::analysis_neutral::ids::PlaceId(u64::MAX);

        let error = digest_result(&db, &output).expect_err("dangling place must reject digest");

        assert!(matches!(
            error,
            DigestBuildError::DanglingRelation {
                relation: "place",
                ..
            }
        ));
    }

    #[test]
    fn stable_json_payload_preserves_serialization_failure_type() {
        let payload = std::collections::BTreeMap::from([((1_u8, 2_u8), 3_u8)]);

        let error =
            stable_json_payload("test_row", payload).expect_err("JSON object keys must be strings");

        assert!(matches!(
            error,
            DigestBuildError::Serialization {
                row_kind: "test_row",
                ..
            }
        ));
    }
}
