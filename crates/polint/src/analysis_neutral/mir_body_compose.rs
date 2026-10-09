//! Language-neutral MIR output composition.

use std::collections::BTreeMap;

use crate::analysis_neutral::ids::{
    CallSiteId, MirBodyId, MirOpId, MirStatementId, MirTerminatorId, PlaceId, UnsupportedId,
};
use crate::analysis_neutral::mir_body::{MirBlockId, MirOutput, MirTerminatorKind};
use crate::analysis_neutral::mir_op::{MirOperationKind, MirValue};
use crate::analysis_neutral::places::{PlaceProjection, PlaceRoot};
use crate::internal_core::{FileId, Language, StableKeyInterner};

pub fn merge_language_outputs(
    outputs: impl IntoIterator<Item = MirOutput>,
    interner: &StableKeyInterner,
) -> MirOutput {
    let mut merged = MirOutput {
        bodies: Vec::new(),
        places: Vec::new(),
        operations: Vec::new(),
        unsupported: Vec::new(),
        ..MirOutput::default()
    };
    for output in outputs {
        append_language_output(&mut merged, output);
    }
    remap_call_site_ids(&mut merged, interner);
    merged.normalized(interner)
}

/// Concatenates outputs whose ids each start at zero into one output with
/// disjoint ids, in the order given; nothing is sorted or renumbered.
pub fn concat_outputs(outputs: impl IntoIterator<Item = MirOutput>) -> MirOutput {
    let mut merged = MirOutput::default();
    for output in outputs {
        append_language_output(&mut merged, output);
    }
    merged
}

fn append_language_output(merged: &mut MirOutput, mut output: MirOutput) {
    let body_offset = merged.bodies.len() as u64;
    let block_offset = merged.blocks.len() as u64;
    let statement_offset = merged.statements.len() as u64;
    let terminator_offset = merged.terminators.len() as u64;
    let place_offset = merged.places.len() as u64;
    let operation_offset = merged
        .operations
        .iter()
        .map(|operation| operation.id.0)
        .max()
        .map_or(0, |id| id + 1);
    let unsupported_offset = merged.unsupported.len() as u64;

    for body in &mut output.bodies {
        body.id = offset_body_id(body.id, body_offset);
    }
    for block in &mut output.blocks {
        block.id = offset_block_id(block.id, block_offset);
        block.body = offset_body_id(block.body, body_offset);
        block.terminator = offset_terminator_id(block.terminator, terminator_offset);
        for statement in &mut block.statements {
            *statement = offset_statement_id(*statement, statement_offset);
        }
    }
    for statement in &mut output.statements {
        statement.id = offset_statement_id(statement.id, statement_offset);
        statement.body = offset_body_id(statement.body, body_offset);
        statement.operation = offset_operation_id(statement.operation, operation_offset);
    }
    for terminator in &mut output.terminators {
        terminator.id = offset_terminator_id(terminator.id, terminator_offset);
        terminator.body = offset_body_id(terminator.body, body_offset);
        offset_terminator_kind_refs(
            &mut terminator.kind,
            body_offset,
            block_offset,
            place_offset,
            unsupported_offset,
        );
    }
    for place in &mut output.places {
        place.id = offset_place_id(place.id, place_offset);
        if let PlaceRoot::Temporary { body, .. } = &mut place.root {
            *body = offset_body_id(*body, body_offset);
        }
    }
    for place_type in &mut output.place_types {
        place_type.place = offset_place_id(place_type.place, place_offset);
    }
    for operation in &mut output.operations {
        operation.id = offset_operation_id(operation.id, operation_offset);
        operation.body = offset_body_id(operation.body, body_offset);
        offset_operation_kind_refs(
            &mut operation.kind,
            body_offset,
            place_offset,
            unsupported_offset,
        );
    }
    for row in &mut output.unsupported {
        row.id = offset_unsupported_id(row.id, unsupported_offset);
        row.body = row.body.map(|body| offset_body_id(body, body_offset));
        row.operation = row
            .operation
            .map(|operation| offset_operation_id(operation, operation_offset));
        for place in &mut row.affected_places {
            *place = offset_place_id(*place, place_offset);
        }
    }

    merged.bodies.extend(output.bodies);
    merged.blocks.extend(output.blocks);
    merged.statements.extend(output.statements);
    merged.terminators.extend(output.terminators);
    merged.places.extend(output.places);
    merged.place_types.extend(output.place_types);
    merged.operations.extend(output.operations);
    merged.unsupported.extend(output.unsupported);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct CallSiteContext {
    language: Language,
    file: FileId,
    body: MirBodyId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct CallSiteSource {
    context: CallSiteContext,
    local_id: CallSiteId,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CallSiteOrderKey {
    body_stable_key: String,
    operation_stable_key: String,
    language: Language,
    file: FileId,
    start_byte: u32,
    end_byte: u32,
    ordinal: u32,
    local_id: CallSiteId,
}

#[derive(Debug, Clone)]
struct CallSiteDescriptor {
    source: CallSiteSource,
    operation: MirOpId,
    order: CallSiteOrderKey,
}

fn remap_call_site_ids(output: &mut MirOutput, interner: &StableKeyInterner) {
    let body_contexts = output
        .bodies
        .iter()
        .map(|body| {
            (
                body.id,
                CallSiteContext {
                    language: body.language,
                    file: body.file,
                    body: body.id,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let body_stable_keys = output
        .bodies
        .iter()
        .map(|body| (body.id, body.stable_key))
        .collect::<BTreeMap<_, _>>();

    let mut descriptors = output
        .operations
        .iter()
        .filter_map(|operation| {
            let MirOperationKind::Call { site, .. } = &operation.kind else {
                return None;
            };
            let context = body_contexts.get(&operation.body).copied()?;
            let body_stable_key = body_stable_keys.get(&operation.body).copied()?;
            Some(CallSiteDescriptor {
                source: CallSiteSource {
                    context,
                    local_id: *site,
                },
                operation: operation.id,
                order: CallSiteOrderKey {
                    body_stable_key: interner.resolve(body_stable_key).to_string(),
                    operation_stable_key: interner.resolve(operation.stable_key).to_string(),
                    language: context.language,
                    file: context.file,
                    start_byte: operation.span.start_byte,
                    end_byte: operation.span.end_byte,
                    ordinal: operation.ordinal,
                    local_id: *site,
                },
            })
        })
        .collect::<Vec<_>>();
    descriptors.sort_by(|left, right| left.order.cmp(&right.order));

    let mut by_operation = BTreeMap::new();
    let mut by_context = BTreeMap::<CallSiteSource, Option<CallSiteId>>::new();
    let mut by_file_site = BTreeMap::<(Language, FileId, CallSiteId), Option<CallSiteId>>::new();
    for (index, descriptor) in descriptors.into_iter().enumerate() {
        let remapped = CallSiteId(index as u64);
        by_operation.insert(
            (descriptor.source.context.body, descriptor.operation),
            remapped,
        );
        match by_context.entry(descriptor.source) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(Some(remapped));
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                *entry.get_mut() = None;
            }
        }
        match by_file_site.entry((
            descriptor.source.context.language,
            descriptor.source.context.file,
            descriptor.source.local_id,
        )) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(Some(remapped));
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                // A file-local duplicate cannot be associated with a place by
                // coordinate alone. Keep that place reference unresolved instead
                // of attaching it to an arbitrary call.
                *entry.get_mut() = None;
            }
        }
    }

    for operation in &mut output.operations {
        let Some(context) = body_contexts.get(&operation.body).copied() else {
            continue;
        };
        remap_operation_call_sites(
            operation,
            context,
            &by_operation,
            &by_context,
            &by_file_site,
        );
    }

    for terminator in &mut output.terminators {
        let Some(context) = body_contexts.get(&terminator.body).copied() else {
            continue;
        };
        remap_terminator_call_sites(&mut terminator.kind, context, &by_context, &by_file_site);
    }

    for place in &mut output.places {
        let Some(file) = place.file else {
            continue;
        };
        remap_place_call_sites(place, place.language, file, &by_file_site);
    }
}

fn remap_operation_call_sites(
    operation: &mut crate::analysis_neutral::mir_op::MirOperation,
    context: CallSiteContext,
    by_operation: &BTreeMap<(MirBodyId, MirOpId), CallSiteId>,
    by_context: &BTreeMap<CallSiteSource, Option<CallSiteId>>,
    by_file_site: &BTreeMap<(Language, FileId, CallSiteId), Option<CallSiteId>>,
) {
    match &mut operation.kind {
        MirOperationKind::StorageLive { .. } | MirOperationKind::Read { .. } => {}
        MirOperationKind::Bind { value, .. }
        | MirOperationKind::Assign { value, .. }
        | MirOperationKind::Write { value, .. }
        | MirOperationKind::Return { value: Some(value) } => {
            remap_value_call_sites(value, context, by_context, by_file_site);
        }
        MirOperationKind::Return { value: None } => {}
        MirOperationKind::Branch { .. } => {}
        MirOperationKind::Call { site, callee, .. } => {
            if let Some(remapped) = by_operation.get(&(operation.body, operation.id)) {
                *site = *remapped;
            }
            remap_value_call_sites(callee, context, by_context, by_file_site);
        }
        MirOperationKind::Unsupported { .. } => {}
    }
}

fn remap_terminator_call_sites(
    kind: &mut MirTerminatorKind,
    context: CallSiteContext,
    by_context: &BTreeMap<CallSiteSource, Option<CallSiteId>>,
    by_file_site: &BTreeMap<(Language, FileId, CallSiteId), Option<CallSiteId>>,
) {
    match kind {
        MirTerminatorKind::Goto { .. }
        | MirTerminatorKind::Branch { .. }
        | MirTerminatorKind::Unreachable
        | MirTerminatorKind::Unsupported { .. } => {}
        MirTerminatorKind::Switch {
            discriminant,
            cases,
            ..
        } => {
            remap_value_call_sites(discriminant, context, by_context, by_file_site);
            for (value, _) in cases {
                remap_value_call_sites(value, context, by_context, by_file_site);
            }
        }
        MirTerminatorKind::Return { value } | MirTerminatorKind::Throw { value, .. } => {
            if let Some(value) = value {
                remap_value_call_sites(value, context, by_context, by_file_site);
            }
        }
        MirTerminatorKind::Call { site, callee, .. } => {
            *site = remap_call_site(*site, context, by_context, by_file_site);
            remap_value_call_sites(callee, context, by_context, by_file_site);
        }
        MirTerminatorKind::Suspend { value, .. } => {
            if let Some(value) = value {
                remap_value_call_sites(value, context, by_context, by_file_site);
            }
        }
    }
}

fn remap_value_call_sites(
    value: &mut MirValue,
    context: CallSiteContext,
    by_context: &BTreeMap<CallSiteSource, Option<CallSiteId>>,
    by_file_site: &BTreeMap<(Language, FileId, CallSiteId), Option<CallSiteId>>,
) {
    match value {
        MirValue::CallReturn(site) => {
            *site = remap_call_site(*site, context, by_context, by_file_site)
        }
        MirValue::BinOp { lhs, rhs, .. } => {
            remap_value_call_sites(lhs, context, by_context, by_file_site);
            remap_value_call_sites(rhs, context, by_context, by_file_site);
        }
        MirValue::Aggregate { fields, .. } => {
            for field in fields {
                remap_value_call_sites(&mut field.value, context, by_context, by_file_site);
            }
        }
        MirValue::Literal { .. }
        | MirValue::Place(_)
        | MirValue::Temporary(_)
        | MirValue::Closure { .. }
        | MirValue::Unknown { .. } => {}
    }
}

fn remap_call_site(
    local_id: CallSiteId,
    context: CallSiteContext,
    by_context: &BTreeMap<CallSiteSource, Option<CallSiteId>>,
    by_file_site: &BTreeMap<(Language, FileId, CallSiteId), Option<CallSiteId>>,
) -> CallSiteId {
    by_context
        .get(&CallSiteSource { context, local_id })
        .copied()
        .flatten()
        .or_else(|| {
            by_file_site
                .get(&(context.language, context.file, local_id))
                .copied()
                .flatten()
        })
        .unwrap_or(local_id)
}

fn remap_place_call_sites(
    place: &mut crate::analysis_neutral::places::PlaceFact,
    language: Language,
    file: FileId,
    by_file_site: &BTreeMap<(Language, FileId, CallSiteId), Option<CallSiteId>>,
) {
    let remap = |site: &mut CallSiteId| {
        if let Some(Some(remapped)) = by_file_site.get(&(language, file, *site)) {
            *site = *remapped;
        }
    };
    match &mut place.root {
        PlaceRoot::CallReturn { call } => remap(call),
        PlaceRoot::Local { .. }
        | PlaceRoot::Parameter { .. }
        | PlaceRoot::Global { .. }
        | PlaceRoot::Temporary { .. }
        | PlaceRoot::Unknown { .. } => {}
    }
    for projection in &mut place.projections {
        if let PlaceProjection::CallReturn(call) = projection {
            remap(call);
        }
    }
}

fn offset_terminator_kind_refs(
    kind: &mut MirTerminatorKind,
    body_offset: u64,
    block_offset: u64,
    place_offset: u64,
    unsupported_offset: u64,
) {
    match kind {
        MirTerminatorKind::Goto { target } => {
            *target = offset_block_id(*target, block_offset);
        }
        MirTerminatorKind::Branch {
            predicate_place,
            then_target,
            else_target,
            ..
        } => {
            *then_target = offset_block_id(*then_target, block_offset);
            *else_target = offset_block_id(*else_target, block_offset);
            if let Some(place) = predicate_place {
                *place = offset_place_id(*place, place_offset);
            }
        }
        MirTerminatorKind::Switch {
            discriminant,
            cases,
            otherwise,
        } => {
            offset_value_ref(discriminant, body_offset, place_offset);
            for (value, target) in cases {
                offset_value_ref(value, body_offset, place_offset);
                *target = offset_block_id(*target, block_offset);
            }
            *otherwise = offset_block_id(*otherwise, block_offset);
        }
        MirTerminatorKind::Return { value } => {
            if let Some(value) = value {
                offset_value_ref(value, body_offset, place_offset);
            }
        }
        MirTerminatorKind::Throw { value, unwind } => {
            if let Some(value) = value {
                offset_value_ref(value, body_offset, place_offset);
            }
            *unwind = offset_block_id(*unwind, block_offset);
        }
        MirTerminatorKind::Call {
            callee,
            arguments,
            return_place,
            normal,
            unwind,
            ..
        } => {
            offset_value_ref(callee, body_offset, place_offset);
            for argument in arguments {
                *argument = offset_place_id(*argument, place_offset);
            }
            *return_place = offset_place_id(*return_place, place_offset);
            *normal = offset_block_id(*normal, block_offset);
            if let Some(unwind) = unwind {
                *unwind = offset_block_id(*unwind, block_offset);
            }
        }
        MirTerminatorKind::Suspend { value, resume, .. } => {
            if let Some(value) = value {
                offset_value_ref(value, body_offset, place_offset);
            }
            *resume = offset_block_id(*resume, block_offset);
        }
        MirTerminatorKind::Unreachable => {}
        MirTerminatorKind::Unsupported { unsupported } => {
            *unsupported = offset_unsupported_id(*unsupported, unsupported_offset);
        }
    }
}

fn offset_operation_kind_refs(
    kind: &mut MirOperationKind,
    body_offset: u64,
    place_offset: u64,
    unsupported_offset: u64,
) {
    match kind {
        MirOperationKind::StorageLive { place } | MirOperationKind::Read { place } => {
            *place = offset_place_id(*place, place_offset);
        }
        MirOperationKind::Bind { place, value }
        | MirOperationKind::Assign { place, value, .. }
        | MirOperationKind::Write { place, value } => {
            *place = offset_place_id(*place, place_offset);
            offset_value_ref(value, body_offset, place_offset);
        }
        MirOperationKind::Branch {
            predicate_place, ..
        } => {
            if let Some(place) = predicate_place {
                *place = offset_place_id(*place, place_offset);
            }
        }
        MirOperationKind::Call {
            callee,
            arguments,
            return_place,
            ..
        } => {
            offset_value_ref(callee, body_offset, place_offset);
            for argument in arguments {
                *argument = offset_place_id(*argument, place_offset);
            }
            *return_place = offset_place_id(*return_place, place_offset);
        }
        MirOperationKind::Return { value } => {
            if let Some(value) = value {
                offset_value_ref(value, body_offset, place_offset);
            }
        }
        MirOperationKind::Unsupported { unsupported } => {
            *unsupported = offset_unsupported_id(*unsupported, unsupported_offset);
        }
    }
}

fn offset_value_ref(value: &mut MirValue, body_offset: u64, place_offset: u64) {
    match value {
        MirValue::Place(place) => *place = offset_place_id(*place, place_offset),
        MirValue::BinOp { lhs, rhs, .. } => {
            offset_value_ref(lhs, body_offset, place_offset);
            offset_value_ref(rhs, body_offset, place_offset);
        }
        MirValue::Aggregate { fields, .. } => {
            for field in fields {
                offset_value_ref(&mut field.value, body_offset, place_offset);
            }
        }
        MirValue::Closure { body, captures } => {
            *body = offset_body_id(*body, body_offset);
            for capture in captures {
                *capture = offset_place_id(*capture, place_offset);
            }
        }
        MirValue::Literal { .. }
        | MirValue::Temporary(_)
        | MirValue::CallReturn(_)
        | MirValue::Unknown { .. } => {}
    }
}

fn offset_body_id(id: MirBodyId, offset: u64) -> MirBodyId {
    MirBodyId(id.0 + offset)
}

fn offset_block_id(id: MirBlockId, offset: u64) -> MirBlockId {
    MirBlockId(id.0 + offset)
}

fn offset_statement_id(id: MirStatementId, offset: u64) -> MirStatementId {
    MirStatementId(id.0 + offset)
}

fn offset_terminator_id(id: MirTerminatorId, offset: u64) -> MirTerminatorId {
    MirTerminatorId(id.0 + offset)
}

fn offset_place_id(id: PlaceId, offset: u64) -> PlaceId {
    PlaceId(id.0 + offset)
}

fn offset_operation_id(id: MirOpId, offset: u64) -> MirOpId {
    MirOpId(id.0 + offset)
}

fn offset_unsupported_id(id: UnsupportedId, offset: u64) -> UnsupportedId {
    UnsupportedId(id.0 + offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::mir_body::{MirBody, MirStatus, MirTerminator};
    use crate::analysis_neutral::mir_op::{MirAggregateField, MirAggregateKind, MirOperation};
    use crate::analysis_neutral::places::{PlaceFact, PlaceStatus};
    use crate::internal_core::{FunctionId, Span};

    fn body(
        interner: &StableKeyInterner,
        language: Language,
        file: FileId,
        function: u64,
        stable_key: &str,
    ) -> MirBody {
        MirBody {
            id: MirBodyId(0),
            language,
            file,
            function: FunctionId::from_raw(function),
            package: None,
            module: None,
            owner_stable_key: interner.intern(format!("{stable_key}:owner")),
            span: Span::new(file, 0, 20, 1, 1, 1, 21),
            stable_key: interner.intern(stable_key.to_string()),
            status: MirStatus::Resolved,
        }
    }

    fn output(
        interner: &StableKeyInterner,
        language: Language,
        file: FileId,
        function: u64,
        stable_key: &str,
    ) -> MirOutput {
        let operation = MirOperation {
            id: MirOpId(0),
            body: MirBodyId(0),
            ordinal: 4,
            span: Span::new(file, 4, 7, 1, 5, 1, 8),
            kind: MirOperationKind::Call {
                site: CallSiteId(0),
                callee: MirValue::Aggregate {
                    kind: MirAggregateKind::Array,
                    fields: vec![MirAggregateField {
                        name: None,
                        value: MirValue::CallReturn(CallSiteId(0)),
                    }],
                },
                arguments: Vec::new(),
                return_place: PlaceId(0),
            },
            stable_key: interner.intern(format!("{stable_key}:operation")),
            status: MirStatus::Resolved,
        };
        MirOutput {
            bodies: vec![body(interner, language, file, function, stable_key)],
            places: vec![PlaceFact {
                id: PlaceId(0),
                language,
                file: Some(file),
                function: Some(FunctionId::from_raw(function)),
                root: PlaceRoot::CallReturn {
                    call: CallSiteId(0),
                },
                projections: vec![PlaceProjection::CallReturn(CallSiteId(0))],
                stable_key: interner.intern(format!("{stable_key}:place")),
                status: PlaceStatus::Resolved,
            }],
            operations: vec![operation],
            terminators: vec![MirTerminator {
                id: MirTerminatorId(0),
                body: MirBodyId(0),
                ordinal: 4,
                kind: MirTerminatorKind::Call {
                    site: CallSiteId(0),
                    callee: MirValue::CallReturn(CallSiteId(0)),
                    arguments: Vec::new(),
                    return_place: PlaceId(0),
                    normal: MirBlockId(0),
                    unwind: None,
                },
                stable_key: interner.intern(format!("{stable_key}:terminator")),
                status: MirStatus::Resolved,
            }],
            ..MirOutput::default()
        }
    }

    fn operation_sites(
        output: &MirOutput,
        interner: &StableKeyInterner,
    ) -> Vec<(String, CallSiteId, MirValue)> {
        let mut sites = output
            .operations
            .iter()
            .filter_map(|operation| {
                let MirOperationKind::Call { site, callee, .. } = &operation.kind else {
                    return None;
                };
                let body = output
                    .bodies
                    .iter()
                    .find(|body| body.id == operation.body)?;
                Some((
                    interner.resolve(body.stable_key).to_string(),
                    *site,
                    callee.clone(),
                ))
            })
            .collect::<Vec<_>>();
        sites.sort_by(|left, right| left.0.cmp(&right.0));
        sites
    }

    #[test]
    fn merge_language_outputs_assigns_global_ids_and_remaps_call_returns() {
        let interner = StableKeyInterner::default();
        let go = output(&interner, Language::Go, FileId::from_raw(1), 1, "body:go");
        let ts = output(
            &interner,
            Language::TypeScript,
            FileId::from_raw(2),
            2,
            "body:ts",
        );
        let merged = merge_language_outputs([go.clone(), ts.clone()], &interner);
        let reversed = merge_language_outputs([ts, go], &interner);

        let merged_sites = operation_sites(&merged, &interner);
        assert_eq!(
            merged_sites
                .iter()
                .map(|(body, site, _)| (body.as_str(), *site))
                .collect::<Vec<_>>(),
            vec![("body:go", CallSiteId(0)), ("body:ts", CallSiteId(1))]
        );
        assert_eq!(merged_sites, operation_sites(&reversed, &interner));

        for operation in &merged.operations {
            let MirOperationKind::Call { site, callee, .. } = &operation.kind else {
                continue;
            };
            let expected = *site;
            assert!(matches!(
                callee,
                MirValue::Aggregate { fields, .. }
                    if matches!(fields[0].value, MirValue::CallReturn(call) if call == expected)
            ));
        }
        for terminator in &merged.terminators {
            let MirTerminatorKind::Call { site, callee, .. } = &terminator.kind else {
                continue;
            };
            assert!(matches!(callee, MirValue::CallReturn(call) if call == site));
        }
        for place in &merged.places {
            let PlaceRoot::CallReturn { call } = place.root else {
                panic!("expected call-return place root")
            };
            assert!(matches!(
                place.projections.as_slice(),
                [PlaceProjection::CallReturn(projection)] if *projection == call
            ));
        }
    }
}
