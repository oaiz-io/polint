use std::borrow::Cow;
use std::collections::BTreeMap;

use super::core::{
    ConstantDomain, InitializednessDomain, NilnessDomain, ReachabilityDomain, StringDomain,
    TruthinessDomain,
};
use super::facts::{
    DomainEventFact, DomainLocation, DomainObservationFact, DomainPrecision, DomainSlot,
    DomainStatus, DomainValue,
};
use super::lattice::{AbstractDomain, TopReason};
use super::results::{DomainResults, SolverStatus};
use super::state::ProductState;
use crate::analysis_api::{FactFamily, stable_key_from_key_parts, stable_key_from_parts};
use crate::analysis_neutral::cfg::ids::BasicBlockId;
use crate::analysis_neutral::ids::{
    DomainEventId, DomainObservationId, MirBodyId, MirOpId, PlaceId,
};
use crate::internal_core::KeyPart;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DomainOutput {
    pub observations: Vec<DomainObservationFact>,
    pub events: Vec<DomainEventFact>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainMaterialization {
    Full,
    SummaryInputs,
}

impl DomainOutput {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn from_results(
        interner: &crate::internal_core::StableKeyInterner,
        results: &DomainResults,
    ) -> Self {
        Self::from_results_with_place_filter(interner, results, None)
    }

    pub fn from_results_with_place_keys(
        interner: &crate::internal_core::StableKeyInterner,
        results: &DomainResults,
        place_stable_keys: &BTreeMap<PlaceId, String>,
    ) -> Self {
        Self::from_results_with_place_filter(interner, results, Some(place_stable_keys))
    }

    pub fn from_results_with_materialization(
        interner: &crate::internal_core::StableKeyInterner,
        results: &DomainResults,
        place_stable_keys: Option<&BTreeMap<PlaceId, String>>,
        materialization: DomainMaterialization,
    ) -> Self {
        match materialization {
            DomainMaterialization::Full => {
                Self::from_results_with_place_filter(interner, results, place_stable_keys)
            }
            DomainMaterialization::SummaryInputs => {
                Self::from_results_for_summary_inputs(interner, results)
            }
        }
    }

    fn from_results_with_place_filter(
        interner: &crate::internal_core::StableKeyInterner,
        results: &DomainResults,
        place_stable_keys: Option<&BTreeMap<PlaceId, String>>,
    ) -> Self {
        let mut output = Self::empty();
        for function in results.functions(interner) {
            push_state_observations(
                interner,
                &mut output.observations,
                function.body,
                None,
                None,
                DomainLocation::FunctionEntry,
                interner.resolve(function.body_stable_key).as_ref(),
                &function.entry_state,
                place_stable_keys,
            );
            if function.status == SolverStatus::BudgetExceeded {
                output.events.push(DomainEventFact {
                    id: DomainEventId(0),
                    body: function.body,
                    block: None,
                    operation: None,
                    slot: None,
                    status: DomainStatus::BudgetExceeded,
                    precision: DomainPrecision::Unknown,
                    reason: "solver_budget_exceeded".to_string(),
                    stable_key: stable_key_from_key_parts(
                        interner,
                        FactFamily::DomainEvent,
                        [
                            ("body", KeyPart::Key(function.body_stable_key)),
                            ("reason", KeyPart::Text("solver_budget_exceeded")),
                        ],
                    ),
                });
            }
        }
        for block in results.blocks(interner) {
            push_state_observations(
                interner,
                &mut output.observations,
                block.body,
                Some(block.block),
                None,
                DomainLocation::BlockEntry,
                interner.resolve(block.stable_key).as_ref(),
                &block.entry,
                place_stable_keys,
            );
            push_state_observations(
                interner,
                &mut output.observations,
                block.body,
                Some(block.block),
                None,
                DomainLocation::BlockExit,
                interner.resolve(block.stable_key).as_ref(),
                &block.exit,
                place_stable_keys,
            );
        }
        for operation in results.operations(interner) {
            push_state_observations(
                interner,
                &mut output.observations,
                operation.body,
                Some(operation.block),
                Some(operation.operation),
                DomainLocation::BeforeOperation,
                interner.resolve(operation.stable_key).as_ref(),
                &operation.before,
                place_stable_keys,
            );
            push_state_observations(
                interner,
                &mut output.observations,
                operation.body,
                Some(operation.block),
                Some(operation.operation),
                DomainLocation::AfterOperation,
                interner.resolve(operation.stable_key).as_ref(),
                &operation.after,
                place_stable_keys,
            );
        }
        for event in results.unknown_top_events(interner) {
            output.events.push(DomainEventFact {
                id: DomainEventId(0),
                body: event.body,
                block: event.block,
                operation: event.operation,
                slot: None,
                status: status_for_top_reason(event.reason),
                precision: precision_for_top_reason(event.reason),
                reason: event.reason.as_str().to_string(),
                stable_key: stable_key_from_key_parts(
                    interner,
                    FactFamily::DomainEvent,
                    [
                        ("source", KeyPart::Key(event.stable_key)),
                        ("reason", KeyPart::Text(event.reason.as_str())),
                    ],
                ),
            });
        }
        output.normalized(interner)
    }

    fn from_results_for_summary_inputs(
        interner: &crate::internal_core::StableKeyInterner,
        results: &DomainResults,
    ) -> Self {
        let mut output = Self::empty();
        for function in results.functions(interner) {
            push_reachability_observation(
                interner,
                &mut output.observations,
                function.body,
                None,
                None,
                DomainLocation::FunctionEntry,
                (
                    interner.resolve(function.body_stable_key).as_ref(),
                    &function.entry_state,
                ),
            );
            if function.status == SolverStatus::BudgetExceeded {
                output.events.push(DomainEventFact {
                    id: DomainEventId(0),
                    body: function.body,
                    block: None,
                    operation: None,
                    slot: None,
                    status: DomainStatus::BudgetExceeded,
                    precision: DomainPrecision::Unknown,
                    reason: "solver_budget_exceeded".to_string(),
                    stable_key: stable_key_from_key_parts(
                        interner,
                        FactFamily::DomainEvent,
                        [
                            ("body", KeyPart::Key(function.body_stable_key)),
                            ("reason", KeyPart::Text("solver_budget_exceeded")),
                        ],
                    ),
                });
            }
        }
        for block in results.blocks(interner) {
            push_reachability_observation(
                interner,
                &mut output.observations,
                block.body,
                Some(block.block),
                None,
                DomainLocation::BlockEntry,
                (interner.resolve(block.stable_key).as_ref(), &block.entry),
            );
        }
        for event in results.unknown_top_events(interner) {
            output.events.push(DomainEventFact {
                id: DomainEventId(0),
                body: event.body,
                block: event.block,
                operation: event.operation,
                slot: None,
                status: status_for_top_reason(event.reason),
                precision: precision_for_top_reason(event.reason),
                reason: event.reason.as_str().to_string(),
                stable_key: stable_key_from_key_parts(
                    interner,
                    FactFamily::DomainEvent,
                    [
                        ("source", KeyPart::Key(event.stable_key)),
                        ("reason", KeyPart::Text(event.reason.as_str())),
                    ],
                ),
            });
        }
        output.normalized(interner)
    }

    pub fn normalized(mut self, interner: &crate::internal_core::StableKeyInterner) -> Self {
        self.observations.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.body.cmp(&right.body))
                .then_with(|| left.block.cmp(&right.block))
                .then_with(|| left.operation.cmp(&right.operation))
                .then_with(|| left.place.cmp(&right.place))
                .then_with(|| left.slot.cmp(&right.slot))
                .then_with(|| left.location.cmp(&right.location))
                .then_with(|| left.status.cmp(&right.status))
                .then_with(|| left.id.cmp(&right.id))
        });
        self.events.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.body.cmp(&right.body))
                .then_with(|| left.block.cmp(&right.block))
                .then_with(|| left.operation.cmp(&right.operation))
                .then_with(|| left.slot.cmp(&right.slot))
                .then_with(|| left.status.cmp(&right.status))
                .then_with(|| left.reason.as_str().cmp(right.reason.as_str()))
                .then_with(|| left.id.cmp(&right.id))
        });
        for (index, fact) in self.observations.iter_mut().enumerate() {
            fact.id = DomainObservationId(index as u64);
        }
        for (index, fact) in self.events.iter_mut().enumerate() {
            fact.id = DomainEventId(index as u64);
        }
        self
    }
}

#[allow(clippy::too_many_arguments)]
fn push_state_observations(
    interner: &crate::internal_core::StableKeyInterner,
    rows: &mut Vec<DomainObservationFact>,
    body: MirBodyId,
    block: Option<BasicBlockId>,
    operation: Option<MirOpId>,
    location: DomainLocation,
    source_stable_key: &str,
    state: &ProductState,
    place_stable_keys: Option<&BTreeMap<PlaceId, String>>,
) {
    push_reachability_observation(
        interner,
        rows,
        body,
        block,
        operation,
        location,
        (source_stable_key, state),
    );
    for (place, value) in &state.core.nilness {
        let (status, precision, value) = nilness_fact_value(value);
        let (place_ref, stable_place) = stable_place_ref(*place, place_stable_keys);
        rows.push(observation(
            interner,
            body,
            block,
            operation,
            place_ref,
            stable_place,
            DomainSlot::Nilness,
            location,
            source_stable_key,
            status,
            precision,
            value,
        ));
    }
    for (place, value) in &state.core.truthiness {
        let (status, precision, value) = truthiness_fact_value(value);
        let (place_ref, stable_place) = stable_place_ref(*place, place_stable_keys);
        rows.push(observation(
            interner,
            body,
            block,
            operation,
            place_ref,
            stable_place,
            DomainSlot::Truthiness,
            location,
            source_stable_key,
            status,
            precision,
            value,
        ));
    }
    for (place, value) in &state.core.constants {
        let (status, precision, value) = constant_fact_value(value);
        let (place_ref, stable_place) = stable_place_ref(*place, place_stable_keys);
        rows.push(observation(
            interner,
            body,
            block,
            operation,
            place_ref,
            stable_place,
            DomainSlot::Constants,
            location,
            source_stable_key,
            status,
            precision,
            value,
        ));
    }
    for (place, value) in &state.core.strings {
        let (status, precision, value) = string_fact_value(value);
        let (place_ref, stable_place) = stable_place_ref(*place, place_stable_keys);
        rows.push(observation(
            interner,
            body,
            block,
            operation,
            place_ref,
            stable_place,
            DomainSlot::Strings,
            location,
            source_stable_key,
            status,
            precision,
            value,
        ));
    }
    for (place, value) in &state.core.initializedness {
        let (status, precision, value) = initializedness_fact_value(value);
        let (place_ref, stable_place) = stable_place_ref(*place, place_stable_keys);
        rows.push(observation(
            interner,
            body,
            block,
            operation,
            place_ref,
            stable_place,
            DomainSlot::Initializedness,
            location,
            source_stable_key,
            status,
            precision,
            value,
        ));
    }
}

fn push_reachability_observation(
    interner: &crate::internal_core::StableKeyInterner,
    rows: &mut Vec<DomainObservationFact>,
    body: MirBodyId,
    block: Option<BasicBlockId>,
    operation: Option<MirOpId>,
    location: DomainLocation,
    pair: (&str, &ProductState),
) {
    let (source_stable_key, state) = pair;
    let (status, precision, value) = reachability_fact_value(&state.core.reachability);
    rows.push(observation(
        interner,
        body,
        block,
        operation,
        None,
        None,
        DomainSlot::Reachability,
        location,
        source_stable_key,
        status,
        precision,
        value,
    ));
}

fn stable_place_ref(
    place: PlaceId,
    place_stable_keys: Option<&BTreeMap<PlaceId, String>>,
) -> (Option<PlaceId>, Option<Cow<'_, str>>) {
    match place_stable_keys {
        Some(place_stable_keys) => place_stable_keys
            .get(&place)
            .map(|stable_key| (Some(place), Some(Cow::Borrowed(stable_key.as_str()))))
            .unwrap_or((None, None)),
        None => (Some(place), Some(Cow::Owned(format!("place:{}", place.0)))),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Domain fact identity is a normalized tuple over location, slot, and optional place."
)]
fn observation(
    interner: &crate::internal_core::StableKeyInterner,
    body: MirBodyId,
    block: Option<BasicBlockId>,
    operation: Option<MirOpId>,
    place: Option<PlaceId>,
    stable_place: Option<Cow<'_, str>>,
    slot: DomainSlot,
    location: DomainLocation,
    source_stable_key: &str,
    status: DomainStatus,
    precision: DomainPrecision,
    value: DomainValue,
) -> DomainObservationFact {
    let (status, precision, value) =
        normalize_observation_value_for_location(operation, status, precision, value);
    DomainObservationFact {
        id: DomainObservationId(0),
        body,
        block,
        operation,
        place,
        slot,
        location,
        value,
        status,
        precision,
        stable_key: stable_key_from_parts(
            interner,
            FactFamily::DomainObservation,
            &[
                ("source", source_stable_key.to_string()),
                ("slot", slot.as_str().to_string()),
                ("location", location.as_str().to_string()),
                (
                    "place",
                    stable_place.as_deref().unwrap_or("none").to_string(),
                ),
            ],
        ),
    }
}

fn normalize_observation_value_for_location(
    operation: Option<MirOpId>,
    status: DomainStatus,
    precision: DomainPrecision,
    value: DomainValue,
) -> (DomainStatus, DomainPrecision, DomainValue) {
    if operation.is_none()
        && matches!(
            value,
            DomainValue::TopReason(ref reason) if reason == TopReason::UnresolvedCall.as_str()
        )
    {
        return top_value(TopReason::UnknownValue);
    }
    (status, precision, value)
}

fn reachability_fact_value(
    domain: &ReachabilityDomain,
) -> (DomainStatus, DomainPrecision, DomainValue) {
    match domain {
        ReachabilityDomain::Unreachable => label("unreachable", DomainPrecision::ExactLocal),
        ReachabilityDomain::Reachable => label("reachable", DomainPrecision::ExactLocal),
        ReachabilityDomain::Ambiguous => label("ambiguous", DomainPrecision::Conservative),
        ReachabilityDomain::Top(reason) => top_value(*reason),
    }
}

fn nilness_fact_value(domain: &NilnessDomain) -> (DomainStatus, DomainPrecision, DomainValue) {
    match domain {
        NilnessDomain::Bottom => top_value(TopReason::UnknownValue),
        NilnessDomain::Nil => label("nil", DomainPrecision::ExactLocal),
        NilnessDomain::NonNil => label("non_nil", DomainPrecision::ExactLocal),
        NilnessDomain::MaybeNil => label("maybe_nil", DomainPrecision::Conservative),
        NilnessDomain::Top(reason) => top_value(*reason),
    }
}

fn truthiness_fact_value(
    domain: &TruthinessDomain,
) -> (DomainStatus, DomainPrecision, DomainValue) {
    match domain {
        TruthinessDomain::Bottom => top_value(TopReason::UnknownValue),
        TruthinessDomain::Truthy => label("truthy", DomainPrecision::ExactLocal),
        TruthinessDomain::Falsy => label("falsy", DomainPrecision::ExactLocal),
        TruthinessDomain::Maybe => label("maybe", DomainPrecision::Conservative),
        TruthinessDomain::Top(reason) => top_value(*reason),
    }
}

fn constant_fact_value(domain: &ConstantDomain) -> (DomainStatus, DomainPrecision, DomainValue) {
    match domain {
        ConstantDomain::Bottom => top_value(TopReason::UnknownValue),
        ConstantDomain::Values(_) => digest_value(domain.stable_digest_parts()),
        ConstantDomain::Top(reason) => top_value(*reason),
    }
}

fn string_fact_value(domain: &StringDomain) -> (DomainStatus, DomainPrecision, DomainValue) {
    match domain {
        StringDomain::Bottom => top_value(TopReason::UnknownValue),
        StringDomain::Values(_) => digest_value(domain.stable_digest_parts()),
        StringDomain::Top(reason) => top_value(*reason),
    }
}

fn initializedness_fact_value(
    domain: &InitializednessDomain,
) -> (DomainStatus, DomainPrecision, DomainValue) {
    match domain {
        InitializednessDomain::Bottom => top_value(TopReason::UnknownValue),
        InitializednessDomain::Initialized => label("initialized", DomainPrecision::ExactLocal),
        InitializednessDomain::Uninitialized => label("uninitialized", DomainPrecision::ExactLocal),
        InitializednessDomain::MaybeUninitialized => {
            label("maybe_uninitialized", DomainPrecision::Conservative)
        }
        InitializednessDomain::Top(reason) => top_value(*reason),
    }
}

fn label(value: &str, precision: DomainPrecision) -> (DomainStatus, DomainPrecision, DomainValue) {
    let status = if precision == DomainPrecision::Unknown {
        DomainStatus::Unknown
    } else {
        DomainStatus::Present
    };
    (status, precision, DomainValue::Label(value.to_string()))
}

fn digest_value(parts: Vec<String>) -> (DomainStatus, DomainPrecision, DomainValue) {
    (
        DomainStatus::Present,
        DomainPrecision::ExactLocal,
        DomainValue::DigestParts(parts),
    )
}

fn top_value(reason: TopReason) -> (DomainStatus, DomainPrecision, DomainValue) {
    (
        status_for_top_reason(reason),
        precision_for_top_reason(reason),
        DomainValue::TopReason(reason.as_str().to_string()),
    )
}

fn status_for_top_reason(reason: TopReason) -> DomainStatus {
    match reason {
        TopReason::UnknownValue | TopReason::DynamicWrite | TopReason::UnresolvedCall => {
            DomainStatus::Unknown
        }
        TopReason::UnsupportedSemantic => DomainStatus::Unsupported,
        TopReason::SetupMissing => DomainStatus::SetupMissing,
        TopReason::BudgetExceeded => DomainStatus::BudgetExceeded,
        TopReason::Widened | TopReason::ConflictingFacts => DomainStatus::Top,
    }
}

fn precision_for_top_reason(reason: TopReason) -> DomainPrecision {
    match reason {
        TopReason::UnsupportedSemantic => DomainPrecision::Unsupported,
        TopReason::SetupMissing => DomainPrecision::SetupAware,
        TopReason::UnknownValue | TopReason::DynamicWrite | TopReason::UnresolvedCall => {
            DomainPrecision::Unknown
        }
        TopReason::BudgetExceeded | TopReason::Widened | TopReason::ConflictingFacts => {
            DomainPrecision::Conservative
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct DomainStore {
    output: DomainOutput,
    observations_by_body: BTreeMap<MirBodyId, Vec<usize>>,
    observations_by_block: BTreeMap<BasicBlockId, Vec<usize>>,
    observations_by_operation: BTreeMap<MirOpId, Vec<usize>>,
    observations_by_place: BTreeMap<PlaceId, Vec<usize>>,
    observations_by_slot: BTreeMap<DomainSlot, Vec<usize>>,
    observations_by_status: BTreeMap<DomainStatus, Vec<usize>>,
    observations_by_stable_key: BTreeMap<crate::internal_core::StableKeyId, usize>,
    events_by_body: BTreeMap<MirBodyId, Vec<usize>>,
    events_by_status: BTreeMap<DomainStatus, Vec<usize>>,
    events_by_stable_key: BTreeMap<crate::internal_core::StableKeyId, usize>,
}

impl DomainStore {
    pub fn from_output(
        output: DomainOutput,
        interner: &crate::internal_core::StableKeyInterner,
    ) -> Self {
        Self::from_normalized_output(output.normalized(interner))
    }

    pub fn from_normalized_output(output: DomainOutput) -> Self {
        let mut store = Self {
            output,
            ..Self::default()
        };

        for (index, fact) in store.output.observations.iter().enumerate() {
            store
                .observations_by_body
                .entry(fact.body)
                .or_default()
                .push(index);
            if let Some(block) = fact.block {
                store
                    .observations_by_block
                    .entry(block)
                    .or_default()
                    .push(index);
            }
            if let Some(operation) = fact.operation {
                store
                    .observations_by_operation
                    .entry(operation)
                    .or_default()
                    .push(index);
            }
            if let Some(place) = fact.place {
                store
                    .observations_by_place
                    .entry(place)
                    .or_default()
                    .push(index);
            }
            store
                .observations_by_slot
                .entry(fact.slot)
                .or_default()
                .push(index);
            store
                .observations_by_status
                .entry(fact.status)
                .or_default()
                .push(index);
            store
                .observations_by_stable_key
                .insert(fact.stable_key, index);
        }

        for (index, fact) in store.output.events.iter().enumerate() {
            store
                .events_by_body
                .entry(fact.body)
                .or_default()
                .push(index);
            store
                .events_by_status
                .entry(fact.status)
                .or_default()
                .push(index);
            store.events_by_stable_key.insert(fact.stable_key, index);
        }

        store
    }

    pub fn observations(&self) -> &[DomainObservationFact] {
        &self.output.observations
    }

    pub fn events(&self) -> &[DomainEventFact] {
        &self.output.events
    }

    pub fn observations_by_body(&self, body: MirBodyId) -> Vec<&DomainObservationFact> {
        self.observation_refs(self.observations_by_body.get(&body))
    }

    pub fn observations_by_block(&self, block: BasicBlockId) -> Vec<&DomainObservationFact> {
        self.observation_refs(self.observations_by_block.get(&block))
    }

    pub fn observations_by_operation(&self, operation: MirOpId) -> Vec<&DomainObservationFact> {
        self.observation_refs(self.observations_by_operation.get(&operation))
    }

    pub fn observations_by_place(&self, place: PlaceId) -> Vec<&DomainObservationFact> {
        self.observation_refs(self.observations_by_place.get(&place))
    }

    pub fn observations_by_slot(&self, slot: DomainSlot) -> Vec<&DomainObservationFact> {
        self.observation_refs(self.observations_by_slot.get(&slot))
    }

    pub fn observations_by_status(&self, status: DomainStatus) -> Vec<&DomainObservationFact> {
        self.observation_refs(self.observations_by_status.get(&status))
    }

    pub fn observation_by_stable_key(
        &self,
        stable_key: crate::internal_core::StableKeyId,
    ) -> Option<&DomainObservationFact> {
        self.observations_by_stable_key
            .get(&stable_key)
            .map(|&index| &self.output.observations[index])
    }

    pub fn events_by_status(&self, status: DomainStatus) -> Vec<&DomainEventFact> {
        self.event_refs(self.events_by_status.get(&status))
    }

    pub fn event_by_stable_key(
        &self,
        stable_key: crate::internal_core::StableKeyId,
    ) -> Option<&DomainEventFact> {
        self.events_by_stable_key
            .get(&stable_key)
            .map(|&index| &self.output.events[index])
    }

    fn observation_refs(&self, indexes: Option<&Vec<usize>>) -> Vec<&DomainObservationFact> {
        indexes.map_or_else(Vec::new, |indexes| {
            indexes
                .iter()
                .map(|&index| &self.output.observations[index])
                .collect()
        })
    }

    fn event_refs(&self, indexes: Option<&Vec<usize>>) -> Vec<&DomainEventFact> {
        indexes.map_or_else(Vec::new, |indexes| {
            indexes
                .iter()
                .map(|&index| &self.output.events[index])
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_api::{FactFamily, FactRef};
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::cfg::ids::BasicBlockId;
    use crate::analysis_neutral::domains::core::ConstantLiteral;
    use crate::analysis_neutral::domains::facts::{
        DomainLocation, DomainObservationFact, DomainPrecision, DomainSlot, DomainStatus,
        DomainValue,
    };
    use crate::analysis_neutral::ids::{DomainObservationId, MirBodyId, MirOpId, PlaceId};

    fn observation(id: u64, stable_key: &str, status: DomainStatus) -> DomainObservationFact {
        DomainObservationFact {
            id: DomainObservationId(id),
            body: MirBodyId(1),
            block: Some(BasicBlockId(2)),
            operation: Some(MirOpId(id)),
            place: Some(PlaceId(4)),
            slot: DomainSlot::Nilness,
            location: DomainLocation::AfterOperation,
            value: DomainValue::Label(format!("{status:?}")),
            status,
            precision: DomainPrecision::Conservative,
            stable_key: crate::internal_core::stable_key_for_test(stable_key),
        }
    }

    #[test]
    fn abstract_domain_fact_storage_normalized_sorts_rows_without_dropping_unknown_statuses() {
        let interner = crate::internal_core::test_stable_key_interner();
        let output = DomainOutput {
            observations: vec![
                observation(2, "domain:z", DomainStatus::Unsupported),
                observation(1, "domain:a", DomainStatus::Unknown),
                observation(3, "domain:m", DomainStatus::BudgetExceeded),
            ],
            events: Vec::new(),
        }
        .normalized(&interner);

        assert_eq!(
            output
                .observations
                .iter()
                .map(|fact| (interner.resolve(fact.stable_key), fact.status))
                .collect::<Vec<_>>(),
            vec![
                (
                    std::sync::Arc::<str>::from("domain:a"),
                    DomainStatus::Unknown
                ),
                (
                    std::sync::Arc::<str>::from("domain:m"),
                    DomainStatus::BudgetExceeded,
                ),
                (
                    std::sync::Arc::<str>::from("domain:z"),
                    DomainStatus::Unsupported,
                ),
            ]
        );
    }

    #[test]
    fn abstract_domain_fact_metadata_replace_removes_stale_rows_and_refreshes_metadata() {
        let mut db = LocalAnalysisDb::new();
        db.replace_abstract_domain_facts(DomainOutput {
            observations: vec![observation(1, "domain:first", DomainStatus::Present)],
            events: Vec::new(),
        });
        db.replace_abstract_domain_facts(DomainOutput {
            observations: vec![observation(2, "domain:second", DomainStatus::Top)],
            events: Vec::new(),
        });

        assert_eq!(db.abstract_domain_observations().len(), 1);
        assert_eq!(
            db.resolve_stable_key(db.abstract_domain_observations()[0].stable_key)
                .as_ref(),
            "domain:second"
        );
        assert!(
            db.fact_meta()
                .get(FactRef::new(FactFamily::DomainObservation, 0))
                .is_some()
        );
        assert!(
            db.fact_meta()
                .get(FactRef::new(FactFamily::DomainObservation, 1))
                .is_none()
        );
    }

    #[test]
    fn domain_observation_stable_key_uses_place_stable_key_not_dense_place_id() {
        let first = domain_output_for_place(PlaceId(7), "place:stable");
        let second = domain_output_for_place(PlaceId(99), "place:stable");
        let first_key = first
            .observations
            .iter()
            .find(|row| row.slot == DomainSlot::Constants)
            .expect("constant row")
            .stable_key;
        let second_key = second
            .observations
            .iter()
            .find(|row| row.slot == DomainSlot::Constants)
            .expect("constant row")
            .stable_key;

        assert_eq!(first_key, second_key);
    }

    fn domain_output_for_place(place: PlaceId, place_key: &str) -> DomainOutput {
        let interner = crate::internal_core::test_stable_key_interner();
        let mut results = DomainResults::new();
        let mut state = ProductState::entry();
        state.core.constants.insert(
            place,
            ConstantDomain::from_literal(ConstantLiteral::Bool(true)),
        );
        results.insert_function(
            MirBodyId(1),
            interner.intern("body:stable"),
            SolverStatus::Solved,
            state,
        );
        DomainOutput::from_results_with_place_keys(
            &interner,
            &results,
            &BTreeMap::from([(place, place_key.to_string())]),
        )
    }
}
