use std::collections::{BTreeMap, BTreeSet};

use super::core::{
    ConstantDomain, ConstantLiteral, InitializednessDomain, NilnessDomain, ReachabilityDomain,
    StringDomain, TruthinessDomain,
};
use super::lattice::{AbstractDomain, Changed, TopReason, WidenFuel, WidenSite};
use crate::analysis_neutral::ids::PlaceId;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreDomains {
    pub reachability: ReachabilityDomain,
    pub nilness: BTreeMap<PlaceId, NilnessDomain>,
    pub truthiness: BTreeMap<PlaceId, TruthinessDomain>,
    pub constants: BTreeMap<PlaceId, ConstantDomain>,
    pub strings: BTreeMap<PlaceId, StringDomain>,
    pub initializedness: BTreeMap<PlaceId, InitializednessDomain>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductState {
    pub core: CoreDomains,
    extension_slots: ExtensionDomainSlots,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ExtensionDomainSlots;

impl CoreDomains {
    fn bottom() -> Self {
        Self {
            reachability: ReachabilityDomain::bottom(),
            nilness: BTreeMap::new(),
            truthiness: BTreeMap::new(),
            constants: BTreeMap::new(),
            strings: BTreeMap::new(),
            initializedness: BTreeMap::new(),
        }
    }

    fn entry() -> Self {
        Self {
            reachability: ReachabilityDomain::Reachable,
            ..Self::bottom()
        }
    }
}

impl ProductState {
    pub fn bottom() -> Self {
        Self {
            core: CoreDomains::bottom(),
            extension_slots: ExtensionDomainSlots,
        }
    }

    pub fn entry() -> Self {
        Self {
            core: CoreDomains::entry(),
            extension_slots: ExtensionDomainSlots,
        }
    }

    pub fn join_into(&mut self, incoming: &Self) -> Changed {
        let mut changed = self
            .core
            .reachability
            .join_into(&incoming.core.reachability);
        changed |= join_place_map(&mut self.core.nilness, &incoming.core.nilness);
        changed |= join_place_map(&mut self.core.truthiness, &incoming.core.truthiness);
        changed |= join_place_map(&mut self.core.constants, &incoming.core.constants);
        changed |= join_place_map(&mut self.core.strings, &incoming.core.strings);
        changed |= join_place_map(
            &mut self.core.initializedness,
            &incoming.core.initializedness,
        );
        changed
    }

    pub fn leq(&self, other: &Self) -> bool {
        self.core.reachability.leq(&other.core.reachability)
            && place_map_leq(&self.core.nilness, &other.core.nilness)
            && place_map_leq(&self.core.truthiness, &other.core.truthiness)
            && place_map_leq(&self.core.constants, &other.core.constants)
            && place_map_leq(&self.core.strings, &other.core.strings)
            && place_map_leq(&self.core.initializedness, &other.core.initializedness)
    }

    pub fn mark_reachability_top(&mut self, reason: TopReason) {
        self.core.reachability = ReachabilityDomain::top(reason);
    }

    pub fn mark_place_top(&mut self, place: PlaceId, reason: TopReason) {
        self.core.nilness.insert(place, NilnessDomain::top(reason));
        self.core
            .truthiness
            .insert(place, TruthinessDomain::top(reason));
        self.core
            .constants
            .insert(place, ConstantDomain::top(reason));
        self.core.strings.insert(place, StringDomain::top(reason));
        self.core
            .initializedness
            .insert(place, InitializednessDomain::top(reason));
    }

    pub fn observed_places(&self) -> BTreeSet<PlaceId> {
        self.core
            .nilness
            .keys()
            .chain(self.core.truthiness.keys())
            .chain(self.core.constants.keys())
            .chain(self.core.strings.keys())
            .chain(self.core.initializedness.keys())
            .copied()
            .collect()
    }

    pub fn retain_places(&mut self, places: &BTreeSet<PlaceId>) {
        self.retain_places_where(|place| places.contains(place));
    }

    /// Keeps the facts of the places `keep` accepts.
    pub fn retain_places_where(&mut self, keep: impl Fn(&PlaceId) -> bool) {
        self.core.nilness.retain(|place, _| keep(place));
        self.core.truthiness.retain(|place, _| keep(place));
        self.core.constants.retain(|place, _| keep(place));
        self.core.strings.retain(|place, _| keep(place));
        self.core.initializedness.retain(|place, _| keep(place));
    }

    pub fn copy_place_from(&mut self, source_state: &Self, target: PlaceId, source: PlaceId) {
        self.core.nilness.remove(&target);
        self.core.truthiness.remove(&target);
        self.core.constants.remove(&target);
        self.core.strings.remove(&target);
        self.core.initializedness.remove(&target);
        copy_slot(
            &mut self.core.nilness,
            &source_state.core.nilness,
            target,
            source,
        );
        copy_slot(
            &mut self.core.truthiness,
            &source_state.core.truthiness,
            target,
            source,
        );
        copy_slot(
            &mut self.core.constants,
            &source_state.core.constants,
            target,
            source,
        );
        copy_slot(
            &mut self.core.strings,
            &source_state.core.strings,
            target,
            source,
        );
        copy_slot(
            &mut self.core.initializedness,
            &source_state.core.initializedness,
            target,
            source,
        );
    }

    pub fn widen(&self, next: &Self, site: WidenSite, fuel: WidenFuel) -> Self {
        let mut widened = self.clone();
        widened.core.reachability =
            self.core
                .reachability
                .widen(&next.core.reachability, site.clone(), fuel);
        widen_place_map(
            &mut widened.core.nilness,
            &self.core.nilness,
            &next.core.nilness,
            &site,
            fuel,
        );
        widen_place_map(
            &mut widened.core.truthiness,
            &self.core.truthiness,
            &next.core.truthiness,
            &site,
            fuel,
        );
        widen_place_map(
            &mut widened.core.constants,
            &self.core.constants,
            &next.core.constants,
            &site,
            fuel,
        );
        widen_place_map(
            &mut widened.core.strings,
            &self.core.strings,
            &next.core.strings,
            &site,
            fuel,
        );
        widen_place_map(
            &mut widened.core.initializedness,
            &self.core.initializedness,
            &next.core.initializedness,
            &site,
            fuel,
        );
        widened
    }

    pub fn reduce_value_only(&mut self, max_rounds: u32) -> Changed {
        let mut total_changed = Changed::No;
        for _ in 0..max_rounds {
            let constants = self.core.constants.clone();
            let mut round_changed = Changed::No;
            for (place, constant) in constants {
                round_changed |= self.reduce_constant(place, &constant);
            }
            total_changed |= round_changed;
            if round_changed == Changed::No {
                break;
            }
        }
        total_changed
    }

    pub fn stable_digest_parts(&self) -> Vec<String> {
        let mut parts = Vec::new();
        parts.extend(
            self.core
                .reachability
                .stable_digest_parts()
                .into_iter()
                .map(|part| format!("slot=reachability;{part}")),
        );
        push_map_parts("nilness", &self.core.nilness, &mut parts);
        push_map_parts("truthiness", &self.core.truthiness, &mut parts);
        push_map_parts("constants", &self.core.constants, &mut parts);
        push_map_parts("strings", &self.core.strings, &mut parts);
        push_map_parts("initializedness", &self.core.initializedness, &mut parts);
        parts.extend(self.extension_slots.stable_digest_parts());
        parts
    }

    fn reduce_constant(&mut self, place: PlaceId, constant: &ConstantDomain) -> Changed {
        let mut changed = Changed::No;
        let ConstantDomain::Values(values) = constant else {
            return changed;
        };

        let mut nilness = NilnessDomain::bottom();
        let mut truthiness = TruthinessDomain::bottom();
        let mut strings = StringDomain::bottom();

        for literal in values {
            if let Some(value) = nilness_from_literal(literal) {
                nilness.join_into(&value);
            }
            if let Some(value) = truthiness_from_literal(literal) {
                truthiness.join_into(&value);
            }
            if let ConstantLiteral::String(value) = literal {
                strings.join_into(&StringDomain::from_literal(value));
            }
        }

        changed |= join_single(&mut self.core.nilness, place, &nilness);
        changed |= join_single(&mut self.core.truthiness, place, &truthiness);
        changed |= join_single(&mut self.core.strings, place, &strings);
        changed
    }
}

impl ExtensionDomainSlots {
    fn stable_digest_parts(&self) -> Vec<String> {
        vec!["slot=extensions;kind=empty".to_string()]
    }
}

fn join_place_map<D>(target: &mut BTreeMap<PlaceId, D>, incoming: &BTreeMap<PlaceId, D>) -> Changed
where
    D: AbstractDomain,
{
    let mut changed = Changed::No;
    for (place, incoming_value) in incoming {
        changed |= join_single(target, *place, incoming_value);
    }
    changed
}

fn copy_slot<D: Clone>(
    target_map: &mut BTreeMap<PlaceId, D>,
    source_map: &BTreeMap<PlaceId, D>,
    target: PlaceId,
    source: PlaceId,
) {
    if let Some(value) = source_map.get(&source) {
        target_map.insert(target, value.clone());
    }
}

fn place_map_leq<D>(left: &BTreeMap<PlaceId, D>, right: &BTreeMap<PlaceId, D>) -> bool
where
    D: AbstractDomain,
{
    left.iter().all(|(place, left_value)| {
        let right_value = right.get(place).cloned().unwrap_or_else(D::bottom);
        left_value.leq(&right_value)
    })
}

fn join_single<D>(target: &mut BTreeMap<PlaceId, D>, place: PlaceId, incoming: &D) -> Changed
where
    D: AbstractDomain,
{
    if incoming.is_bottom() {
        return Changed::No;
    }
    match target.get_mut(&place) {
        Some(current) => current.join_into(incoming),
        None => {
            target.insert(place, incoming.clone());
            Changed::Yes
        }
    }
}

fn widen_place_map<D>(
    output: &mut BTreeMap<PlaceId, D>,
    current: &BTreeMap<PlaceId, D>,
    next: &BTreeMap<PlaceId, D>,
    site: &WidenSite,
    fuel: WidenFuel,
) where
    D: AbstractDomain,
{
    *output = current.clone();
    for (place, next_value) in next {
        let current_value = current.get(place).cloned().unwrap_or_else(D::bottom);
        let widened = current_value.widen(next_value, site.clone(), fuel);
        if widened.is_bottom() {
            output.remove(place);
        } else {
            output.insert(*place, widened);
        }
    }
}

fn push_map_parts<D>(slot: &str, values: &BTreeMap<PlaceId, D>, parts: &mut Vec<String>)
where
    D: AbstractDomain,
{
    for (place, domain) in values {
        for part in domain.stable_digest_parts() {
            parts.push(format!("slot={slot};place={};{part}", place.0));
        }
    }
}

fn nilness_from_literal(literal: &ConstantLiteral) -> Option<NilnessDomain> {
    match literal {
        ConstantLiteral::Null | ConstantLiteral::Undefined | ConstantLiteral::Nil => {
            Some(NilnessDomain::Nil)
        }
        ConstantLiteral::Bool(_) | ConstantLiteral::String(_) | ConstantLiteral::Number(_) => {
            Some(NilnessDomain::NonNil)
        }
        ConstantLiteral::Unknown(_) => None,
    }
}

fn truthiness_from_literal(literal: &ConstantLiteral) -> Option<TruthinessDomain> {
    match literal {
        ConstantLiteral::Null | ConstantLiteral::Undefined | ConstantLiteral::Nil => {
            Some(TruthinessDomain::Falsy)
        }
        ConstantLiteral::Bool(value) => Some(if *value {
            TruthinessDomain::Truthy
        } else {
            TruthinessDomain::Falsy
        }),
        ConstantLiteral::String(value) => Some(if value.is_empty() {
            TruthinessDomain::Falsy
        } else {
            TruthinessDomain::Truthy
        }),
        ConstantLiteral::Number(value) => Some(if is_zero_number_label(value) {
            TruthinessDomain::Falsy
        } else {
            TruthinessDomain::Truthy
        }),
        ConstantLiteral::Unknown(_) => Some(TruthinessDomain::top(TopReason::UnknownValue)),
    }
}

fn is_zero_number_label(value: &str) -> bool {
    matches!(value, "0" | "0.0" | "-0" | "-0.0")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::domains::core::{
        ConstantDomain, ConstantLiteral, InitializednessDomain, NilnessDomain, StringDomain,
        TruthinessDomain,
    };
    use crate::analysis_neutral::domains::lattice::Changed;
    use crate::analysis_neutral::ids::PlaceId;

    fn string_constant(value: &str) -> ConstantDomain {
        ConstantDomain::from_literal(ConstantLiteral::String(value.to_string()))
    }

    #[test]
    fn product_join_into_joins_every_place_slot() {
        let place = PlaceId(1);
        let mut left = ProductState::entry();
        left.core.nilness.insert(place, NilnessDomain::Nil);
        left.core.truthiness.insert(place, TruthinessDomain::Truthy);
        left.core
            .initializedness
            .insert(place, InitializednessDomain::Initialized);

        let mut right = ProductState::entry();
        right.core.nilness.insert(place, NilnessDomain::NonNil);
        right.core.truthiness.insert(place, TruthinessDomain::Falsy);
        right
            .core
            .initializedness
            .insert(place, InitializednessDomain::Uninitialized);

        assert_eq!(left.join_into(&right), Changed::Yes);
        assert_eq!(left.core.nilness[&place], NilnessDomain::MaybeNil);
        assert_eq!(left.core.truthiness[&place], TruthinessDomain::Maybe);
        assert_eq!(
            left.core.initializedness[&place],
            InitializednessDomain::MaybeUninitialized
        );
    }

    #[test]
    fn stable_digest_parts_are_independent_of_insertion_order() {
        let mut first = ProductState::entry();
        first
            .core
            .constants
            .insert(PlaceId(2), string_constant("second"));
        first
            .core
            .constants
            .insert(PlaceId(1), string_constant("first"));

        let mut second = ProductState::entry();
        second
            .core
            .constants
            .insert(PlaceId(1), string_constant("first"));
        second
            .core
            .constants
            .insert(PlaceId(2), string_constant("second"));

        assert_eq!(first.stable_digest_parts(), second.stable_digest_parts());
    }

    #[test]
    fn reduce_value_only_derives_value_facts_from_constants() {
        let nullish = PlaceId(1);
        let route = PlaceId(2);
        let mut state = ProductState::entry();
        state
            .core
            .constants
            .insert(nullish, ConstantDomain::from_literal(ConstantLiteral::Null));
        state
            .core
            .constants
            .insert(route, string_constant("/users/:id"));

        assert_eq!(state.reduce_value_only(4), Changed::Yes);
        assert_eq!(state.core.nilness[&nullish], NilnessDomain::Nil);
        assert_eq!(state.core.truthiness[&nullish], TruthinessDomain::Falsy);
        assert_eq!(state.core.nilness[&route], NilnessDomain::NonNil);
        assert_eq!(state.core.truthiness[&route], TruthinessDomain::Truthy);
        assert_eq!(
            state.core.strings[&route],
            StringDomain::from_literal("/users/:id")
        );
    }
}
