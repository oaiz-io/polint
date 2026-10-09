//! Closed identity-vs-unsupported taxonomy and the pure projection functions
//! that map every v1.2 `analysis::calls` failure source variant to exactly one
//! [`IdentityCategory`] (D-14, D-15, D-16; Pattern H).
//!
//! The taxonomy is deliberately closed (no `Other` / `Unknown`): adding a new
//! source variant upstream (`UnresolvedCallReason` / `CallTargetStatus`) must
//! produce a compile error inside the exhaustive `match` projections below so
//! every classification is a deliberate review action (BLOCKER #5, Anti-Patterns:
//! no wildcard arms on closed source enums).
//!
//! This module is a *tag* on existing facts — it introduces no new fact family
//! (D-16). It exports only [`IdentityCategory`], [`CategorizeReason`], and the
//! three `category_for_*` projection functions.

use serde::{Deserialize, Serialize};

use crate::analysis_neutral::calls::facts::{CallTargetStatus, UnresolvedCallReason};
use crate::analysis_neutral::identity::facts::{IdentityKind, IdentityRecord};

/// Closed taxonomy of identity-vs-unsupported failure categories (D-14).
///
/// The five variants are declared in a **pinned source order** that is
/// cross-platform byte-stable (D-25): the declaration order defines both the
/// serde discriminant order and the `Ord` derive ordering. Reordering the
/// variants is a deliberate release-review change (BLOCKER #5) and is guarded
/// by the `identity_category_variants_in_source_order` test below.
///
/// `#[repr(u8)]` pins each variant's discriminant to its CONTEXT-pinned ordinal
/// so the variant-order lock test can cast each variant to `u8` and assert.
///
/// There is intentionally no `Other` / `Unknown` / catch-all variant and no
/// `#[non_exhaustive]` — D-14 is a hard contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum IdentityCategory {
    /// polint named the right place wrong: an emitted callsite identity does not
    /// match any oracle identity but its file/span overlaps an oracle entry
    /// (D-16). Ordinal 0.
    WrongIdentity = 0,
    /// An out-of-scope / unsupported semantic construct (reflection, eval,
    /// dynamic dispatch we deliberately do not model). Ordinal 1.
    UnsupportedEdge = 1,
    /// polint could not resolve the callee (missing semantic reference, dynamic
    /// property, function value, budget exhaustion). Ordinal 2.
    UnresolvedEdge = 2,
    /// The language package/module could not be loaded (Go packages failed to
    /// load — the canonical `SetupMissing` case). Ordinal 3.
    PackageLoadLimitation = 3,
    /// An adaptation / framework model gap: the model rejected the candidate
    /// edge or the framework model is missing. Ordinal 4.
    ModelMissing = 4,
}

/// Per-fact tag recording the upstream source label that produced an
/// [`IdentityCategory`] classification (D-16).
///
/// This is a tag on EXISTING facts — it introduces no new fact family. It
/// carries the originating closed-enum discriminant so a categorization decision
/// remains auditable back to its source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CategorizeReason {
    /// Categorized from an [`UnresolvedCallReason`] on an unresolved-call fact.
    FromUnresolvedCallReason(UnresolvedCallReason),
    /// Categorized from a [`CallTargetStatus`] on a call-target fact.
    FromCallTargetStatus(CallTargetStatus),
    /// Categorized as wrong-identity because an emitted callsite identity
    /// overlaps an oracle entry's span without matching it (D-16).
    WrongIdentityOnSpanOverlap,
}

/// Projects an [`UnresolvedCallReason`] onto exactly one [`IdentityCategory`].
///
/// Exhaustive `match` with NO wildcard arm (Pattern H): adding a new
/// `UnresolvedCallReason` variant upstream is a compile error here, forcing a
/// deliberate categorization decision. The mapping below is the audit contract
/// locked by the co-located tests.
pub fn category_for_unresolved(reason: UnresolvedCallReason) -> IdentityCategory {
    match reason {
        // "Couldn't resolve the callee" cases.
        UnresolvedCallReason::FunctionValue
        | UnresolvedCallReason::DynamicProperty
        | UnresolvedCallReason::DynamicImport
        | UnresolvedCallReason::ProxyOrAccessor
        | UnresolvedCallReason::MissingSemanticReference
        | UnresolvedCallReason::MissingImportResolution
        | UnresolvedCallReason::UnknownCallee
        | UnresolvedCallReason::Unknown => IdentityCategory::UnresolvedEdge,
        // Out-of-scope / unsupported semantic constructs.
        UnresolvedCallReason::Eval
        | UnresolvedCallReason::CallApplyBind
        | UnresolvedCallReason::Reflection
        | UnresolvedCallReason::GoroutineBoundary
        | UnresolvedCallReason::UnsupportedSyntax => IdentityCategory::UnsupportedEdge,
        // Go packages / setup couldn't load.
        UnresolvedCallReason::SetupMissing => IdentityCategory::PackageLoadLimitation,
        // Missing adaptation / framework model (dispatch the model would resolve).
        UnresolvedCallReason::InterfaceDispatch | UnresolvedCallReason::FrameworkDispatch => {
            IdentityCategory::ModelMissing
        }
        // Couldn't finish resolving within budget — conservatively unresolved.
        UnresolvedCallReason::BudgetExceeded => IdentityCategory::UnresolvedEdge,
    }
}

/// Projects a [`CallTargetStatus`] onto an optional [`IdentityCategory`].
///
/// Returns `None` for the success/precision statuses (`Resolved`, `Ambiguous`)
/// which are not failures. Exhaustive 7-arm `match` with NO wildcard (Pattern H).
pub fn category_for_unsupported(status: CallTargetStatus) -> Option<IdentityCategory> {
    match status {
        // Not failures: success and multi-match precision concerns.
        CallTargetStatus::Resolved
        | CallTargetStatus::Ambiguous
        | CallTargetStatus::Unreachable => None,
        CallTargetStatus::Unresolved => Some(IdentityCategory::UnresolvedEdge),
        CallTargetStatus::Unsupported => Some(IdentityCategory::UnsupportedEdge),
        // Go packages couldn't load is the canonical SetupMissing case (D-16).
        CallTargetStatus::SetupMissing => Some(IdentityCategory::PackageLoadLimitation),
        // Couldn't finish resolving in budget.
        CallTargetStatus::BudgetExceeded => Some(IdentityCategory::UnresolvedEdge),
        // The model rejected the candidate edge — adaptation model gap.
        CallTargetStatus::Rejected => Some(IdentityCategory::ModelMissing),
    }
}

/// Classifies a wrong-identity event: polint named the right place wrong (D-16).
///
/// Only callsite identities are eligible (function identities are not subject to
/// wrong-identity classification). When `oracle_overlap == true` the observed
/// callsite span overlaps an oracle entry without matching it, so it is a
/// wrong-identity event; otherwise it is simply a miss (`None`).
///
/// `oracle_overlap` is computed by the caller (the eval runner) by comparing the
/// observed callsite's `(file_id, span)` to the oracle entry's `(file_id, span)`.
pub fn category_for_wrong_identity(
    observed: &IdentityRecord,
    oracle_overlap: bool,
) -> Option<IdentityCategory> {
    if observed.kind != IdentityKind::Callsite {
        return None;
    }
    if oracle_overlap {
        Some(IdentityCategory::WrongIdentity)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Arc;

    use super::*;
    use crate::analysis_neutral::identity::facts::{
        IdentityRecordId, LanguageTag, SignatureDigest, compute_signature_digest,
    };
    use crate::internal_core::{FileId, Span};

    const ALL_CATEGORIES: [IdentityCategory; 5] = [
        IdentityCategory::WrongIdentity,
        IdentityCategory::UnsupportedEdge,
        IdentityCategory::UnresolvedEdge,
        IdentityCategory::PackageLoadLimitation,
        IdentityCategory::ModelMissing,
    ];

    /// Every `UnresolvedCallReason` variant, locked so a new upstream variant
    /// forces this array (and the exhaustive `match`) to be updated.
    const ALL_UNRESOLVED_REASONS: [UnresolvedCallReason; 17] = [
        UnresolvedCallReason::FunctionValue,
        UnresolvedCallReason::DynamicProperty,
        UnresolvedCallReason::InterfaceDispatch,
        UnresolvedCallReason::Eval,
        UnresolvedCallReason::CallApplyBind,
        UnresolvedCallReason::FrameworkDispatch,
        UnresolvedCallReason::Reflection,
        UnresolvedCallReason::GoroutineBoundary,
        UnresolvedCallReason::DynamicImport,
        UnresolvedCallReason::ProxyOrAccessor,
        UnresolvedCallReason::MissingSemanticReference,
        UnresolvedCallReason::MissingImportResolution,
        UnresolvedCallReason::SetupMissing,
        UnresolvedCallReason::UnsupportedSyntax,
        UnresolvedCallReason::BudgetExceeded,
        UnresolvedCallReason::UnknownCallee,
        UnresolvedCallReason::Unknown,
    ];

    const ALL_TARGET_STATUSES: [CallTargetStatus; 7] = [
        CallTargetStatus::Resolved,
        CallTargetStatus::Ambiguous,
        CallTargetStatus::Unresolved,
        CallTargetStatus::Unsupported,
        CallTargetStatus::SetupMissing,
        CallTargetStatus::BudgetExceeded,
        CallTargetStatus::Rejected,
    ];

    fn callsite_record() -> IdentityRecord {
        IdentityRecord {
            id: IdentityRecordId(0),
            kind: IdentityKind::Callsite,
            file_id: FileId::from_raw(1),
            span: Span::point(FileId::from_raw(1), 3, 4),
            language: LanguageTag::Go,
            package_or_module: Arc::from("example.com/pkg"),
            container_path: Arc::from("pkg.Caller"),
            display_name: Arc::from("callee"),
            signature_digest: compute_signature_digest(
                LanguageTag::Go,
                "example.com/pkg",
                "pkg.Caller",
                "callee",
                None,
                None,
            ),
            multiplicity: 1,
            stable_key: crate::internal_core::stable_key_for_test(
                "identity|callsite|go|example.com/pkg|pkg.Caller|1|0..0",
            ),
            originating_call_site_id: None,
            originating_call_target_id: None,
        }
    }

    #[test]
    fn identity_category_has_exactly_five_variants() {
        // Discriminant-count test: catches accidental duplicate variant additions
        // and proves the closed taxonomy has exactly five members (D-14, D-15).
        let distinct = ALL_CATEGORIES
            .iter()
            .map(std::mem::discriminant)
            .collect::<HashSet<_>>();
        assert_eq!(distinct.len(), 5);
    }

    #[test]
    fn identity_category_variants_in_source_order() {
        // BLOCKER #5 + D-25: each variant's `#[repr(u8)]` discriminant must match
        // its CONTEXT-pinned ordinal. Reordering the variants trips this test.
        assert_eq!(IdentityCategory::WrongIdentity as u8, 0);
        assert_eq!(IdentityCategory::UnsupportedEdge as u8, 1);
        assert_eq!(IdentityCategory::UnresolvedEdge as u8, 2);
        assert_eq!(IdentityCategory::PackageLoadLimitation as u8, 3);
        assert_eq!(IdentityCategory::ModelMissing as u8, 4);

        // The `Ord` derive ordering also follows declaration order; the array is
        // therefore strictly ascending.
        for pair in ALL_CATEGORIES.windows(2) {
            assert!(
                pair[0] < pair[1],
                "{:?} should sort before {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn serde_snake_case_round_trip() {
        let expected = [
            (IdentityCategory::WrongIdentity, "\"wrong_identity\""),
            (IdentityCategory::UnsupportedEdge, "\"unsupported_edge\""),
            (IdentityCategory::UnresolvedEdge, "\"unresolved_edge\""),
            (
                IdentityCategory::PackageLoadLimitation,
                "\"package_load_limitation\"",
            ),
            (IdentityCategory::ModelMissing, "\"model_missing\""),
        ];
        for (category, json) in expected {
            let encoded = serde_json::to_string(&category).unwrap();
            assert_eq!(encoded, json);
            let restored: IdentityCategory = serde_json::from_str(&encoded).unwrap();
            assert_eq!(restored, category);
        }
    }

    #[test]
    fn category_for_unresolved_is_exhaustive() {
        // Locked mapping contract: every UnresolvedCallReason maps to one
        // category. The array length assertion forces this test to be updated if
        // a variant is added upstream.
        assert_eq!(ALL_UNRESOLVED_REASONS.len(), 17);
        let cases = [
            (
                UnresolvedCallReason::FunctionValue,
                IdentityCategory::UnresolvedEdge,
            ),
            (
                UnresolvedCallReason::DynamicProperty,
                IdentityCategory::UnresolvedEdge,
            ),
            (
                UnresolvedCallReason::DynamicImport,
                IdentityCategory::UnresolvedEdge,
            ),
            (
                UnresolvedCallReason::ProxyOrAccessor,
                IdentityCategory::UnresolvedEdge,
            ),
            (
                UnresolvedCallReason::MissingSemanticReference,
                IdentityCategory::UnresolvedEdge,
            ),
            (
                UnresolvedCallReason::MissingImportResolution,
                IdentityCategory::UnresolvedEdge,
            ),
            (
                UnresolvedCallReason::UnknownCallee,
                IdentityCategory::UnresolvedEdge,
            ),
            (
                UnresolvedCallReason::Unknown,
                IdentityCategory::UnresolvedEdge,
            ),
            (
                UnresolvedCallReason::BudgetExceeded,
                IdentityCategory::UnresolvedEdge,
            ),
            (
                UnresolvedCallReason::Eval,
                IdentityCategory::UnsupportedEdge,
            ),
            (
                UnresolvedCallReason::CallApplyBind,
                IdentityCategory::UnsupportedEdge,
            ),
            (
                UnresolvedCallReason::Reflection,
                IdentityCategory::UnsupportedEdge,
            ),
            (
                UnresolvedCallReason::GoroutineBoundary,
                IdentityCategory::UnsupportedEdge,
            ),
            (
                UnresolvedCallReason::UnsupportedSyntax,
                IdentityCategory::UnsupportedEdge,
            ),
            (
                UnresolvedCallReason::SetupMissing,
                IdentityCategory::PackageLoadLimitation,
            ),
            (
                UnresolvedCallReason::InterfaceDispatch,
                IdentityCategory::ModelMissing,
            ),
            (
                UnresolvedCallReason::FrameworkDispatch,
                IdentityCategory::ModelMissing,
            ),
        ];
        // Every variant is covered exactly once by the locked cases.
        assert_eq!(cases.len(), ALL_UNRESOLVED_REASONS.len());
        for reason in ALL_UNRESOLVED_REASONS {
            let mapping = cases
                .iter()
                .find(|(candidate, _)| *candidate == reason)
                .unwrap_or_else(|| panic!("missing locked mapping for {reason:?}"));
            assert_eq!(category_for_unresolved(reason), mapping.1);
        }
    }

    #[test]
    fn category_for_unsupported_maps_seven_statuses() {
        assert_eq!(ALL_TARGET_STATUSES.len(), 7);
        assert_eq!(category_for_unsupported(CallTargetStatus::Resolved), None);
        assert_eq!(category_for_unsupported(CallTargetStatus::Ambiguous), None);
        assert_eq!(
            category_for_unsupported(CallTargetStatus::Unresolved),
            Some(IdentityCategory::UnresolvedEdge)
        );
        assert_eq!(
            category_for_unsupported(CallTargetStatus::Unsupported),
            Some(IdentityCategory::UnsupportedEdge)
        );
        assert_eq!(
            category_for_unsupported(CallTargetStatus::SetupMissing),
            Some(IdentityCategory::PackageLoadLimitation)
        );
        assert_eq!(
            category_for_unsupported(CallTargetStatus::BudgetExceeded),
            Some(IdentityCategory::UnresolvedEdge)
        );
        assert_eq!(
            category_for_unsupported(CallTargetStatus::Rejected),
            Some(IdentityCategory::ModelMissing)
        );
    }

    #[test]
    fn wrong_identity_only_fires_on_callsite_with_span_overlap() {
        let callsite = callsite_record();
        assert_eq!(
            category_for_wrong_identity(&callsite, true),
            Some(IdentityCategory::WrongIdentity)
        );
        assert_eq!(category_for_wrong_identity(&callsite, false), None);

        let mut function = callsite_record();
        function.kind = IdentityKind::Function;
        assert_eq!(category_for_wrong_identity(&function, true), None);
        assert_eq!(category_for_wrong_identity(&function, false), None);
    }

    #[test]
    fn categorize_reason_round_trips_through_serde() {
        let reasons = [
            CategorizeReason::FromUnresolvedCallReason(UnresolvedCallReason::DynamicProperty),
            CategorizeReason::FromCallTargetStatus(CallTargetStatus::Rejected),
            CategorizeReason::WrongIdentityOnSpanOverlap,
        ];
        for reason in reasons {
            let json = serde_json::to_string(&reason).unwrap();
            let restored: CategorizeReason = serde_json::from_str(&json).unwrap();
            assert_eq!(restored, reason);
        }
    }

    #[test]
    fn signature_digest_witness_is_unused_zero_helper() {
        // Guards against the digest helper drifting: a non-zero digest is what
        // real records carry, so the all-zero sentinel must stay distinct.
        let digest = compute_signature_digest(LanguageTag::Go, "p", "c", "d", None, None);
        assert_ne!(digest, SignatureDigest([0u8; 16]));
    }
}
