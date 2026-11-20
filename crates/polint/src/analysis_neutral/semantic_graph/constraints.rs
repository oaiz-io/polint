use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::analysis_neutral::ids::{SemanticConstraintId, SemanticNodeId, TypeFactId};
use crate::analysis_neutral::points_to::facts::{PointsToPrecision, PointsToStatus};
use crate::internal_core::StableKeyId;

// ---------------------------------------------------------------------------
// ConstraintKind
// ---------------------------------------------------------------------------

/// The unified, closed constraint vocabulary the whole frontend graph emits into
/// (GRAPH-02). Its variants keep a pinned declaration order so the
/// derived `Ord` and serde representation are declaration-driven and byte-stable,
/// matching the established `RootKind`/`PointsToConstraintKind` convention. No
/// `#[repr(u8)]` — byte-stability is achieved purely via pinned order + derived
/// `Ord` + serde rename + an `as_str()` label.
///
/// Because the variants carry payloads they are NOT `Copy`; every payload field is
/// an `Ord` type (`SemanticNodeId` / `TypeFactId` / `String`) so the derived `Ord`
/// that drives byte-stable ordering survives.
///
/// # D-09 conceptual map to `points_to::PointsToConstraintKind` (NO code coupling)
///
/// This enum is the unified frontend vocabulary that sits *above* the points-to
/// sub-domain's internal `crate::analysis_neutral::points_to::facts::PointsToConstraintKind`.
/// The two are distinct enums and are not merged, renamed, imported, or otherwise
/// coupled. Their conceptual relationship is:
///
/// - `ConstraintKind::CopyEdge` <-> `PointsToConstraintKind::Copy`
/// - `ConstraintKind::Alloc` <-> `PointsToConstraintKind::AddressOf`
/// - `ConstraintKind::FieldLoad` <-> `PointsToConstraintKind::FieldLoad`
/// - `ConstraintKind::FieldStore`<-> `PointsToConstraintKind::FieldStore`
///
/// Crucially, this module references `points_to::facts` ONLY for the shared
/// `PointsToStatus`/`PointsToPrecision` status/precision field types (D-10 mandates
/// `ConstraintFact` mirror `PointsToConstraintFact` exactly); it imports neither
/// `PointsToConstraintKind` nor any points-to solver type. The named variants here
/// reference semantic-graph `SemanticNodeId`s / existing fact IDs (`TypeFactId`),
/// never the points-to sub-domain's `PtVarId`/`ObjectTokenId` run-local IDs.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintKind {
    /// `dst <- src` value copy. Maps conceptually to points-to `Copy`.
    CopyEdge {
        dst: SemanticNodeId,
        src: SemanticNodeId,
    },
    /// `dst <- &object` allocation/address-of. Maps conceptually to points-to
    /// `AddressOf`.
    Alloc {
        dst: SemanticNodeId,
        object: SemanticNodeId,
    },
    /// `dst <- base.field` field/property load.
    FieldLoad {
        dst: SemanticNodeId,
        base: SemanticNodeId,
        field: String,
    },
    /// `base.field <- src` field/property store.
    FieldStore {
        base: SemanticNodeId,
        field: String,
        src: SemanticNodeId,
    },
    /// A call obligation anchored at a callsite node, consumed by the
    /// unified solver to resolve dynamic dispatch / refine targets.
    ///
    /// The callsite node's points-to set is the site's callees. `callee` is the
    /// place whose value is called, `receiver` the value bound to the callee's
    /// `this`, `arguments` the argument places in order and `result` the place
    /// that receives the return value; each is `None` (or empty) when the
    /// frontend has no place for it. Arguments from `spread_from` on come from
    /// a spread and are not bound to parameters by position.
    CallConstraint {
        callsite: SemanticNodeId,
        callee: Option<SemanticNodeId>,
        receiver: Option<SemanticNodeId>,
        arguments: Vec<SemanticNodeId>,
        spread_from: Option<usize>,
        result: Option<SemanticNodeId>,
    },
    /// Accepted repo-local adaptation model edge. Only validated model facts produce
    /// this variant; rejected facts remain report-only and never lower to a
    /// constraint.
    ModelEdge {
        source: SemanticNodeId,
        target: SemanticNodeId,
        language: String,
        scope: String,
        confidence: String,
        evidence: Vec<String>,
    },
    /// A type obligation on a node, referencing an existing `TypeFactId` from the
    /// type substrate (never a run-local dense ID of another family).
    TypeConstraint {
        node: SemanticNodeId,
        type_fact: TypeFactId,
    },
    /// How `node` is invoked. On a function node, the ways the function can be
    /// invoked without throwing: called, constructed with `new`, both or
    /// neither. On a call site node, the one way the site invokes its callee.
    /// A call site or a function without this constraint invokes, or can be
    /// invoked, in a way the frontend does not know.
    Invocation {
        node: SemanticNodeId,
        call: bool,
        construct: bool,
    },
}

impl ConstraintKind {
    /// A call constraint that names only its callsite.
    pub fn call_at(callsite: SemanticNodeId) -> Self {
        Self::CallConstraint {
            callsite,
            callee: None,
            receiver: None,
            arguments: Vec::new(),
            spread_from: None,
            result: None,
        }
    }

    /// Stable lowercase tag label used in stable keys and digest payloads.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CopyEdge { .. } => "copy_edge",
            Self::Alloc { .. } => "alloc",
            Self::FieldLoad { .. } => "field_load",
            Self::FieldStore { .. } => "field_store",
            Self::CallConstraint { .. } => "call_constraint",
            Self::ModelEdge { .. } => "model_edge",
            Self::TypeConstraint { .. } => "type_constraint",
            Self::Invocation { .. } => "invocation",
        }
    }

    /// Every `SemanticNodeId` this payload references — the single source of truth
    /// for both referential validation (`store`/`validate`) and dense-ID remapping.
    ///
    /// Matched WITHOUT a `..` rest pattern and binding every field (non-node fields
    /// via `_`), so adding any field to a variant is a **compile error here** rather
    /// than a silent escape from remap/validation. Non-node fields (`field`,
    /// `type_fact`) are intentionally excluded — a `TypeFactId` is a type-substrate
    /// identity, not a graph node.
    pub fn referenced_nodes(&self) -> Vec<SemanticNodeId> {
        match self {
            Self::CopyEdge { dst, src } => vec![*dst, *src],
            Self::Alloc { dst, object } => vec![*dst, *object],
            Self::FieldLoad {
                dst,
                base,
                field: _,
            } => vec![*dst, *base],
            Self::FieldStore {
                base,
                field: _,
                src,
            } => vec![*base, *src],
            Self::CallConstraint {
                callsite,
                callee,
                receiver,
                arguments,
                spread_from: _,
                result,
            } => std::iter::once(*callsite)
                .chain(*callee)
                .chain(*receiver)
                .chain(arguments.iter().copied())
                .chain(*result)
                .collect(),
            Self::ModelEdge {
                source,
                target,
                language: _,
                scope: _,
                confidence: _,
                evidence: _,
            } => vec![*source, *target],
            Self::TypeConstraint { node, type_fact: _ } => vec![*node],
            Self::Invocation {
                node,
                call: _,
                construct: _,
            } => vec![*node],
        }
    }

    /// Applies `map` in place to every `SemanticNodeId` this payload references,
    /// used to rewrite endpoints through the node-densification remap. Matched
    /// WITHOUT `..` for the same drift-proofing as [`ConstraintKind::referenced_nodes`].
    pub fn remap_nodes(&mut self, map: impl Fn(SemanticNodeId) -> SemanticNodeId) {
        match self {
            Self::CopyEdge { dst, src } => {
                *dst = map(*dst);
                *src = map(*src);
            }
            Self::Alloc { dst, object } => {
                *dst = map(*dst);
                *object = map(*object);
            }
            Self::FieldLoad {
                dst,
                base,
                field: _,
            } => {
                *dst = map(*dst);
                *base = map(*base);
            }
            Self::FieldStore {
                base,
                field: _,
                src,
            } => {
                *base = map(*base);
                *src = map(*src);
            }
            Self::CallConstraint {
                callsite,
                callee,
                receiver,
                arguments,
                spread_from: _,
                result,
            } => {
                *callsite = map(*callsite);
                for node in callee
                    .iter_mut()
                    .chain(receiver.iter_mut())
                    .chain(arguments.iter_mut())
                    .chain(result.iter_mut())
                {
                    *node = map(*node);
                }
            }
            Self::ModelEdge {
                source,
                target,
                language: _,
                scope: _,
                confidence: _,
                evidence: _,
            } => {
                *source = map(*source);
                *target = map(*target);
            }
            Self::TypeConstraint { node, type_fact: _ } => {
                *node = map(*node);
            }
            Self::Invocation {
                node,
                call: _,
                construct: _,
            } => {
                *node = map(*node);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ConstraintFact family
// ---------------------------------------------------------------------------

/// One unified semantic-graph constraint. Mirrors `points_to::facts::
/// PointsToConstraintFact` exactly (D-10): `{ id, kind, status, precision,
/// stable_key }`, reusing the points-to `PointsToStatus`/`PointsToPrecision`
/// vocabulary as the status/precision field types rather than inventing a redundant
/// enum.
///
/// The dense `id` is a run-local post-normalization read concern only and MUST NOT
/// enter any serialized stable payload that feeds the output digest (D-06) —
/// `#[serde(skip)]` strips it; serde restores it via `SemanticConstraintId::default()`
/// (= 0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstraintFact {
    pub id: SemanticConstraintId,
    pub kind: ConstraintKind,
    pub status: PointsToStatus,
    pub precision: PointsToPrecision,
    /// Built from the referenced existing identity (D-06), never run-local dense
    /// IDs. Populated by `build_semantic_graph`.
    pub stable_key: StableKeyId,
}

/// The ways a function can be invoked without throwing a `TypeError`.
///
/// A plain function can be called and constructed. An arrow, a method, an
/// accessor, an async function and a generator can only be called; a class
/// can only be constructed; the code the frontend makes of a class member
/// is never invoked by a call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Invocable {
    pub(crate) call: bool,
    pub(crate) construct: bool,
}

impl Invocable {
    pub(crate) const PLAIN: Self = Self {
        call: true,
        construct: true,
    };
    pub(crate) const CALL: Self = Self {
        call: true,
        construct: false,
    };
    pub(crate) const CONSTRUCT: Self = Self {
        call: false,
        construct: true,
    };
    pub(crate) const NEVER: Self = Self {
        call: false,
        construct: false,
    };

    /// Whether a site that invokes as `site` can run a function invocable as
    /// `self`.
    pub(crate) fn admits(self, site: Self) -> bool {
        (site.call && self.call) || (site.construct && self.construct)
    }
}

/// The invocation constraints of `constraints`, by node. A node whose
/// constraints disagree is left out, as one with no constraint is.
pub(crate) fn invocations(constraints: &[ConstraintFact]) -> BTreeMap<SemanticNodeId, Invocable> {
    let mut by_node = BTreeMap::<SemanticNodeId, Option<Invocable>>::new();
    for constraint in constraints {
        if let ConstraintKind::Invocation {
            node,
            call,
            construct,
        } = constraint.kind
        {
            let invocable = Invocable { call, construct };
            by_node
                .entry(node)
                .and_modify(|known| {
                    if *known != Some(invocable) {
                        *known = None;
                    }
                })
                .or_insert(Some(invocable));
        }
    }
    by_node
        .into_iter()
        .filter_map(|(node, invocable)| Some((node, invocable?)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_constraint_references_and_remaps_every_place() {
        let mut kind = ConstraintKind::CallConstraint {
            callsite: SemanticNodeId(1),
            callee: Some(SemanticNodeId(2)),
            receiver: Some(SemanticNodeId(3)),
            arguments: vec![SemanticNodeId(4), SemanticNodeId(5)],
            spread_from: Some(1),
            result: Some(SemanticNodeId(6)),
        };

        assert_eq!(
            kind.referenced_nodes(),
            (1..=6).map(SemanticNodeId).collect::<Vec<_>>()
        );
        kind.remap_nodes(|node| SemanticNodeId(node.0 + 10));
        assert_eq!(
            kind.referenced_nodes(),
            (11..=16).map(SemanticNodeId).collect::<Vec<_>>()
        );
        assert!(matches!(
            kind,
            ConstraintKind::CallConstraint {
                spread_from: Some(1),
                ..
            }
        ));
    }

    #[test]
    fn constraint_kind_has_exactly_8_variants() {
        // Compile-time exhaustive match over every arm; the array length lock fails
        // to compile if a variant is added without updating this test.
        fn assert_all(kind: &ConstraintKind) -> &'static str {
            match kind {
                ConstraintKind::CopyEdge { .. } => "copy_edge",
                ConstraintKind::Alloc { .. } => "alloc",
                ConstraintKind::FieldLoad { .. } => "field_load",
                ConstraintKind::FieldStore { .. } => "field_store",
                ConstraintKind::CallConstraint { .. } => "call_constraint",
                ConstraintKind::ModelEdge { .. } => "model_edge",
                ConstraintKind::TypeConstraint { .. } => "type_constraint",
                ConstraintKind::Invocation { .. } => "invocation",
            }
        }
        let variants = [
            assert_all(&ConstraintKind::CopyEdge {
                dst: SemanticNodeId(0),
                src: SemanticNodeId(1),
            }),
            assert_all(&ConstraintKind::Alloc {
                dst: SemanticNodeId(0),
                object: SemanticNodeId(1),
            }),
            assert_all(&ConstraintKind::FieldLoad {
                dst: SemanticNodeId(0),
                base: SemanticNodeId(1),
                field: "f".to_string(),
            }),
            assert_all(&ConstraintKind::FieldStore {
                base: SemanticNodeId(0),
                field: "f".to_string(),
                src: SemanticNodeId(1),
            }),
            assert_all(&ConstraintKind::call_at(SemanticNodeId(0))),
            assert_all(&model_edge()),
            assert_all(&ConstraintKind::TypeConstraint {
                node: SemanticNodeId(0),
                type_fact: TypeFactId(1),
            }),
            assert_all(&ConstraintKind::Invocation {
                node: SemanticNodeId(0),
                call: true,
                construct: false,
            }),
        ];
        assert_eq!(variants.len(), 8);
    }

    #[test]
    fn constraint_kind_sorts_in_pinned_declaration_order() {
        // A permuted list sorts back to declaration order: the variant discriminant
        // ordering is declaration-driven, so kind ordering is stable regardless of
        // payload values.
        let mut kinds = [
            ConstraintKind::Invocation {
                node: SemanticNodeId(0),
                call: false,
                construct: false,
            },
            ConstraintKind::TypeConstraint {
                node: SemanticNodeId(0),
                type_fact: TypeFactId(0),
            },
            model_edge(),
            ConstraintKind::call_at(SemanticNodeId(0)),
            ConstraintKind::FieldStore {
                base: SemanticNodeId(0),
                field: "f".to_string(),
                src: SemanticNodeId(0),
            },
            ConstraintKind::FieldLoad {
                dst: SemanticNodeId(0),
                base: SemanticNodeId(0),
                field: "f".to_string(),
            },
            ConstraintKind::Alloc {
                dst: SemanticNodeId(0),
                object: SemanticNodeId(0),
            },
            ConstraintKind::CopyEdge {
                dst: SemanticNodeId(0),
                src: SemanticNodeId(0),
            },
        ];
        kinds.sort();
        let tags: Vec<&'static str> = kinds.iter().map(ConstraintKind::as_str).collect();
        assert_eq!(
            tags,
            vec![
                "copy_edge",
                "alloc",
                "field_load",
                "field_store",
                "call_constraint",
                "model_edge",
                "type_constraint",
                "invocation",
            ]
        );
    }

    #[test]
    fn constraint_kind_labels_are_stable_snake_case() {
        assert_eq!(
            ConstraintKind::CopyEdge {
                dst: SemanticNodeId(0),
                src: SemanticNodeId(1)
            }
            .as_str(),
            "copy_edge"
        );
        assert_eq!(
            ConstraintKind::Alloc {
                dst: SemanticNodeId(0),
                object: SemanticNodeId(1)
            }
            .as_str(),
            "alloc"
        );
        assert_eq!(model_edge().as_str(), "model_edge");
        assert_eq!(
            ConstraintKind::TypeConstraint {
                node: SemanticNodeId(0),
                type_fact: TypeFactId(1)
            }
            .as_str(),
            "type_constraint"
        );
    }

    fn model_edge() -> ConstraintKind {
        ConstraintKind::ModelEdge {
            source: SemanticNodeId(0),
            target: SemanticNodeId(1),
            language: "typescript".to_string(),
            scope: "src/app.ts".to_string(),
            confidence: "heuristic".to_string(),
            evidence: vec!["src/app.ts:10".to_string()],
        }
    }

    #[test]
    fn constraint_fact_keeps_dense_and_stable_ids_separate() {
        // D-10: the fact carries exactly { id, kind, status, precision, stable_key }
        // and reuses the points-to status/precision vocabulary.
        let db = crate::analysis_neutral::LocalAnalysisDb::new();
        let fact = ConstraintFact {
            id: SemanticConstraintId(7),
            kind: ConstraintKind::call_at(SemanticNodeId(3)),
            status: PointsToStatus::Present,
            precision: PointsToPrecision::FlowInsensitive,
            stable_key: db
                .stable_key_interner()
                .intern("constraint|call_constraint|cs"),
        };
        assert_eq!(fact.id, SemanticConstraintId(7));
        assert_eq!(
            db.resolve_stable_key(fact.stable_key).as_ref(),
            "constraint|call_constraint|cs"
        );
    }

    #[test]
    fn type_constraint_references_existing_type_fact_id() {
        // The TypeConstraint variant references an existing TypeFactId from the type
        // substrate, not a run-local dense ID of another family.
        let kind = ConstraintKind::TypeConstraint {
            node: SemanticNodeId(2),
            type_fact: TypeFactId(99),
        };
        match kind {
            ConstraintKind::TypeConstraint { type_fact, .. } => {
                assert_eq!(type_fact, TypeFactId(99));
            }
            _ => panic!("expected type_constraint variant"),
        }
    }
}
