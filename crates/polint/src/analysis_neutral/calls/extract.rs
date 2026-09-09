use std::collections::BTreeMap;

use crate::analysis_api::FunctionFact;
use crate::analysis_api::{FactFamily, FactRef};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::calls::facts::{
    CallCallee, CallPrecision, CallSiteFact, CallSyntaxKind, CallTargetStatus, UnresolvedCallReason,
};
use crate::analysis_neutral::ids::{MirValueId, PlaceId};
use crate::analysis_neutral::mir_body::MirBody;
use crate::analysis_neutral::mir_op::{MirOperation, MirOperationKind, MirValue};
use crate::analysis_neutral::places::{PlaceFact, PlaceProjection, PlaceRoot};
use crate::analysis_neutral::stable_key::semantic_stable_key;
use crate::internal_core::{FileId, FunctionId, Language, Span, SymbolId};

pub fn extract_call_sites(db: &impl AnalysisHost) -> Vec<CallSiteFact> {
    let bodies = db
        .mir_bodies()
        .iter()
        .map(|body| (body.id, body))
        .collect::<BTreeMap<_, _>>();
    let places = db
        .mir_places()
        .iter()
        .map(|place| (place.id, place))
        .collect::<BTreeMap<_, _>>();
    // Names each Go function binds as a local or a parameter. A callee naming one
    // of these is a call *through a variable*; see `variable_callee`.
    //
    // Indexed for Go bodies only, and skipped outright when the program has none.
    // `variable_callee` is Go-scoped, so indexing a large TypeScript repository
    // here would be pure overhead — and not cheap overhead: excalidraw carries
    // enough places that cloning a name per place cost minutes of wall clock and
    // gigabytes of resident set before this filter was added.
    let go_functions = db
        .mir_bodies()
        .iter()
        .filter(|body| matches!(body.language, Language::Go))
        .map(|body| body.function)
        .collect::<std::collections::BTreeSet<_>>();
    let mut variable_names: BTreeMap<FunctionId, std::collections::BTreeSet<String>> =
        BTreeMap::new();
    if !go_functions.is_empty() {
        for place in db.mir_places() {
            let (function, name) = match &place.root {
                PlaceRoot::Local { function, name } => (function, name),
                PlaceRoot::Parameter {
                    function,
                    name: Some(name),
                    ..
                } => (function, name),
                _ => continue,
            };
            if go_functions.contains(function) {
                variable_names
                    .entry(*function)
                    .or_default()
                    .insert(name.clone());
            }
        }
    }
    let functions = db
        .functions()
        .iter()
        .map(|function| (function.id, function))
        .collect::<BTreeMap<_, _>>();

    let mut call_operations = db
        .mir_operations()
        .iter()
        .filter_map(|operation| bodies.get(&operation.body).map(|body| (*body, operation)))
        .filter(|(_, operation)| matches!(operation.kind, MirOperationKind::Call { .. }))
        .collect::<Vec<_>>();
    call_operations.sort_by(
        |(left_body, left_operation), (right_body, right_operation)| {
            (
                db.resolve_stable_key(left_body.stable_key),
                span_key(&left_operation.span),
                left_operation.ordinal,
                db.resolve_stable_key(left_operation.stable_key),
                left_operation.id,
            )
                .cmp(&(
                    db.resolve_stable_key(right_body.stable_key),
                    span_key(&right_operation.span),
                    right_operation.ordinal,
                    db.resolve_stable_key(right_operation.stable_key),
                    right_operation.id,
                ))
        },
    );

    // Spans of `throw` statements, per file — a call site whose span is contained
    // in one is lexically inside a `throw` argument (error path). The TS/JS MIR
    // lowering records a `throw` unsupported-semantic fact spanning the whole throw
    // statement, so containment marks `f()` in `throw new E(... f() ...)`.
    let mut throw_spans: BTreeMap<FileId, Vec<(u32, u32)>> = BTreeMap::new();
    for fact in db.unsupported_semantics() {
        if fact.construct == "throw" {
            throw_spans
                .entry(fact.file)
                .or_default()
                .push((fact.span.start_byte, fact.span.end_byte));
        }
    }
    let is_in_throw = |file: FileId, span: &Span| -> bool {
        throw_spans.get(&file).is_some_and(|spans| {
            spans
                .iter()
                .any(|(start, end)| *start <= span.start_byte && span.end_byte <= *end)
        })
    };

    let mut sites = Vec::with_capacity(call_operations.len());
    for (body, operation) in call_operations {
        let MirOperationKind::Call {
            site,
            callee,
            arguments,
            return_place,
        } = &operation.kind
        else {
            continue;
        };
        let (call_callee, receiver, kind, callee_shape) = call_callee(
            callee,
            body.language,
            &places,
            body.function,
            &variable_names,
        );
        let operation_stable_key = db.resolve_stable_key(operation.stable_key);

        sites.push(CallSiteFact {
            id: *site,
            language: body.language,
            file: body.file,
            caller: body.function,
            owner_symbol: owner_symbol(db, &functions, body.function),
            body: body.id,
            operation: operation.id,
            span: operation.span.clone(),
            kind,
            callee: call_callee,
            receiver,
            arguments: arguments.clone(),
            result: Some(*return_place),
            status: CallTargetStatus::Unresolved,
            precision: CallPrecision::Conservative,
            in_throw: is_in_throw(body.file, &operation.span),
            stable_key: call_site_stable_key(
                db,
                body,
                operation,
                kind,
                &callee_shape,
                &operation_stable_key,
            ),
        });
    }

    sites.sort_by(|left, right| {
        (db.resolve_stable_key(left.stable_key), left.id)
            .cmp(&(db.resolve_stable_key(right.stable_key), right.id))
    });
    sites
}

fn call_callee(
    value: &MirValue,
    language: Language,
    places: &BTreeMap<PlaceId, &PlaceFact>,
    caller: FunctionId,
    variable_names: &BTreeMap<FunctionId, std::collections::BTreeSet<String>>,
) -> (CallCallee, Option<PlaceId>, CallSyntaxKind, String) {
    match value {
        MirValue::Place(place) => place_callee(*place, language, places.get(place).copied()),
        MirValue::Temporary(value) => temporary_callee(*value),
        MirValue::CallReturn(site) => (
            CallCallee::Unknown {
                reason: UnresolvedCallReason::FunctionValue,
            },
            None,
            CallSyntaxKind::FunctionValue,
            format!("call_return:{}", site.0),
        ),
        MirValue::Unknown { evidence } => {
            variable_callee(evidence, language, caller, variable_names)
                .unwrap_or_else(|| evidence_callee(evidence, language))
        }
        MirValue::Closure { body, .. } => (
            CallCallee::Unknown {
                reason: UnresolvedCallReason::FunctionValue,
            },
            None,
            CallSyntaxKind::FunctionValue,
            format!("closure:{}", body.0),
        ),
        MirValue::BinOp { .. } | MirValue::Aggregate { .. } => (
            CallCallee::Unknown {
                reason: UnresolvedCallReason::UnsupportedSyntax,
            },
            None,
            CallSyntaxKind::Unknown,
            "structured-value".to_string(),
        ),
        MirValue::Literal { value } => (
            CallCallee::Unknown {
                reason: UnresolvedCallReason::UnsupportedSyntax,
            },
            None,
            CallSyntaxKind::Unknown,
            format!("literal:{}", value.trim()),
        ),
    }
}

/// A Go callee naming one of the caller's own locals or parameters is a call
/// through a variable holding a function — `UnresolvedCallReason::FunctionValue`
/// — not a name the frontend merely failed to look up.
///
/// `place_callee` already draws exactly this line, but only for callees a
/// frontend hands over as a place: a `PlaceRoot::Local`/`Parameter` root is a
/// function value. Neither frontend does that today — both lower every callee as
/// `MirValue::Unknown` carrying the callee's source text — so that arm is
/// unreachable and the classification has to be recovered from the caller's own
/// place table, the one place the frontend left the signal.
///
/// Deliberately **not** the `matches!(evidence, "fn" | "callable" | "callback")`
/// branch this replaces. That matched on spelling, so it labelled a variable
/// named `fn` correctly and an identically-shaped `apply` wrongly, and it fired
/// on those three spellings whether or not a variable of that name existed.
///
/// Restricted to Go, on evidence rather than by preference. In Go this shape is
/// never resolvable: the frontend supplies no semantic reference for a local
/// variable callee, so `FunctionValue` states exactly what is known and costs
/// nothing. In TypeScript the same shape *is* resolvable — the callable-flow
/// collector resolves 243 of them in the Jelly corpus — and `FunctionValue`
/// carries a `PlaceId::MAX` sentinel with no points-to reach, so applying it
/// there is a give-up marker that drops those edges (measured: Jelly TP 986 ->
/// 743, F1 0.790381 -> 0.663393). The real repair for both languages is for the
/// frontends to lower callee places so `place_callee` can do this properly;
/// until then this keeps Go's classification honest without touching TypeScript.
fn variable_callee(
    evidence: &str,
    language: Language,
    caller: FunctionId,
    variable_names: &BTreeMap<FunctionId, std::collections::BTreeSet<String>>,
) -> Option<(CallCallee, Option<PlaceId>, CallSyntaxKind, String)> {
    if !matches!(language, Language::Go) || !is_identifier_like(evidence) {
        return None;
    }
    // Keyed by function so the name lookup borrows `evidence` instead of
    // allocating a tuple for every unresolved callee in the program.
    if !variable_names
        .get(&caller)
        .is_some_and(|names| names.contains(evidence))
    {
        return None;
    }
    Some((
        CallCallee::FunctionValue {
            place: PlaceId(u64::MAX),
        },
        None,
        CallSyntaxKind::FunctionValue,
        "function_value".to_string(),
    ))
}

fn evidence_callee(
    evidence: &str,
    language: Language,
) -> (CallCallee, Option<PlaceId>, CallSyntaxKind, String) {
    let evidence = evidence.trim();
    if evidence.is_empty() {
        return unknown_evidence_callee(evidence, UnresolvedCallReason::Unknown);
    }

    if let Some(name) = constructor_evidence_name(evidence) {
        return (
            CallCallee::Constructor {
                reference: None,
                name: Some(name.to_string()),
            },
            None,
            CallSyntaxKind::Constructor,
            format!("constructor:{name}"),
        );
    }

    if evidence.starts_with("import(") {
        return (
            CallCallee::Import,
            None,
            CallSyntaxKind::DynamicImport,
            "dynamic_import".to_string(),
        );
    }

    if evidence.to_ascii_lowercase().contains("dynamicimport") {
        return unknown_evidence_callee(evidence, UnresolvedCallReason::DynamicImport);
    }

    if evidence.to_ascii_lowercase().contains("setupmissing")
        || evidence.to_ascii_lowercase().contains("setup missing")
    {
        return unknown_evidence_callee(evidence, UnresolvedCallReason::SetupMissing);
    }

    if evidence == "eval" {
        return unknown_evidence_callee(evidence, UnresolvedCallReason::Eval);
    }

    if let Some((_, property)) = evidence.rsplit_once('.')
        && is_identifier_like(property)
    {
        if matches!(property, "call" | "apply" | "bind") {
            return unknown_evidence_callee(evidence, UnresolvedCallReason::CallApplyBind);
        }
        let kind = if is_static_member_evidence(language, evidence) {
            CallSyntaxKind::StaticMember
        } else {
            CallSyntaxKind::Member
        };
        return (
            CallCallee::Member {
                base: PlaceId(u64::MAX),
                property: property.to_string(),
            },
            None,
            kind,
            format!("member:{property}"),
        );
    }

    if crate::analysis_api::is_anonymous_callable_name(evidence) {
        return (
            CallCallee::Identifier {
                reference: None,
                name: evidence.to_string(),
            },
            None,
            CallSyntaxKind::Function,
            format!("identifier:{evidence}"),
        );
    }

    if is_identifier_like(evidence) {
        if is_constructor_name(language, evidence) {
            return (
                CallCallee::Constructor {
                    reference: None,
                    name: Some(evidence.to_string()),
                },
                None,
                CallSyntaxKind::Constructor,
                format!("constructor:{evidence}"),
            );
        }
        return (
            CallCallee::Identifier {
                reference: None,
                name: evidence.to_string(),
            },
            None,
            CallSyntaxKind::Function,
            format!("identifier:{evidence}"),
        );
    }

    unknown_evidence_callee(evidence, UnresolvedCallReason::Unknown)
}

fn unknown_evidence_callee(
    evidence: &str,
    reason: UnresolvedCallReason,
) -> (CallCallee, Option<PlaceId>, CallSyntaxKind, String) {
    (
        CallCallee::Unknown { reason },
        None,
        CallSyntaxKind::Unknown,
        format!("unknown:{}", evidence.trim()),
    )
}

fn constructor_evidence_name(evidence: &str) -> Option<&str> {
    evidence
        .strip_prefix("new ")
        .and_then(|rest| rest.split(['(', '<', ' ']).next())
        .filter(|name| is_identifier_like(name))
}

fn is_static_member_evidence(language: Language, evidence: &str) -> bool {
    matches!(language, Language::TypeScript | Language::JavaScript)
        && evidence
            .split('.')
            .next()
            .is_some_and(|base| base.chars().next().is_some_and(char::is_uppercase))
}

fn is_identifier_like(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first == '$' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch == '$' || ch.is_ascii_alphanumeric())
}

fn temporary_callee(value: MirValueId) -> (CallCallee, Option<PlaceId>, CallSyntaxKind, String) {
    (
        CallCallee::Unknown {
            reason: UnresolvedCallReason::MissingSemanticReference,
        },
        None,
        CallSyntaxKind::Unknown,
        format!("temporary:{}", value.0),
    )
}

fn place_callee(
    place: PlaceId,
    language: Language,
    fact: Option<&PlaceFact>,
) -> (CallCallee, Option<PlaceId>, CallSyntaxKind, String) {
    let Some(fact) = fact else {
        return (
            CallCallee::FunctionValue { place },
            Some(place),
            CallSyntaxKind::FunctionValue,
            "function_value".to_string(),
        );
    };

    if let Some((callee, kind, shape)) = projection_callee(place, &fact.projections) {
        return (callee, Some(place), kind, shape);
    }

    match &fact.root {
        PlaceRoot::Global { name, .. } if is_constructor_name(language, name) => (
            CallCallee::Constructor {
                reference: None,
                name: Some(name.clone()),
            },
            None,
            CallSyntaxKind::Constructor,
            format!("constructor:{name}"),
        ),
        PlaceRoot::Global { name, .. } => (
            CallCallee::Identifier {
                reference: None,
                name: name.clone(),
            },
            None,
            CallSyntaxKind::Function,
            format!("identifier:{name}"),
        ),
        PlaceRoot::Unknown { evidence } => (
            CallCallee::Unknown {
                reason: UnresolvedCallReason::Unknown,
            },
            None,
            CallSyntaxKind::Unknown,
            format!("unknown:{}", evidence.trim()),
        ),
        PlaceRoot::Local { .. }
        | PlaceRoot::Parameter { .. }
        | PlaceRoot::Temporary { .. }
        | PlaceRoot::CallReturn { .. } => (
            CallCallee::FunctionValue { place },
            Some(place),
            CallSyntaxKind::FunctionValue,
            "function_value".to_string(),
        ),
    }
}

fn projection_callee(
    place: PlaceId,
    projections: &[PlaceProjection],
) -> Option<(CallCallee, CallSyntaxKind, String)> {
    let projection = projections.last()?;
    match projection {
        PlaceProjection::Field(property) | PlaceProjection::Property(property) => Some((
            CallCallee::Member {
                base: place,
                property: property.clone(),
            },
            CallSyntaxKind::Member,
            format!("member:{property}"),
        )),
        PlaceProjection::IndexKnown(index) => Some((
            CallCallee::Index {
                base: place,
                index: None,
            },
            CallSyntaxKind::Index,
            format!("index_known:{index}"),
        )),
        PlaceProjection::IndexUnknown { evidence } => Some((
            CallCallee::Index {
                base: place,
                index: None,
            },
            CallSyntaxKind::Index,
            format!("index_unknown:{}", evidence.trim()),
        )),
        PlaceProjection::CallReturn(call) => Some((
            CallCallee::Unknown {
                reason: UnresolvedCallReason::FunctionValue,
            },
            CallSyntaxKind::FunctionValue,
            format!("call_return:{}", call.0),
        )),
        PlaceProjection::Unknown { evidence } => Some((
            CallCallee::Unknown {
                reason: UnresolvedCallReason::Unknown,
            },
            CallSyntaxKind::Unknown,
            format!("unknown_projection:{}", evidence.trim()),
        )),
        PlaceProjection::Deref | PlaceProjection::AwaitResult => None,
    }
}

fn call_site_stable_key(
    db: &impl AnalysisHost,
    body: &MirBody,
    operation: &MirOperation,
    kind: CallSyntaxKind,
    callee_shape: &str,
    operation_stable_key: &str,
) -> crate::internal_core::StableKeyId {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    interner.intern(
        semantic_stable_key(
            FactFamily::CallSite,
            &[
                ("language", format!("{:?}", body.language)),
                ("file_key", file_key(db, body.file)),
                ("caller_key", caller_key(db, body.function)),
                ("span", span_key(&operation.span)),
                ("callee_shape", callee_shape.to_string()),
                ("operation_key", operation_stable_key.to_string()),
                ("call_kind", format!("{kind:?}")),
            ],
        )
        .into_string(),
    )
}

fn owner_symbol(
    db: &impl AnalysisHost,
    functions: &BTreeMap<FunctionId, &FunctionFact>,
    function: FunctionId,
) -> Option<SymbolId> {
    let function = functions.get(&function)?;
    db.symbols()
        .iter()
        .find(|symbol| {
            symbol.file == Some(function.file)
                && symbol.name == function.name
                && symbol.primary_span.as_ref() == Some(&function.span)
        })
        .map(|symbol| symbol.id)
}

fn file_key(db: &impl AnalysisHost, file: FileId) -> String {
    db.metadata_for(FactRef::new(FactFamily::SourceFile, u64::from(file.0)))
        .map(|metadata| db.resolve_stable_key(metadata.stable_key).to_string())
        .or_else(|| {
            db.files()
                .iter()
                .find(|source_file| source_file.id == file)
                .map(|source_file| source_file.relative_path.replace('\\', "/"))
        })
        .unwrap_or_else(|| format!("<missing-file:{}>", file.0))
}

fn caller_key(db: &impl AnalysisHost, function: FunctionId) -> String {
    db.metadata_for(FactRef::new(FactFamily::Function, function.0))
        .map(|metadata| db.resolve_stable_key(metadata.stable_key).to_string())
        .unwrap_or_else(|| format!("<missing-function:{}>", function.0))
}

fn span_key(span: &Span) -> String {
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

fn is_constructor_name(language: Language, name: &str) -> bool {
    matches!(language, Language::TypeScript | Language::JavaScript)
        && name.chars().next().is_some_and(char::is_uppercase)
}

#[cfg(test)]
mod tests {
    use crate::analysis_neutral::AnalysisHost;
    use std::path::PathBuf;

    use crate::analysis_api::FunctionFact;
    use crate::analysis_api::anonymous_callable_name;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::calls::facts::{
        CallCallee, CallPrecision, CallSyntaxKind, CallTargetStatus,
    };
    use crate::analysis_neutral::ids::{CallSiteId, MirBodyId, MirOpId, PlaceId};
    use crate::analysis_neutral::mir_body::{MirBody, MirOutput, MirStatus};
    use crate::analysis_neutral::mir_op::{MirOperation, MirOperationKind, MirValue};
    use crate::analysis_neutral::places::{PlaceFact, PlaceProjection, PlaceRoot, PlaceStatus};
    use crate::internal_core::{FileId, FunctionId, Language, Span};

    fn span(file: FileId, line: u32, start_byte: u32) -> Span {
        Span::new(file, start_byte, start_byte + 4, line, 1, line, 5)
    }

    fn add_file_and_function(
        db: &mut impl AnalysisHost,
        relative_path: &str,
    ) -> (FileId, FunctionId) {
        let file = db.add_file(
            PathBuf::from(relative_path),
            relative_path.to_string(),
            "function caller() {}".to_string(),
        );
        let function = db.push_function(FunctionFact::new(
            FunctionId::from_raw(999),
            file,
            "caller".to_string(),
            span(file, 1, 0),
            Language::TypeScript,
            false,
            true,
            1,
            Vec::new(),
        ));
        (file, function)
    }

    fn key(db: &impl AnalysisHost, text: impl Into<String>) -> crate::internal_core::StableKeyId {
        db.stable_key_interner().intern(text.into())
    }

    fn body(
        db: &impl AnalysisHost,
        file: FileId,
        function: FunctionId,
        language: Language,
    ) -> MirBody {
        MirBody {
            id: MirBodyId(1),
            language,
            file,
            function,
            package: None,
            module: None,
            owner_stable_key: key(db, "function:caller:stable"),
            span: span(file, 1, 0),
            stable_key: key(db, "mir-body:caller"),
            status: MirStatus::Resolved,
        }
    }

    fn place(
        db: &impl AnalysisHost,
        id: u64,
        file: FileId,
        function: FunctionId,
        root: PlaceRoot,
        projections: Vec<PlaceProjection>,
    ) -> PlaceFact {
        PlaceFact {
            id: PlaceId(id),
            language: Language::TypeScript,
            file: Some(file),
            function: Some(function),
            root,
            projections,
            stable_key: key(db, format!("place:{id}")),
            status: PlaceStatus::Resolved,
        }
    }

    fn call_op(
        db: &impl AnalysisHost,
        id: u64,
        ordinal: u32,
        file: FileId,
        site: u64,
        callee: MirValue,
        arguments: Vec<PlaceId>,
    ) -> MirOperation {
        MirOperation {
            id: MirOpId(id),
            body: MirBodyId(1),
            ordinal,
            span: span(file, 2, 10),
            kind: MirOperationKind::Call {
                site: CallSiteId(site),
                callee,
                arguments,
                return_place: PlaceId(9),
            },
            stable_key: key(db, format!("mir-op:call:{id}")),
            status: MirStatus::Resolved,
        }
    }

    #[test]
    fn extract_call_sites_maps_mir_calls_to_complete_call_site_facts() {
        let mut db = LocalAnalysisDb::new();
        let (file, function) = add_file_and_function(&mut db, "src/app.ts");
        db.replace_semantic_mir(MirOutput {
            bodies: vec![body(&db, file, function, Language::TypeScript)],
            places: vec![
                place(
                    &db,
                    1,
                    file,
                    function,
                    PlaceRoot::Global {
                        symbol: None,
                        name: "run".to_string(),
                    },
                    Vec::new(),
                ),
                place(
                    &db,
                    2,
                    file,
                    function,
                    PlaceRoot::Local {
                        function,
                        name: "arg".to_string(),
                    },
                    Vec::new(),
                ),
                place(
                    &db,
                    9,
                    file,
                    function,
                    PlaceRoot::CallReturn {
                        call: CallSiteId(10),
                    },
                    Vec::new(),
                ),
            ],
            operations: vec![call_op(
                &db,
                1,
                0,
                file,
                10,
                MirValue::Place(PlaceId(1)),
                vec![PlaceId(2)],
            )],
            unsupported: Vec::new(),
            ..MirOutput::default()
        })
        .expect("semantic MIR should store");

        let sites = super::extract_call_sites(&db);

        assert_eq!(sites.len(), 1);
        let site = &sites[0];
        assert_eq!(site.id, CallSiteId(10));
        assert_eq!(site.language, Language::TypeScript);
        assert_eq!(site.file, file);
        assert_eq!(site.caller, function);
        assert_eq!(site.body, MirBodyId(0));
        assert_eq!(site.operation, MirOpId(0));
        assert_eq!(site.kind, CallSyntaxKind::Function);
        assert_eq!(
            site.callee,
            CallCallee::Identifier {
                reference: None,
                name: "run".to_string()
            }
        );
        assert_eq!(site.arguments, vec![PlaceId(1)]);
        assert_eq!(site.result, Some(PlaceId(2)));
        assert_eq!(site.status, CallTargetStatus::Unresolved);
        assert_eq!(site.precision, CallPrecision::Conservative);
    }

    /// A Go callee naming one of the caller's own variables is a function value,
    /// whatever the variable is called — the branch this replaces matched three
    /// hardcoded spellings, so it labelled `fn` right and an identically-shaped
    /// `apply` wrong, and fired on those spellings with no such variable in
    /// scope. TypeScript keeps its identifier evidence: there the shape is
    /// resolvable, and the sentinel-placed `FunctionValue` would be a give-up
    /// marker that drops real edges.
    #[test]
    fn a_go_callee_naming_a_caller_variable_is_a_function_value() {
        let caller = FunctionId::from_raw(7);
        let other = FunctionId::from_raw(8);
        let variables = std::collections::BTreeMap::from([(
            caller,
            ["fn", "apply", "handler"]
                .into_iter()
                .map(str::to_string)
                .collect::<std::collections::BTreeSet<_>>(),
        )]);

        for name in ["fn", "apply", "handler"] {
            let (callee, receiver, kind, shape) =
                super::variable_callee(name, Language::Go, caller, &variables)
                    .expect("a caller variable in Go");
            assert_eq!(
                callee,
                CallCallee::FunctionValue {
                    place: PlaceId(u64::MAX)
                }
            );
            assert_eq!(receiver, None);
            assert_eq!(kind, CallSyntaxKind::FunctionValue);
            assert_eq!(shape, "function_value");

            // The same shape in TypeScript stays an identifier.
            assert!(
                super::variable_callee(name, Language::TypeScript, caller, &variables).is_none()
            );
            assert!(
                super::variable_callee(name, Language::JavaScript, caller, &variables).is_none()
            );
        }

        // A variable of a *different* function, a name no variable binds, and a
        // non-identifier callee all fall through to the evidence classifier
        // rather than claiming a function value.
        assert!(super::variable_callee("fn", Language::Go, other, &variables).is_none());
        assert!(
            super::variable_callee("directFunction", Language::Go, caller, &variables).is_none()
        );
        assert!(super::variable_callee("a.b", Language::Go, caller, &variables).is_none());
    }

    #[test]
    fn common_callback_names_retain_their_identifier_evidence() {
        for name in ["fn", "callable", "callback", "handler"] {
            let (callee, receiver, kind, _) = super::evidence_callee(name, Language::JavaScript);
            assert_eq!(
                callee,
                CallCallee::Identifier {
                    reference: None,
                    name: name.to_string(),
                }
            );
            assert_eq!(receiver, None);
            assert_eq!(kind, CallSyntaxKind::Function);
        }
    }

    #[test]
    fn extract_call_sites_treats_anonymous_callable_evidence_as_lexical_callee() {
        let mut db = LocalAnalysisDb::new();
        let (file, function) = add_file_and_function(&mut db, "src/iife.ts");
        let anonymous = anonymous_callable_name(1, 14);
        db.replace_semantic_mir(MirOutput {
            bodies: vec![body(&db, file, function, Language::TypeScript)],
            places: vec![place(
                &db,
                9,
                file,
                function,
                PlaceRoot::CallReturn {
                    call: CallSiteId(10),
                },
                Vec::new(),
            )],
            operations: vec![call_op(
                &db,
                1,
                0,
                file,
                10,
                MirValue::Unknown {
                    evidence: anonymous.clone(),
                },
                Vec::new(),
            )],
            unsupported: Vec::new(),
            ..MirOutput::default()
        })
        .expect("semantic MIR should store");

        let sites = super::extract_call_sites(&db);

        assert_eq!(
            sites[0].callee,
            CallCallee::Identifier {
                reference: None,
                name: anonymous
            }
        );
        assert_eq!(sites[0].kind, CallSyntaxKind::Function);
    }

    #[test]
    fn extract_call_sites_stable_key_uses_required_stable_inputs() {
        let mut db = LocalAnalysisDb::new();
        let (file, function) = add_file_and_function(&mut db, "src/app.ts");
        db.replace_semantic_mir(MirOutput {
            bodies: vec![body(&db, file, function, Language::TypeScript)],
            places: vec![
                place(
                    &db,
                    1,
                    file,
                    function,
                    PlaceRoot::Local {
                        function,
                        name: "callback".to_string(),
                    },
                    Vec::new(),
                ),
                place(
                    &db,
                    9,
                    file,
                    function,
                    PlaceRoot::CallReturn {
                        call: CallSiteId(10),
                    },
                    Vec::new(),
                ),
            ],
            operations: vec![call_op(
                &db,
                1,
                0,
                file,
                10,
                MirValue::Place(PlaceId(1)),
                Vec::new(),
            )],
            unsupported: Vec::new(),
            ..MirOutput::default()
        })
        .expect("semantic MIR should store");

        let sites = super::extract_call_sites(&db);
        let stable_key = db.resolve_stable_key(sites[0].stable_key);

        assert!(stable_key.contains("8:CallSite"));
        assert!(stable_key.contains("8:language=10:TypeScript"));
        assert!(stable_key.contains("8:file_key="));
        assert!(stable_key.contains("10:caller_key="));
        assert!(stable_key.contains("4:span="));
        assert!(stable_key.contains("12:callee_shape=14:function_value"));
        assert!(stable_key.contains("13:operation_key=13:mir-op:call:1"));
        assert!(stable_key.contains("9:call_kind=13:FunctionValue"));
    }

    #[test]
    fn extract_call_sites_is_deterministic_for_different_operation_orders() {
        let mut first = LocalAnalysisDb::new();
        let (file, function) = add_file_and_function(&mut first, "src/app.ts");
        let output = MirOutput {
            bodies: vec![body(&first, file, function, Language::TypeScript)],
            places: vec![
                place(
                    &first,
                    1,
                    file,
                    function,
                    PlaceRoot::Global {
                        symbol: None,
                        name: "alpha".to_string(),
                    },
                    Vec::new(),
                ),
                place(
                    &first,
                    2,
                    file,
                    function,
                    PlaceRoot::Global {
                        symbol: None,
                        name: "beta".to_string(),
                    },
                    Vec::new(),
                ),
                place(
                    &first,
                    9,
                    file,
                    function,
                    PlaceRoot::CallReturn {
                        call: CallSiteId(10),
                    },
                    Vec::new(),
                ),
            ],
            operations: vec![
                call_op(
                    &first,
                    2,
                    1,
                    file,
                    20,
                    MirValue::Place(PlaceId(2)),
                    Vec::new(),
                ),
                call_op(
                    &first,
                    1,
                    0,
                    file,
                    10,
                    MirValue::Place(PlaceId(1)),
                    Vec::new(),
                ),
            ],
            unsupported: Vec::new(),
            ..MirOutput::default()
        };
        first
            .replace_semantic_mir(output.clone())
            .expect("semantic MIR should store");

        let mut second = LocalAnalysisDb::new();
        let (second_file, second_function) = add_file_and_function(&mut second, "src/app.ts");
        let mut reordered = output;
        reordered.bodies = vec![body(
            &second,
            second_file,
            second_function,
            Language::TypeScript,
        )];
        reordered.operations.reverse();
        second
            .replace_semantic_mir(reordered)
            .expect("semantic MIR should store");

        let first_keys = super::extract_call_sites(&first)
            .into_iter()
            .map(|site| first.resolve_stable_key(site.stable_key))
            .collect::<Vec<_>>();
        let second_keys = super::extract_call_sites(&second)
            .into_iter()
            .map(|site| second.resolve_stable_key(site.stable_key))
            .collect::<Vec<_>>();

        assert_eq!(first_keys, second_keys);
    }
}
