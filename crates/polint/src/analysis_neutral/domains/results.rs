use std::collections::BTreeMap;

use super::lattice::TopReason;
use super::state::ProductState;
use crate::analysis_neutral::cfg::ids::BasicBlockId;
use crate::analysis_neutral::ids::{MirBodyId, MirOpId, PlaceId};
use crate::internal_core::StableKeyId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SolverStatus {
    Solved,
    Widened,
    BudgetExceeded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainResults {
    functions: BTreeMap<MirBodyId, FunctionResult>,
    block_states: BTreeMap<BasicBlockId, BlockState>,
    operation_states: BTreeMap<MirOpId, OperationState>,
    top_events: BTreeMap<StableKeyId, TopEvent>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionResult {
    pub body: MirBodyId,
    pub body_stable_key: StableKeyId,
    pub status: SolverStatus,
    pub entry_state: ProductState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockState {
    pub body: MirBodyId,
    pub block: BasicBlockId,
    pub stable_key: StableKeyId,
    pub entry: ProductState,
    pub exit: ProductState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationState {
    pub body: MirBodyId,
    pub block: BasicBlockId,
    pub operation: MirOpId,
    pub stable_key: StableKeyId,
    pub before: ProductState,
    pub after: ProductState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaceObservation {
    pub body: MirBodyId,
    pub block: Option<BasicBlockId>,
    pub operation: Option<MirOpId>,
    pub place: PlaceId,
    pub stable_key: StableKeyId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopEvent {
    pub body: MirBodyId,
    pub block: Option<BasicBlockId>,
    pub operation: Option<MirOpId>,
    pub reason: TopReason,
    pub stable_key: StableKeyId,
}

impl Default for DomainResults {
    fn default() -> Self {
        Self::new()
    }
}

impl DomainResults {
    pub fn new() -> Self {
        Self {
            functions: BTreeMap::new(),
            block_states: BTreeMap::new(),
            operation_states: BTreeMap::new(),
            top_events: BTreeMap::new(),
        }
    }

    pub fn insert_function(
        &mut self,
        body: MirBodyId,
        body_stable_key: StableKeyId,
        status: SolverStatus,
        entry_state: ProductState,
    ) {
        self.functions.insert(
            body,
            FunctionResult {
                body,
                body_stable_key,
                status,
                entry_state,
            },
        );
    }

    pub fn insert_block_state(
        &mut self,
        body: MirBodyId,
        block: BasicBlockId,
        stable_key: StableKeyId,
        entry: ProductState,
        exit: ProductState,
    ) {
        self.block_states.insert(
            block,
            BlockState {
                body,
                block,
                stable_key,
                entry,
                exit,
            },
        );
    }

    pub fn insert_operation_state(
        &mut self,
        body: MirBodyId,
        block: BasicBlockId,
        operation: MirOpId,
        stable_key: StableKeyId,
        before: ProductState,
        after: ProductState,
    ) {
        self.operation_states.insert(
            operation,
            OperationState {
                body,
                block,
                operation,
                stable_key,
                before,
                after,
            },
        );
    }

    pub fn record_top_event(
        &mut self,
        body: MirBodyId,
        block: Option<BasicBlockId>,
        operation: Option<MirOpId>,
        reason: TopReason,
        stable_key: StableKeyId,
    ) {
        self.top_events.insert(
            stable_key,
            TopEvent {
                body,
                block,
                operation,
                reason,
                stable_key,
            },
        );
    }

    pub fn entry_state(&self, body: MirBodyId) -> Option<&ProductState> {
        self.functions
            .get(&body)
            .map(|function| &function.entry_state)
    }

    pub fn block_entry(&self, block: BasicBlockId) -> Option<&ProductState> {
        self.block_states.get(&block).map(|state| &state.entry)
    }

    pub fn before_operation(&self, operation: MirOpId) -> Option<&ProductState> {
        self.operation_states
            .get(&operation)
            .map(|state| &state.before)
    }

    pub fn after_operation(&self, operation: MirOpId) -> Option<&ProductState> {
        self.operation_states
            .get(&operation)
            .map(|state| &state.after)
    }

    pub fn block_exit(&self, block: BasicBlockId) -> Option<&ProductState> {
        self.block_states.get(&block).map(|state| &state.exit)
    }

    pub fn statuses(&self) -> impl Iterator<Item = SolverStatus> + '_ {
        self.functions.values().map(|function| function.status)
    }

    pub fn functions(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
    ) -> impl Iterator<Item = &FunctionResult> {
        let mut functions = self.functions.values().collect::<Vec<_>>();
        functions.sort_by(|left, right| {
            interner.compare_canonical(left.body_stable_key, right.body_stable_key)
        });
        functions.into_iter()
    }

    pub fn top_events(&self) -> impl Iterator<Item = &TopEvent> {
        self.top_events.values()
    }

    pub fn unknown_top_events(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
    ) -> impl Iterator<Item = &TopEvent> {
        let mut events = self.top_events.values().collect::<Vec<_>>();
        events.sort_by(|left, right| interner.compare_canonical(left.stable_key, right.stable_key));
        events.into_iter()
    }

    pub fn blocks(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
    ) -> impl Iterator<Item = &BlockState> {
        let mut blocks = self.block_states.values().collect::<Vec<_>>();
        blocks.sort_by(|left, right| interner.compare_canonical(left.stable_key, right.stable_key));
        blocks.into_iter()
    }

    pub fn operations(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
    ) -> impl Iterator<Item = &OperationState> {
        let mut operations = self.operation_states.values().collect::<Vec<_>>();
        operations
            .sort_by(|left, right| interner.compare_canonical(left.stable_key, right.stable_key));
        operations.into_iter()
    }

    pub fn place_observations(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
    ) -> impl Iterator<Item = PlaceObservation> + '_ {
        let mut rows = Vec::new();
        for block in self.blocks(interner) {
            push_place_observations(
                &mut rows,
                block.body,
                Some(block.block),
                None,
                block.stable_key,
                interner,
                &block.entry,
            );
            push_place_observations(
                &mut rows,
                block.body,
                Some(block.block),
                None,
                block.stable_key,
                interner,
                &block.exit,
            );
        }
        for operation in self.operations(interner) {
            push_place_observations(
                &mut rows,
                operation.body,
                Some(operation.block),
                Some(operation.operation),
                operation.stable_key,
                interner,
                &operation.before,
            );
            push_place_observations(
                &mut rows,
                operation.body,
                Some(operation.block),
                Some(operation.operation),
                operation.stable_key,
                interner,
                &operation.after,
            );
        }
        rows.sort_by(|left, right| interner.compare_canonical(left.stable_key, right.stable_key));
        rows.into_iter()
    }

    pub fn has_top_reason(&self, reason: TopReason) -> bool {
        self.top_events.values().any(|event| event.reason == reason)
    }

    pub fn stable_digest_parts(
        &self,
        interner: &crate::internal_core::StableKeyInterner,
    ) -> Vec<String> {
        let mut parts = Vec::new();
        for function in self.functions.values() {
            parts.push(format!(
                "function;stable_key={};status={:?}",
                interner.resolve(function.body_stable_key),
                function.status
            ));
            parts.extend(
                function
                    .entry_state
                    .stable_digest_parts()
                    .into_iter()
                    .map(|part| {
                        format!(
                            "function;stable_key={};entry;{part}",
                            interner.resolve(function.body_stable_key)
                        )
                    }),
            );
        }
        for block in self.blocks(interner) {
            parts.extend(block.entry.stable_digest_parts().into_iter().map(|part| {
                format!(
                    "block;stable_key={};entry;{part}",
                    interner.resolve(block.stable_key)
                )
            }));
            parts.extend(block.exit.stable_digest_parts().into_iter().map(|part| {
                format!(
                    "block;stable_key={};exit;{part}",
                    interner.resolve(block.stable_key)
                )
            }));
        }
        for operation in self.operations(interner) {
            parts.extend(
                operation
                    .before
                    .stable_digest_parts()
                    .into_iter()
                    .map(|part| {
                        format!(
                            "operation;stable_key={};before;{part}",
                            interner.resolve(operation.stable_key)
                        )
                    }),
            );
            parts.extend(
                operation
                    .after
                    .stable_digest_parts()
                    .into_iter()
                    .map(|part| {
                        format!(
                            "operation;stable_key={};after;{part}",
                            interner.resolve(operation.stable_key)
                        )
                    }),
            );
        }
        for event in self.top_events.values() {
            parts.push(format!(
                "top;stable_key={};reason={}",
                interner.resolve(event.stable_key),
                event.reason.as_str()
            ));
        }
        parts.sort();
        parts
    }

    #[cfg(test)]
    pub fn for_test(
        interner: &crate::internal_core::StableKeyInterner,
        body: MirBodyId,
        block: BasicBlockId,
        operation: MirOpId,
        place: PlaceId,
    ) -> Self {
        let mut state = ProductState::entry();
        state.mark_place_top(place, TopReason::UnknownValue);

        let mut results = Self::new();
        results.insert_function(
            body,
            interner.intern("stable_key:test-body"),
            SolverStatus::Solved,
            state.clone(),
        );
        results.insert_block_state(
            body,
            block,
            interner.intern("stable_key:test-block"),
            state.clone(),
            state.clone(),
        );
        results.insert_operation_state(
            body,
            block,
            operation,
            interner.intern("stable_key:test-operation"),
            state.clone(),
            state,
        );
        results
    }
}

fn push_place_observations(
    rows: &mut Vec<PlaceObservation>,
    body: MirBodyId,
    block: Option<BasicBlockId>,
    operation: Option<MirOpId>,
    prefix: StableKeyId,
    interner: &crate::internal_core::StableKeyInterner,
    state: &ProductState,
) {
    for place in state.observed_places() {
        rows.push(PlaceObservation {
            body,
            block,
            operation,
            place,
            stable_key: interner.intern(format!(
                "{};block={:?};operation={:?};place={}",
                interner.resolve(prefix),
                block,
                operation,
                place.0
            )),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::cfg::ids::BasicBlockId;
    use crate::analysis_neutral::ids::{MirBodyId, MirOpId, PlaceId};

    #[test]
    fn result_cursor_exposes_entry_block_operation_and_exit_states() {
        let interner = crate::internal_core::test_stable_key_interner();
        let body = MirBodyId(1);
        let block = BasicBlockId(2);
        let operation = MirOpId(3);
        let place = PlaceId(4);
        let results = DomainResults::for_test(&interner, body, block, operation, place);

        assert!(results.entry_state(body).is_some());
        assert!(results.block_entry(block).is_some());
        assert!(results.before_operation(operation).is_some());
        assert!(results.after_operation(operation).is_some());
        assert!(results.block_exit(block).is_some());
        assert!(
            results
                .place_observations(&interner)
                .any(|row| row.place == place)
        );
    }

    #[test]
    fn stable_key_result_iteration_is_deterministic() {
        let interner = crate::internal_core::test_stable_key_interner();
        let results = DomainResults::for_test(
            &interner,
            MirBodyId(1),
            BasicBlockId(2),
            MirOpId(3),
            PlaceId(4),
        );

        assert_eq!(
            results.stable_digest_parts(&interner),
            results.stable_digest_parts(&interner)
        );
    }

    #[test]
    fn function_and_unknown_top_event_iterators_are_stable_key_ordered() {
        let interner = crate::internal_core::test_stable_key_interner();
        let mut results = DomainResults::for_test(
            &interner,
            MirBodyId(1),
            BasicBlockId(2),
            MirOpId(3),
            PlaceId(4),
        );
        results.record_top_event(
            MirBodyId(1),
            Some(BasicBlockId(2)),
            Some(MirOpId(3)),
            TopReason::UnknownValue,
            interner.intern("event:unknown"),
        );

        assert_eq!(
            results
                .functions(&interner)
                .map(|function| interner.resolve(function.body_stable_key))
                .collect::<Vec<_>>(),
            vec![std::sync::Arc::<str>::from("stable_key:test-body")]
        );
        assert_eq!(
            results
                .unknown_top_events(&interner)
                .map(|event| interner.resolve(event.stable_key))
                .collect::<Vec<_>>(),
            vec![std::sync::Arc::<str>::from("event:unknown")]
        );
    }
}
