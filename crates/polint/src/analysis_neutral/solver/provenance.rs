//! Derived-edge provenance (GRAPH-04, D-08/D-09/D-10).
//!
//! Every solver-derived edge carries a [`DerivedEdgeProvenance`] recording the
//! three roadmap-named fields:
//!
//! 1. the **contributing fact IDs**, TOTALLY ORDERED BY RESOLVED STABLE TEXT (the
//!    dedup total-order rule). Provenance references EXISTING stable identities by
//!    their interned [`StableKeyId`] (composition over duplication — it does not mint a parallel
//!    identity space); the total order is the `stable_key_from_parts`
//!    length-prefixed, label-sorted recipe's resolved text, so provenance is itself byte-stable and
//!    insensitive to the order contributing facts are discovered in.
//! 2. the producing **constraint kind**, sourced from
//!    [`crate::analysis_neutral::semantic_graph::constraints::ConstraintKind::as_str`] — a
//!    stable snake_case label reused wholesale (no parallel enum is minted).
//! 3. the **solver step** — the monotonic `u64` worklist step counter the
//!    [`super::engine::SolverEngine`] maintains.
//!
//! Provenance is SOUND and LOAD-BEARING per edge FACT, not decorative: it records a
//! deterministic WITNESSING derivation, and the derived-edge fact's `stable_key`
//! embeds that witness (the totally-ordered contributing keys + constraint kind). The
//! deletion property test (D-09, in [`super::engine`]::`tests`) proves that removing
//! any single witness fact means THAT derived-edge fact (by stable key) is not
//! reproduced. On a multi-path graph the underlying `source -> target` value-flow may
//! still hold via an alternate witness under a DIFFERENT edge identity — provenance
//! describes a witness, not a global dependency of the (source, target) pair (see the
//! diamond test in [`super::engine`]).
//!
//! All types are `pub` (D-16); nothing here reaches the public SDK surface.
//! Wire/cache/test byte stability must go through [`DerivedEdgeProvenance::stable_payload`]
//! (resolved text), never raw numeric interner ids.

use serde::Serialize;

use crate::analysis_api::{FactFamily, stable_key_from_parts};
use crate::analysis_neutral::semantic_graph::constraints::ConstraintKind;
use crate::internal_core::{StableKeyId, StableKeyInterner};

/// A reference to one EXISTING upstream fact that contributed to deriving an edge.
///
/// Provenance references existing stable identities — it carries the contributing
/// fact's interned `stable_key` rather than a run-local dense ID, so the reference survives
/// re-densification and is byte-stable across runs when resolved (D-08, composition over
/// duplication). The `stable_key` is built via `stable_key_from_parts`, which
/// length-prefixes and embeds the originating [`FactFamily`] label, so the family is
/// captured INSIDE the stable key (no separate non-serializable `FactFamily` field
/// is carried). The total order over a set of these is the lexicographic order of
/// the resolved stable-key text (never raw [`StableKeyId`] allocation order).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContributingFact {
    /// The contributing fact's stable key — its EXISTING stable identity, built via
    /// the `stable_key_from_parts` recipe (which embeds the `FactFamily` label).
    /// Never a run-local dense ID.
    pub stable_key: StableKeyId,
}

impl ContributingFact {
    /// Build a contributing-fact reference from a family + labeled key parts, using
    /// the canonical length-prefixed `stable_key_from_parts` recipe so the stable
    /// key is byte-identical to the upstream producer's key for the same identity.
    /// The `family` is folded into the stable key, not stored separately.
    pub fn from_parts(
        interner: &StableKeyInterner,
        family: FactFamily,
        parts: &[(&str, String)],
    ) -> Self {
        Self {
            stable_key: stable_key_from_parts(interner, family, parts),
        }
    }
}

/// Resolved-text wire/cache payload for one contributing fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContributingFactStablePayload {
    #[serde(rename = "stable_key")]
    pub stable_key_text: String,
}

/// Full provenance for one solver-derived edge (GRAPH-04, D-08).
///
/// The three roadmap-named fields: contributing fact IDs (total-ordered by resolved
/// stable text), the producing constraint kind, and the monotonic solver step. Two
/// provenances built from the SAME contributing facts in different discovery order
/// are equal and project to identical resolved-text payloads, because
/// [`DerivedEdgeProvenance::new`] sorts and de-duplicates the contributing set by
/// resolved `stable_key` text (which embeds the originating family).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DerivedEdgeProvenance {
    /// Contributing fact identities, TOTALLY ORDERED BY RESOLVED STABLE TEXT (sorted by
    /// resolved `stable_key`, de-duplicated). This is the load-bearing set the deletion
    /// property test (D-09) operates on.
    pub contributing_facts: Vec<ContributingFact>,
    /// The producing `ConstraintKind`, as its stable snake_case label
    /// (`ConstraintKind::as_str()`, owned). Reuses the existing vocabulary; no
    /// parallel enum is minted.
    pub constraint_kind: String,
    /// The monotonic `u64` solver step (the engine's worklist step counter) at
    /// which this edge was derived.
    pub solver_step: u64,
}

/// Resolved-text wire/cache payload for provenance. Never carries numeric interner ids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DerivedEdgeProvenanceStablePayload {
    pub contributing_facts: Vec<ContributingFactStablePayload>,
    pub constraint_kind: String,
    pub solver_step: u64,
}

impl DerivedEdgeProvenance {
    /// Build provenance from an UNORDERED set of contributing facts, the producing
    /// constraint kind, and the solver step.
    ///
    /// The contributing facts are sorted by resolved stable-key text and de-duplicated
    /// so the result is byte-stable regardless of the order facts were discovered in
    /// (the total-order rule). The constraint kind is captured as its stable
    /// label via [`ConstraintKind::as_str`] — referencing the existing vocabulary
    /// rather than duplicating it.
    pub fn new(
        interner: &StableKeyInterner,
        contributing_facts: impl IntoIterator<Item = ContributingFact>,
        constraint_kind: &ConstraintKind,
        solver_step: u64,
    ) -> Self {
        let mut contributing_facts: Vec<ContributingFact> =
            contributing_facts.into_iter().collect();
        // Total order by resolved stable text (never StableKeyId allocation order),
        // then de-duplicate so a fact referenced twice does not perturb the byte layout.
        contributing_facts
            .sort_by(|left, right| interner.compare_canonical(left.stable_key, right.stable_key));
        contributing_facts.dedup_by(|left, right| left.stable_key == right.stable_key);
        Self {
            contributing_facts,
            constraint_kind: constraint_kind.as_str().to_string(),
            solver_step,
        }
    }

    /// The number of contributing facts this derived edge depends on.
    pub fn contributing_len(&self) -> usize {
        self.contributing_facts.len()
    }

    /// A stable-key fragment summarizing the provenance, suitable for embedding in a
    /// derived-edge fact's `stable_key`. Built from the totally-ordered contributing
    /// stable keys + the constraint kind (the solver step is a run-local progress
    /// counter and is intentionally excluded from the stable key so two byte-equal
    /// derivations that converge at different steps still dedup).
    pub fn stable_key_fragment(&self, interner: &StableKeyInterner) -> String {
        let mut fragment = self.constraint_kind.clone();
        for fact in &self.contributing_facts {
            fragment.push('|');
            fragment.push_str(interner.resolve(fact.stable_key).as_ref());
        }
        fragment
    }

    /// Project provenance into a resolved-text stable payload.
    pub fn stable_payload(
        &self,
        interner: &StableKeyInterner,
    ) -> DerivedEdgeProvenanceStablePayload {
        DerivedEdgeProvenanceStablePayload {
            contributing_facts: self
                .contributing_facts
                .iter()
                .map(|fact| ContributingFactStablePayload {
                    stable_key_text: interner.resolve(fact.stable_key).to_string(),
                })
                .collect(),
            constraint_kind: self.constraint_kind.clone(),
            solver_step: self.solver_step,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::ids::SemanticNodeId;

    fn copy_edge() -> ConstraintKind {
        ConstraintKind::CopyEdge {
            dst: SemanticNodeId(1),
            src: SemanticNodeId(2),
        }
    }

    fn fact_on(interner: &StableKeyInterner, name: &str) -> ContributingFact {
        ContributingFact::from_parts(
            interner,
            FactFamily::PointsToConstraint,
            &[("constraint", name.to_string())],
        )
    }

    #[test]
    fn provenance_is_shuffle_stable_in_contributing_order() {
        // Provenance built from the same contributing facts in DIFFERENT input order
        // is equal and projects to identical resolved-text payloads.
        let interner = StableKeyInterner::default();
        let forward = DerivedEdgeProvenance::new(
            &interner,
            vec![
                fact_on(&interner, "addr-a"),
                fact_on(&interner, "copy"),
                fact_on(&interner, "field-store"),
            ],
            &copy_edge(),
            7,
        );
        let reversed = DerivedEdgeProvenance::new(
            &interner,
            vec![
                fact_on(&interner, "field-store"),
                fact_on(&interner, "copy"),
                fact_on(&interner, "addr-a"),
            ],
            &copy_edge(),
            7,
        );

        assert_eq!(forward, reversed);
        assert_eq!(
            serde_json::to_string(&forward.stable_payload(&interner)).expect("serialize forward"),
            serde_json::to_string(&reversed.stable_payload(&interner)).expect("serialize reversed"),
        );
    }

    #[test]
    fn reverse_intern_allocation_order_yields_identical_stable_payload() {
        // Two independent interners that allocate the same texts in opposite order
        // must still produce byte-identical resolved-text provenance payloads.
        let forward_interner = StableKeyInterner::default();
        let reverse_interner = StableKeyInterner::default();
        let _ = fact_on(&forward_interner, "addr-a");
        let _ = fact_on(&forward_interner, "copy");
        let _ = fact_on(&forward_interner, "field-store");
        let _ = fact_on(&reverse_interner, "field-store");
        let _ = fact_on(&reverse_interner, "copy");
        let _ = fact_on(&reverse_interner, "addr-a");

        let forward = DerivedEdgeProvenance::new(
            &forward_interner,
            vec![
                fact_on(&forward_interner, "addr-a"),
                fact_on(&forward_interner, "copy"),
                fact_on(&forward_interner, "field-store"),
            ],
            &copy_edge(),
            7,
        );
        let reversed = DerivedEdgeProvenance::new(
            &reverse_interner,
            vec![
                fact_on(&reverse_interner, "field-store"),
                fact_on(&reverse_interner, "copy"),
                fact_on(&reverse_interner, "addr-a"),
            ],
            &copy_edge(),
            7,
        );

        let forward_first = forward.contributing_facts[0].stable_key;
        let reverse_first = reversed.contributing_facts[0].stable_key;
        assert_eq!(
            forward_interner.resolve(forward_first).as_ref(),
            reverse_interner.resolve(reverse_first).as_ref()
        );
        assert_ne!(
            forward_first.0, reverse_first.0,
            "same text must receive different allocation ids across reverse-interned tables"
        );
        assert_eq!(
            serde_json::to_string(&forward.stable_payload(&forward_interner))
                .expect("serialize forward"),
            serde_json::to_string(&reversed.stable_payload(&reverse_interner))
                .expect("serialize reversed"),
        );
    }

    #[test]
    fn provenance_dedups_repeated_contributing_facts() {
        let interner = StableKeyInterner::default();
        let with_dup = DerivedEdgeProvenance::new(
            &interner,
            vec![
                fact_on(&interner, "copy"),
                fact_on(&interner, "copy"),
                fact_on(&interner, "addr-a"),
            ],
            &copy_edge(),
            3,
        );
        assert_eq!(with_dup.contributing_len(), 2);
    }

    #[test]
    fn provenance_captures_constraint_kind_label_and_step() {
        let interner = StableKeyInterner::default();
        let provenance = DerivedEdgeProvenance::new(
            &interner,
            vec![fact_on(&interner, "copy")],
            &copy_edge(),
            42,
        );
        assert_eq!(provenance.constraint_kind, "copy_edge");
        assert_eq!(provenance.solver_step, 42);
    }

    #[test]
    fn stable_key_fragment_is_independent_of_input_order() {
        let interner = StableKeyInterner::default();
        let a = DerivedEdgeProvenance::new(
            &interner,
            vec![fact_on(&interner, "a"), fact_on(&interner, "b")],
            &copy_edge(),
            1,
        );
        // A different solver step must NOT change the stable-key fragment (the step
        // is run-local progress, not identity).
        let b = DerivedEdgeProvenance::new(
            &interner,
            vec![fact_on(&interner, "b"), fact_on(&interner, "a")],
            &copy_edge(),
            99,
        );
        assert_eq!(
            a.stable_key_fragment(&interner),
            b.stable_key_fragment(&interner)
        );
    }
}
