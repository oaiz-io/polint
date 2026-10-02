use crate::analysis::mir::body::MirOutput;
use crate::analysis::mir::op::{MirOperationKind, MirValue};
use crate::analysis::places::{PlaceProjection, PlaceRoot};
use crate::analysis_api::{ProviderExecution, ProviderFailureReason, ProviderFailureStage};
use crate::analysis_kernel::ProviderManifest;
use crate::analysis_kernel::incremental::{
    CacheStats, Digest, DigestKind, InputComponent, InputSnapshot,
};
#[cfg(test)]
use crate::analysis_neutral::mir_body_compose::merge_language_outputs;
use crate::core::AnalysisDb;
use crate::diagnostics::{Diagnostic, TextRange};
#[cfg(feature = "lang-go")]
use crate::go::lower_go_mir_by_file;
#[cfg(feature = "lang-typescript")]
use crate::ts::lower_ts_mir;

#[derive(Debug, Clone, Default)]
pub(crate) struct SemanticMirProviderOutput {
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) cache_stats: CacheStats,
    pub(crate) output_digest: Option<Digest>,
    pub(crate) execution: ProviderExecution,
}

pub(crate) fn derive_semantic_mir_with_cache_stats(
    db: &mut AnalysisDb,
    input_snapshot: &InputSnapshot,
    manifest: &ProviderManifest,
    module_topology_output_digest: Digest,
    symbol_graph_output_digest: Digest,
    upstream_syntax_output_digests: Vec<Digest>,
) -> SemanticMirProviderOutput {
    let mut started = std::time::Instant::now();
    let mut checkpoint = |step: &'static str| {
        tracing::debug!(target: "polint::kernel::stage", provider = "polint.semantic_mir", step, elapsed_ms = started.elapsed().as_millis() as u64, "provider step");
        started = std::time::Instant::now();
    };
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    #[cfg(feature = "lang-go")]
    let go_outputs = lower_go_mir_by_file(db);
    #[cfg(not(feature = "lang-go"))]
    let go_outputs: Vec<MirOutput> = Vec::new();
    checkpoint("lower_go");
    #[cfg(feature = "lang-typescript")]
    let ts_output = lower_ts_mir(db);
    checkpoint("lower_ts");
    let output = crate::analysis_neutral::mir_body_compose::merge_language_outputs(
        go_outputs.into_iter().chain([
            #[cfg(feature = "lang-typescript")]
            ts_output,
        ]),
        interner,
    );
    checkpoint("merge");
    let output_digest = semantic_mir_output_digest(
        manifest,
        input_snapshot,
        &module_topology_output_digest,
        &symbol_graph_output_digest,
        &upstream_syntax_output_digests,
        &output,
        interner,
    );
    checkpoint("digest");
    let mut cache_stats = CacheStats::default();
    cache_stats.record_recompute();

    let replaced = db.replace_semantic_mir(output);
    checkpoint("store_metadata");
    match replaced {
        Ok(()) => SemanticMirProviderOutput {
            diagnostics: Vec::new(),
            cache_stats,
            output_digest: Some(output_digest),
            execution: Default::default(),
        },
        Err(error) => SemanticMirProviderOutput {
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

fn semantic_mir_output_digest(
    manifest: &ProviderManifest,
    input_snapshot: &InputSnapshot,
    module_topology_output_digest: &Digest,
    symbol_graph_output_digest: &Digest,
    upstream_syntax_output_digests: &[Digest],
    output: &MirOutput,
    interner: &crate::core::StableKeyInterner,
) -> Digest {
    let mut parts = vec![
        format!("provider_id={}", manifest.id),
        format!("provider_version={}", manifest.provider_version()),
        format!("schema={}", manifest.primary_schema_label()),
        format!("config={}", input_snapshot.config.digest),
        format!("module_topology={module_topology_output_digest}"),
        format!("symbol_graph={symbol_graph_output_digest}"),
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

    let mut syntax = upstream_syntax_output_digests
        .iter()
        .map(|digest| format!("upstream_syntax={digest}"))
        .collect::<Vec<_>>();
    syntax.sort();
    parts.extend(syntax);

    for body in &output.bodies {
        parts.push(format!(
            "body={} status={:?} owner={} span={}:{} file={:?} function={:?}",
            interner.resolve(body.stable_key),
            body.status,
            interner.resolve(body.owner_stable_key),
            body.span.start_byte,
            body.span.end_byte,
            body.file,
            body.function,
        ));
    }
    for block in &output.blocks {
        parts.push(format!(
            "block={} body={:?} ordinal={} statements={:?} terminator={:?}",
            interner.resolve(block.stable_key),
            block.body,
            block.ordinal,
            block.statements,
            block.terminator,
        ));
    }
    for statement in &output.statements {
        parts.push(format!(
            "statement={} body={:?} ordinal={} operation={:?} status={:?}",
            interner.resolve(statement.stable_key),
            statement.body,
            statement.ordinal,
            statement.operation,
            statement.status,
        ));
    }
    for terminator in &output.terminators {
        parts.push(format!(
            "terminator={} body={:?} ordinal={} kind={:?} status={:?}",
            interner.resolve(terminator.stable_key),
            terminator.body,
            terminator.ordinal,
            terminator.kind,
            terminator.status,
        ));
    }
    for place in &output.places {
        parts.push(format!(
            "place={} status={:?} root={} projections={}",
            interner.resolve(place.stable_key),
            place.status,
            place_root_fragment(&place.root),
            place
                .projections
                .iter()
                .map(place_projection_fragment)
                .collect::<Vec<_>>()
                .join("/")
        ));
    }
    for operation in &output.operations {
        parts.push(format!(
            "operation={} body={:?} ordinal={} kind={} status={:?}",
            interner.resolve(operation.stable_key),
            operation.body,
            operation.ordinal,
            operation_kind_fragment(&operation.kind),
            operation.status,
        ));
    }
    for unsupported in &output.unsupported {
        parts.push(format!(
            "unsupported={} construct={} action={:?} status={:?}",
            interner.resolve(unsupported.stable_key),
            unsupported.construct,
            unsupported.conservative_action,
            unsupported.status,
        ));
    }

    parts.sort();
    let digest_refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    Digest::from_parts(
        DigestKind::ProviderOutput,
        "semantic_mir_output",
        &digest_refs,
    )
}

fn extend_component_parts(parts: &mut Vec<String>, prefix: &str, components: &[InputComponent]) {
    let mut rows = components
        .iter()
        .map(|component| {
            format!(
                "{prefix}:{}={:?}:{}",
                component.name, component.status, component.digest
            )
        })
        .collect::<Vec<_>>();
    rows.sort();
    parts.extend(rows);
}

fn place_root_fragment(root: &PlaceRoot) -> String {
    match root {
        PlaceRoot::Local { function, name } => {
            format!("local:{function:?}:{name}")
        }
        PlaceRoot::Parameter {
            function,
            index,
            name,
        } => format!(
            "parameter:{function:?}:{index}:{}",
            name.as_deref().unwrap_or("")
        ),
        PlaceRoot::Global { symbol, name } => format!("global:{symbol:?}:{name}"),
        PlaceRoot::Temporary { body, ordinal } => format!("temporary:{body:?}:{ordinal}"),
        PlaceRoot::CallReturn { call } => format!("call_return:{call:?}"),
        PlaceRoot::Unknown { evidence } => format!("unknown:{evidence}"),
    }
}

fn place_projection_fragment(projection: &PlaceProjection) -> String {
    match projection {
        PlaceProjection::Field(name) => format!("field:{name}"),
        PlaceProjection::Property(name) => format!("property:{name}"),
        PlaceProjection::IndexKnown(index) => format!("index_known:{index}"),
        PlaceProjection::IndexUnknown { evidence } => format!("index_unknown:{evidence}"),
        PlaceProjection::Deref => "deref".to_string(),
        PlaceProjection::AwaitResult => "await_result".to_string(),
        PlaceProjection::CallReturn(call) => format!("call_return:{call:?}"),
        PlaceProjection::Unknown { evidence } => format!("unknown:{evidence}"),
    }
}

fn operation_kind_fragment(kind: &MirOperationKind) -> String {
    match kind {
        MirOperationKind::StorageLive { place } => format!("storage_live:{place:?}"),
        MirOperationKind::Bind { place, value } => {
            format!("bind:{place:?}:{}", value_fragment(value))
        }
        MirOperationKind::Assign { place, value, mode } => {
            format!("assign:{place:?}:{mode:?}:{}", value_fragment(value))
        }
        MirOperationKind::Write { place, value } => {
            format!("write:{place:?}:{}", value_fragment(value))
        }
        MirOperationKind::Read { place } => format!("read:{place:?}"),
        MirOperationKind::Branch {
            predicate,
            predicate_place,
            nil_test,
        } => format!("branch:{predicate:?}:{predicate_place:?}:{nil_test:?}"),
        MirOperationKind::Call {
            site,
            callee,
            arguments,
            return_place,
        } => format!(
            "call:{site:?}:{}:{}:{return_place:?}",
            value_fragment(callee),
            arguments
                .iter()
                .map(|argument| format!("{argument:?}"))
                .collect::<Vec<_>>()
                .join(",")
        ),
        MirOperationKind::Return { value } => {
            format!(
                "return:{}",
                value.as_ref().map(value_fragment).unwrap_or_default()
            )
        }
        MirOperationKind::Unsupported { unsupported } => {
            format!("unsupported:{unsupported:?}")
        }
    }
}

fn value_fragment(value: &MirValue) -> String {
    match value {
        MirValue::Place(place) => format!("place:{place:?}"),
        MirValue::Literal { value } => format!("literal:{value}"),
        MirValue::Temporary(value) => format!("temporary:{value:?}"),
        MirValue::CallReturn(call) => format!("call_return:{call:?}"),
        MirValue::BinOp { op, lhs, rhs } => {
            format!("binop:{op}:{}:{}", value_fragment(lhs), value_fragment(rhs))
        }
        MirValue::Aggregate { kind, fields } => format!(
            "aggregate:{kind:?}:{}",
            fields
                .iter()
                .map(|field| format!(
                    "{}={}",
                    field.name.as_deref().unwrap_or_default(),
                    value_fragment(&field.value)
                ))
                .collect::<Vec<_>>()
                .join(",")
        ),
        MirValue::Closure { body, captures } => format!(
            "closure:{body:?}:{}",
            captures
                .iter()
                .map(|capture| capture.0.to_string())
                .collect::<Vec<_>>()
                .join(",")
        ),
        MirValue::Unknown { evidence } => format!("unknown:{evidence}"),
    }
}

fn provider_error_diagnostic(reason: String) -> Diagnostic {
    Diagnostic::error(
        "polint/internal",
        "semantic MIR provider",
        TextRange::point(1, 1),
        "Semantic MIR provider rejected lowered rows.",
    )
    .with_evidence("family", "SemanticMir")
    .with_evidence("stable_key", "")
    .with_evidence("field", "provider")
    .with_evidence("reason", reason)
}

#[cfg(test)]
mod semantic_mir_provider {
    #[cfg(any(feature = "lang-go", feature = "lang-typescript"))]
    use crate::analysis::ids::CallSiteId;
    use crate::analysis::ids::{MirBodyId, MirOpId, MirPredicateId, PlaceId};
    use crate::analysis::mir::body::{MirBody, MirOutput, MirStatus};
    use crate::analysis::mir::op::{MirOperation, MirOperationKind};
    use crate::analysis::places::{PlaceFact, PlaceRoot, PlaceStatus};
    use crate::analysis_kernel::AnalysisKernel;
    #[cfg(any(feature = "lang-go", feature = "lang-typescript"))]
    use crate::analysis_kernel::KernelInput;
    use crate::analysis_kernel::incremental::{Digest, DigestKind};
    #[cfg(any(feature = "lang-go", feature = "lang-typescript"))]
    use crate::analysis_plan::AnalysisPlan;
    #[cfg(any(feature = "lang-go", feature = "lang-typescript"))]
    use crate::cache::Cache;
    use crate::config::load_config;
    use crate::core::{FileId, FunctionId, Language, Span};

    fn span() -> Span {
        Span::new(FileId::from_raw(1), 1, 2, 1, 1, 1, 2)
    }

    #[cfg(any(feature = "lang-go", feature = "lang-typescript"))]
    fn call_sites_from_real_provider(
        files: &[(&str, &str)],
    ) -> Vec<(String, Language, CallSiteId, Span)> {
        let temp = tempfile::tempdir().expect("tempdir");
        for (relative_path, source) in files {
            let path = temp.path().join(relative_path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create source parent");
            }
            std::fs::write(path, source).expect("write source");
        }
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new("", false);
        let plan = AnalysisPlan::from_capability_names_for_test(&["calls"]);
        let output = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("kernel should run");
        let mut sites = output
            .db
            .call_sites()
            .iter()
            .map(|site| {
                (
                    output.db.path_for(site.file),
                    site.language,
                    site.id,
                    site.span.clone(),
                )
            })
            .collect::<Vec<_>>();
        sites.sort_by(|left, right| left.0.cmp(&right.0));
        sites
    }

    fn body(
        interner: &crate::core::StableKeyInterner,
        id: u64,
        function: u64,
        stable_key: &str,
    ) -> MirBody {
        MirBody {
            id: MirBodyId(id),
            language: Language::Go,
            file: FileId::from_raw(1),
            function: FunctionId::from_raw(function),
            package: None,
            module: None,
            owner_stable_key: interner.intern(stable_key.to_string()),
            span: span(),
            stable_key: interner.intern(stable_key.to_string()),
            status: MirStatus::Resolved,
        }
    }

    fn local_place(
        interner: &crate::core::StableKeyInterner,
        id: u64,
        function: u64,
        stable_key: &str,
    ) -> PlaceFact {
        PlaceFact {
            id: PlaceId(id),
            language: Language::Go,
            file: Some(FileId::from_raw(1)),
            function: Some(FunctionId::from_raw(function)),
            root: PlaceRoot::Local {
                function: FunctionId::from_raw(function),
                name: stable_key.to_string(),
            },
            projections: Vec::new(),
            stable_key: interner.intern(stable_key.to_string()),
            status: PlaceStatus::Resolved,
        }
    }

    fn branch_operation(
        interner: &crate::core::StableKeyInterner,
        id: u64,
        body: u64,
        predicate_place: PlaceId,
        stable_key: &str,
    ) -> MirOperation {
        MirOperation {
            id: MirOpId(id),
            body: MirBodyId(body),
            ordinal: 1,
            span: span(),
            kind: MirOperationKind::Branch {
                predicate: MirPredicateId(1),
                predicate_place: Some(predicate_place),
                nil_test: None,
            },
            stable_key: interner.intern(stable_key.to_string()),
            status: MirStatus::Resolved,
        }
    }

    #[cfg(all(feature = "lang-go", feature = "lang-typescript"))]
    #[test]
    fn derivation_stores_non_empty_mixed_language_mir() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join("main.go"),
            "package main\nfunc add(left int, right int) int { total := left + right; return total }\n",
        )
        .expect("write go");
        std::fs::write(
            temp.path().join("app.ts"),
            "export function render(value: number) { const next = value + 1; return next; }\n",
        )
        .expect("write ts");
        let loaded = load_config(temp.path()).expect("default config loads");
        let cache = Cache::new("", false);
        let plan = AnalysisPlan::from_capability_names_for_test(&["control_flow"]);

        let output = AnalysisKernel::run(KernelInput {
            loaded: &loaded,
            cache: &cache,
            config_digest: "config",
            rule_digest: "rules",
            plan: &plan,
            parallel: false,
        })
        .expect("kernel should run");

        assert!(
            output.db.mir_bodies().len() >= 2,
            "expected Go and TS MIR bodies"
        );
        assert!(output.db.mir_places().len() >= 2, "expected lowered places");
        assert!(
            output.db.mir_operations().len() >= 2,
            "expected lowered operations"
        );
        assert!(
            output.db.missing_fact_metadata().is_empty(),
            "scheduled provider run left missing fact metadata"
        );
    }

    #[test]
    fn derivation_output_digest_changes_with_upstream_inputs() {
        let temp = tempfile::tempdir().expect("tempdir");
        let loaded = load_config(temp.path()).expect("default config loads");
        let mut db = crate::core::AnalysisDb::new();
        let manifest = AnalysisKernel::provider_manifests()
            .iter()
            .find(|manifest| manifest.id == "polint.semantic_mir")
            .expect("semantic MIR manifest");
        let input_snapshot = crate::analysis_kernel::incremental::input_snapshot_from_run_inputs(
            &loaded,
            &db,
            "config",
            "rules",
            "plan",
            AnalysisKernel::provider_manifests(),
        );
        let module_topology = Digest::from_parts(
            DigestKind::ProviderOutput,
            "module_topology",
            &["topology-a"],
        );
        let symbol_graph =
            Digest::from_parts(DigestKind::ProviderOutput, "symbol_graph", &["symbols-a"]);
        let syntax_a = vec![Digest::from_parts(
            DigestKind::ProviderOutput,
            "syntax",
            &["go-a"],
        )];
        let syntax_b = vec![Digest::from_parts(
            DigestKind::ProviderOutput,
            "syntax",
            &["go-b"],
        )];

        let first = super::derive_semantic_mir_with_cache_stats(
            &mut db,
            &input_snapshot,
            manifest,
            module_topology.clone(),
            symbol_graph.clone(),
            syntax_a,
        );
        let second = super::derive_semantic_mir_with_cache_stats(
            &mut db,
            &input_snapshot,
            manifest,
            module_topology,
            symbol_graph,
            syntax_b,
        );

        assert_ne!(first.output_digest, second.output_digest);
    }

    #[test]
    fn merge_language_outputs_offsets_branch_predicate_place_references() {
        let interner = crate::core::StableKeyInterner::default();
        let first = MirOutput {
            bodies: vec![body(&interner, 0, 1, "body:first")],
            places: vec![local_place(&interner, 0, 1, "place:first")],
            operations: Vec::new(),
            unsupported: Vec::new(),
            ..MirOutput::default()
        };
        let second = MirOutput {
            bodies: vec![body(&interner, 0, 2, "body:second")],
            places: vec![local_place(&interner, 0, 2, "place:second")],
            operations: vec![branch_operation(
                &interner,
                0,
                0,
                PlaceId(0),
                "op:second:branch",
            )],
            unsupported: Vec::new(),
            ..MirOutput::default()
        };

        let output = super::merge_language_outputs([first, second], &interner);
        let branch = output
            .operations
            .iter()
            .find(|operation| interner.resolve(operation.stable_key).as_ref() == "op:second:branch")
            .expect("merged branch operation");

        assert!(matches!(
            branch.kind,
            MirOperationKind::Branch {
                predicate_place: Some(PlaceId(1)),
                ..
            }
        ));
    }

    #[cfg(feature = "lang-go")]
    #[test]
    fn real_go_provider_assigns_distinct_call_site_ids_per_file() {
        let sites = call_sites_from_real_provider(&[
            (
                "a.go",
                "package p\n\nfunc caller() { helper() }\nfunc helper() {}\n",
            ),
            (
                "b.go",
                "package p\n\nfunc caller() { helper() }\nfunc helper() {}\n",
            ),
        ]);
        let reversed = call_sites_from_real_provider(&[
            (
                "b.go",
                "package p\n\nfunc caller() { helper() }\nfunc helper() {}\n",
            ),
            (
                "a.go",
                "package p\n\nfunc caller() { helper() }\nfunc helper() {}\n",
            ),
        ]);

        assert_eq!(sites.len(), 2);
        assert_eq!(reversed.len(), 2);
        let stable_ids = sites
            .iter()
            .map(|site| (site.0.clone(), site.2))
            .collect::<Vec<_>>();
        let reversed_stable_ids = reversed
            .iter()
            .map(|site| (site.0.clone(), site.2))
            .collect::<Vec<_>>();
        assert_eq!(stable_ids, reversed_stable_ids);
        assert_eq!(sites[0].1, Language::Go);
        assert_eq!(sites[1].1, Language::Go);
        assert_eq!(sites[0].3.start_byte, sites[1].3.start_byte);
        assert_ne!(sites[0].2, sites[1].2);
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn real_ts_provider_assigns_distinct_call_site_ids_per_file() {
        let sites = call_sites_from_real_provider(&[
            (
                "a.ts",
                "export function caller() { helper(); }\nfunction helper() {}\n",
            ),
            (
                "b.ts",
                "export function caller() { helper(); }\nfunction helper() {}\n",
            ),
        ]);

        assert_eq!(sites.len(), 2);
        assert!(sites.iter().all(|site| site.1 == Language::TypeScript));
        assert_eq!(sites[0].3.start_byte, sites[1].3.start_byte);
        assert_ne!(sites[0].2, sites[1].2);
    }

    #[cfg(all(feature = "lang-go", feature = "lang-typescript"))]
    #[test]
    fn real_polyglot_provider_separates_equal_legacy_call_site_coordinates() {
        let sites = call_sites_from_real_provider(&[
            ("caller.go", "package p\n\nfunc caller() { f() }\n"),
            ("caller.ts", "aaaaaaaaaaaaaaaaaaaaaaaaa();\n"),
        ]);

        assert_eq!(sites.len(), 2);
        let go = sites
            .iter()
            .find(|site| site.1 == Language::Go)
            .expect("Go call site");
        let ts = sites
            .iter()
            .find(|site| site.1 == Language::TypeScript)
            .expect("TS call site");
        let legacy_go = u64::from(go.3.start_byte);
        let legacy_ts = (u64::from(ts.3.start_byte) << 32) | u64::from(ts.3.end_byte);
        assert_eq!(legacy_go, legacy_ts);
        assert_ne!(go.2, ts.2);
    }
}
