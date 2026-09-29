use std::collections::{BTreeMap, BTreeSet};

use crate::analysis_api::SemanticImportKind;
use crate::analysis_api::SourceFile;
use crate::analysis_api::{FactFamily, stable_key_from_key_parts, stable_key_from_parts};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::access_paths::facts::{
    AccessPathFact, AccessPathProjection, AccessPathStatus,
};
use crate::analysis_neutral::access_paths::store::AccessPathOutput;
use crate::analysis_neutral::ids::{
    AbstractValueId, AccessPathId, AllocationTokenId, NarrowedTypeId, PlaceId, TypeFactId,
    TypeSetId, ValueFactId,
};
use crate::analysis_neutral::mir_body::MirBody;
use crate::analysis_neutral::mir_op::{
    MirAggregateKind, MirOperation, MirOperationKind, MirValue, UnsupportedPrecision,
};
use crate::analysis_neutral::places::{PlaceFact, PlaceProjection, PlaceRoot, PlaceStatus};
use crate::analysis_neutral::types::facts::{
    NarrowedTypeFact, TypeConfidence, TypeFact, TypePhase, TypePrecision, TypeProvenance,
    TypeShape, TypeStatus, TypeSubject,
};
use crate::analysis_neutral::types::store::{TypeOutput, TypeValueAliasOutput};
use crate::analysis_neutral::values::facts::{
    AllocationKind, AllocationTokenFact, ValueFact, ValueKind, ValuePrecision, ValueProvenance,
    ValueStatus, ValueSubject,
};
use crate::analysis_neutral::values::store::ValueOutput;
use crate::internal_core::{FileId, FunctionId, KeyPart, Language, Span};

type RootPlaceKey = (PlaceRoot, Option<FileId>, Option<FunctionId>);

pub fn derive_ts_js_type_value_alias(db: &impl AnalysisHost) -> TypeValueAliasOutput {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let body_by_id = db
        .mir_bodies()
        .iter()
        .map(|body| (body.id, body))
        .collect::<BTreeMap<_, _>>();
    let body_by_function = db
        .mir_bodies()
        .iter()
        .map(|body| ((body.file, body.function), body))
        .collect::<BTreeMap<_, _>>();
    let place_by_id = db
        .mir_places()
        .iter()
        .map(|place| (place.id, place))
        .collect::<BTreeMap<_, _>>();
    let file_by_id = db
        .files()
        .iter()
        .map(|file| (file.id, file))
        .collect::<BTreeMap<_, _>>();
    let root_place_by_key = root_place_by_key(db);
    let place_types = db
        .mir_place_types()
        .iter()
        .map(|fact| (fact.place, &fact.ty))
        .collect::<BTreeMap<_, _>>();

    let mut types = Vec::new();
    let mut narrowed = Vec::new();
    let mut access_paths = Vec::new();
    let mut values = Vec::new();
    let mut allocations = Vec::new();

    for place in db
        .mir_places()
        .iter()
        .filter(|place| place.language.is_ts_family())
    {
        let body = body_for_place(place, &body_by_id, &body_by_function);
        types.push(type_fact_for_place(
            interner,
            types.len() as u64,
            place,
            body,
            place_types.get(&place.id).copied(),
        ));
        access_paths.push(access_path_for_place(
            interner,
            access_paths.len() as u64,
            place,
            body,
            &root_place_by_key,
        ));
    }

    collect_module_namespace_values(db, &mut values, &mut allocations);

    for operation in db.mir_operations().iter().filter(|operation| {
        body_by_id
            .get(&operation.body)
            .is_some_and(|body| body.language.is_ts_family())
    }) {
        collect_values_for_operation(
            interner,
            operation,
            &body_by_id,
            &place_by_id,
            &file_by_id,
            &mut values,
            &mut allocations,
        );
        collect_narrowing_for_operation(
            interner,
            operation,
            &body_by_id,
            &place_by_id,
            &file_by_id,
            &mut types,
            &mut narrowed,
        );
    }

    for unsupported in db
        .unsupported_semantics()
        .iter()
        .filter(|row| row.language.is_ts_family())
    {
        let body = unsupported
            .body
            .and_then(|body| body_by_id.get(&body).copied());
        if unsupported.affected_places.is_empty() {
            types.push(unsupported_type_fact(
                interner,
                types.len() as u64,
                TypeSubject::Unknown(unsupported.construct.clone()),
                unsupported,
                body,
                None,
                &unsupported.construct,
            ));
        } else {
            for place in &unsupported.affected_places {
                let Some(place_fact) = place_by_id.get(place) else {
                    continue;
                };
                let subject_identity = interner.resolve(place_fact.stable_key);
                types.push(unsupported_type_fact(
                    interner,
                    types.len() as u64,
                    TypeSubject::Place(*place),
                    unsupported,
                    body,
                    Some(*place),
                    &subject_identity,
                ));
                values.push(unsupported_value_fact(
                    interner,
                    values.len() as u64,
                    ValueSubject::Place(*place),
                    unsupported,
                    body,
                    &subject_identity,
                ));
            }
        }
    }

    TypeValueAliasOutput {
        types: TypeOutput { types, narrowed },
        values: ValueOutput {
            values,
            allocations,
        },
        access_paths: AccessPathOutput { access_paths },
        ..TypeValueAliasOutput::default()
    }
    .normalized(interner)
}

fn body_for_place<'db>(
    place: &PlaceFact,
    body_by_id: &BTreeMap<crate::analysis_neutral::ids::MirBodyId, &'db MirBody>,
    body_by_function: &BTreeMap<(FileId, FunctionId), &'db MirBody>,
) -> Option<&'db MirBody> {
    place
        .root
        .body()
        .and_then(|body| body_by_id.get(&body).copied())
        .or_else(|| match (place.file, place.function) {
            (Some(file), Some(function)) => body_by_function.get(&(file, function)).copied(),
            _ => None,
        })
}

fn type_fact_for_place(
    interner: &crate::internal_core::StableKeyInterner,
    id: u64,
    place: &PlaceFact,
    body: Option<&MirBody>,
    ty: Option<&TypeShape>,
) -> TypeFact {
    let (status, phase, precision, confidence, shape) = match place.status {
        PlaceStatus::Resolved => (
            TypeStatus::Unknown,
            TypePhase::Inferred,
            TypePrecision::SetupAware,
            TypeConfidence::Medium,
            type_shape_for_place(place, ty),
        ),
        PlaceStatus::Partial => (
            TypeStatus::Unknown,
            TypePhase::Inferred,
            TypePrecision::Conservative,
            TypeConfidence::Low,
            type_shape_for_place(place, ty),
        ),
        PlaceStatus::Unknown => (
            TypeStatus::Unknown,
            TypePhase::Unknown,
            TypePrecision::Unknown,
            TypeConfidence::Low,
            TypeShape::Unknown {
                reason: interner.resolve(place.stable_key).to_string(),
            },
        ),
        PlaceStatus::Unsupported => (
            TypeStatus::Unsupported,
            TypePhase::Unsupported,
            TypePrecision::Unsupported,
            TypeConfidence::Low,
            TypeShape::Unsupported {
                reason: interner.resolve(place.stable_key).to_string(),
            },
        ),
    };

    TypeFact {
        id: TypeFactId(id),
        subject: TypeSubject::Place(place.id),
        type_set: TypeSetId(id),
        shape,
        phase,
        language: place.language,
        file: place.file.or_else(|| body.map(|body| body.file)),
        function: place.function.or_else(|| body.map(|body| body.function)),
        body: body.map(|body| body.id),
        place: Some(place.id),
        cfg_block: None,
        operation: None,
        precision,
        confidence,
        status,
        provenance: TypeProvenance::Native,
        stable_key: stable_key_ref(
            interner,
            FactFamily::Type,
            [
                ("language", KeyPart::Text(language_label(place.language))),
                ("place", KeyPart::Key(place.stable_key)),
                ("phase", KeyPart::Text(type_phase_label(phase))),
            ],
        ),
    }
}

fn type_shape_for_place(place: &PlaceFact, ty: Option<&TypeShape>) -> TypeShape {
    if let Some(ty) = ty {
        return ty.clone();
    }
    match &place.root {
        PlaceRoot::Parameter { index, name, .. } => TypeShape::Unknown {
            reason: format!(
                "ts/js parameter {}{}",
                index,
                name.as_ref()
                    .map(|name| format!(":{name}"))
                    .unwrap_or_default()
            ),
        },
        PlaceRoot::Local { name, .. } => TypeShape::Unknown {
            reason: format!("ts/js local:{name}"),
        },
        PlaceRoot::Global { name, .. } => TypeShape::Module {
            module_key: format!("ts/js global:{name}"),
        },
        PlaceRoot::Temporary { .. } => TypeShape::Unknown {
            reason: "ts/js temporary".to_string(),
        },
        PlaceRoot::CallReturn { .. } => TypeShape::Unknown {
            reason: "ts/js call return".to_string(),
        },
        PlaceRoot::Unknown { evidence } => TypeShape::Unknown {
            reason: evidence.clone(),
        },
    }
}

fn access_path_for_place(
    interner: &crate::internal_core::StableKeyInterner,
    id: u64,
    place: &PlaceFact,
    body: Option<&MirBody>,
    root_place_by_key: &BTreeMap<RootPlaceKey, PlaceId>,
) -> AccessPathFact {
    let projections = place
        .projections
        .iter()
        .map(access_path_projection)
        .collect::<Vec<_>>();
    let status = match place.status {
        PlaceStatus::Resolved => AccessPathStatus::Resolved,
        PlaceStatus::Partial => AccessPathStatus::Partial,
        PlaceStatus::Unknown => AccessPathStatus::Unknown,
        PlaceStatus::Unsupported => AccessPathStatus::Unsupported,
    };
    AccessPathFact {
        id: AccessPathId(id),
        base: root_place_id(place, root_place_by_key),
        depth: projections.len() as u32,
        projections,
        language: place.language,
        file: place.file.or_else(|| body.map(|body| body.file)),
        function: place.function.or_else(|| body.map(|body| body.function)),
        body: body.map(|body| body.id).or_else(|| place.root.body()),
        status,
        stable_key: stable_key_ref(
            interner,
            FactFamily::AccessPath,
            [
                ("language", KeyPart::Text(language_label(place.language))),
                ("place", KeyPart::Key(place.stable_key)),
            ],
        ),
    }
}

fn root_place_by_key(db: &impl AnalysisHost) -> BTreeMap<RootPlaceKey, PlaceId> {
    db.mir_places()
        .iter()
        .filter(|place| place.language.is_ts_family() && place.projections.is_empty())
        .map(|place| (root_place_key(place), place.id))
        .collect()
}

fn root_place_id(
    place: &PlaceFact,
    root_place_by_key: &BTreeMap<RootPlaceKey, PlaceId>,
) -> PlaceId {
    root_place_by_key
        .get(&root_place_key(place))
        .copied()
        .unwrap_or(place.id)
}

fn root_place_key(place: &PlaceFact) -> RootPlaceKey {
    (place.root.clone(), place.file, place.function)
}

fn access_path_projection(projection: &PlaceProjection) -> AccessPathProjection {
    match projection {
        PlaceProjection::Field(name) => AccessPathProjection::Field(name.clone()),
        PlaceProjection::Property(name) => AccessPathProjection::Property(name.clone()),
        PlaceProjection::IndexKnown(index) => AccessPathProjection::IndexKnown(index.clone()),
        PlaceProjection::IndexUnknown { evidence } => AccessPathProjection::IndexUnknown {
            evidence: evidence.clone(),
        },
        PlaceProjection::Deref => AccessPathProjection::Deref,
        PlaceProjection::AwaitResult => AccessPathProjection::AwaitResult,
        PlaceProjection::CallReturn(call) => AccessPathProjection::CallReturn(*call),
        PlaceProjection::Unknown { evidence } => AccessPathProjection::Unknown {
            evidence: evidence.clone(),
        },
    }
}

fn collect_values_for_operation(
    interner: &crate::internal_core::StableKeyInterner,
    operation: &MirOperation,
    body_by_id: &BTreeMap<crate::analysis_neutral::ids::MirBodyId, &MirBody>,
    place_by_id: &BTreeMap<PlaceId, &PlaceFact>,
    file_by_id: &BTreeMap<FileId, &SourceFile>,
    values: &mut Vec<ValueFact>,
    allocations: &mut Vec<AllocationTokenFact>,
) {
    match &operation.kind {
        MirOperationKind::Bind { place, value }
        | MirOperationKind::Assign { place, value, .. }
        | MirOperationKind::Write { place, value } => {
            push_value_for_mir_value(
                interner,
                operation,
                Some(*place),
                value,
                body_by_id,
                file_by_id,
                (values, allocations),
            );
        }
        MirOperationKind::Call {
            callee,
            return_place,
            ..
        } => {
            push_function_value(interner, operation, callee, body_by_id, values);
            values.push(ValueFact {
                id: ValueFactId(values.len() as u64),
                subject: ValueSubject::Place(*return_place),
                value: AbstractValueId(values.len() as u64),
                kind: ValueKind::CallReturn(*return_place),
                language: body_by_id
                    .get(&operation.body)
                    .map_or(Language::Unknown, |body| body.language),
                file: Some(operation.span.file),
                function: place_by_id
                    .get(return_place)
                    .and_then(|place| place.function)
                    .or_else(|| body_by_id.get(&operation.body).map(|body| body.function)),
                body: Some(operation.body),
                precision: ValuePrecision::Conservative,
                status: ValueStatus::Unknown,
                provenance: ValueProvenance::Native,
                stable_key: stable_key_ref(
                    interner,
                    FactFamily::Value,
                    [
                        ("language", KeyPart::Text("ts-js")),
                        ("operation", KeyPart::Key(operation.stable_key)),
                        ("kind", KeyPart::Text("call_return")),
                    ],
                ),
            });
        }
        _ => {}
    }
}

fn push_value_for_mir_value(
    interner: &crate::internal_core::StableKeyInterner,
    operation: &MirOperation,
    place: Option<PlaceId>,
    value: &MirValue,
    body_by_id: &BTreeMap<crate::analysis_neutral::ids::MirBodyId, &MirBody>,
    file_by_id: &BTreeMap<FileId, &SourceFile>,
    pair: (&mut Vec<ValueFact>, &mut Vec<AllocationTokenFact>),
) {
    let (values, allocations) = pair;
    let source = file_by_id
        .get(&operation.span.file)
        .and_then(|file| source_text(file.source.as_ref(), &operation.span));
    let operation_language = body_by_id
        .get(&operation.body)
        .map_or(Language::Unknown, |body| body.language);
    let (kind, status, precision) = match value {
        MirValue::Literal { value } => {
            literal_value_kind(interner, value, operation, operation_language, allocations)
        }
        MirValue::Place(place_ref) => {
            inferred_value_from_source(interner, source, operation, operation_language, allocations)
                .unwrap_or((
                    ValueKind::PlaceRef(*place_ref),
                    ValueStatus::Present,
                    ValuePrecision::SetupAware,
                ))
        }
        MirValue::Temporary(_) => {
            inferred_value_from_source(interner, source, operation, operation_language, allocations)
                .unwrap_or((
                    ValueKind::Unknown {
                        evidence: "ts/js temporary".to_string(),
                    },
                    ValueStatus::Unknown,
                    ValuePrecision::Unknown,
                ))
        }
        MirValue::CallReturn(call) => (
            ValueKind::Unknown {
                evidence: format!("call:{}", call.0),
            },
            ValueStatus::Unknown,
            ValuePrecision::Conservative,
        ),
        MirValue::BinOp { op, .. } => (
            ValueKind::Unknown {
                evidence: format!("binary:{op}"),
            },
            ValueStatus::Present,
            ValuePrecision::Conservative,
        ),
        MirValue::Aggregate { kind, .. } => {
            let (allocation_kind, label) = match kind {
                MirAggregateKind::Array => (AllocationKind::ArrayLiteral, "array_literal"),
                MirAggregateKind::Object => (AllocationKind::ObjectLiteral, "object_literal"),
                MirAggregateKind::Composite => {
                    (AllocationKind::CompositeLiteral, "composite_literal")
                }
            };
            let token = push_allocation(
                interner,
                operation,
                allocation_kind,
                label,
                operation_language,
                allocations,
            );
            let kind = match kind {
                MirAggregateKind::Array => ValueKind::Array(token),
                MirAggregateKind::Object => ValueKind::Object(token),
                MirAggregateKind::Composite => ValueKind::CompositeLiteral(token),
            };
            (kind, ValueStatus::Present, ValuePrecision::ExactLocal)
        }
        MirValue::Closure { .. } => {
            let _ = push_allocation(
                interner,
                operation,
                AllocationKind::Closure,
                "closure",
                operation_language,
                allocations,
            );
            (
                ValueKind::FunctionObject,
                ValueStatus::Present,
                ValuePrecision::ExactLocal,
            )
        }
        MirValue::Unknown { evidence } => (
            ValueKind::Unknown {
                evidence: evidence.clone(),
            },
            ValueStatus::Unknown,
            ValuePrecision::Unknown,
        ),
    };
    values.push(ValueFact {
        id: ValueFactId(values.len() as u64),
        subject: place
            .map(ValueSubject::Place)
            .unwrap_or(ValueSubject::Operation(operation.id)),
        value: AbstractValueId(values.len() as u64),
        kind,
        language: body_by_id
            .get(&operation.body)
            .map_or(Language::Unknown, |body| body.language),
        file: Some(operation.span.file),
        function: body_by_id.get(&operation.body).map(|body| body.function),
        body: Some(operation.body),
        precision,
        status,
        provenance: ValueProvenance::Native,
        stable_key: stable_key_ref(
            interner,
            FactFamily::Value,
            [
                ("language", KeyPart::Text("ts-js")),
                ("operation", KeyPart::Key(operation.stable_key)),
                ("role", KeyPart::Text("assigned_value")),
            ],
        ),
    });
}

fn literal_value_kind(
    interner: &crate::internal_core::StableKeyInterner,
    value: &str,
    operation: &MirOperation,
    language: Language,
    allocations: &mut Vec<AllocationTokenFact>,
) -> (ValueKind, ValueStatus, ValuePrecision) {
    let trimmed = value.trim();
    match trimmed {
        "null" => {
            return (
                ValueKind::Null,
                ValueStatus::Present,
                ValuePrecision::ExactLocal,
            );
        }
        "undefined" => {
            return (
                ValueKind::Undefined,
                ValueStatus::Present,
                ValuePrecision::ExactLocal,
            );
        }
        "true" | "false" => {
            return (
                ValueKind::Bool(trimmed.to_string()),
                ValueStatus::Present,
                ValuePrecision::ExactLocal,
            );
        }
        _ => {}
    }
    if trimmed.starts_with('"') || trimmed.starts_with('\'') || trimmed.starts_with('`') {
        return (
            ValueKind::String(trimmed.to_string()),
            ValueStatus::Present,
            ValuePrecision::ExactLocal,
        );
    }
    if trimmed.parse::<f64>().is_ok() {
        return (
            ValueKind::Number(trimmed.to_string()),
            ValueStatus::Present,
            ValuePrecision::ExactLocal,
        );
    }
    if trimmed.starts_with("function") || trimmed.contains("=>") {
        let _ = push_allocation(
            interner,
            operation,
            AllocationKind::FunctionObject,
            "function_object",
            language,
            allocations,
        );
        return (
            ValueKind::FunctionObject,
            ValueStatus::Present,
            ValuePrecision::SetupAware,
        );
    }
    if trimmed.starts_with("class") {
        let _ = push_allocation(
            interner,
            operation,
            AllocationKind::ClassObject,
            "class_object",
            language,
            allocations,
        );
        return (
            ValueKind::ClassObject,
            ValueStatus::Present,
            ValuePrecision::SetupAware,
        );
    }
    (
        ValueKind::Literal(trimmed.to_string()),
        ValueStatus::Present,
        ValuePrecision::Heuristic,
    )
}

fn inferred_value_from_source(
    interner: &crate::internal_core::StableKeyInterner,
    source: Option<&str>,
    operation: &MirOperation,
    language: Language,
    allocations: &mut Vec<AllocationTokenFact>,
) -> Option<(ValueKind, ValueStatus, ValuePrecision)> {
    let source = source?.trim();
    if source.contains("=>") || source.contains("function") {
        let _ = push_allocation(
            interner,
            operation,
            AllocationKind::FunctionObject,
            "function_object",
            language,
            allocations,
        );
        return Some((
            ValueKind::FunctionObject,
            ValueStatus::Present,
            ValuePrecision::Heuristic,
        ));
    }
    if source.contains("class ") {
        let _ = push_allocation(
            interner,
            operation,
            AllocationKind::ClassObject,
            "class_object",
            language,
            allocations,
        );
        return Some((
            ValueKind::ClassObject,
            ValueStatus::Present,
            ValuePrecision::Heuristic,
        ));
    }
    if source.contains('=') && source.contains('{') && source.contains('}') {
        let token = push_allocation(
            interner,
            operation,
            AllocationKind::ObjectLiteral,
            "object_literal",
            language,
            allocations,
        );
        return Some((
            ValueKind::Object(token),
            ValueStatus::Present,
            ValuePrecision::Heuristic,
        ));
    }
    if source.contains('=') && source.contains('[') && source.contains(']') {
        let token = push_allocation(
            interner,
            operation,
            AllocationKind::ArrayLiteral,
            "array_literal",
            language,
            allocations,
        );
        return Some((
            ValueKind::Array(token),
            ValueStatus::Present,
            ValuePrecision::Heuristic,
        ));
    }
    None
}

fn push_function_value(
    interner: &crate::internal_core::StableKeyInterner,
    operation: &MirOperation,
    callee: &MirValue,
    body_by_id: &BTreeMap<crate::analysis_neutral::ids::MirBodyId, &MirBody>,
    values: &mut Vec<ValueFact>,
) {
    if !matches!(callee, MirValue::Unknown { .. }) {
        return;
    }
    values.push(ValueFact {
        id: ValueFactId(values.len() as u64),
        subject: ValueSubject::Operation(operation.id),
        value: AbstractValueId(values.len() as u64),
        kind: ValueKind::FunctionObject,
        language: body_by_id
            .get(&operation.body)
            .map_or(Language::Unknown, |body| body.language),
        file: Some(operation.span.file),
        function: body_by_id.get(&operation.body).map(|body| body.function),
        body: Some(operation.body),
        precision: ValuePrecision::Heuristic,
        status: ValueStatus::Present,
        provenance: ValueProvenance::Native,
        stable_key: stable_key_ref(
            interner,
            FactFamily::Value,
            [
                ("language", KeyPart::Text("ts-js")),
                ("operation", KeyPart::Key(operation.stable_key)),
                ("kind", KeyPart::Text("function_object")),
            ],
        ),
    });
}

fn collect_module_namespace_values(
    db: &impl AnalysisHost,
    values: &mut Vec<ValueFact>,
    allocations: &mut Vec<AllocationTokenFact>,
) {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let mut seen = BTreeSet::new();
    for import in db.semantic_imports().iter().filter(|import| {
        import.language.is_ts_family() && import.kind == SemanticImportKind::StaticNamespace
    }) {
        let local = import
            .local_name
            .clone()
            .unwrap_or_else(|| import.import_path.clone());
        let key = format!(
            "{}:{local}:{}",
            language_label(import.language),
            interner.resolve(import.stable_key)
        );
        if !seen.insert(key.clone()) {
            continue;
        }
        let allocation = AllocationTokenId(allocations.len() as u64);
        allocations.push(AllocationTokenFact {
            id: allocation,
            kind: AllocationKind::ModuleNamespace,
            language: import.language,
            file: import.file,
            function: None,
            body: None,
            source_place: None,
            source_operation: None,
            span: None,
            provenance: ValueProvenance::Native,
            stable_key: stable_key_ref(
                interner,
                FactFamily::AllocationToken,
                [
                    ("language", KeyPart::Text(language_label(import.language))),
                    ("semantic_import", KeyPart::Key(import.stable_key)),
                    ("kind", KeyPart::Text("module_namespace")),
                ],
            ),
        });
        values.push(ValueFact {
            id: ValueFactId(values.len() as u64),
            subject: ValueSubject::Synthetic(local),
            value: AbstractValueId(values.len() as u64),
            kind: ValueKind::ModuleObject,
            language: import.language,
            file: import.file,
            function: None,
            body: None,
            precision: ValuePrecision::SetupAware,
            status: ValueStatus::Present,
            provenance: ValueProvenance::Native,
            stable_key: stable_key_ref(
                interner,
                FactFamily::Value,
                [
                    ("language", KeyPart::Text(language_label(import.language))),
                    ("semantic_import", KeyPart::Key(import.stable_key)),
                    ("kind", KeyPart::Text("module_object")),
                ],
            ),
        });
    }
}

fn collect_narrowing_for_operation(
    interner: &crate::internal_core::StableKeyInterner,
    operation: &MirOperation,
    body_by_id: &BTreeMap<crate::analysis_neutral::ids::MirBodyId, &MirBody>,
    place_by_id: &BTreeMap<PlaceId, &PlaceFact>,
    file_by_id: &BTreeMap<FileId, &SourceFile>,
    types: &mut Vec<TypeFact>,
    narrowed: &mut Vec<NarrowedTypeFact>,
) {
    if !matches!(operation.kind, MirOperationKind::Branch { .. }) {
        return;
    }
    let Some(body) = body_by_id.get(&operation.body).copied() else {
        return;
    };
    let Some(file) = file_by_id.get(&operation.span.file).copied() else {
        return;
    };
    let Some(evidence) = source_text(file.source.as_ref(), &operation.span) else {
        return;
    };
    let Some(shape) = narrowing_shape(evidence) else {
        return;
    };
    let candidate_places = place_by_id
        .values()
        .filter(|place| place.language.is_ts_family())
        .filter(|place| place.function == Some(body.function))
        .filter(|place| evidence_mentions_place(evidence, place))
        .copied()
        .collect::<Vec<_>>();
    for place_fact in candidate_places {
        let place = place_fact.id;
        let place_stable_key = interner.resolve(place_fact.stable_key).to_string();
        let type_set = TypeSetId(types.len() as u64);
        let type_stable_key = stable_key_ref(
            interner,
            FactFamily::Type,
            [
                ("language", KeyPart::Text(language_label(body.language))),
                ("operation", KeyPart::Key(operation.stable_key)),
                ("place", KeyPart::Text(&place_stable_key.clone())),
                ("phase", KeyPart::Text("flow_narrowed")),
            ],
        );
        types.push(TypeFact {
            id: TypeFactId(types.len() as u64),
            subject: TypeSubject::Place(place),
            type_set,
            shape: shape.clone(),
            phase: TypePhase::FlowNarrowed,
            language: body.language,
            file: Some(body.file),
            function: Some(body.function),
            body: Some(body.id),
            place: Some(place),
            cfg_block: None,
            operation: Some(operation.id),
            precision: TypePrecision::Heuristic,
            confidence: TypeConfidence::Medium,
            status: TypeStatus::Present,
            provenance: TypeProvenance::Native,
            stable_key: type_stable_key,
        });
        narrowed.push(NarrowedTypeFact {
            id: NarrowedTypeId(narrowed.len() as u64),
            place,
            type_set,
            cfg_block: None,
            operation: Some(operation.id),
            predicate: None,
            evidence: evidence.trim().to_string(),
            language: body.language,
            file: Some(body.file),
            function: Some(body.function),
            body: Some(body.id),
            precision: TypePrecision::Heuristic,
            status: TypeStatus::Present,
            stable_key: stable_key_ref(
                interner,
                FactFamily::NarrowedType,
                [
                    ("language", KeyPart::Text(language_label(body.language))),
                    ("operation", KeyPart::Key(operation.stable_key)),
                    ("place", KeyPart::Text(&place_stable_key.clone())),
                    ("evidence", KeyPart::Text(evidence.trim())),
                ],
            ),
        });
    }
}

fn evidence_mentions_place(evidence: &str, place: &PlaceFact) -> bool {
    match &place.root {
        PlaceRoot::Parameter {
            name: Some(name), ..
        }
        | PlaceRoot::Local { name, .. } => evidence.contains(name),
        PlaceRoot::Global { name, .. } => evidence.contains(name),
        PlaceRoot::Unknown {
            evidence: place_evidence,
        } => evidence.contains(place_evidence),
        PlaceRoot::Parameter { name: None, .. }
        | PlaceRoot::Temporary { .. }
        | PlaceRoot::CallReturn { .. } => false,
    }
}

fn narrowing_shape(evidence: &str) -> Option<TypeShape> {
    let trimmed = evidence.trim();
    if trimmed.contains("typeof") {
        return Some(TypeShape::Primitive("typeof-refinement".to_string()));
    }
    if trimmed.contains("instanceof") {
        return Some(TypeShape::Nominal {
            type_id: "instanceof-refinement".to_string(),
        });
    }
    if trimmed.contains(" in ") || trimmed.contains(".hasOwnProperty") {
        return Some(TypeShape::Object {
            shape_id: Some("property-presence".to_string()),
        });
    }
    if trimmed.contains("?.")
        || trimmed.contains("??")
        || trimmed.contains("== null")
        || trimmed.contains("!= null")
        || trimmed.contains("=== null")
        || trimmed.contains("!== null")
        || trimmed.contains("=== undefined")
        || trimmed.contains("!== undefined")
    {
        return Some(TypeShape::Nullish("nullish-refinement".to_string()));
    }
    if trimmed.contains("===") || trimmed.contains("!==") {
        return Some(TypeShape::Literal("strict-equality-refinement".to_string()));
    }
    if trimmed.contains("if ") || trimmed.starts_with("if(") || trimmed.contains("&&") {
        return Some(TypeShape::Unknown {
            reason: "truthiness-refinement".to_string(),
        });
    }
    None
}

fn push_allocation(
    interner: &crate::internal_core::StableKeyInterner,
    operation: &MirOperation,
    kind: AllocationKind,
    label: &str,
    language: Language,
    allocations: &mut Vec<AllocationTokenFact>,
) -> AllocationTokenId {
    let id = AllocationTokenId(allocations.len() as u64);
    allocations.push(AllocationTokenFact {
        id,
        kind,
        language,
        file: Some(operation.span.file),
        function: None,
        body: Some(operation.body),
        source_place: None,
        source_operation: Some(operation.id),
        span: Some(operation.span.clone()),
        provenance: ValueProvenance::Native,
        stable_key: stable_key_ref(
            interner,
            FactFamily::AllocationToken,
            [
                ("language", KeyPart::Text(language_label(language))),
                ("operation", KeyPart::Key(operation.stable_key)),
                ("kind", KeyPart::Text(label)),
            ],
        ),
    });
    id
}

fn unsupported_type_fact(
    interner: &crate::internal_core::StableKeyInterner,
    id: u64,
    subject: TypeSubject,
    unsupported: &crate::analysis_neutral::mir_op::UnsupportedSemanticFact,
    body: Option<&MirBody>,
    place: Option<PlaceId>,
    subject_identity: &str,
) -> TypeFact {
    let (status, phase, precision, shape) = match unsupported.precision {
        UnsupportedPrecision::Partial | UnsupportedPrecision::Unknown => (
            TypeStatus::Unknown,
            TypePhase::Unknown,
            TypePrecision::Unknown,
            TypeShape::Unknown {
                reason: unsupported.source_evidence.clone(),
            },
        ),
        UnsupportedPrecision::Unsupported => (
            TypeStatus::Unsupported,
            TypePhase::Unsupported,
            TypePrecision::Unsupported,
            TypeShape::Unsupported {
                reason: unsupported.source_evidence.clone(),
            },
        ),
    };
    TypeFact {
        id: TypeFactId(id),
        subject,
        type_set: TypeSetId(id),
        shape,
        phase,
        language: unsupported.language,
        file: Some(unsupported.file),
        function: body.map(|body| body.function),
        body: unsupported.body,
        place,
        cfg_block: None,
        operation: unsupported.operation,
        precision,
        confidence: TypeConfidence::Low,
        status,
        provenance: TypeProvenance::Native,
        stable_key: stable_key_ref(
            interner,
            FactFamily::Type,
            [
                (
                    "language",
                    KeyPart::Text(language_label(unsupported.language)),
                ),
                ("unsupported", KeyPart::Key(unsupported.stable_key)),
                ("subject", KeyPart::Text(subject_identity)),
            ],
        ),
    }
}

fn unsupported_value_fact(
    interner: &crate::internal_core::StableKeyInterner,
    id: u64,
    subject: ValueSubject,
    unsupported: &crate::analysis_neutral::mir_op::UnsupportedSemanticFact,
    body: Option<&MirBody>,
    subject_identity: &str,
) -> ValueFact {
    ValueFact {
        id: ValueFactId(id),
        subject,
        value: AbstractValueId(id),
        kind: ValueKind::Unknown {
            evidence: unsupported.source_evidence.clone(),
        },
        language: unsupported.language,
        file: Some(unsupported.file),
        function: body.map(|body| body.function),
        body: unsupported.body,
        precision: ValuePrecision::Unsupported,
        status: ValueStatus::Unsupported,
        provenance: ValueProvenance::Native,
        stable_key: stable_key_ref(
            interner,
            FactFamily::Value,
            [
                (
                    "language",
                    KeyPart::Text(language_label(unsupported.language)),
                ),
                ("unsupported", KeyPart::Key(unsupported.stable_key)),
                ("subject", KeyPart::Text(subject_identity)),
            ],
        ),
    }
}

fn source_text<'source>(source: &'source str, span: &Span) -> Option<&'source str> {
    source.get(span.start_byte as usize..span.end_byte as usize)
}

fn type_phase_label(phase: TypePhase) -> &'static str {
    match phase {
        TypePhase::Declared => "declared",
        TypePhase::Inferred => "inferred",
        TypePhase::Resolved => "resolved",
        TypePhase::FlowNarrowed => "flow_narrowed",
        TypePhase::ExtensionProvided => "extension_provided",
        TypePhase::Unknown => "unknown",
        TypePhase::Unsupported => "unsupported",
        TypePhase::SetupMissing => "setup_missing",
    }
}

fn stable_key<const N: usize>(
    interner: &crate::internal_core::StableKeyInterner,
    family: FactFamily,
    parts: [(&'static str, String); N],
) -> crate::internal_core::StableKeyId {
    stable_key_from_parts(interner, family, &parts)
}

/// Like [`stable_key`], for a key that embeds another key's identity.
fn stable_key_ref<const N: usize>(
    interner: &crate::internal_core::StableKeyInterner,
    family: FactFamily,
    parts: [(&str, KeyPart<'_>); N],
) -> crate::internal_core::StableKeyId {
    stable_key_from_key_parts(interner, family, parts)
}

fn language_label(language: Language) -> &'static str {
    match language {
        Language::TypeScript => "typescript",
        Language::Tsx => "tsx",
        Language::JavaScript => "javascript",
        Language::Jsx => "jsx",
        Language::Go | Language::Unknown => "unknown",
        _ => unreachable!(),
    }
}

trait PlaceRootBody {
    fn body(&self) -> Option<crate::analysis_neutral::ids::MirBodyId>;
}

impl PlaceRootBody for PlaceRoot {
    fn body(&self) -> Option<crate::analysis_neutral::ids::MirBodyId> {
        match self {
            PlaceRoot::Temporary { body, .. } => Some(*body),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_api::FunctionFact;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::ids::{MirBodyId, MirOpId, UnsupportedId};
    use crate::analysis_neutral::mir_body::{MirOutput, MirStatus};
    use crate::analysis_neutral::mir_op::{AssignMode, ConservativeAction, UnsupportedDomain};
    use crate::analysis_neutral::places::PlaceProjection;
    use crate::internal_core::FunctionId;
    use std::path::PathBuf;

    #[test]
    fn ts_js_narrowing_rows_cover_high_yield_predicates() {
        let db = db_with_ts_mir();
        let output = derive_ts_js_type_value_alias(&db);
        let evidence = output
            .types
            .narrowed
            .iter()
            .map(|row| row.evidence.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        for expected in [
            "!= null",
            "typeof",
            "instanceof",
            " in ",
            "===",
            "?.",
            "if (user)",
        ] {
            assert!(
                evidence.contains(expected),
                "missing narrowing evidence {expected}: {evidence}"
            );
        }
    }

    #[test]
    fn ts_js_values_allocations_and_access_paths_distinguish_dynamic_cases() {
        let db = db_with_ts_mir();
        let output = derive_ts_js_type_value_alias(&db);

        assert!(output.values.allocations.iter().any(|row| {
            matches!(
                row.kind,
                AllocationKind::ObjectLiteral
                    | AllocationKind::FunctionObject
                    | AllocationKind::ClassObject
            )
        }));
        assert!(
            output
                .values
                .values
                .iter()
                .any(|row| row.kind == ValueKind::FunctionObject)
        );
        assert!(
            output
                .values
                .values
                .iter()
                .any(|row| row.kind == ValueKind::ClassObject)
        );
        assert!(output.access_paths.access_paths.iter().any(|row| {
            row.projections.iter().any(|projection| {
                matches!(projection, AccessPathProjection::IndexUnknown { evidence } if evidence == "dynamicKey")
            }) && row.status != AccessPathStatus::Resolved
        }));
    }

    #[test]
    fn ts_js_unsupported_dynamic_rows_never_claim_exact_precision() {
        let db = db_with_ts_mir();
        let interner = db.stable_key_interner();
        let output = derive_ts_js_type_value_alias(&db);

        assert!(output.types.types.iter().any(|row| {
            row.status == TypeStatus::Unsupported
                && matches!(row.shape, TypeShape::Unsupported { .. })
        }));
        assert!(!output.types.types.iter().any(|row| {
            interner.resolve(row.stable_key).contains("unsupported")
                && row.status == TypeStatus::Present
                && row.precision == TypePrecision::ExactLocal
        }));
        assert!(output.values.values.iter().any(|row| {
            row.status == ValueStatus::Unsupported && matches!(row.kind, ValueKind::Unknown { .. })
        }));
    }

    fn db_with_ts_mir() -> LocalAnalysisDb {
        let mut db = LocalAnalysisDb::new();
        let interner = db.stable_key_interner();
        let source = r#"
export function narrow(user, value, dynamicKey) {
  if (user != null) {}
  if (typeof value === "string") {}
  if (value instanceof Widget) {}
  if ("name" in user) {}
  if (value === "ready") {}
  if (user?.name) {}
  if (user) {}
  const obj = { name: value };
  const fn = () => value;
  const cls = class Widget {};
  const dyn = user[dynamicKey];
  return dyn;
}
"#;
        let file = db.add_file(
            PathBuf::from("src/narrow.ts"),
            "src/narrow.ts".to_string(),
            source.to_string(),
        );
        let function = db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "narrow".to_string(),
            span_for(source, file, "export function narrow", "}"),
            Language::TypeScript,
            false,
            true,
            1,
            Vec::new(),
        ));
        let body = MirBody {
            id: MirBodyId(0),
            language: Language::TypeScript,
            file,
            function,
            package: None,
            module: None,
            owner_stable_key: interner.intern("ts:function:narrow".to_string()),
            span: span_for(source, file, "export function narrow", "}"),
            stable_key: interner.intern("ts:body:narrow".to_string()),
            status: MirStatus::Partial,
        };
        let user = place(&interner, function, file, PlaceId(0), "user");
        let value = place(&interner, function, file, PlaceId(1), "value");
        let dynamic = PlaceFact {
            id: PlaceId(2),
            projections: vec![PlaceProjection::IndexUnknown {
                evidence: "dynamicKey".to_string(),
            }],
            stable_key: interner.intern("ts:place:user[dynamicKey]".to_string()),
            status: PlaceStatus::Partial,
            ..user.clone()
        };
        let obj = place(&interner, function, file, PlaceId(3), "obj");
        let func = place(&interner, function, file, PlaceId(4), "fn");
        let class = place(&interner, function, file, PlaceId(5), "cls");
        let mut operations = Vec::new();
        for needle in [
            "if (user != null)",
            "if (typeof value",
            "if (value instanceof",
            "if (\"name\" in user)",
            "if (value ===",
            "if (user?.name)",
            "if (user)",
        ] {
            operations.push(operation(
                &interner,
                MirOpId(operations.len() as u64),
                body.id,
                span_for(source, file, needle, "{}"),
                MirOperationKind::Branch {
                    predicate: crate::analysis_neutral::ids::MirPredicateId(operations.len() as u64),
                    predicate_place: None,
                    nil_test: None,
                },
            ));
        }
        operations.push(operation(
            &interner,
            MirOpId(operations.len() as u64),
            body.id,
            span_for(source, file, "const obj", ";"),
            MirOperationKind::Assign {
                place: obj.id,
                value: MirValue::Place(dynamic.id),
                mode: AssignMode::DeclarationBinding,
            },
        ));
        operations.push(operation(
            &interner,
            MirOpId(operations.len() as u64),
            body.id,
            span_for(source, file, "const fn", ";"),
            MirOperationKind::Assign {
                place: func.id,
                value: MirValue::Place(value.id),
                mode: AssignMode::DeclarationBinding,
            },
        ));
        operations.push(operation(
            &interner,
            MirOpId(operations.len() as u64),
            body.id,
            span_for(source, file, "const cls", ";"),
            MirOperationKind::Assign {
                place: class.id,
                value: MirValue::Place(value.id),
                mode: AssignMode::DeclarationBinding,
            },
        ));
        db.replace_semantic_mir(MirOutput {
            bodies: vec![body],
            places: vec![user, value, dynamic, obj, func, class],
            operations,
            unsupported: vec![crate::analysis_neutral::mir_op::UnsupportedSemanticFact {
                id: UnsupportedId(0),
                body: Some(MirBodyId(0)),
                operation: Some(MirOpId(0)),
                language: Language::TypeScript,
                file,
                span: span_for(source, file, "user[dynamicKey]", ";"),
                construct: "dynamic property key".to_string(),
                source_evidence: "user[dynamicKey]".to_string(),
                affected_places: vec![PlaceId(2)],
                affected_domains: vec![UnsupportedDomain::Aliases],
                conservative_action: ConservativeAction::HavocAffectedPlaces,
                precision: UnsupportedPrecision::Unsupported,
                status: MirStatus::Unsupported,
                stable_key: interner.intern("ts:unsupported:dynamic-key".to_string()),
            }],
            ..MirOutput::default()
        })
        .expect("semantic MIR replacement");
        db
    }

    fn place(
        interner: &crate::internal_core::StableKeyInterner,
        function: FunctionId,
        file: FileId,
        id: PlaceId,
        name: &str,
    ) -> PlaceFact {
        PlaceFact {
            id,
            language: Language::TypeScript,
            file: Some(file),
            function: Some(function),
            root: PlaceRoot::Local {
                function,
                name: name.to_string(),
            },
            projections: Vec::new(),
            stable_key: interner.intern(format!("ts:place:{name}")),
            status: PlaceStatus::Resolved,
        }
    }

    fn operation(
        interner: &crate::internal_core::StableKeyInterner,
        id: MirOpId,
        body: MirBodyId,
        span: Span,
        kind: MirOperationKind,
    ) -> MirOperation {
        MirOperation {
            id,
            body,
            ordinal: id.0 as u32,
            span,
            kind,
            stable_key: interner.intern(format!("ts:op:{}", id.0)),
            status: MirStatus::Partial,
        }
    }

    fn span_for(source: &str, file: FileId, start: &str, end: &str) -> Span {
        let start_byte = source.find(start).expect(start);
        let end_byte = source[start_byte..]
            .find(end)
            .map(|offset| start_byte + offset + end.len())
            .expect(end);
        let index = crate::internal_core::SourceTextIndex::new(source);
        crate::internal_core::span_from_byte_range(file, source, &index, start_byte, end_byte)
    }
}
