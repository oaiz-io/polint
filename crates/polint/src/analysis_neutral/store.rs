use std::collections::BTreeMap;

use crate::analysis_neutral::SEMANTIC_MIR_PROVIDER_ID;
use crate::analysis_neutral::error::AnalysisError;
use crate::analysis_neutral::ids::{
    MirBodyId, MirOpId, MirStatementId, MirTerminatorId, PlaceId, UnsupportedId,
};
use crate::analysis_neutral::mir_body::{
    MirBlock, MirBlockId, MirBody, MirOutput, MirStatement, MirTerminator, MirTerminatorKind,
};
use crate::analysis_neutral::mir_op::{
    MirOperation, MirOperationKind, MirValue, UnsupportedSemanticFact,
};
use crate::analysis_neutral::places::{PlaceFact, PlaceRoot, PlaceTypeFact};
use crate::internal_core::{StableKeyId, StableKeyInterner};

#[derive(Debug, Default, Clone)]
pub struct SemanticStore {
    mir_bodies: Vec<MirBody>,
    mir_blocks: Vec<MirBlock>,
    mir_statements: Vec<MirStatement>,
    mir_terminators: Vec<MirTerminator>,
    mir_operations: Vec<MirOperation>,
    places: Vec<PlaceFact>,
    place_types: Vec<PlaceTypeFact>,
    unsupported_semantics: Vec<UnsupportedSemanticFact>,
    mir_bodies_by_id: BTreeMap<MirBodyId, usize>,
    mir_operations_by_id: BTreeMap<MirOpId, usize>,
    places_by_id: BTreeMap<PlaceId, usize>,
    unsupported_semantics_by_id: BTreeMap<UnsupportedId, usize>,
}

impl SemanticStore {
    pub fn from_output(
        output: MirOutput,
        interner: &StableKeyInterner,
    ) -> Result<Self, AnalysisError> {
        let output = output.normalized(interner);
        let (mir_bodies, body_ids) = normalize_bodies(output.bodies, interner);
        let (places, place_ids) = normalize_places(output.places, &body_ids, interner)?;
        let place_types = normalize_place_types(output.place_types, &place_ids)?;
        let unsupported_ids = unsupported_id_map(&output.unsupported, interner);
        let (mir_operations, operation_ids) = normalize_operations(
            output.operations,
            &body_ids,
            &place_ids,
            &unsupported_ids,
            interner,
        )?;
        let block_ids = stable_id_map(
            &output.blocks,
            interner,
            |row| row.stable_key,
            |row| row.id,
            MirBlockId,
        );
        let statement_ids = stable_id_map(
            &output.statements,
            interner,
            |row| row.stable_key,
            |row| row.id,
            MirStatementId,
        );
        let terminator_ids = stable_id_map(
            &output.terminators,
            interner,
            |row| row.stable_key,
            |row| row.id,
            MirTerminatorId,
        );
        let mir_statements = normalize_statements(
            output.statements,
            &body_ids,
            &operation_ids,
            &statement_ids,
            interner,
        )?;
        let mir_terminators = normalize_terminators(
            output.terminators,
            &body_ids,
            &block_ids,
            &place_ids,
            &unsupported_ids,
            &terminator_ids,
            interner,
        )?;
        let mir_blocks = normalize_blocks(
            output.blocks,
            &body_ids,
            &block_ids,
            &statement_ids,
            &terminator_ids,
            interner,
        )?;
        let unsupported_semantics = normalize_unsupported_with_ids(
            output.unsupported,
            &body_ids,
            &operation_ids,
            &place_ids,
            &unsupported_ids,
            interner,
        )?;

        Ok(Self {
            mir_bodies_by_id: index_by_id(&mir_bodies, |body| body.id),
            mir_operations_by_id: index_by_id(&mir_operations, |operation| operation.id),
            places_by_id: index_by_id(&places, |place| place.id),
            unsupported_semantics_by_id: index_by_id(&unsupported_semantics, |row| row.id),
            mir_bodies,
            mir_blocks,
            mir_statements,
            mir_terminators,
            mir_operations,
            places,
            place_types,
            unsupported_semantics,
        })
    }

    pub fn mir_bodies(&self) -> &[MirBody] {
        &self.mir_bodies
    }

    pub fn mir_operations(&self) -> &[MirOperation] {
        &self.mir_operations
    }

    pub fn mir_blocks(&self) -> &[MirBlock] {
        &self.mir_blocks
    }

    pub fn mir_statements(&self) -> &[MirStatement] {
        &self.mir_statements
    }

    pub fn mir_terminators(&self) -> &[MirTerminator] {
        &self.mir_terminators
    }

    pub fn places(&self) -> &[PlaceFact] {
        &self.places
    }

    pub fn place_types(&self) -> &[PlaceTypeFact] {
        &self.place_types
    }

    pub fn unsupported_semantics(&self) -> &[UnsupportedSemanticFact] {
        &self.unsupported_semantics
    }

    pub fn mir_body(&self, id: MirBodyId) -> Option<&MirBody> {
        self.mir_bodies_by_id
            .get(&id)
            .and_then(|index| self.mir_bodies.get(*index))
    }

    pub fn mir_operation(&self, id: MirOpId) -> Option<&MirOperation> {
        self.mir_operations_by_id
            .get(&id)
            .and_then(|index| self.mir_operations.get(*index))
    }

    pub fn place(&self, id: PlaceId) -> Option<&PlaceFact> {
        self.places_by_id
            .get(&id)
            .and_then(|index| self.places.get(*index))
    }

    pub fn unsupported_semantic(&self, id: UnsupportedId) -> Option<&UnsupportedSemanticFact> {
        self.unsupported_semantics_by_id
            .get(&id)
            .and_then(|index| self.unsupported_semantics.get(*index))
    }
}

fn normalize_place_types(
    mut place_types: Vec<PlaceTypeFact>,
    place_ids: &BTreeMap<PlaceId, PlaceId>,
) -> Result<Vec<PlaceTypeFact>, AnalysisError> {
    for fact in &mut place_types {
        fact.place = remap_place_id(fact.place, place_ids, "dangling typed place")?;
    }
    place_types.sort_by_key(|fact| fact.place);
    Ok(place_types)
}

fn normalize_bodies(
    mut bodies: Vec<MirBody>,
    interner: &StableKeyInterner,
) -> (Vec<MirBody>, BTreeMap<MirBodyId, MirBodyId>) {
    bodies.sort_by(|body, other| interner.compare_canonical(body.stable_key, other.stable_key));
    let mut body_ids = BTreeMap::new();
    for (index, body) in bodies.iter_mut().enumerate() {
        let new_id = MirBodyId(index as u64);
        body_ids.insert(body.id, new_id);
        body.id = new_id;
    }
    (bodies, body_ids)
}

fn normalize_places(
    mut places: Vec<PlaceFact>,
    body_ids: &BTreeMap<MirBodyId, MirBodyId>,
    interner: &StableKeyInterner,
) -> Result<(Vec<PlaceFact>, BTreeMap<PlaceId, PlaceId>), AnalysisError> {
    places.sort_by(|place, other| interner.compare_canonical(place.stable_key, other.stable_key));
    let mut place_ids = BTreeMap::new();
    for (index, place) in places.iter_mut().enumerate() {
        let new_id = PlaceId(index as u64);
        place_ids.insert(place.id, new_id);
        place.id = new_id;
        remap_place_root(&mut place.root, body_ids)?;
    }
    Ok((places, place_ids))
}

fn normalize_operations(
    mut operations: Vec<MirOperation>,
    body_ids: &BTreeMap<MirBodyId, MirBodyId>,
    place_ids: &BTreeMap<PlaceId, PlaceId>,
    unsupported_ids: &BTreeMap<UnsupportedId, UnsupportedId>,
    interner: &StableKeyInterner,
) -> Result<(Vec<MirOperation>, BTreeMap<MirOpId, MirOpId>), AnalysisError> {
    operations.sort_by(|operation, other| {
        interner.compare_canonical(operation.stable_key, other.stable_key)
    });
    let mut operation_ids = BTreeMap::new();
    for (index, operation) in operations.iter_mut().enumerate() {
        let new_id = MirOpId(index as u64);
        operation_ids.insert(operation.id, new_id);
        operation.id = new_id;
        operation.body = remap_body_id(operation.body, body_ids, "dangling MIR operation body")?;
        remap_operation_kind(&mut operation.kind, body_ids, place_ids, unsupported_ids)?;
    }
    Ok((operations, operation_ids))
}

fn stable_id_map<T, Id: Copy + Ord>(
    rows: &[T],
    interner: &StableKeyInterner,
    stable_key: impl Fn(&T) -> StableKeyId,
    old_id: impl Fn(&T) -> Id,
    make_id: impl Fn(u64) -> Id,
) -> BTreeMap<Id, Id> {
    let mut sorted = rows
        .iter()
        .map(|row| (interner.resolve(stable_key(row)), row))
        .collect::<Vec<_>>();
    sorted.sort_by(|left, right| left.0.cmp(&right.0));
    sorted
        .into_iter()
        .enumerate()
        .map(|(index, (_, row))| (old_id(row), make_id(index as u64)))
        .collect()
}

fn normalize_statements(
    mut rows: Vec<MirStatement>,
    body_ids: &BTreeMap<MirBodyId, MirBodyId>,
    operation_ids: &BTreeMap<MirOpId, MirOpId>,
    statement_ids: &BTreeMap<MirStatementId, MirStatementId>,
    interner: &StableKeyInterner,
) -> Result<Vec<MirStatement>, AnalysisError> {
    rows.sort_by(|row, other| interner.compare_canonical(row.stable_key, other.stable_key));
    for row in &mut rows {
        row.id = remap_id(row.id, statement_ids, "dangling MIR statement id")?;
        row.body = remap_body_id(row.body, body_ids, "dangling MIR statement body")?;
        row.operation = remap_id(
            row.operation,
            operation_ids,
            "dangling MIR statement operation",
        )?;
    }
    Ok(rows)
}

fn normalize_terminators(
    mut rows: Vec<MirTerminator>,
    body_ids: &BTreeMap<MirBodyId, MirBodyId>,
    block_ids: &BTreeMap<MirBlockId, MirBlockId>,
    place_ids: &BTreeMap<PlaceId, PlaceId>,
    unsupported_ids: &BTreeMap<UnsupportedId, UnsupportedId>,
    terminator_ids: &BTreeMap<MirTerminatorId, MirTerminatorId>,
    interner: &StableKeyInterner,
) -> Result<Vec<MirTerminator>, AnalysisError> {
    rows.sort_by(|row, other| interner.compare_canonical(row.stable_key, other.stable_key));
    for row in &mut rows {
        row.id = remap_id(row.id, terminator_ids, "dangling MIR terminator id")?;
        row.body = remap_body_id(row.body, body_ids, "dangling MIR terminator body")?;
        remap_terminator_kind(
            &mut row.kind,
            body_ids,
            block_ids,
            place_ids,
            unsupported_ids,
        )?;
    }
    Ok(rows)
}

fn normalize_blocks(
    mut rows: Vec<MirBlock>,
    body_ids: &BTreeMap<MirBodyId, MirBodyId>,
    block_ids: &BTreeMap<MirBlockId, MirBlockId>,
    statement_ids: &BTreeMap<MirStatementId, MirStatementId>,
    terminator_ids: &BTreeMap<MirTerminatorId, MirTerminatorId>,
    interner: &StableKeyInterner,
) -> Result<Vec<MirBlock>, AnalysisError> {
    rows.sort_by(|row, other| interner.compare_canonical(row.stable_key, other.stable_key));
    for row in &mut rows {
        row.id = remap_id(row.id, block_ids, "dangling MIR block id")?;
        row.body = remap_body_id(row.body, body_ids, "dangling MIR block body")?;
        for statement in &mut row.statements {
            *statement = remap_id(*statement, statement_ids, "dangling MIR block statement")?;
        }
        row.terminator = remap_id(
            row.terminator,
            terminator_ids,
            "dangling MIR block terminator",
        )?;
    }
    Ok(rows)
}

fn remap_terminator_kind(
    kind: &mut MirTerminatorKind,
    body_ids: &BTreeMap<MirBodyId, MirBodyId>,
    block_ids: &BTreeMap<MirBlockId, MirBlockId>,
    place_ids: &BTreeMap<PlaceId, PlaceId>,
    unsupported_ids: &BTreeMap<UnsupportedId, UnsupportedId>,
) -> Result<(), AnalysisError> {
    match kind {
        MirTerminatorKind::Goto { target } => {
            *target = remap_id(*target, block_ids, "dangling MIR goto target")?;
        }
        MirTerminatorKind::Branch {
            predicate_place,
            then_target,
            else_target,
            ..
        } => {
            *then_target = remap_id(*then_target, block_ids, "dangling MIR branch then target")?;
            *else_target = remap_id(*else_target, block_ids, "dangling MIR branch else target")?;
            if let Some(place) = predicate_place {
                *place = remap_place_id(*place, place_ids, "dangling MIR predicate place")?;
            }
        }
        MirTerminatorKind::Switch {
            discriminant,
            cases,
            otherwise,
        } => {
            remap_value(discriminant, body_ids, place_ids)?;
            for (value, target) in cases {
                remap_value(value, body_ids, place_ids)?;
                *target = remap_id(*target, block_ids, "dangling MIR switch case target")?;
            }
            *otherwise = remap_id(
                *otherwise,
                block_ids,
                "dangling MIR switch otherwise target",
            )?;
        }
        MirTerminatorKind::Return { value } => {
            if let Some(value) = value {
                remap_value(value, body_ids, place_ids)?;
            }
        }
        MirTerminatorKind::Throw { value, unwind } => {
            if let Some(value) = value {
                remap_value(value, body_ids, place_ids)?;
            }
            *unwind = remap_id(*unwind, block_ids, "dangling MIR throw unwind target")?;
        }
        MirTerminatorKind::Call {
            callee,
            arguments,
            return_place,
            normal,
            unwind,
            ..
        } => {
            remap_value(callee, body_ids, place_ids)?;
            for argument in arguments {
                *argument =
                    remap_place_id(*argument, place_ids, "dangling MIR call argument place")?;
            }
            *return_place =
                remap_place_id(*return_place, place_ids, "dangling MIR call return place")?;
            *normal = remap_id(*normal, block_ids, "dangling MIR call normal target")?;
            if let Some(unwind) = unwind {
                *unwind = remap_id(*unwind, block_ids, "dangling MIR call unwind target")?;
            }
        }
        MirTerminatorKind::Suspend { value, resume, .. } => {
            if let Some(value) = value {
                remap_value(value, body_ids, place_ids)?;
            }
            *resume = remap_id(*resume, block_ids, "dangling MIR suspend resume target")?;
        }
        MirTerminatorKind::Unreachable => {}
        MirTerminatorKind::Unsupported { unsupported } => {
            *unsupported = remap_unsupported_id(
                *unsupported,
                unsupported_ids,
                "dangling MIR unsupported terminator",
            )?;
        }
    }
    Ok(())
}

fn normalize_unsupported_with_ids(
    mut rows: Vec<UnsupportedSemanticFact>,
    body_ids: &BTreeMap<MirBodyId, MirBodyId>,
    operation_ids: &BTreeMap<MirOpId, MirOpId>,
    place_ids: &BTreeMap<PlaceId, PlaceId>,
    unsupported_ids: &BTreeMap<UnsupportedId, UnsupportedId>,
    interner: &StableKeyInterner,
) -> Result<Vec<UnsupportedSemanticFact>, AnalysisError> {
    rows.sort_by(|row, other| interner.compare_canonical(row.stable_key, other.stable_key));
    for (index, row) in rows.iter_mut().enumerate() {
        let new_id = UnsupportedId(index as u64);
        let expected_id = unsupported_ids
            .get(&row.id)
            .copied()
            .ok_or_else(|| invalid_fact("dangling unsupported semantic id"))?;
        if expected_id != new_id {
            return Err(invalid_fact("inconsistent unsupported semantic id remap"));
        }
        row.id = new_id;
        row.body = row
            .body
            .map(|body| remap_body_id(body, body_ids, "dangling unsupported semantic body"))
            .transpose()?;
        row.operation = row
            .operation
            .map(|operation| {
                operation_ids
                    .get(&operation)
                    .copied()
                    .ok_or_else(|| invalid_fact("dangling unsupported semantic operation"))
            })
            .transpose()?;
        for place in &mut row.affected_places {
            *place = remap_place_id(*place, place_ids, "dangling unsupported semantic place")?;
        }
    }
    Ok(rows)
}

fn unsupported_id_map(
    rows: &[UnsupportedSemanticFact],
    interner: &StableKeyInterner,
) -> BTreeMap<UnsupportedId, UnsupportedId> {
    let mut sorted = rows
        .iter()
        .map(|row| (interner.resolve(row.stable_key), row.id))
        .collect::<Vec<_>>();
    sorted.sort_by(|left, right| left.0.cmp(&right.0));
    sorted
        .into_iter()
        .enumerate()
        .map(|(index, (_, old_id))| (old_id, UnsupportedId(index as u64)))
        .collect()
}

fn remap_place_root(
    root: &mut PlaceRoot,
    body_ids: &BTreeMap<MirBodyId, MirBodyId>,
) -> Result<(), AnalysisError> {
    if let PlaceRoot::Temporary { body, .. } = root {
        *body = remap_body_id(*body, body_ids, "dangling temporary place body")?;
    }
    Ok(())
}

fn remap_operation_kind(
    kind: &mut MirOperationKind,
    body_ids: &BTreeMap<MirBodyId, MirBodyId>,
    place_ids: &BTreeMap<PlaceId, PlaceId>,
    unsupported_ids: &BTreeMap<UnsupportedId, UnsupportedId>,
) -> Result<(), AnalysisError> {
    match kind {
        MirOperationKind::StorageLive { place } | MirOperationKind::Read { place } => {
            *place = remap_place_id(*place, place_ids, "dangling MIR operation place")?;
        }
        MirOperationKind::Bind { place, value }
        | MirOperationKind::Assign { place, value, .. }
        | MirOperationKind::Write { place, value } => {
            *place = remap_place_id(*place, place_ids, "dangling MIR operation place")?;
            remap_value(value, body_ids, place_ids)?;
        }
        MirOperationKind::Branch {
            predicate_place, ..
        } => {
            if let Some(place) = predicate_place {
                *place = remap_place_id(*place, place_ids, "dangling MIR predicate place")?;
            }
        }
        MirOperationKind::Call {
            callee,
            arguments,
            return_place,
            ..
        } => {
            remap_value(callee, body_ids, place_ids)?;
            for argument in arguments {
                *argument =
                    remap_place_id(*argument, place_ids, "dangling MIR call argument place")?;
            }
            *return_place =
                remap_place_id(*return_place, place_ids, "dangling MIR call return place")?;
        }
        MirOperationKind::Return { value } => {
            if let Some(value) = value {
                remap_value(value, body_ids, place_ids)?;
            }
        }
        MirOperationKind::Unsupported { unsupported } => {
            *unsupported = remap_unsupported_id(
                *unsupported,
                unsupported_ids,
                "dangling MIR unsupported operation",
            )?;
        }
    }
    Ok(())
}

fn remap_value(
    value: &mut MirValue,
    body_ids: &BTreeMap<MirBodyId, MirBodyId>,
    place_ids: &BTreeMap<PlaceId, PlaceId>,
) -> Result<(), AnalysisError> {
    match value {
        MirValue::Place(place) => {
            *place = remap_place_id(*place, place_ids, "dangling MIR value place")?;
        }
        MirValue::BinOp { lhs, rhs, .. } => {
            remap_value(lhs, body_ids, place_ids)?;
            remap_value(rhs, body_ids, place_ids)?;
        }
        MirValue::Aggregate { fields, .. } => {
            for field in fields {
                remap_value(&mut field.value, body_ids, place_ids)?;
            }
        }
        MirValue::Closure { body, captures } => {
            *body = remap_body_id(*body, body_ids, "dangling MIR closure body")?;
            for capture in captures {
                *capture = remap_place_id(*capture, place_ids, "dangling MIR closure capture")?;
            }
        }
        MirValue::Literal { .. }
        | MirValue::Temporary(_)
        | MirValue::CallReturn(_)
        | MirValue::Unknown { .. } => {}
    }
    Ok(())
}

fn remap_body_id(
    id: MirBodyId,
    body_ids: &BTreeMap<MirBodyId, MirBodyId>,
    reason: &'static str,
) -> Result<MirBodyId, AnalysisError> {
    body_ids
        .get(&id)
        .copied()
        .ok_or_else(|| invalid_fact(reason))
}

fn remap_id<Id: Copy + Ord>(
    id: Id,
    ids: &BTreeMap<Id, Id>,
    reason: &'static str,
) -> Result<Id, AnalysisError> {
    ids.get(&id).copied().ok_or_else(|| invalid_fact(reason))
}

fn remap_place_id(
    id: PlaceId,
    place_ids: &BTreeMap<PlaceId, PlaceId>,
    reason: &'static str,
) -> Result<PlaceId, AnalysisError> {
    place_ids
        .get(&id)
        .copied()
        .ok_or_else(|| invalid_fact(reason))
}

fn remap_unsupported_id(
    id: UnsupportedId,
    unsupported_ids: &BTreeMap<UnsupportedId, UnsupportedId>,
    reason: &'static str,
) -> Result<UnsupportedId, AnalysisError> {
    unsupported_ids
        .get(&id)
        .copied()
        .ok_or_else(|| invalid_fact(reason))
}

fn index_by_id<T, Id>(rows: &[T], id: impl Fn(&T) -> Id) -> BTreeMap<Id, usize>
where
    Id: Ord,
{
    rows.iter()
        .enumerate()
        .map(|(index, row)| (id(row), index))
        .collect()
}

fn invalid_fact(reason: impl Into<String>) -> AnalysisError {
    AnalysisError::InvalidFact {
        provider: SEMANTIC_MIR_PROVIDER_ID,
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::ids::{
        MirBodyId, MirOpId, MirPredicateId, PlaceId, UnsupportedId,
    };
    use crate::analysis_neutral::mir_body::{MirBody, MirOutput, MirStatus};
    use crate::analysis_neutral::mir_op::{
        ConservativeAction, MirOperation, MirOperationKind, UnsupportedDomain,
        UnsupportedPrecision, UnsupportedSemanticFact,
    };
    use crate::analysis_neutral::places::{PlaceFact, PlaceRoot, PlaceStatus};
    use crate::internal_core::{FileId, FunctionId, Language, Span};

    fn span() -> Span {
        Span::new(FileId::from_raw(1), 1, 2, 1, 1, 1, 2)
    }

    fn body(interner: &crate::internal_core::StableKeyInterner) -> MirBody {
        MirBody {
            id: MirBodyId(0),
            language: Language::Go,
            file: FileId::from_raw(1),
            function: FunctionId::from_raw(1),
            package: None,
            module: None,
            owner_stable_key: interner.intern("owner".to_string()),
            span: span(),
            stable_key: interner.intern("body".to_string()),
            status: MirStatus::Partial,
        }
    }

    fn place(interner: &crate::internal_core::StableKeyInterner) -> PlaceFact {
        PlaceFact {
            id: PlaceId(0),
            language: Language::Go,
            file: Some(FileId::from_raw(1)),
            function: Some(FunctionId::from_raw(1)),
            root: PlaceRoot::Local {
                function: FunctionId::from_raw(1),
                name: "value".to_string(),
            },
            projections: Vec::new(),
            stable_key: interner.intern("place".to_string()),
            status: PlaceStatus::Resolved,
        }
    }

    fn unsupported(
        interner: &crate::internal_core::StableKeyInterner,
        id: u64,
        stable_key: &str,
        operation: MirOpId,
    ) -> UnsupportedSemanticFact {
        UnsupportedSemanticFact {
            id: UnsupportedId(id),
            body: Some(MirBodyId(0)),
            operation: Some(operation),
            language: Language::Go,
            file: FileId::from_raw(1),
            span: span(),
            construct: stable_key.to_string(),
            source_evidence: stable_key.to_string(),
            affected_places: vec![PlaceId(0)],
            affected_domains: vec![UnsupportedDomain::Mir],
            conservative_action: ConservativeAction::HavocAffectedPlaces,
            precision: UnsupportedPrecision::Unsupported,
            status: MirStatus::Unsupported,
            stable_key: interner.intern(stable_key.to_string()),
        }
    }

    #[test]
    fn semantic_store_remaps_unsupported_operation_refs_after_unsupported_sorting() {
        let interner = crate::internal_core::StableKeyInterner::default();
        let output = MirOutput {
            bodies: vec![body(&interner)],
            places: vec![place(&interner)],
            operations: vec![MirOperation {
                id: MirOpId(0),
                body: MirBodyId(0),
                ordinal: 0,
                span: span(),
                kind: MirOperationKind::Unsupported {
                    unsupported: UnsupportedId(0),
                },
                stable_key: interner.intern("operation".to_string()),
                status: MirStatus::Unsupported,
            }],
            unsupported: vec![
                unsupported(&interner, 0, "unsupported:z", MirOpId(0)),
                unsupported(&interner, 1, "unsupported:a", MirOpId(0)),
            ],
            ..MirOutput::default()
        };

        let store = SemanticStore::from_output(output, &interner).expect("store normalizes");
        let operation = store
            .mir_operations()
            .first()
            .expect("unsupported operation exists");
        let MirOperationKind::Unsupported { unsupported } = operation.kind else {
            panic!("expected unsupported MIR operation");
        };

        assert_eq!(unsupported, UnsupportedId(1));
        let stable_key = store
            .unsupported_semantic(unsupported)
            .expect("remapped unsupported row exists")
            .stable_key;
        assert_eq!(interner.resolve(stable_key).as_ref(), "unsupported:z");
    }

    #[test]
    fn semantic_store_remaps_branch_predicate_places_after_place_sorting() {
        let interner = crate::internal_core::StableKeyInterner::default();
        let mut predicate = place(&interner);
        predicate.stable_key = interner.intern("place:z".to_string());
        let mut sorts_first = place(&interner);
        sorts_first.id = PlaceId(1);
        sorts_first.stable_key = interner.intern("place:a".to_string());
        let output = MirOutput {
            bodies: vec![body(&interner)],
            places: vec![predicate, sorts_first],
            operations: vec![MirOperation {
                id: MirOpId(0),
                body: MirBodyId(0),
                ordinal: 0,
                span: span(),
                kind: MirOperationKind::Branch {
                    predicate: MirPredicateId(0),
                    predicate_place: Some(PlaceId(0)),
                    nil_test: None,
                },
                stable_key: interner.intern("operation:branch".to_string()),
                status: MirStatus::Partial,
            }],
            ..MirOutput::default()
        };

        let store = SemanticStore::from_output(output, &interner).expect("store normalizes");
        assert!(matches!(
            store.mir_operations()[0].kind,
            MirOperationKind::Branch {
                predicate_place: Some(PlaceId(1)),
                ..
            }
        ));
    }
}
