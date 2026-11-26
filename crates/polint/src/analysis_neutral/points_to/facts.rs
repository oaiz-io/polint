use serde::{Deserialize, Serialize};

use crate::analysis_neutral::ids::{
    ObjectTokenId, PointsToConstraintId, PointsToSetId, PtVarId, ValueFactId,
};
use crate::internal_core::StableKeyId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointsToConstraintFact {
    pub id: PointsToConstraintId,
    pub kind: PointsToConstraintKind,
    pub status: PointsToStatus,
    pub precision: PointsToPrecision,
    pub stable_key: StableKeyId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointsToSetFact {
    pub id: PointsToSetId,
    pub variable: PtVarId,
    pub objects: Vec<ObjectTokenId>,
    pub status: PointsToStatus,
    pub precision: PointsToPrecision,
    pub budget: PointsToBudgetStatus,
    pub stable_key: StableKeyId,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PointsToConstraintKind {
    AddressOf {
        dst: PtVarId,
        object: ObjectTokenId,
    },
    Copy {
        dst: PtVarId,
        src: PtVarId,
    },
    Load {
        dst: PtVarId,
        pointer: PtVarId,
    },
    Store {
        pointer: PtVarId,
        src: PtVarId,
    },
    FieldLoad {
        dst: PtVarId,
        base: PtVarId,
        field: String,
    },
    FieldStore {
        base: PtVarId,
        field: String,
        src: PtVarId,
    },
    ElementLoad {
        dst: PtVarId,
        base: PtVarId,
        index: String,
    },
    ElementStore {
        base: PtVarId,
        index: String,
        src: PtVarId,
    },
    CallReturn {
        dst: PtVarId,
        value: ValueFactId,
    },
    SummaryFlow {
        dst: PtVarId,
        src: PtVarId,
        summary_key: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PointsToStatus {
    Present,
    Unknown,
    Unsupported,
    SetupMissing,
    BudgetExceeded,
    /// The set may lack values that cross more field loads and stores than
    /// the solve's indirection bound lets through.
    IndirectionBound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PointsToPrecision {
    FlowInsensitive,
    LocalFlowSensitive,
    SummaryProjected,
    Heuristic,
    Unknown,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PointsToBudgetStatus {
    WithinBudget,
    BudgetExceeded,
    NotRun,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_to_constraints_include_field_and_call_flow() {
        let constraints = [
            PointsToConstraintKind::FieldLoad {
                dst: PtVarId(1),
                base: PtVarId(2),
                field: "name".to_string(),
            },
            PointsToConstraintKind::CallReturn {
                dst: PtVarId(1),
                value: ValueFactId(3),
            },
        ];

        assert_eq!(constraints.len(), 2);
    }
}
