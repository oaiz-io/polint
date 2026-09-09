use crate::analysis_neutral::ids::{
    AbstractValueId, AccessPathId, AllocationTokenId, ObjectTokenId, PlaceId, PtVarId,
    SemanticNodeId, ValueFactId,
};

const TAG_BITS: u64 = 4;
const PAYLOAD_MAX: u64 = u64::MAX >> TAG_BITS;

const PLACE_VAR_TAG: u64 = 1;
const OPERATION_VAR_TAG: u64 = 2;
const ALLOCATION_VAR_TAG: u64 = 3;
const ACCESS_PATH_VAR_TAG: u64 = 4;
const DYNAMIC_VAR_TAG: u64 = 5;
const ACCESS_PATH_PREFIX_VAR_TAG: u64 = 6;
const SEMANTIC_NODE_VAR_TAG: u64 = 7;

const ALLOCATION_OBJECT_TAG: u64 = 1;
const ABSTRACT_VALUE_OBJECT_TAG: u64 = 2;
const VALUE_FACT_OBJECT_TAG: u64 = 3;

pub fn place_var(id: PlaceId) -> PtVarId {
    tagged_var(PLACE_VAR_TAG, id.0)
}

pub fn operation_var(id: crate::analysis_neutral::ids::MirOpId) -> PtVarId {
    tagged_var(OPERATION_VAR_TAG, id.0)
}

pub fn allocation_var(id: AllocationTokenId) -> PtVarId {
    tagged_var(ALLOCATION_VAR_TAG, id.0)
}

pub fn access_path_var(id: AccessPathId) -> PtVarId {
    tagged_var(ACCESS_PATH_VAR_TAG, id.0)
}

pub fn access_path_prefix_var(id: AccessPathId, projection_index: usize) -> PtVarId {
    const PREFIX_COMPONENT_BITS: u64 = 30;
    const PREFIX_COMPONENT_MAX: u64 = (1_u64 << PREFIX_COMPONENT_BITS) - 1;
    let projection_index =
        u64::try_from(projection_index).expect("access path projection index exceeds u64 capacity");
    assert!(
        id.0 <= PREFIX_COMPONENT_MAX,
        "access path id exceeds prefix variable capacity"
    );
    assert!(
        projection_index <= PREFIX_COMPONENT_MAX,
        "access path projection index exceeds prefix variable capacity"
    );
    tagged_var(
        ACCESS_PATH_PREFIX_VAR_TAG,
        (id.0 << PREFIX_COMPONENT_BITS) | projection_index,
    )
}

pub fn dynamic_var(slot_index: usize) -> PtVarId {
    tagged_var(DYNAMIC_VAR_TAG, slot_index as u64)
}

pub(crate) fn semantic_node_var(id: SemanticNodeId) -> PtVarId {
    tagged_var(SEMANTIC_NODE_VAR_TAG, id.0)
}

pub fn allocation_object(id: AllocationTokenId) -> ObjectTokenId {
    tagged_object(ALLOCATION_OBJECT_TAG, id.0)
}

pub fn abstract_value_object(id: AbstractValueId) -> ObjectTokenId {
    tagged_object(ABSTRACT_VALUE_OBJECT_TAG, id.0)
}

pub fn value_fact_object(id: ValueFactId) -> ObjectTokenId {
    tagged_object(VALUE_FACT_OBJECT_TAG, id.0)
}

fn tagged_var(tag: u64, payload: u64) -> PtVarId {
    assert!(
        payload <= PAYLOAD_MAX,
        "points-to variable payload exceeds tagged id capacity"
    );
    PtVarId((payload << TAG_BITS) | tag)
}

fn tagged_object(tag: u64, payload: u64) -> ObjectTokenId {
    assert!(
        payload <= PAYLOAD_MAX,
        "points-to object payload exceeds tagged id capacity"
    );
    ObjectTokenId((payload << TAG_BITS) | tag)
}

pub fn is_solver_dynamic_var(var: PtVarId) -> bool {
    matches!(
        var.0 & ((1_u64 << TAG_BITS) - 1),
        DYNAMIC_VAR_TAG | ACCESS_PATH_PREFIX_VAR_TAG
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::ids::{
        AbstractValueId, AccessPathId, AllocationTokenId, MirOpId, PlaceId, ValueFactId,
    };

    #[test]
    fn point_to_variable_ids_are_namespaced_by_source_kind() {
        let id = 42;

        let vars = [
            place_var(PlaceId(id)),
            operation_var(MirOpId(id)),
            allocation_var(AllocationTokenId(id)),
            access_path_var(AccessPathId(id)),
            dynamic_var(id as usize),
            access_path_prefix_var(AccessPathId(id), 0),
            semantic_node_var(SemanticNodeId(id)),
        ];

        for (left_index, left) in vars.iter().enumerate() {
            for right in vars.iter().skip(left_index + 1) {
                assert_ne!(left, right);
            }
        }
    }

    #[test]
    fn object_ids_are_namespaced_by_source_kind() {
        let id = 7;

        assert_ne!(
            allocation_object(AllocationTokenId(id)),
            abstract_value_object(AbstractValueId(id))
        );
        assert_ne!(
            allocation_object(AllocationTokenId(id)),
            value_fact_object(ValueFactId(id))
        );
        assert_ne!(
            abstract_value_object(AbstractValueId(id)),
            value_fact_object(ValueFactId(id))
        );
    }

    #[test]
    fn tagged_ids_do_not_truncate_high_payload_bits() {
        let high = 1_u64 << 56;

        assert_ne!(place_var(PlaceId(0)), place_var(PlaceId(high)));
    }

    #[test]
    fn access_path_prefix_vars_do_not_collide_at_legacy_stride_boundary() {
        assert_ne!(
            access_path_prefix_var(AccessPathId(1), 1024),
            access_path_prefix_var(AccessPathId(2), 0)
        );
    }
}
