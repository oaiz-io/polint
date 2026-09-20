use std::collections::{BTreeMap, BTreeSet};

use crate::analysis_api::FactFamily;
use crate::analysis_neutral::cfg::facts::{
    BasicBlockKind, CfgEdgeFact, CfgPrecision, CfgStatus, CfgView, ControlDependenceFact,
    DominatorFact, PostDominatorFact, ReachabilityFact,
};
use crate::analysis_neutral::cfg::graph::{CfgGraph, CfgGraphIndex};
use crate::analysis_neutral::cfg::ids::{
    BasicBlockId, CfgFunctionId, ControlDependenceId, DominatorId, PostDominatorId, ReachabilityId,
};
use crate::analysis_neutral::cfg::store::CfgOutput;
use crate::analysis_neutral::stable_key::semantic_stable_key;

/// How much of a dominance relation a run materialises as facts.
///
/// Dominance is the reflexive transitive closure of the immediate-dominator
/// tree, so [`Self::ImmediateOnly`] loses no information — only the
/// materialisation. See [`super::budget`] for why that matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DominanceMaterialization {
    /// Every *(dominated, dominator)* pair.
    Full,
    /// The dominator tree: rows whose `immediate` flag is set.
    ImmediateOnly,
}

pub fn derive_reachability(
    interner: &crate::internal_core::StableKeyInterner,
    output: &CfgOutput,
    view: CfgView,
) -> Vec<ReachabilityFact> {
    let mut facts = Vec::new();
    let mut next_id = 1;
    let index = CfgGraphIndex::new(interner, output);
    for graph in index.graphs(view) {
        let function = graph.function_id();
        let function_key = graph.function_stable_key();
        let reachable = reachable_blocks(&graph);
        for block in graph.block_refs() {
            facts.push(ReachabilityFact {
                id: ReachabilityId(next_id),
                cfg_function: function,
                view,
                block: block.id,
                reachable: reachable.contains(&block.id),
                stable_key: stable_key(
                    interner,
                    FactFamily::CfgReachability,
                    &[
                        ("function", function_key.clone()),
                        ("view", format!("{view:?}")),
                        ("block", interner.resolve(block.stable_key).to_string()),
                    ],
                ),
                status: CfgStatus::Resolved,
                precision: CfgPrecision::ExactLowered,
            });
            next_id += 1;
        }
    }
    facts.sort_by_cached_key(|row| interner.resolve(row.stable_key));
    facts
}

pub fn derive_dominators(
    interner: &crate::internal_core::StableKeyInterner,
    output: &CfgOutput,
    view: CfgView,
    materialization: DominanceMaterialization,
) -> Vec<DominatorFact> {
    let mut facts = Vec::new();
    let mut next_id = 1;
    let index = CfgGraphIndex::new(interner, output);
    for graph in index.graphs(view) {
        let function = graph.function_id();
        let function_key = graph.function_stable_key();
        let block_keys = block_key_map(interner, &graph);
        let Some(entry) = graph.entry_block() else {
            continue;
        };
        let reachable = reachable_blocks(&graph);
        let tree = dom_tree(
            &graph,
            entry,
            Direction::Forward,
            &reachable,
            &BTreeSet::new(),
        );
        for dominated in tree.universe() {
            let immediate = tree.immediate(dominated, false);
            for dominator in tree.dominators(dominated) {
                if materialization == DominanceMaterialization::ImmediateOnly
                    && immediate != Some(dominator)
                {
                    continue;
                }
                facts.push(DominatorFact {
                    id: DominatorId(next_id),
                    cfg_function: function,
                    view,
                    dominator,
                    dominated,
                    immediate: immediate == Some(dominator),
                    stable_key: stable_key(
                        interner,
                        FactFamily::CfgDominator,
                        &[
                            ("function", function_key.clone()),
                            ("view", format!("{view:?}")),
                            ("dominator", stable_block_key(&block_keys, dominator)),
                            ("dominated", stable_block_key(&block_keys, dominated)),
                        ],
                    ),
                    status: CfgStatus::Resolved,
                    precision: CfgPrecision::ExactLowered,
                });
                next_id += 1;
            }
        }
    }
    facts.sort_by_cached_key(|row| interner.resolve(row.stable_key));
    facts
}

pub fn derive_postdominators(
    interner: &crate::internal_core::StableKeyInterner,
    output: &CfgOutput,
    view: CfgView,
    materialization: DominanceMaterialization,
) -> Vec<PostDominatorFact> {
    let mut facts = Vec::new();
    let mut next_id = 1;
    let index = CfgGraphIndex::new(interner, output);
    for graph in index.graphs(view) {
        let function = graph.function_id();
        let function_key = graph.function_stable_key();
        let block_keys = block_key_map(interner, &graph);
        let Some(tree) = reverse_dom_tree(&graph) else {
            continue;
        };
        let virtual_exit = virtual_exit_for(function);
        for postdominated in tree.universe() {
            if postdominated == virtual_exit {
                continue;
            }
            let immediate = tree.immediate(postdominated, false);
            for postdominator in tree.dominators(postdominated) {
                if postdominator == virtual_exit {
                    continue;
                }
                if materialization == DominanceMaterialization::ImmediateOnly
                    && immediate != Some(postdominator)
                {
                    continue;
                }
                facts.push(PostDominatorFact {
                    id: PostDominatorId(next_id),
                    cfg_function: function,
                    view,
                    postdominator,
                    postdominated,
                    immediate: immediate == Some(postdominator),
                    stable_key: stable_key(
                        interner,
                        FactFamily::CfgPostDominator,
                        &[
                            ("function", function_key.clone()),
                            ("view", format!("{view:?}")),
                            (
                                "postdominator",
                                stable_block_key(&block_keys, postdominator),
                            ),
                            (
                                "postdominated",
                                stable_block_key(&block_keys, postdominated),
                            ),
                        ],
                    ),
                    status: CfgStatus::Resolved,
                    precision: CfgPrecision::ExactLowered,
                });
                next_id += 1;
            }
        }
    }
    facts.sort_by_cached_key(|row| interner.resolve(row.stable_key));
    facts
}

pub fn derive_control_dependence(
    interner: &crate::internal_core::StableKeyInterner,
    output: &CfgOutput,
    view: CfgView,
) -> Vec<ControlDependenceFact> {
    let mut facts = Vec::new();
    let mut next_id = 1;
    let index = CfgGraphIndex::new(interner, output);
    for graph in index.graphs(view) {
        let function = graph.function_id();
        let function_key = graph.function_stable_key();
        let tree = reverse_dom_tree(&graph);
        let block_keys = block_key_map(interner, &graph);
        let mut seen = BTreeSet::new();
        // The runner walks the post-dominator tree, which is acyclic — except
        // through the vacuous blocks, where the immediate rule maps the smallest
        // to the second smallest and every other one to the smallest. An edge
        // from a reachable block into such a region makes the walk alternate
        // between two of them forever, so it stops at the first repeat and keeps
        // the facts it produced up to there.
        let mut walked = Vec::new();

        for edge in graph.edge_refs() {
            if edge.from_block == edge.to_block {
                continue;
            }
            if tree
                .as_ref()
                .is_some_and(|tree| tree.dominates(edge.to_block, edge.from_block))
            {
                continue;
            }
            let stop = tree
                .as_ref()
                .and_then(|tree| tree.immediate(edge.from_block, true));
            let mut runner = edge.to_block;
            walked.clear();
            while Some(runner) != stop {
                if walked.contains(&runner) {
                    break;
                }
                walked.push(runner);
                let key = (edge.id, runner);
                if seen.insert(key) {
                    facts.push(control_dependence_fact(
                        interner,
                        next_id,
                        function_key.as_str(),
                        function,
                        view,
                        edge,
                        (runner, stable_block_key(&block_keys, runner)),
                    ));
                    next_id += 1;
                }
                let Some(next) = tree.as_ref().and_then(|tree| tree.immediate(runner, true)) else {
                    break;
                };
                if next == runner {
                    break;
                }
                runner = next;
            }
        }
    }
    facts.sort_by_cached_key(|row| interner.resolve(row.stable_key));
    facts
}

fn control_dependence_fact(
    interner: &crate::internal_core::StableKeyInterner,
    id: u64,
    function_key: &str,
    function: CfgFunctionId,
    view: CfgView,
    edge: &CfgEdgeFact,
    pair: (BasicBlockId, String),
) -> ControlDependenceFact {
    let (controlled_block, controlled_block_key) = pair;
    ControlDependenceFact {
        id: ControlDependenceId(id),
        cfg_function: function,
        view,
        controlling_edge: edge.id,
        controlling_edge_kind: edge.kind,
        controlled_block,
        stable_key: stable_key(
            interner,
            FactFamily::CfgControlDependence,
            &[
                ("function", function_key.to_string()),
                ("view", format!("{view:?}")),
                ("edge", interner.resolve(edge.stable_key).to_string()),
                ("controlled_block", controlled_block_key),
            ],
        ),
        status: edge.status,
        precision: edge.precision,
    }
}

fn block_key_map(
    interner: &crate::internal_core::StableKeyInterner,
    graph: &CfgGraph<'_>,
) -> BTreeMap<BasicBlockId, String> {
    graph
        .block_refs()
        .iter()
        .map(|block| (block.id, interner.resolve(block.stable_key).to_string()))
        .collect()
}

fn stable_block_key(block_keys: &BTreeMap<BasicBlockId, String>, block: BasicBlockId) -> String {
    block_keys
        .get(&block)
        .cloned()
        .unwrap_or_else(|| format!("<missing-block:{}>", block.0))
}

fn reachable_blocks(graph: &CfgGraph<'_>) -> BTreeSet<BasicBlockId> {
    let Some(entry) = graph.entry_block() else {
        return BTreeSet::new();
    };
    let mut seen = BTreeSet::new();
    let mut stack = vec![entry];
    while let Some(block) = stack.pop() {
        if !seen.insert(block) {
            continue;
        }
        stack.extend(graph.successor_blocks(block).iter().rev().copied());
    }
    seen
}

#[derive(Debug, Clone, Copy)]
enum Direction {
    Forward,
    Reverse,
}

/// Immediate dominators for one function graph in the given direction.
///
/// Computed with the Cooper-Harvey-Kennedy iteration over an `idom` array in
/// reverse postorder (<https://www.cs.tufts.edu/~nr/cs257/archive/keith-cooper/dom14.pdf>,
/// the `doms` array formulation), never as a set-per-block relation. Dominance
/// is the reflexive transitive closure of this tree, so the tree *is* the
/// relation; holding it instead of the closure is what lets a bounded run skip
/// the quadratic materialisation entirely ([`super::budget`]).
#[derive(Debug)]
struct DomTree {
    /// Blocks in reverse postorder of the walked direction; index = rpo position.
    order: Vec<BasicBlockId>,
    position: BTreeMap<BasicBlockId, u32>,
    /// `idom[i]`, as an rpo position, for `order[i]`. `None` at the root.
    idom: Vec<Option<u32>>,
    /// Universe members the walk from `root` never reached: the exit-unreachable
    /// blocks in the reverse direction (an infinite loop with no return, a block
    /// whose only successors cycle back). The set-intersection relation this
    /// replaces never updates them — their seeded value is the whole universe and
    /// every reverse-reachable neighbour is itself seeded — so their relation
    /// stays at the universe and is not a tree. Naming them here lets every query
    /// answer for them by that rule, so all three consumers (dominator emission,
    /// post-dominator emission, control dependence) see the relation they saw
    /// when it was materialised.
    vacuous: BTreeSet<BasicBlockId>,
    root: BasicBlockId,
    universe_without_root: BTreeSet<BasicBlockId>,
}

/// An `idom` slot the iteration has not assigned yet. `u32::MAX` is not an rpo
/// position: a function with that many blocks cannot be lowered.
const UNPROCESSED: u32 = u32::MAX;

impl DomTree {
    /// Every block the relation is defined over, ascending, the order the
    /// materialised relation's `BTreeMap` keys came out in.
    fn universe(&self) -> BTreeSet<BasicBlockId> {
        let mut universe = self.universe_without_root.clone();
        universe.insert(self.root);
        universe
    }

    fn is_vacuous(&self, block: BasicBlockId) -> bool {
        self.vacuous.contains(&block)
    }

    /// The immediate dominator, by the rule the materialised relation answered by:
    ///
    /// * a vacuous block's relation is the whole universe, so the first strict
    ///   candidate in ascending order whose own relation contains every other
    ///   candidate is the smallest *other* vacuous block — no block with a real
    ///   tree row can qualify, because its relation cannot contain a vacuous one;
    /// * every other block answers with its tree parent.
    ///
    /// `strip_root` reproduces the caller that removed the virtual exit from the
    /// relation before reading it (control dependence): an exit block's immediate
    /// post-dominator is `None` there and the virtual exit for emission.
    fn immediate(&self, block: BasicBlockId, strip_root: bool) -> Option<BasicBlockId> {
        if self.is_vacuous(block) {
            return self
                .vacuous
                .iter()
                .copied()
                .find(|candidate| *candidate != block);
        }
        let position = *self.position.get(&block)?;
        let parent = self.order[self.idom[position as usize]? as usize];
        if strip_root && parent == self.root {
            return None;
        }
        Some(parent)
    }

    /// The reflexive dominator set of `block`, ascending: the universe for a
    /// vacuous block, the block and its tree ancestors otherwise. A block outside
    /// the universe has no facts and yields nothing.
    fn dominators(&self, block: BasicBlockId) -> impl Iterator<Item = BasicBlockId> + '_ {
        self.dominator_set(block).into_iter()
    }

    fn dominator_set(&self, block: BasicBlockId) -> BTreeSet<BasicBlockId> {
        if self.is_vacuous(block) {
            return self.universe();
        }
        let mut dominators = BTreeSet::new();
        let Some(&position) = self.position.get(&block) else {
            return dominators;
        };
        let mut cursor = position;
        loop {
            dominators.insert(self.order[cursor as usize]);
            match self.idom[cursor as usize] {
                Some(parent) => cursor = parent,
                None => break,
            }
        }
        dominators
    }

    /// `a` is in `dominators(b)`, answered without building either set. A
    /// dominator always precedes its dominated block in reverse postorder, so the
    /// walk up the tree stops as soon as it passes `a`'s position.
    fn dominates(&self, a: BasicBlockId, b: BasicBlockId) -> bool {
        if self.is_vacuous(b) {
            return a == self.root || self.universe_without_root.contains(&a);
        }
        let (Some(&target), Some(&start)) = (self.position.get(&a), self.position.get(&b)) else {
            return false;
        };
        let mut cursor = start;
        while cursor > target {
            let Some(parent) = self.idom[cursor as usize] else {
                return false;
            };
            cursor = parent;
        }
        cursor == target
    }
}

/// The dominator tree of `graph` over `universe`, rooted at `root`.
///
/// `universe` is the block set the relation is defined over; blocks outside it
/// get no facts. Forward: the entry-reachable set, rooted at the entry. Reverse:
/// every block of the function plus the virtual exit, rooted at the virtual
/// exit, with `selected_exits` feeding it.
fn dom_tree(
    graph: &CfgGraph<'_>,
    root: BasicBlockId,
    direction: Direction,
    universe: &BTreeSet<BasicBlockId>,
    selected_exits: &BTreeSet<BasicBlockId>,
) -> DomTree {
    let order = reverse_postorder(graph, root, direction, universe, selected_exits);
    let position = order
        .iter()
        .enumerate()
        .map(|(index, block)| (*block, index as u32))
        .collect::<BTreeMap<_, _>>();

    // Predecessors as rpo positions. A predecessor outside `order` is a vacuous
    // block, whose relation is the whole universe and is therefore the identity
    // of the intersection: dropping it here is what the set version did by
    // intersecting with the universe.
    let mut neighbors = Vec::new();
    let mut predecessors = Vec::with_capacity(order.len());
    for block in &order {
        neighbors.clear();
        walk_predecessors(
            graph,
            *block,
            root,
            direction,
            universe,
            selected_exits,
            &mut neighbors,
        );
        predecessors.push(
            neighbors
                .iter()
                .filter_map(|neighbor| position.get(neighbor).copied())
                .collect::<Vec<_>>(),
        );
    }

    let mut doms = vec![UNPROCESSED; order.len()];
    if !order.is_empty() {
        doms[0] = 0;
    }
    let mut changed = true;
    while changed {
        changed = false;
        for index in 1..order.len() {
            let mut candidate: Option<u32> = None;
            for &predecessor in &predecessors[index] {
                if doms[predecessor as usize] == UNPROCESSED {
                    continue;
                }
                candidate = Some(match candidate {
                    None => predecessor,
                    Some(current) => intersect(&doms, predecessor, current),
                });
            }
            if let Some(candidate) = candidate
                && doms[index] != candidate
            {
                doms[index] = candidate;
                changed = true;
            }
        }
    }

    let idom = doms
        .iter()
        .enumerate()
        .map(|(index, parent)| (index > 0 && *parent != UNPROCESSED).then_some(*parent))
        .collect::<Vec<_>>();
    let mut vacuous = universe.clone();
    for block in &order {
        vacuous.remove(block);
    }
    let mut universe_without_root = universe.clone();
    universe_without_root.remove(&root);

    DomTree {
        order,
        position,
        idom,
        vacuous,
        root,
        universe_without_root,
    }
}

/// The reverse dominator tree of `graph`: post-dominance over every block plus a
/// virtual exit the selected exits feed. `None` when the relation is empty, the
/// two cases the materialised version returned an empty map for.
fn reverse_dom_tree(graph: &CfgGraph<'_>) -> Option<DomTree> {
    let blocks = graph
        .block_refs()
        .iter()
        .map(|block| block.id)
        .collect::<BTreeSet<_>>();
    let exits = selected_exit_blocks(graph);
    if blocks.is_empty() || exits.is_empty() {
        return None;
    }
    let virtual_exit = virtual_exit_for(graph.function_id());
    let mut universe = blocks;
    universe.insert(virtual_exit);
    Some(dom_tree(
        graph,
        virtual_exit,
        Direction::Reverse,
        &universe,
        &exits,
    ))
}

/// The two-finger walk on rpo positions: the nearest common ancestor of two
/// already-processed nodes.
fn intersect(doms: &[u32], mut left: u32, mut right: u32) -> u32 {
    while left != right {
        while left > right {
            left = doms[left as usize];
        }
        while right > left {
            right = doms[right as usize];
        }
    }
    left
}

/// Reverse postorder of the blocks of `universe` reachable from `root` in the
/// walked direction. Members the walk never reaches are the vacuous ones.
fn reverse_postorder(
    graph: &CfgGraph<'_>,
    root: BasicBlockId,
    direction: Direction,
    universe: &BTreeSet<BasicBlockId>,
    selected_exits: &BTreeSet<BasicBlockId>,
) -> Vec<BasicBlockId> {
    if !universe.contains(&root) {
        return Vec::new();
    }
    let mut visited = BTreeSet::from([root]);
    let mut postorder = Vec::new();
    let mut successors = Vec::new();
    walk_successors(
        graph,
        root,
        root,
        direction,
        universe,
        selected_exits,
        &mut successors,
    );
    let mut stack = vec![(root, successors, 0usize)];
    while let Some((block, successors, cursor)) = stack.last_mut() {
        if *cursor >= successors.len() {
            let block = *block;
            postorder.push(block);
            stack.pop();
            continue;
        }
        let next = successors[*cursor];
        *cursor += 1;
        if !visited.insert(next) {
            continue;
        }
        let mut successors = Vec::new();
        walk_successors(
            graph,
            next,
            root,
            direction,
            universe,
            selected_exits,
            &mut successors,
        );
        stack.push((next, successors, 0));
    }
    postorder.reverse();
    postorder
}

/// The nodes `block`'s relation is intersected over: predecessors walking
/// forward, the reversed predecessors walking backward.
fn walk_predecessors(
    graph: &CfgGraph<'_>,
    block: BasicBlockId,
    root: BasicBlockId,
    direction: Direction,
    universe: &BTreeSet<BasicBlockId>,
    selected_exits: &BTreeSet<BasicBlockId>,
    out: &mut Vec<BasicBlockId>,
) {
    match direction {
        Direction::Forward => out.extend(
            graph
                .predecessor_blocks(block)
                .iter()
                .copied()
                .filter(|neighbor| universe.contains(neighbor)),
        ),
        Direction::Reverse => {
            collect_reversed_predecessors(graph, block, root, selected_exits, universe, out);
        }
    }
}

/// The dual of [`walk_predecessors`], the edge the reverse postorder walk
/// follows: `out` holds every block whose predecessor list contains `block`.
fn walk_successors(
    graph: &CfgGraph<'_>,
    block: BasicBlockId,
    root: BasicBlockId,
    direction: Direction,
    universe: &BTreeSet<BasicBlockId>,
    selected_exits: &BTreeSet<BasicBlockId>,
    out: &mut Vec<BasicBlockId>,
) {
    match direction {
        Direction::Forward => out.extend(
            graph
                .successor_blocks(block)
                .iter()
                .copied()
                .filter(|neighbor| universe.contains(neighbor)),
        ),
        Direction::Reverse => {
            if block == root {
                out.extend(
                    selected_exits
                        .iter()
                        .copied()
                        .filter(|neighbor| universe.contains(neighbor)),
                );
                return;
            }
            out.extend(
                graph
                    .predecessor_blocks(block)
                    .iter()
                    .copied()
                    .filter(|neighbor| universe.contains(neighbor)),
            );
        }
    }
}

fn collect_reversed_predecessors(
    graph: &CfgGraph<'_>,
    block: BasicBlockId,
    virtual_exit: BasicBlockId,
    selected_exits: &BTreeSet<BasicBlockId>,
    universe: &BTreeSet<BasicBlockId>,
    out: &mut Vec<BasicBlockId>,
) {
    if block == virtual_exit {
        return;
    }
    out.extend(
        graph
            .successor_blocks(block)
            .iter()
            .copied()
            .filter(|neighbor| universe.contains(neighbor)),
    );
    if selected_exits.contains(&block) {
        out.push(virtual_exit);
    }
}

fn selected_exit_blocks(graph: &CfgGraph<'_>) -> BTreeSet<BasicBlockId> {
    let mut exits = graph
        .block_refs()
        .iter()
        .filter(|block| {
            matches!(
                block.kind,
                BasicBlockKind::ExitNormal | BasicBlockKind::ExitExceptional
            ) || graph.successor_blocks(block.id).is_empty()
        })
        .map(|block| block.id)
        .collect::<BTreeSet<_>>();
    if let Some(exit) = graph.synthetic_exit_block(CfgView::NormalControl) {
        exits.insert(exit);
    }
    exits
}

fn virtual_exit_for(function: CfgFunctionId) -> BasicBlockId {
    BasicBlockId(u64::MAX - function.0)
}

fn stable_key(
    interner: &crate::internal_core::StableKeyInterner,
    family: FactFamily,
    parts: &[(&str, String)],
) -> crate::internal_core::StableKeyId {
    interner.intern(semantic_stable_key(family, parts).into_string())
}

// ---------------------------------------------------------------------------
// The materialised relation, kept for the differential
// ---------------------------------------------------------------------------
//
// What `derive_dominators`, `derive_postdominators` and `derive_control_dependence`
// computed before the tree: one `BTreeSet` of dominators per block, iterated to a
// fixpoint from a seed of the whole universe. It is `O(blocks^2)` in space and
// time per function, and is the relation the tree above is the compressed form of.
//
// It stays here, compiled only under `cfg(test)`, as the oracle the differential
// test compares against: every fixture and every seeded random graph is emitted
// twice, once from each, and the two fact vectors must be equal.

#[cfg(test)]
fn legacy_dominator_relation(
    graph: &CfgGraph<'_>,
    start: BasicBlockId,
    universe: &BTreeSet<BasicBlockId>,
    direction: Direction,
) -> BTreeMap<BasicBlockId, BTreeSet<BasicBlockId>> {
    legacy_dominator_relation_with_extra_exit(graph, start, universe, &BTreeSet::new(), direction)
}

#[cfg(test)]
fn legacy_dominator_relation_with_extra_exit(
    graph: &CfgGraph<'_>,
    start: BasicBlockId,
    universe: &BTreeSet<BasicBlockId>,
    selected_exits: &BTreeSet<BasicBlockId>,
    direction: Direction,
) -> BTreeMap<BasicBlockId, BTreeSet<BasicBlockId>> {
    let mut relation = universe
        .iter()
        .map(|block| {
            let initial = if *block == start {
                BTreeSet::from([start])
            } else {
                universe.clone()
            };
            (*block, initial)
        })
        .collect::<BTreeMap<_, _>>();

    let mut changed = true;
    while changed {
        changed = false;
        for block in universe.iter().copied().filter(|block| *block != start) {
            let mut neighbors = Vec::new();
            match direction {
                Direction::Forward => {
                    neighbors.extend(
                        graph
                            .predecessor_blocks(block)
                            .iter()
                            .copied()
                            .filter(|neighbor| universe.contains(neighbor)),
                    );
                }
                Direction::Reverse => {
                    collect_reversed_predecessors(
                        graph,
                        block,
                        start,
                        selected_exits,
                        universe,
                        &mut neighbors,
                    );
                }
            }

            let mut new_set = if neighbors.is_empty() {
                BTreeSet::new()
            } else {
                legacy_intersect_sets(
                    neighbors
                        .iter()
                        .filter_map(|neighbor| relation.get(neighbor)),
                )
            };
            new_set.insert(block);
            if relation.get(&block) != Some(&new_set) {
                relation.insert(block, new_set);
                changed = true;
            }
        }
    }
    relation
}

#[cfg(test)]
fn legacy_intersect_sets<'a>(
    mut sets: impl Iterator<Item = &'a BTreeSet<BasicBlockId>>,
) -> BTreeSet<BasicBlockId> {
    let Some(first) = sets.next() else {
        return BTreeSet::new();
    };
    sets.fold(first.clone(), |acc, set| {
        acc.intersection(set).copied().collect()
    })
}

#[cfg(test)]
fn legacy_immediate_relation(
    relation: &BTreeMap<BasicBlockId, BTreeSet<BasicBlockId>>,
) -> BTreeMap<BasicBlockId, BasicBlockId> {
    let mut immediate = BTreeMap::new();
    for (node, dominators) in relation {
        let strict = dominators
            .iter()
            .copied()
            .filter(|candidate| candidate != node)
            .collect::<BTreeSet<_>>();
        for candidate in strict.iter().copied() {
            let candidate_dominators = relation.get(&candidate).cloned().unwrap_or_default();
            if strict
                .iter()
                .copied()
                .filter(|other| *other != candidate)
                .all(|other| candidate_dominators.contains(&other))
            {
                immediate.insert(*node, candidate);
                break;
            }
        }
    }
    immediate
}

#[cfg(test)]
fn legacy_postdominator_relation_for_graph(
    graph: &CfgGraph<'_>,
) -> BTreeMap<BasicBlockId, BTreeSet<BasicBlockId>> {
    let blocks = graph
        .block_refs()
        .iter()
        .map(|block| block.id)
        .collect::<BTreeSet<_>>();
    let exits = selected_exit_blocks(graph);
    if blocks.is_empty() || exits.is_empty() {
        return BTreeMap::new();
    }
    let virtual_exit = virtual_exit_for(graph.function_id());
    let mut universe = blocks;
    universe.insert(virtual_exit);
    let mut relation = legacy_dominator_relation_with_extra_exit(
        graph,
        virtual_exit,
        &universe,
        &exits,
        Direction::Reverse,
    );
    relation.remove(&virtual_exit);
    for set in relation.values_mut() {
        set.remove(&virtual_exit);
    }
    relation
}

/// Runner steps the legacy control-dependence copy gives one graph before it
/// declares the walk non-terminating. Every terminating shape in the suite uses
/// a handful; the non-terminating one never stops.
#[cfg(test)]
const LEGACY_RUNNER_FUEL: u32 = 100_000;

#[cfg(test)]
fn legacy_derive_dominators(
    interner: &crate::internal_core::StableKeyInterner,
    output: &CfgOutput,
    view: CfgView,
    materialization: DominanceMaterialization,
) -> Vec<DominatorFact> {
    let mut facts = Vec::new();
    let mut next_id = 1;
    let index = CfgGraphIndex::new(interner, output);
    for graph in index.graphs(view) {
        let function = graph.function_id();
        let function_key = graph.function_stable_key();
        let block_keys = block_key_map(interner, &graph);
        let Some(entry) = graph.entry_block() else {
            continue;
        };
        let reachable = reachable_blocks(&graph);
        let relation = legacy_dominator_relation(&graph, entry, &reachable, Direction::Forward);
        let immediate = legacy_immediate_relation(&relation);
        for (dominated, dominators) in &relation {
            for dominator in dominators {
                if materialization == DominanceMaterialization::ImmediateOnly
                    && immediate.get(dominated) != Some(dominator)
                {
                    continue;
                }
                facts.push(DominatorFact {
                    id: DominatorId(next_id),
                    cfg_function: function,
                    view,
                    dominator: *dominator,
                    dominated: *dominated,
                    immediate: immediate.get(dominated) == Some(dominator),
                    stable_key: stable_key(
                        interner,
                        FactFamily::CfgDominator,
                        &[
                            ("function", function_key.clone()),
                            ("view", format!("{view:?}")),
                            ("dominator", stable_block_key(&block_keys, *dominator)),
                            ("dominated", stable_block_key(&block_keys, *dominated)),
                        ],
                    ),
                    status: CfgStatus::Resolved,
                    precision: CfgPrecision::ExactLowered,
                });
                next_id += 1;
            }
        }
    }
    facts.sort_by_cached_key(|row| interner.resolve(row.stable_key));
    facts
}

#[cfg(test)]
fn legacy_derive_postdominators(
    interner: &crate::internal_core::StableKeyInterner,
    output: &CfgOutput,
    view: CfgView,
    materialization: DominanceMaterialization,
) -> Vec<PostDominatorFact> {
    let mut facts = Vec::new();
    let mut next_id = 1;
    let index = CfgGraphIndex::new(interner, output);
    for graph in index.graphs(view) {
        let function = graph.function_id();
        let function_key = graph.function_stable_key();
        let block_keys = block_key_map(interner, &graph);
        let blocks = graph
            .block_refs()
            .iter()
            .map(|block| block.id)
            .collect::<BTreeSet<_>>();
        if blocks.is_empty() {
            continue;
        }
        let exits = selected_exit_blocks(&graph);
        if exits.is_empty() {
            continue;
        }
        let virtual_exit = virtual_exit_for(function);
        let mut universe = blocks.clone();
        universe.insert(virtual_exit);
        let relation = legacy_dominator_relation_with_extra_exit(
            &graph,
            virtual_exit,
            &universe,
            &exits,
            Direction::Reverse,
        );
        let immediate = legacy_immediate_relation(&relation);
        for (postdominated, postdominators) in &relation {
            if *postdominated == virtual_exit {
                continue;
            }
            for postdominator in postdominators {
                if *postdominator == virtual_exit {
                    continue;
                }
                if materialization == DominanceMaterialization::ImmediateOnly
                    && immediate.get(postdominated) != Some(postdominator)
                {
                    continue;
                }
                facts.push(PostDominatorFact {
                    id: PostDominatorId(next_id),
                    cfg_function: function,
                    view,
                    postdominator: *postdominator,
                    postdominated: *postdominated,
                    immediate: immediate.get(postdominated) == Some(postdominator),
                    stable_key: stable_key(
                        interner,
                        FactFamily::CfgPostDominator,
                        &[
                            ("function", function_key.clone()),
                            ("view", format!("{view:?}")),
                            (
                                "postdominator",
                                stable_block_key(&block_keys, *postdominator),
                            ),
                            (
                                "postdominated",
                                stable_block_key(&block_keys, *postdominated),
                            ),
                        ],
                    ),
                    status: CfgStatus::Resolved,
                    precision: CfgPrecision::ExactLowered,
                });
                next_id += 1;
            }
        }
    }
    facts.sort_by_cached_key(|row| interner.resolve(row.stable_key));
    facts
}

#[cfg(test)]
fn legacy_derive_control_dependence(
    interner: &crate::internal_core::StableKeyInterner,
    output: &CfgOutput,
    view: CfgView,
) -> Option<Vec<ControlDependenceFact>> {
    // The legacy runner loop stops only on `stop`, on `None`, or on a self-loop,
    // so it spins forever between two vacuous blocks. `None` is "this input is
    // one the legacy computation does not finish on", and no byte-identity claim
    // can be made against it.
    let mut fuel = LEGACY_RUNNER_FUEL;
    let mut facts = Vec::new();
    let mut next_id = 1;
    let index = CfgGraphIndex::new(interner, output);
    for graph in index.graphs(view) {
        let function = graph.function_id();
        let function_key = graph.function_stable_key();
        let postdominators = legacy_postdominator_relation_for_graph(&graph);
        let immediate = legacy_immediate_relation(&postdominators);
        let block_keys = block_key_map(interner, &graph);
        let mut seen = BTreeSet::new();

        for edge in graph.edge_refs() {
            if edge.from_block == edge.to_block {
                continue;
            }
            if postdominators
                .get(&edge.from_block)
                .is_some_and(|set| set.contains(&edge.to_block))
            {
                continue;
            }
            let stop = immediate.get(&edge.from_block).copied();
            let mut runner = edge.to_block;
            while Some(runner) != stop {
                fuel = fuel.checked_sub(1)?;
                let key = (edge.id, runner);
                if seen.insert(key) {
                    facts.push(control_dependence_fact(
                        interner,
                        next_id,
                        function_key.as_str(),
                        function,
                        view,
                        edge,
                        (runner, stable_block_key(&block_keys, runner)),
                    ));
                    next_id += 1;
                }
                let Some(next) = immediate.get(&runner).copied() else {
                    break;
                };
                if next == runner {
                    break;
                }
                runner = next;
            }
        }
    }
    facts.sort_by_cached_key(|row| interner.resolve(row.stable_key));
    Some(facts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::cfg::builder::CfgBuilder;
    use crate::analysis_neutral::cfg::facts::{BasicBlockKind, CfgEdgeKind, CfgNodeKind};
    use crate::analysis_neutral::cfg::ids::{CfgEdgeId, CfgNodeId};
    use crate::analysis_neutral::ids::{MirBodyId, MirOpId, PlaceId};
    use crate::analysis_neutral::mir_body::{MirBody, MirStatus};
    use crate::analysis_neutral::mir_op::{AssignMode, MirOperation, MirOperationKind, MirValue};
    use crate::internal_core::{FileId, FunctionId, Language, Span};

    fn span() -> Span {
        Span::new(FileId::from_raw(1), 1, 2, 1, 1, 1, 2)
    }

    fn body(interner: &crate::internal_core::StableKeyInterner) -> MirBody {
        MirBody {
            id: MirBodyId(1),
            language: Language::Go,
            file: FileId::from_raw(1),
            function: FunctionId::from_raw(1),
            package: None,
            module: None,
            owner_stable_key: interner.intern("owner"),
            span: span(),
            stable_key: interner.intern("body:one"),
            status: MirStatus::Resolved,
        }
    }

    fn op(
        interner: &crate::internal_core::StableKeyInterner,
        id: u64,
        ordinal: u32,
    ) -> MirOperation {
        MirOperation {
            id: MirOpId(id),
            body: MirBodyId(1),
            ordinal,
            span: span(),
            kind: MirOperationKind::Assign {
                place: PlaceId(1),
                value: MirValue::Place(PlaceId(2)),
                mode: AssignMode::Overwrite,
            },
            stable_key: interner.intern(format!("op:{ordinal}")),
            status: MirStatus::Resolved,
        }
    }

    fn if_else_graph(interner: &crate::internal_core::StableKeyInterner) -> CfgOutput {
        let mut builder = CfgBuilder::new();
        builder.start_function(interner, &body(interner), false);
        let entry = builder.current_block();
        let condition = builder.start_block(interner, BasicBlockKind::Branch);
        builder.append_operation_node(
            interner,
            Some(&op(interner, 1, 1)),
            CfgNodeKind::Condition,
            Some(span()),
        );
        let then_block = builder.start_block(interner, BasicBlockKind::StraightLine);
        builder.append_operation_node(
            interner,
            Some(&op(interner, 2, 2)),
            CfgNodeKind::Operation,
            Some(span()),
        );
        let else_block = builder.start_block(interner, BasicBlockKind::StraightLine);
        builder.append_operation_node(
            interner,
            Some(&op(interner, 3, 3)),
            CfgNodeKind::Operation,
            Some(span()),
        );
        let join = builder.start_block(interner, BasicBlockKind::Join);
        builder.append_operation_node(
            interner,
            Some(&op(interner, 4, 4)),
            CfgNodeKind::Operation,
            Some(span()),
        );

        builder.add_edge(interner, entry, condition, CfgEdgeKind::Normal);
        builder.add_edge(interner, condition, then_block, CfgEdgeKind::True);
        builder.add_edge(interner, condition, else_block, CfgEdgeKind::False);
        builder.add_edge(interner, then_block, join, CfgEdgeKind::Normal);
        builder.add_edge(interner, else_block, join, CfgEdgeKind::Normal);
        builder.finish_function();
        builder.finish(interner)
    }

    #[test]
    fn reachability_excludes_unreachable_blocks_from_dominators() {
        let interner = crate::internal_core::StableKeyInterner::default();
        let mut builder = CfgBuilder::new();
        builder.start_function(&interner, &body(&interner), false);
        let entry = builder.current_block();
        let reachable = builder.start_block(&interner, BasicBlockKind::StraightLine);
        builder.append_operation_node(
            &interner,
            Some(&op(&interner, 1, 1)),
            CfgNodeKind::Operation,
            Some(span()),
        );
        let unreachable = builder.start_block(&interner, BasicBlockKind::Unreachable);
        builder.append_operation_node(
            &interner,
            Some(&op(&interner, 2, 2)),
            CfgNodeKind::Operation,
            Some(span()),
        );
        builder.mark_unreachable(unreachable);
        builder.add_edge(&interner, entry, reachable, CfgEdgeKind::Normal);
        builder.finish_function();
        let output = builder.finish(&interner);

        let reachability = derive_reachability(&interner, &output, CfgView::NormalControl);
        assert!(
            reachability
                .iter()
                .any(|fact| fact.block == reachable && fact.reachable)
        );
        assert!(
            reachability
                .iter()
                .any(|fact| fact.block == unreachable && !fact.reachable)
        );

        let dominators = derive_dominators(
            &interner,
            &output,
            CfgView::NormalControl,
            DominanceMaterialization::Full,
        );
        assert!(
            !dominators
                .iter()
                .any(|fact| fact.dominated == unreachable || fact.dominator == unreachable)
        );
    }

    #[test]
    fn dominators_are_deterministic_for_branch_join_graphs() {
        let interner = crate::internal_core::StableKeyInterner::default();
        let output = if_else_graph(&interner);
        let first = derive_dominators(
            &interner,
            &output,
            CfgView::NormalControl,
            DominanceMaterialization::Full,
        );
        let second = derive_dominators(
            &interner,
            &output,
            CfgView::NormalControl,
            DominanceMaterialization::Full,
        );
        assert_eq!(first, second);
        assert!(first.iter().any(|fact| fact.immediate));
    }

    #[test]
    fn postdominators_handle_multiple_returns_and_unified_exit() {
        let interner = crate::internal_core::StableKeyInterner::default();
        let mut builder = CfgBuilder::new();
        builder.start_function(&interner, &body(&interner), false);
        let entry = builder.current_block();
        let first_return = builder.start_block(&interner, BasicBlockKind::StraightLine);
        builder.append_operation_node(
            &interner,
            Some(&op(&interner, 1, 1)),
            CfgNodeKind::Return,
            Some(span()),
        );
        let second_return = builder.start_block(&interner, BasicBlockKind::StraightLine);
        builder.append_operation_node(
            &interner,
            Some(&op(&interner, 2, 2)),
            CfgNodeKind::Return,
            Some(span()),
        );
        let exit = builder.normal_exit_block();
        builder.add_edge(&interner, entry, first_return, CfgEdgeKind::True);
        builder.add_edge(&interner, entry, second_return, CfgEdgeKind::False);
        builder.add_edge(&interner, first_return, exit, CfgEdgeKind::Return);
        builder.add_edge(&interner, second_return, exit, CfgEdgeKind::Return);
        builder.finish_function();
        let output = builder.finish(&interner);

        let postdominators = derive_postdominators(
            &interner,
            &output,
            CfgView::NormalControl,
            DominanceMaterialization::Full,
        );
        assert!(postdominators.iter().any(|fact| fact.immediate));
        assert_eq!(
            postdominators
                .iter()
                .filter(|fact| fact.postdominated == first_return && fact.immediate)
                .count(),
            1
        );
    }

    #[test]
    fn control_dependence_records_branch_edges_without_unreachable_tails() {
        let interner = crate::internal_core::StableKeyInterner::default();
        let output = if_else_graph(&interner);
        let dependence = derive_control_dependence(&interner, &output, CfgView::NormalControl);
        assert!(
            dependence
                .iter()
                .any(|fact| fact.controlling_edge_kind == CfgEdgeKind::True)
        );
        assert!(
            dependence
                .iter()
                .any(|fact| fact.controlling_edge_kind == CfgEdgeKind::False)
        );
        assert!(
            dependence
                .iter()
                .all(|fact| fact.view == CfgView::NormalControl)
        );
    }

    #[test]
    fn loop_control_dependence_deduplicates_structurally_identical_rows() {
        let interner = crate::internal_core::StableKeyInterner::default();
        let mut builder = CfgBuilder::new();
        builder.start_function(&interner, &body(&interner), false);
        let entry = builder.current_block();
        let header = builder.start_block(&interner, BasicBlockKind::LoopHeader);
        builder.append_operation_node(
            &interner,
            Some(&op(&interner, 1, 1)),
            CfgNodeKind::Condition,
            Some(span()),
        );
        let body_block = builder.start_block(&interner, BasicBlockKind::LoopBody);
        builder.append_operation_node(
            &interner,
            Some(&op(&interner, 2, 2)),
            CfgNodeKind::Operation,
            Some(span()),
        );
        let exit_block = builder.start_block(&interner, BasicBlockKind::Join);
        builder.append_operation_node(
            &interner,
            Some(&op(&interner, 3, 3)),
            CfgNodeKind::Operation,
            Some(span()),
        );
        builder.add_edge(&interner, entry, header, CfgEdgeKind::LoopEnter);
        builder.add_edge(&interner, header, body_block, CfgEdgeKind::True);
        builder.add_edge(&interner, header, exit_block, CfgEdgeKind::LoopExit);
        builder.add_edge(&interner, body_block, header, CfgEdgeKind::LoopBack);
        builder.finish_function();
        let output = builder.finish(&interner);

        let dependence = derive_control_dependence(&interner, &output, CfgView::NormalControl);
        let keys = dependence
            .iter()
            .map(|fact| interner.resolve(fact.stable_key))
            .collect::<BTreeSet<_>>();
        assert_eq!(keys.len(), dependence.len());
        assert!(
            dependence
                .iter()
                .any(|fact| fact.controlling_edge_kind == CfgEdgeKind::True)
        );
    }

    #[test]
    fn derived_stable_keys_do_not_depend_on_dense_ids() {
        let interner = crate::internal_core::StableKeyInterner::default();
        let output = if_else_graph(&interner);
        let shifted = shift_dense_ids(output.clone());

        assert_eq!(
            stable_keys(
                &interner,
                derive_reachability(&interner, &output, CfgView::NormalControl)
            ),
            stable_keys(
                &interner,
                derive_reachability(&interner, &shifted, CfgView::NormalControl)
            )
        );
        assert_eq!(
            stable_keys(
                &interner,
                derive_dominators(
                    &interner,
                    &output,
                    CfgView::NormalControl,
                    DominanceMaterialization::Full
                )
            ),
            stable_keys(
                &interner,
                derive_dominators(
                    &interner,
                    &shifted,
                    CfgView::NormalControl,
                    DominanceMaterialization::Full
                )
            )
        );
        assert_eq!(
            stable_keys(
                &interner,
                derive_postdominators(
                    &interner,
                    &output,
                    CfgView::NormalControl,
                    DominanceMaterialization::Full
                )
            ),
            stable_keys(
                &interner,
                derive_postdominators(
                    &interner,
                    &shifted,
                    CfgView::NormalControl,
                    DominanceMaterialization::Full
                )
            )
        );
        assert_eq!(
            stable_keys(
                &interner,
                derive_control_dependence(&interner, &output, CfgView::NormalControl)
            ),
            stable_keys(
                &interner,
                derive_control_dependence(&interner, &shifted, CfgView::NormalControl)
            )
        );
    }

    // -----------------------------------------------------------------------
    // The differential: the tree against the relation it replaces
    // -----------------------------------------------------------------------

    /// A seeded linear congruential generator, so a failing graph is reproducible
    /// from the seed printed in the assertion.
    struct Seeded(u64);

    impl Seeded {
        fn new(seed: u64) -> Self {
            Self(
                seed.wrapping_mul(0x9E37_79B9_7F4A_7C15)
                    .wrapping_add(0x1234_5678),
            )
        }

        fn draw(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.0 >> 33
        }

        fn below(&mut self, bound: usize) -> usize {
            (self.draw() % bound as u64) as usize
        }
    }

    fn edge_kind(roll: usize) -> CfgEdgeKind {
        match roll % 5 {
            0 => CfgEdgeKind::True,
            1 => CfgEdgeKind::False,
            2 => CfgEdgeKind::LoopBack,
            3 => CfgEdgeKind::LoopExit,
            _ => CfgEdgeKind::Normal,
        }
    }

    fn block_kind(roll: usize) -> BasicBlockKind {
        match roll % 4 {
            0 => BasicBlockKind::Branch,
            1 => BasicBlockKind::LoopHeader,
            2 => BasicBlockKind::Join,
            _ => BasicBlockKind::StraightLine,
        }
    }

    /// A small random graph: reducible or irreducible, with blocks that may be
    /// forward-unreachable (no path from the entry) or exit-unreachable (no path
    /// to a return), which are the two shapes the universes differ on.
    fn random_graph(interner: &crate::internal_core::StableKeyInterner, seed: u64) -> CfgOutput {
        let mut rng = Seeded::new(seed);
        let mut builder = CfgBuilder::new();
        builder.start_function(interner, &body(interner), false);
        let entry = builder.current_block();
        let count = 3 + rng.below(6);
        let mut blocks = Vec::new();
        for index in 0..count {
            let block = builder.start_block(interner, block_kind(rng.below(4)));
            builder.append_operation_node(
                interner,
                Some(&op(interner, index as u64 + 1, index as u32 + 1)),
                CfgNodeKind::Operation,
                Some(span()),
            );
            blocks.push(block);
        }
        let exit = builder.normal_exit_block();

        let mut drawn = BTreeSet::new();
        let mut connect = |builder: &mut CfgBuilder, from: BasicBlockId, to: BasicBlockId, roll| {
            if from != to && drawn.insert((from, to)) {
                builder.add_edge(interner, from, to, edge_kind(roll));
            }
        };
        connect(&mut builder, entry, blocks[0], 4);
        // Some graphs never reach the entry's successor list past the first
        // block, which is how the forward-unreachable shapes appear.
        if rng.below(3) == 0 {
            connect(&mut builder, entry, blocks[rng.below(count)], 0);
        }
        for index in 0..count {
            // A spine, so most graphs carry a deep tree rather than one
            // reachable block and a pile of unreachable ones.
            if index + 1 < count && rng.below(4) > 0 {
                connect(&mut builder, blocks[index], blocks[index + 1], 4);
            }
            for _ in 0..1 + rng.below(2) {
                let roll = rng.below(10);
                let target = if roll < 3 {
                    exit
                } else {
                    blocks[rng.below(count)]
                };
                connect(&mut builder, blocks[index], target, roll);
            }
        }
        builder.finish_function();
        builder.finish(interner)
    }

    /// The exit-unreachable shape stated on its own: a reachable branch into a
    /// region of `region` blocks that cycle among themselves and never return.
    fn graph_with_exit_unreachable_region(
        interner: &crate::internal_core::StableKeyInterner,
        region: usize,
    ) -> CfgOutput {
        let mut builder = CfgBuilder::new();
        builder.start_function(interner, &body(interner), false);
        let entry = builder.current_block();
        let branch = builder.start_block(interner, BasicBlockKind::Branch);
        builder.append_operation_node(
            interner,
            Some(&op(interner, 1, 1)),
            CfgNodeKind::Condition,
            Some(span()),
        );
        let returning = builder.start_block(interner, BasicBlockKind::StraightLine);
        builder.append_operation_node(
            interner,
            Some(&op(interner, 2, 2)),
            CfgNodeKind::Return,
            Some(span()),
        );
        let mut stuck = Vec::new();
        for index in 0..region {
            let block = builder.start_block(interner, BasicBlockKind::LoopHeader);
            builder.append_operation_node(
                interner,
                Some(&op(interner, index as u64 + 3, index as u32 + 3)),
                CfgNodeKind::Operation,
                Some(span()),
            );
            stuck.push(block);
        }
        let exit = builder.normal_exit_block();
        builder.add_edge(interner, entry, branch, CfgEdgeKind::Normal);
        builder.add_edge(interner, branch, returning, CfgEdgeKind::True);
        builder.add_edge(interner, returning, exit, CfgEdgeKind::Return);
        builder.add_edge(interner, branch, stuck[0], CfgEdgeKind::False);
        for index in 0..region {
            // A cycle with no way out: every successor is inside the region, so
            // no member is a selected exit and none of them reaches the virtual
            // one.
            builder.add_edge(
                interner,
                stuck[index],
                stuck[(index + 1) % region],
                CfgEdgeKind::LoopBack,
            );
        }
        builder.finish_function();
        builder.finish(interner)
    }

    /// Emits both dominance families under both materialisations and the control
    /// dependence family, from the tree and from the relation, and asserts they
    /// are equal row for row. Returns whether the relation's runner loop
    /// terminated at all.
    #[must_use]
    fn tree_matches_relation(
        interner: &crate::internal_core::StableKeyInterner,
        output: &CfgOutput,
        label: &str,
    ) -> bool {
        for (mode, materialization) in [
            ("full", DominanceMaterialization::Full),
            ("immediate-only", DominanceMaterialization::ImmediateOnly),
        ] {
            assert_eq!(
                derive_dominators(interner, output, CfgView::NormalControl, materialization),
                legacy_derive_dominators(interner, output, CfgView::NormalControl, materialization),
                "dominator rows differ for {label} under {mode}"
            );
            assert_eq!(
                derive_postdominators(interner, output, CfgView::NormalControl, materialization),
                legacy_derive_postdominators(
                    interner,
                    output,
                    CfgView::NormalControl,
                    materialization
                ),
                "post-dominator rows differ for {label} under {mode}"
            );
        }
        let ported = derive_control_dependence(interner, output, CfgView::NormalControl);
        match legacy_derive_control_dependence(interner, output, CfgView::NormalControl) {
            Some(relation_rows) => {
                assert_eq!(
                    ported, relation_rows,
                    "control dependence differs for {label}"
                );
                true
            }
            None => false,
        }
    }

    /// The shapes the seeded sample has to keep hitting for the differential to
    /// mean anything. A generator change that stops producing one of them fails
    /// here rather than passing on trivial graphs.
    #[derive(Default)]
    struct SampleShapes {
        vacuous: usize,
        forward_unreachable: usize,
        irreducible: usize,
        diverged: usize,
    }

    fn record_shapes(
        interner: &crate::internal_core::StableKeyInterner,
        output: &CfgOutput,
        shapes: &mut SampleShapes,
    ) {
        let index = CfgGraphIndex::new(interner, output);
        let graph = index.graphs(CfgView::NormalControl).remove(0);
        let reverse = reverse_dom_tree(&graph).expect("the builder always makes an exit block");
        if graph
            .block_refs()
            .iter()
            .any(|block| reverse.is_vacuous(block.id))
        {
            shapes.vacuous += 1;
        }
        let reachable = reachable_blocks(&graph);
        if reachable.len() < graph.block_refs().len() {
            shapes.forward_unreachable += 1;
        }
        let entry = graph
            .entry_block()
            .expect("the builder always makes an entry");
        let forward = dom_tree(
            &graph,
            entry,
            Direction::Forward,
            &reachable,
            &BTreeSet::new(),
        );
        // A retreating edge whose target does not dominate its source: the
        // textbook irreducibility witness.
        if graph.edge_refs().iter().any(|edge| {
            reachable.contains(&edge.from_block)
                && reachable.contains(&edge.to_block)
                && forward.position.get(&edge.to_block) < forward.position.get(&edge.from_block)
                && !forward.dominates(edge.to_block, edge.from_block)
        }) {
            shapes.irreducible += 1;
        }
    }

    #[test]
    fn the_dominator_tree_reproduces_the_relation_on_seeded_random_graphs() {
        let interner = crate::internal_core::StableKeyInterner::default();
        let mut shapes = SampleShapes::default();
        for seed in 0..400u64 {
            let output = random_graph(&interner, seed);
            record_shapes(&interner, &output, &mut shapes);
            if !tree_matches_relation(&interner, &output, &format!("random seed {seed}")) {
                shapes.diverged += 1;
            }
        }
        assert!(
            shapes.vacuous >= 20,
            "too few exit-unreachable samples: {}",
            shapes.vacuous
        );
        assert!(
            shapes.forward_unreachable >= 20,
            "too few forward-unreachable samples: {}",
            shapes.forward_unreachable
        );
        assert!(
            shapes.irreducible >= 20,
            "too few irreducible samples: {}",
            shapes.irreducible
        );
        // The relation's runner loop does not terminate on every shape, so some
        // of the sample carries no control-dependence comparison. Most of it
        // has to, or the differential is not testing what it claims to.
        assert!(
            shapes.diverged < 100,
            "the relation's control-dependence loop diverged on {} of 400 graphs",
            shapes.diverged
        );
    }

    #[test]
    fn the_dominator_tree_reproduces_the_relation_on_the_cfg_fixtures() {
        let interner = crate::internal_core::StableKeyInterner::default();
        assert!(tree_matches_relation(
            &interner,
            &if_else_graph(&interner),
            "if/else"
        ));
        assert!(tree_matches_relation(
            &interner,
            &multiple_returns_graph(&interner),
            "multiple returns"
        ));
        assert!(tree_matches_relation(
            &interner,
            &forward_unreachable_graph(&interner),
            "forward-unreachable"
        ));
    }

    #[test]
    fn forward_unreachable_blocks_have_no_dominators_and_do_have_postdominators() {
        let interner = crate::internal_core::StableKeyInterner::default();
        let (output, unreachable) = forward_unreachable_graph_with_block(&interner);

        let dominators = derive_dominators(
            &interner,
            &output,
            CfgView::NormalControl,
            DominanceMaterialization::Full,
        );
        assert!(
            !dominators
                .iter()
                .any(|fact| fact.dominated == unreachable || fact.dominator == unreachable),
            "a forward-unreachable block is outside the forward universe"
        );

        let postdominators = derive_postdominators(
            &interner,
            &output,
            CfgView::NormalControl,
            DominanceMaterialization::Full,
        );
        assert!(
            postdominators
                .iter()
                .any(|fact| fact.postdominated == unreachable),
            "the reverse universe is every block, so it does get post-dominator rows"
        );
        assert!(tree_matches_relation(
            &interner,
            &output,
            "forward-unreachable"
        ));
    }

    #[test]
    fn exit_unreachable_blocks_keep_their_vacuous_postdominator_rows() {
        let interner = crate::internal_core::StableKeyInterner::default();
        let output = graph_with_exit_unreachable_region(&interner, 2);
        let index = CfgGraphIndex::new(&interner, &output);
        let graph = index.graphs(CfgView::NormalControl).remove(0);
        let tree = reverse_dom_tree(&graph).expect("the function has a selected exit");
        let vacuous = graph
            .block_refs()
            .iter()
            .map(|block| block.id)
            .filter(|block| tree.is_vacuous(*block))
            .collect::<Vec<_>>();
        assert_eq!(
            vacuous.len(),
            2,
            "the two blocks of the stuck cycle are the exit-unreachable ones"
        );

        // Their relation is the whole universe, so `Full` emits one row per block
        // of the function (the virtual exit is stripped at emission), and the
        // `immediate` flag falls on the smallest other vacuous block.
        let blocks = graph.block_refs().len();
        let postdominators = derive_postdominators(
            &interner,
            &output,
            CfgView::NormalControl,
            DominanceMaterialization::Full,
        );
        for block in &vacuous {
            let rows = postdominators
                .iter()
                .filter(|fact| fact.postdominated == *block)
                .collect::<Vec<_>>();
            assert_eq!(
                rows.len(),
                blocks,
                "a vacuous block's relation is the universe"
            );
            let immediate = rows
                .iter()
                .filter(|fact| fact.immediate)
                .map(|fact| fact.postdominator)
                .collect::<Vec<_>>();
            assert_eq!(
                immediate,
                vec![
                    *vacuous
                        .iter()
                        .find(|other| *other != block)
                        .expect("a pair")
                ]
            );
        }
        assert_eq!(tree.immediate(vacuous[0], true), Some(vacuous[1]));
        assert_eq!(tree.immediate(vacuous[1], true), Some(vacuous[0]));
    }

    #[test]
    fn control_dependence_into_a_vacuous_region_stops_where_the_relation_loop_does_not() {
        let interner = crate::internal_core::StableKeyInterner::default();
        let output = graph_with_exit_unreachable_region(&interner, 2);

        // The relation's runner alternates between the two vacuous blocks for
        // ever: `immediate` maps the smaller to the larger and the larger back to
        // the smaller, and neither is the `stop` of an edge whose source is a
        // reachable block. There is no byte-identity claim to make here.
        assert!(
            legacy_derive_control_dependence(&interner, &output, CfgView::NormalControl).is_none(),
            "the relation's loop is expected not to terminate on this shape"
        );

        let facts = derive_control_dependence(&interner, &output, CfgView::NormalControl);
        assert!(
            facts
                .iter()
                .any(|fact| fact.controlling_edge_kind == CfgEdgeKind::False),
            "the branch into the region is still control-dependence-bearing"
        );
        // Bounded difference, pinned: the walk stops at the first repeated
        // runner, so each controlling edge contributes the region's two blocks
        // and nothing further.
        let by_edge = facts.iter().fold(BTreeMap::new(), |mut acc, fact| {
            *acc.entry(fact.controlling_edge).or_insert(0usize) += 1;
            acc
        });
        let widest = by_edge.values().copied().max().unwrap_or_default();
        assert!(
            widest <= 2,
            "a runner that repeats stops; one edge produced {widest} rows"
        );
    }

    #[test]
    fn the_dominance_bound_moves_the_rows_and_nothing_else() {
        let interner = crate::internal_core::StableKeyInterner::default();
        for output in [
            if_else_graph(&interner),
            multiple_returns_graph(&interner),
            graph_with_exit_unreachable_region(&interner, 3),
        ] {
            let full = derive_dominators(
                &interner,
                &output,
                CfgView::NormalControl,
                DominanceMaterialization::Full,
            );
            let tree_only = derive_dominators(
                &interner,
                &output,
                CfgView::NormalControl,
                DominanceMaterialization::ImmediateOnly,
            );
            let post_full = derive_postdominators(
                &interner,
                &output,
                CfgView::NormalControl,
                DominanceMaterialization::Full,
            );
            let post_tree_only = derive_postdominators(
                &interner,
                &output,
                CfgView::NormalControl,
                DominanceMaterialization::ImmediateOnly,
            );

            // The bound changes which rows are emitted ...
            assert!(full.len() > tree_only.len());
            assert!(post_full.len() > post_tree_only.len());
            // ... and nothing about the tree itself: the immediate rows of the
            // full relation are exactly the bounded emission, modulo the dense
            // ids a shorter run hands out.
            assert_eq!(
                full.iter()
                    .filter(|fact| fact.immediate)
                    .map(|fact| (fact.dominator, fact.dominated, fact.stable_key))
                    .collect::<Vec<_>>(),
                tree_only
                    .iter()
                    .map(|fact| (fact.dominator, fact.dominated, fact.stable_key))
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                post_full
                    .iter()
                    .filter(|fact| fact.immediate)
                    .map(|fact| (fact.postdominator, fact.postdominated, fact.stable_key))
                    .collect::<Vec<_>>(),
                post_tree_only
                    .iter()
                    .map(|fact| (fact.postdominator, fact.postdominated, fact.stable_key))
                    .collect::<Vec<_>>()
            );
            // Control dependence reads the unbounded tree in both modes, so its
            // rows do not depend on the bound at all.
            assert_eq!(
                derive_control_dependence(&interner, &output, CfgView::NormalControl),
                derive_control_dependence(&interner, &output, CfgView::NormalControl)
            );
        }
    }

    #[test]
    fn a_function_without_a_selected_exit_emits_no_postdominator_rows() {
        let interner = crate::internal_core::StableKeyInterner::default();
        let mut builder = CfgBuilder::new();
        builder.start_function(&interner, &body(&interner), false);
        let entry = builder.current_block();
        let first = builder.start_block(&interner, BasicBlockKind::LoopHeader);
        builder.append_operation_node(
            &interner,
            Some(&op(&interner, 1, 1)),
            CfgNodeKind::Operation,
            Some(span()),
        );
        let second = builder.start_block(&interner, BasicBlockKind::LoopBody);
        builder.append_operation_node(
            &interner,
            Some(&op(&interner, 2, 2)),
            CfgNodeKind::Operation,
            Some(span()),
        );
        let synthetic_exit = builder.normal_exit_block();
        builder.add_edge(&interner, entry, first, CfgEdgeKind::Normal);
        builder.add_edge(&interner, first, second, CfgEdgeKind::True);
        builder.add_edge(&interner, second, first, CfgEdgeKind::LoopBack);
        builder.finish_function();
        let mut output = builder.finish(&interner);
        // A function that never returns and whose synthetic exit was never
        // lowered: no block is a selected exit, so the reverse universe has no
        // root and the relation is empty. The forward direction is unaffected.
        output.blocks.retain(|block| block.id != synthetic_exit);

        assert!(
            derive_postdominators(
                &interner,
                &output,
                CfgView::NormalControl,
                DominanceMaterialization::Full
            )
            .is_empty()
        );
        assert!(
            !derive_dominators(
                &interner,
                &output,
                CfgView::NormalControl,
                DominanceMaterialization::Full
            )
            .is_empty()
        );
        // With no relation to consult, every non-self edge controls its target
        // and the runner stops immediately.
        assert_eq!(
            derive_control_dependence(&interner, &output, CfgView::NormalControl).len(),
            3
        );
        assert!(tree_matches_relation(
            &interner,
            &output,
            "no selected exit"
        ));
    }

    fn multiple_returns_graph(interner: &crate::internal_core::StableKeyInterner) -> CfgOutput {
        let mut builder = CfgBuilder::new();
        builder.start_function(interner, &body(interner), false);
        let entry = builder.current_block();
        let first_return = builder.start_block(interner, BasicBlockKind::StraightLine);
        builder.append_operation_node(
            interner,
            Some(&op(interner, 1, 1)),
            CfgNodeKind::Return,
            Some(span()),
        );
        let second_return = builder.start_block(interner, BasicBlockKind::StraightLine);
        builder.append_operation_node(
            interner,
            Some(&op(interner, 2, 2)),
            CfgNodeKind::Return,
            Some(span()),
        );
        let exit = builder.normal_exit_block();
        builder.add_edge(interner, entry, first_return, CfgEdgeKind::True);
        builder.add_edge(interner, entry, second_return, CfgEdgeKind::False);
        builder.add_edge(interner, first_return, exit, CfgEdgeKind::Return);
        builder.add_edge(interner, second_return, exit, CfgEdgeKind::Return);
        builder.finish_function();
        builder.finish(interner)
    }

    fn forward_unreachable_graph(interner: &crate::internal_core::StableKeyInterner) -> CfgOutput {
        forward_unreachable_graph_with_block(interner).0
    }

    fn forward_unreachable_graph_with_block(
        interner: &crate::internal_core::StableKeyInterner,
    ) -> (CfgOutput, BasicBlockId) {
        let mut builder = CfgBuilder::new();
        builder.start_function(interner, &body(interner), false);
        let entry = builder.current_block();
        let reachable = builder.start_block(interner, BasicBlockKind::StraightLine);
        builder.append_operation_node(
            interner,
            Some(&op(interner, 1, 1)),
            CfgNodeKind::Return,
            Some(span()),
        );
        let unreachable = builder.start_block(interner, BasicBlockKind::Unreachable);
        builder.append_operation_node(
            interner,
            Some(&op(interner, 2, 2)),
            CfgNodeKind::Operation,
            Some(span()),
        );
        let exit = builder.normal_exit_block();
        builder.add_edge(interner, entry, reachable, CfgEdgeKind::Normal);
        builder.add_edge(interner, reachable, exit, CfgEdgeKind::Return);
        builder.add_edge(interner, unreachable, exit, CfgEdgeKind::Return);
        builder.finish_function();
        (builder.finish(interner), unreachable)
    }

    trait StableKeyRow {
        fn stable_key(&self) -> crate::internal_core::StableKeyId;
    }

    impl StableKeyRow for ReachabilityFact {
        fn stable_key(&self) -> crate::internal_core::StableKeyId {
            self.stable_key
        }
    }

    impl StableKeyRow for DominatorFact {
        fn stable_key(&self) -> crate::internal_core::StableKeyId {
            self.stable_key
        }
    }

    impl StableKeyRow for PostDominatorFact {
        fn stable_key(&self) -> crate::internal_core::StableKeyId {
            self.stable_key
        }
    }

    impl StableKeyRow for ControlDependenceFact {
        fn stable_key(&self) -> crate::internal_core::StableKeyId {
            self.stable_key
        }
    }

    fn stable_keys(
        interner: &crate::internal_core::StableKeyInterner,
        rows: impl IntoIterator<Item = impl StableKeyRow>,
    ) -> Vec<String> {
        rows.into_iter()
            .map(|row| interner.resolve(row.stable_key()).to_string())
            .collect()
    }

    fn shift_dense_ids(mut output: CfgOutput) -> CfgOutput {
        for function in &mut output.functions {
            function.id = CfgFunctionId(function.id.0 + 100);
            function.entry_node = CfgNodeId(function.entry_node.0 + 1_000);
            function.normal_exit_node = CfgNodeId(function.normal_exit_node.0 + 1_000);
            function.exceptional_exit_node = function
                .exceptional_exit_node
                .map(|node| CfgNodeId(node.0 + 1_000));
        }
        for node in &mut output.nodes {
            node.id = CfgNodeId(node.id.0 + 1_000);
            node.cfg_function = CfgFunctionId(node.cfg_function.0 + 100);
            node.block = BasicBlockId(node.block.0 + 2_000);
        }
        for block in &mut output.blocks {
            block.id = BasicBlockId(block.id.0 + 2_000);
            block.cfg_function = CfgFunctionId(block.cfg_function.0 + 100);
            block.first_node = block.first_node.map(|node| CfgNodeId(node.0 + 1_000));
            block.last_node = block.last_node.map(|node| CfgNodeId(node.0 + 1_000));
        }
        for edge in &mut output.edges {
            edge.id = CfgEdgeId(edge.id.0 + 3_000);
            edge.cfg_function = CfgFunctionId(edge.cfg_function.0 + 100);
            edge.from = CfgNodeId(edge.from.0 + 1_000);
            edge.to = CfgNodeId(edge.to.0 + 1_000);
            edge.from_block = BasicBlockId(edge.from_block.0 + 2_000);
            edge.to_block = BasicBlockId(edge.to_block.0 + 2_000);
        }
        output
    }
}
