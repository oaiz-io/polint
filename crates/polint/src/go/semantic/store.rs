use std::any::Any;

use crate::analysis_api::{FactFamily, FactStore};
use crate::internal_core::{StableKeyId, StableKeyInterner};

use crate::go::error::AnalysisError;
use crate::go::semantic::facts::{
    GoSemanticAddressTakenFact, GoSemanticCallsiteFact, GoSemanticDynamicDispatchFact,
    GoSemanticFunctionFact, GoSemanticInstantiatedTypeFact, GoSemanticMethodSetFact,
    GoSemanticPackageErrorFact, GoSemanticPackageFact, GoSemanticRtaEdgeFact,
};
use crate::go::semantic::validate::validate_go_semantic_output;

pub const GO_SEMANTIC_PROVIDER_ID: &str = "polint.go.semantic";

/// Per-family count of duplicate STRUCTURAL rows (packages/functions/method_sets) collapsed
/// keep-first by `GoSemanticFactsOutput::collapse_duplicate_structural_keys`, plus whether
/// any collapsed duplicate was "conflicting" (FIX-08).
///
/// The structural families are keyed by official Go identity (`package` / `fn.String()` /
/// canonical type), which is unique per entity — so a same-stable-key duplicate is in
/// practice the SAME entity emitted twice (byte-identical → keep-first is lossless). A
/// `conflicting` duplicate (same stable_key, but the dropped row `!=` the kept row) can only
/// arise from a stable-key-recipe bug; keep-first still proceeds (far better than the legacy
/// behaviour of zeroing ALL Go RTA repo-wide) but it is flagged loudly. All counts zero and
/// `conflicting == false` on a clean run, so the provider stays quiet.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StructuralDuplicateReport {
    pub packages: usize,
    pub functions: usize,
    pub method_sets: usize,
    /// True when at least one collapsed duplicate row DIFFERED from the kept row of the same
    /// stable_key (a practically-impossible genuine identity collision → recipe bug).
    pub conflicting: bool,
}

impl StructuralDuplicateReport {
    /// Total structural duplicate rows collapsed across all three families.
    pub fn total(&self) -> usize {
        self.packages + self.functions + self.method_sets
    }
}

/// Aggregate resilience report returned by [`GoSemanticStore::from_output`].
/// Combines malformed RTA-signal harvest rows dropped with duplicate structural rows
/// collapsed keep-first, so a single bad row is never catastrophic yet always observable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GoSemanticStoreReport {
    pub dropped_harvest_rows: usize,
    pub structural_duplicates: StructuralDuplicateReport,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GoSemanticFactsOutput {
    pub packages: Vec<GoSemanticPackageFact>,
    pub functions: Vec<GoSemanticFunctionFact>,
    pub callsites: Vec<GoSemanticCallsiteFact>,
    pub method_sets: Vec<GoSemanticMethodSetFact>,
    pub address_taken: Vec<GoSemanticAddressTakenFact>,
    pub instantiated_types: Vec<GoSemanticInstantiatedTypeFact>,
    pub dynamic_dispatch: Vec<GoSemanticDynamicDispatchFact>,
    pub rta_edges: Vec<GoSemanticRtaEdgeFact>,
    pub package_errors: Vec<GoSemanticPackageErrorFact>,
}

impl GoSemanticFactsOutput {
    pub fn normalized(mut self, interner: &StableKeyInterner) -> Self {
        self.packages
            .sort_by(|row, other| interner.compare_canonical(row.stable_key, other.stable_key));
        for (index, fact) in self.packages.iter_mut().enumerate() {
            fact.id = crate::go::semantic::facts::GoSemanticPackageId(index as u64);
        }

        self.functions
            .sort_by(|row, other| interner.compare_canonical(row.stable_key, other.stable_key));
        for (index, fact) in self.functions.iter_mut().enumerate() {
            fact.id = crate::go::semantic::facts::GoSemanticFunctionId(index as u64);
        }

        // Callsites, address-taken funcs, instantiated runtime types, and dynamic-dispatch
        // detail are whole-reachable-program harvests over the SSA program: the SAME
        // official identity legitimately recurs (a function whose address is taken from
        // two call sites; a type converted to an interface in two functions; compiler
        // synthetic method-wrappers whose internal calls carry no source position). These
        // are SET facts keyed by official identity, so an identity-duplicate row is the
        // SAME set member, not a conflict — dedup by stable key (keep first) after the
        // stable-key sort, mirroring the solver fixpoint's stable-key dedup. This keeps
        // the whole-program set honest and `validate_unique` green without dropping any
        // distinct fact (verification surfaced this on same-named interface
        // methods + shared callees). Packages/functions/method-sets are keyed by unique
        // declaration identity and are intentionally NOT deduped (a real duplicate there
        // is a genuine conflict the validator must still reject).
        self.callsites
            .sort_by(|row, other| interner.compare_canonical(row.stable_key, other.stable_key));
        self.callsites
            .dedup_by(|left, right| left.stable_key == right.stable_key);
        for (index, fact) in self.callsites.iter_mut().enumerate() {
            fact.id = crate::go::semantic::facts::GoSemanticCallsiteId(index as u64);
        }

        self.method_sets
            .sort_by(|row, other| interner.compare_canonical(row.stable_key, other.stable_key));
        for (index, fact) in self.method_sets.iter_mut().enumerate() {
            fact.id = crate::go::semantic::facts::GoSemanticMethodSetId(index as u64);
        }

        self.address_taken
            .sort_by(|row, other| interner.compare_canonical(row.stable_key, other.stable_key));
        self.address_taken
            .dedup_by(|left, right| left.stable_key == right.stable_key);
        for (index, fact) in self.address_taken.iter_mut().enumerate() {
            fact.id = crate::go::semantic::facts::GoSemanticAddressTakenId(index as u64);
        }

        self.instantiated_types
            .sort_by(|row, other| interner.compare_canonical(row.stable_key, other.stable_key));
        self.instantiated_types
            .dedup_by(|left, right| left.stable_key == right.stable_key);
        for (index, fact) in self.instantiated_types.iter_mut().enumerate() {
            fact.id = crate::go::semantic::facts::GoSemanticInstantiatedTypeId(index as u64);
        }

        self.dynamic_dispatch
            .sort_by(|row, other| interner.compare_canonical(row.stable_key, other.stable_key));
        self.dynamic_dispatch
            .dedup_by(|left, right| left.stable_key == right.stable_key);
        for (index, fact) in self.dynamic_dispatch.iter_mut().enumerate() {
            fact.id = crate::go::semantic::facts::GoSemanticDynamicDispatchId(index as u64);
        }

        self.rta_edges
            .sort_by(|row, other| interner.compare_canonical(row.stable_key, other.stable_key));
        self.rta_edges
            .dedup_by(|left, right| left.stable_key == right.stable_key);
        for (index, fact) in self.rta_edges.iter_mut().enumerate() {
            fact.id = crate::go::semantic::facts::GoSemanticRtaEdgeId(index as u64);
        }

        self.package_errors
            .sort_by(|row, other| interner.compare_canonical(row.stable_key, other.stable_key));
        for (index, fact) in self.package_errors.iter_mut().enumerate() {
            fact.id = crate::go::semantic::facts::GoSemanticPackageErrorId(index as u64);
        }
        self
    }

    /// Drop malformed RTA-signal HARVEST rows so a single bad row never nukes the whole Go
    /// fact set (FINDING B/C). The harvest families (`method_set` / `address_taken` /
    /// `instantiated_type` / `dynamic_dispatch`) are whole-reachable-program SET facts: a
    /// discriminant-less / identity-less / stable-key-less row carries no resolvable signal,
    /// so it is dropped (honest — the honest-unresolved contract tolerates a missing dispatch
    /// detail downstream) rather than failing closed and zeroing every function, callsite,
    /// method-set, and root from the VALID rows (which would drive RTA to zero edges
    /// repo-wide). The STRUCTURAL families (packages/functions/callsites) are deliberately
    /// NOT filtered here — a conflict there is a genuine declaration bug
    /// [`validate_go_semantic_output`] must still reject. Uses the same rejection predicates
    /// as the validator, so "valid" means identically in both places. Deterministic: retains
    /// source order of the surviving rows (the caller re-`normalized()`s afterward).
    ///
    /// Returns the COUNT of rows dropped so the drop is OBSERVABLE rather than silent
    /// (FIX 3): the count is surfaced as a provider diagnostic, so a systematic frontend
    /// regression (e.g. every method_set losing its stable_key → repo-wide under-resolution)
    /// fails loudly instead of being swallowed.
    fn drop_invalid_harvest_rows(mut self, interner: &StableKeyInterner) -> (Self, usize) {
        use crate::go::semantic::validate::{
            address_taken_rejection, dynamic_dispatch_rejection, instantiated_type_rejection,
            method_set_rejection, rta_edge_rejection,
        };
        let before = self.method_sets.len()
            + self.address_taken.len()
            + self.instantiated_types.len()
            + self.dynamic_dispatch.len()
            + self.rta_edges.len();
        self.method_sets
            .retain(|fact| method_set_rejection(fact, interner).is_none());
        self.address_taken
            .retain(|fact| address_taken_rejection(fact, interner).is_none());
        self.instantiated_types
            .retain(|fact| instantiated_type_rejection(fact, interner).is_none());
        self.dynamic_dispatch
            .retain(|fact| dynamic_dispatch_rejection(fact, interner).is_none());
        self.rta_edges
            .retain(|fact| rta_edge_rejection(fact, interner).is_none());
        let after = self.method_sets.len()
            + self.address_taken.len()
            + self.instantiated_types.len()
            + self.dynamic_dispatch.len()
            + self.rta_edges.len();
        let dropped = before - after;
        (self, dropped)
    }

    /// Collapse duplicate stable keys in the STRUCTURAL families (packages / functions /
    /// method_sets) keep-first, BEFORE validation, so a single duplicate row is no longer
    /// CATASTROPHIC (FIX-08). These families are keyed by unique declaration identity and are
    /// deliberately NOT deduped in [`Self::normalized`]; before this pass a single duplicate
    /// stable key reached [`crate::go::semantic::validate::validate_go_semantic_output`]'s
    /// `validate_unique` → `Err` → the provider assigned ZERO Go facts → `GoRtaInputs::from_db`
    /// read nothing → RTA derived zero edges for the ENTIRE repository. One duplicate row zeroed
    /// the whole repo's Go call graph (verified, recurring 3×: FINDING-B and FIX-07's two
    /// sub-bugs). This is the generalized net: ANY future emitter bug producing one duplicate
    /// structural key now drops-the-dup + surfaces a diagnostic instead of zeroing every fact.
    ///
    /// Stable keys are built from official Go identity (`package` / `fn.String()` / canonical
    /// type), which is unique per entity, so a same-stable-key duplicate is in practice the SAME
    /// entity emitted twice (byte-identical → keep-first is lossless). A "conflicting" duplicate
    /// (same stable_key, but the dropped row `!=` the kept row — the practically-impossible
    /// genuine identity collision) can only come from a stable-key-recipe bug; keep-first still
    /// proceeds but the caller flags it loudly.
    ///
    /// Deterministic: sort by `stable_key` then collapse equal runs keep-first (no `HashMap`
    /// reaches output). A no-op when there are no duplicates — the rows are already
    /// `stable_key`-sorted/dense-id-assigned by [`Self::normalized`] afterward, so on a clean
    /// run this is a byte-identical pass-through.
    fn collapse_duplicate_structural_keys(
        mut self,
        interner: &StableKeyInterner,
    ) -> (Self, StructuralDuplicateReport) {
        let mut report = StructuralDuplicateReport::default();
        report.packages =
            collapse_duplicate_stable_keys(&mut self.packages, &mut report.conflicting, interner);
        report.functions =
            collapse_duplicate_stable_keys(&mut self.functions, &mut report.conflicting, interner);
        report.method_sets = collapse_duplicate_stable_keys(
            &mut self.method_sets,
            &mut report.conflicting,
            interner,
        );
        (self, report)
    }
}

/// Collapse runs of equal `stable_key` keep-first in one structural family, returning the
/// count of dropped duplicate rows and OR-ing `conflicting` to true if any dropped row
/// DIFFERED (full `!=`) from the kept row of the same key (FIX-08). Sorts by `stable_key`
/// first (deterministic; `sort_by` is stable so the kept row of a run is the
/// earliest-in-source one). Generic over the three structural fact types via the small
/// [`StableKeyed`] accessor — no per-type duplication.
fn collapse_duplicate_stable_keys<T: StableKeyed + Clone + PartialEq>(
    rows: &mut Vec<T>,
    conflicting: &mut bool,
    interner: &StableKeyInterner,
) -> usize {
    rows.sort_by(|row, other| interner.compare_canonical(row.stable_key(), other.stable_key()));
    let before = rows.len();
    let mut kept: Vec<T> = Vec::with_capacity(before);
    for row in rows.drain(..) {
        match kept.last() {
            Some(previous) if previous.stable_key() == row.stable_key() => {
                // Same official identity emitted twice. Keep-first (drop `row`); flag a
                // conflict only if the dropped row DIFFERS from the kept one in its CONTENT
                // (every field except the synthetic dense `id`, which `normalized()` reassigns
                // by position and is therefore not part of official identity — comparing it
                // would spuriously flag byte-identical double-emits whenever the output was
                // already normalized, as the provider always does).
                if conflicts_ignoring_id(previous, &row) {
                    *conflicting = true;
                }
            }
            _ => kept.push(row),
        }
    }
    *rows = kept;
    before - rows.len()
}

/// True when two same-stable-key rows differ in their CONTENT (ignoring the synthetic dense
/// `id`). Clones both and zeroes their ids so the `!=` compares only official identity, not
/// the normalization-assigned position.
fn conflicts_ignoring_id<T: StableKeyed + Clone + PartialEq>(left: &T, right: &T) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    left.clear_id();
    right.clear_id();
    left != right
}

/// Minimal accessor letting [`collapse_duplicate_stable_keys`] read the discriminating
/// `stable_key` of any structural fact and neutralize the synthetic dense `id` before the
/// conflict comparison — without per-type code duplication.
trait StableKeyed {
    fn stable_key(&self) -> StableKeyId;
    /// Zero the synthetic dense id so [`conflicts_ignoring_id`] compares only official content.
    fn clear_id(&mut self);
}

impl StableKeyed for GoSemanticPackageFact {
    fn stable_key(&self) -> StableKeyId {
        self.stable_key
    }
    fn clear_id(&mut self) {
        self.id = crate::go::semantic::facts::GoSemanticPackageId(0);
    }
}

impl StableKeyed for GoSemanticFunctionFact {
    fn stable_key(&self) -> StableKeyId {
        self.stable_key
    }
    fn clear_id(&mut self) {
        self.id = crate::go::semantic::facts::GoSemanticFunctionId(0);
    }
}

impl StableKeyed for GoSemanticMethodSetFact {
    fn stable_key(&self) -> StableKeyId {
        self.stable_key
    }
    fn clear_id(&mut self) {
        self.id = crate::go::semantic::facts::GoSemanticMethodSetId(0);
    }
}

#[derive(Debug, Clone, Default)]
pub struct GoSemanticStore {
    output: GoSemanticFactsOutput,
    /// Count of malformed RTA-signal harvest rows dropped while building this store
    /// (FIX 3). The provider surfaces a diagnostic when this is non-zero, so a systematic
    /// frontend regression is observable rather than silently under-resolving.
    dropped_harvest_rows: usize,
    /// Per-family count of duplicate STRUCTURAL rows collapsed keep-first while building this
    /// store (FIX-08), plus whether any collapse was conflicting. The provider surfaces a
    /// diagnostic when anything was collapsed, so an emitter bug producing a duplicate
    /// structural key is observable instead of catastrophically zeroing all Go RTA.
    structural_duplicates: StructuralDuplicateReport,
}

impl GoSemanticStore {
    pub fn from_output(
        output: GoSemanticFactsOutput,
        interner: &StableKeyInterner,
    ) -> Result<Self, AnalysisError> {
        // Defense in depth, in order:
        //   1. Collapse duplicate STRUCTURAL stable keys (packages/functions/method_sets)
        //      keep-first (FIX-08) so a single duplicate row never reaches `validate_unique`
        //      and zeroes every Go fact repo-wide. Stable keys are unique per official Go
        //      identity, so a duplicate is the SAME entity emitted twice (keep-first is
        //      lossless); a conflicting (differing-row) duplicate is flagged but still kept.
        //   2. Drop malformed RTA-signal HARVEST rows (FINDING B/C) so a single bad row never
        //      zeroes the fact set either.
        //   3. Normalize (re-sort + dense-id over the survivors).
        //   4. Validate as a final safety net — it no longer fires for the collapsed
        //      structural dups (they are gone) but still guards genuine referential/path/other
        //      conditions.
        // Both resilience counts are retained so the drops are observable (FIX 3 / FIX-08).
        let (output, structural_duplicates) = output.collapse_duplicate_structural_keys(interner);
        let (output, dropped_harvest_rows) = output.drop_invalid_harvest_rows(interner);
        let output = output.normalized(interner);
        validate_go_semantic_output(&output, interner)?;
        Ok(Self {
            output,
            dropped_harvest_rows,
            structural_duplicates,
        })
    }

    pub fn output(&self) -> &GoSemanticFactsOutput {
        &self.output
    }

    /// The number of malformed RTA-signal harvest rows dropped while building this store
    /// (FIX 3). Zero on a clean frontend run; non-zero surfaces a provider diagnostic.
    pub fn dropped_harvest_rows(&self) -> usize {
        self.dropped_harvest_rows
    }

    /// Per-family duplicate STRUCTURAL rows collapsed keep-first while building this store
    /// (FIX-08), plus the conflicting flag. All zero / not-conflicting on a clean run.
    pub fn structural_duplicates(&self) -> StructuralDuplicateReport {
        self.structural_duplicates
    }

    /// The combined resilience report (dropped harvest rows + collapsed structural duplicates)
    /// the provider surfaces as diagnostics (FIX-08).
    pub fn report(&self) -> GoSemanticStoreReport {
        GoSemanticStoreReport {
            dropped_harvest_rows: self.dropped_harvest_rows,
            structural_duplicates: self.structural_duplicates,
        }
    }
}

/// Registry key for [`GoSemanticStore`] in the host fact-store map.
pub const GO_SEMANTIC_STORE_FAMILY: FactFamily = FactFamily::GoSemantic;

impl FactStore for GoSemanticStore {
    fn family(&self) -> FactFamily {
        FactFamily::GoSemantic
    }

    fn clear(&mut self) {
        *self = GoSemanticStore::default();
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn FactStore> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::go::semantic::facts::{
        GoSemanticAddressTakenFact, GoSemanticAddressTakenId, GoSemanticCallStatus,
        GoSemanticCallsiteFact, GoSemanticCallsiteId, GoSemanticDynamicDispatchFact,
        GoSemanticDynamicDispatchId, GoSemanticFunctionId, GoSemanticFunctionKind,
        GoSemanticInstantiatedTypeFact, GoSemanticInstantiatedTypeId, GoSemanticMethodSetFact,
        GoSemanticMethodSetId, GoSemanticPackageFact, GoSemanticPackageId,
    };
    use crate::internal_core::{FileId, Span};

    #[test]
    fn store_normalizes_by_stable_key_before_dense_id_assignment() {
        let interner = StableKeyInterner::default();
        let output = GoSemanticFactsOutput {
            functions: vec![
                GoSemanticFunctionFact {
                    id: GoSemanticFunctionId(99),
                    stable_key: interner.intern("b"),
                    package_id: "pkg".to_string(),
                    package_path: "example.com/pkg".to_string(),
                    name: "B".to_string(),
                    qualified: "example.com/pkg.B".to_string(),
                    signature: "()".to_string(),
                    kind: GoSemanticFunctionKind::Function,
                    receiver: None,
                    relative_file: None,
                    file: None,
                    span: None,
                },
                GoSemanticFunctionFact {
                    id: GoSemanticFunctionId(42),
                    stable_key: interner.intern("a"),
                    package_id: "pkg".to_string(),
                    package_path: "example.com/pkg".to_string(),
                    name: "A".to_string(),
                    qualified: "example.com/pkg.A".to_string(),
                    signature: "()".to_string(),
                    kind: GoSemanticFunctionKind::Function,
                    receiver: None,
                    relative_file: None,
                    file: None,
                    span: None,
                },
            ],
            ..GoSemanticFactsOutput::default()
        };

        let store = GoSemanticStore::from_output(output, &interner).expect("store validates");
        assert_eq!(
            interner
                .resolve(store.output.functions[0].stable_key)
                .as_ref(),
            "a"
        );
        assert_eq!(store.output.functions[0].id, GoSemanticFunctionId(0));
        assert_eq!(
            interner
                .resolve(store.output.functions[1].stable_key)
                .as_ref(),
            "b"
        );
        assert_eq!(store.output.functions[1].id, GoSemanticFunctionId(1));
    }

    fn valid_function(interner: &StableKeyInterner) -> GoSemanticFunctionFact {
        GoSemanticFunctionFact {
            id: GoSemanticFunctionId(0),
            stable_key: interner.intern("fn|main"),
            package_id: "pkg".to_string(),
            package_path: "example.com/pkg".to_string(),
            name: "main".to_string(),
            qualified: "example.com/pkg.main".to_string(),
            signature: "func()".to_string(),
            kind: GoSemanticFunctionKind::Function,
            receiver: None,
            relative_file: None,
            file: None,
            span: None,
        }
    }

    fn valid_callsite(interner: &StableKeyInterner) -> GoSemanticCallsiteFact {
        GoSemanticCallsiteFact {
            id: GoSemanticCallsiteId(0),
            stable_key: interner.intern("cs|main"),
            package_id: "pkg".to_string(),
            package_path: "example.com/pkg".to_string(),
            caller: "example.com/pkg.main".to_string(),
            static_callee: None,
            status: GoSemanticCallStatus::UnresolvedDynamic,
            reason: None,
            relative_file: None,
            file: None,
            span: None,
        }
    }

    #[test]
    fn from_output_drops_a_single_invalid_harvest_row_and_keeps_valid_facts() {
        let interner = StableKeyInterner::default();
        // FINDING B: a single malformed RTA-signal harvest row must NOT nuke the entire Go
        // fact set. Before the fix, one rejected harvest row made `from_output` return Err,
        // `replace_go_semantic_facts` failed, and the DB was left with ZERO Go facts — RTA
        // then derived zero edges repo-wide. The harvest families are row-resilient: the
        // offending row is dropped and the valid functions/callsites SURVIVE.
        let output = GoSemanticFactsOutput {
            functions: vec![valid_function(&interner)],
            callsites: vec![valid_callsite(&interner)],
            // A discriminant-less dynamic_dispatch row (empty interface_type AND empty
            // signature) — the WR-02/WR-03 honest-discriminant guard rejects it. It must be
            // DROPPED, not fatal.
            dynamic_dispatch: vec![GoSemanticDynamicDispatchFact {
                id: GoSemanticDynamicDispatchId(0),
                stable_key: interner.intern("dd|bad"),
                package_id: "pkg".to_string(),
                package_path: "example.com/pkg".to_string(),
                caller: "example.com/pkg.main".to_string(),
                callsite_stable_key: interner.intern("cs|main"),
                interface_type: None,
                method: None,
                signature: None,
            }],
            ..GoSemanticFactsOutput::default()
        };

        let store = GoSemanticStore::from_output(output, &interner)
            .expect("one bad harvest row must not fail the whole output");
        // The valid structural facts survive (NOT zeroed).
        assert_eq!(store.output().functions.len(), 1);
        assert_eq!(
            store.output().functions[0].qualified,
            "example.com/pkg.main"
        );
        assert_eq!(store.output().callsites.len(), 1);
        // The bad harvest row was dropped.
        assert!(store.output().dynamic_dispatch.is_empty());
    }

    #[test]
    fn from_output_counts_dropped_invalid_harvest_rows() {
        let interner = StableKeyInterner::default();
        // FIX 3 (LOW): dropping invalid harvest rows must be OBSERVABLE, not silent — a
        // systematic frontend regression (e.g. every method_set loses its stable_key)
        // would otherwise be swallowed into repo-wide under-resolution. The store exposes
        // a count the provider surfaces as a diagnostic. Here THREE rows are invalid (a
        // discriminant-less dynamic_dispatch, an empty-identity address_taken, and a
        // stable-key-less method_set); valid siblings survive.
        let output = GoSemanticFactsOutput {
            functions: vec![valid_function(&interner)],
            dynamic_dispatch: vec![GoSemanticDynamicDispatchFact {
                id: GoSemanticDynamicDispatchId(0),
                stable_key: interner.intern("dd|bad"),
                package_id: "pkg".to_string(),
                package_path: "example.com/pkg".to_string(),
                caller: "example.com/pkg.main".to_string(),
                callsite_stable_key: interner.intern("cs|main"),
                interface_type: None,
                method: None,
                signature: None,
            }],
            address_taken: vec![GoSemanticAddressTakenFact {
                id: GoSemanticAddressTakenId(0),
                stable_key: interner.intern("at|bad"),
                package_id: "pkg".to_string(),
                package_path: "example.com/pkg".to_string(),
                function: String::new(),
            }],
            method_sets: vec![GoSemanticMethodSetFact {
                id: GoSemanticMethodSetId(0),
                stable_key: interner.intern(String::new()),
                package_id: "pkg".to_string(),
                package_path: "example.com/pkg".to_string(),
                type_name: "example.com/pkg.U".to_string(),
                methods: vec!["N".to_string()],
            }],
            ..GoSemanticFactsOutput::default()
        };

        let store = GoSemanticStore::from_output(output, &interner)
            .expect("bad harvest rows must be dropped, not fatal");
        // The count is the observable signal: exactly three invalid rows were dropped.
        assert_eq!(store.dropped_harvest_rows(), 3);
        // The valid structural fact still survives.
        assert_eq!(store.output().functions.len(), 1);
    }

    #[test]
    fn from_output_reports_zero_dropped_rows_when_all_valid() {
        let interner = StableKeyInterner::default();
        // The observable count is zero when nothing is dropped (so the provider stays
        // quiet on a clean frontend run).
        let output = GoSemanticFactsOutput {
            functions: vec![valid_function(&interner)],
            callsites: vec![valid_callsite(&interner)],
            ..GoSemanticFactsOutput::default()
        };
        let store = GoSemanticStore::from_output(output, &interner).expect("valid output stores");
        assert_eq!(store.dropped_harvest_rows(), 0);
    }

    #[test]
    fn from_output_drops_invalid_rows_across_all_harvest_families_keeping_valid_ones() {
        let interner = StableKeyInterner::default();
        // FINDING B/C: every harvest family is row-resilient. A bad row in each family
        // (empty function identity / empty type_name / no discriminant / missing stable_key)
        // is dropped while a VALID sibling in the same family survives.
        let output = GoSemanticFactsOutput {
            functions: vec![valid_function(&interner)],
            address_taken: vec![
                GoSemanticAddressTakenFact {
                    id: GoSemanticAddressTakenId(0),
                    stable_key: interner.intern("at|good"),
                    package_id: "pkg".to_string(),
                    package_path: "example.com/pkg".to_string(),
                    function: "example.com/pkg.handler".to_string(),
                },
                // Empty function identity → dropped.
                GoSemanticAddressTakenFact {
                    id: GoSemanticAddressTakenId(1),
                    stable_key: interner.intern("at|bad"),
                    package_id: "pkg".to_string(),
                    package_path: "example.com/pkg".to_string(),
                    function: String::new(),
                },
            ],
            instantiated_types: vec![
                GoSemanticInstantiatedTypeFact {
                    id: GoSemanticInstantiatedTypeId(0),
                    stable_key: interner.intern("it|good"),
                    package_id: "pkg".to_string(),
                    package_path: "example.com/pkg".to_string(),
                    type_name: "example.com/pkg.T".to_string(),
                },
                // Empty type_name → dropped.
                GoSemanticInstantiatedTypeFact {
                    id: GoSemanticInstantiatedTypeId(1),
                    stable_key: interner.intern("it|bad"),
                    package_id: "pkg".to_string(),
                    package_path: "example.com/pkg".to_string(),
                    type_name: String::new(),
                },
            ],
            method_sets: vec![
                GoSemanticMethodSetFact {
                    id: GoSemanticMethodSetId(0),
                    stable_key: interner.intern("ms|good"),
                    package_id: "pkg".to_string(),
                    package_path: "example.com/pkg".to_string(),
                    type_name: "example.com/pkg.T".to_string(),
                    methods: vec!["M".to_string()],
                },
                // Missing stable_key (the WR-03 / FINDING C condition) → dropped, not fatal.
                GoSemanticMethodSetFact {
                    id: GoSemanticMethodSetId(1),
                    stable_key: interner.intern(String::new()),
                    package_id: "pkg".to_string(),
                    package_path: "example.com/pkg".to_string(),
                    type_name: "example.com/pkg.U".to_string(),
                    methods: vec!["N".to_string()],
                },
            ],
            ..GoSemanticFactsOutput::default()
        };

        let store = GoSemanticStore::from_output(output, &interner)
            .expect("bad harvest rows must be dropped, not fatal");
        assert_eq!(store.output().functions.len(), 1);
        assert_eq!(store.output().address_taken.len(), 1);
        assert_eq!(
            store.output().address_taken[0].function,
            "example.com/pkg.handler"
        );
        assert_eq!(store.output().instantiated_types.len(), 1);
        assert_eq!(
            store.output().instantiated_types[0].type_name,
            "example.com/pkg.T"
        );
        // The valid method_set survives; the stable-key-less one is dropped.
        assert_eq!(store.output().method_sets.len(), 1);
        assert_eq!(store.output().method_sets[0].type_name, "example.com/pkg.T");
    }

    #[test]
    fn from_output_collapses_duplicate_function_stable_key_keeping_first_not_zeroing() {
        let interner = StableKeyInterner::default();
        // FIX-08 (CATASTROPHIC, recurring 3×): a SINGLE duplicate stable key in a STRUCTURAL
        // family (packages/functions/method_sets) used to reach `validate_unique` → Err →
        // the provider assigned ZERO Go facts → `GoRtaInputs::from_db` read nothing → RTA
        // derived zero edges for the ENTIRE repo. One duplicate row zeroed the whole repo's
        // call graph. Stable keys come from official Go identity (unique per entity), so a
        // same-stable-key duplicate is the SAME entity emitted twice (byte-identical →
        // keep-first is lossless). The store now collapses such duplicates keep-first BEFORE
        // validation, so the valid facts SURVIVE and the drop is reported — NOT a fatal Err.
        let mut a = valid_function(&interner);
        let mut b = valid_function(&interner);
        a.stable_key = interner.intern("dup");
        b.stable_key = interner.intern("dup");
        let output = GoSemanticFactsOutput {
            functions: vec![a, b],
            callsites: vec![valid_callsite(&interner)],
            ..GoSemanticFactsOutput::default()
        };
        let store = GoSemanticStore::from_output(output, &interner)
            .expect("a duplicate structural key must NOT zero the whole Go fact set");
        // The valid facts survive; the function is present EXACTLY ONCE (keep-first).
        assert_eq!(store.output().functions.len(), 1);
        assert_eq!(
            interner
                .resolve(store.output().functions[0].stable_key)
                .as_ref(),
            "dup"
        );
        assert_eq!(store.output().callsites.len(), 1);
        // Exactly one duplicate function row was collapsed; it was byte-identical (benign).
        assert_eq!(store.structural_duplicates().functions, 1);
        assert!(!store.structural_duplicates().conflicting);
    }

    #[test]
    fn from_output_collapses_duplicate_method_set_stable_key_keeping_first() {
        let interner = StableKeyInterner::default();
        // FIX-08: same catastrophic guard for the method_set structural family (FIX-07's
        // alias↔generic instantiation double-emit produced exactly this).
        let method_set = GoSemanticMethodSetFact {
            id: GoSemanticMethodSetId(0),
            stable_key: interner.intern("ms|example.com/pkg.Box[int]"),
            package_id: "pkg".to_string(),
            package_path: "example.com/pkg".to_string(),
            type_name: "example.com/pkg.Box[int]".to_string(),
            methods: vec!["Speak".to_string()],
        };
        let output = GoSemanticFactsOutput {
            functions: vec![valid_function(&interner)],
            method_sets: vec![method_set.clone(), method_set],
            ..GoSemanticFactsOutput::default()
        };
        let store = GoSemanticStore::from_output(output, &interner)
            .expect("a duplicate method_set key must NOT zero the whole Go fact set");
        assert_eq!(store.output().functions.len(), 1);
        assert_eq!(store.output().method_sets.len(), 1);
        assert_eq!(
            store.output().method_sets[0].type_name,
            "example.com/pkg.Box[int]"
        );
        assert_eq!(store.structural_duplicates().method_sets, 1);
        assert!(!store.structural_duplicates().conflicting);
    }

    #[test]
    fn from_output_collapses_duplicate_package_stable_key_keeping_first() {
        let interner = StableKeyInterner::default();
        // FIX-08: same catastrophic guard for the package structural family.
        let package = GoSemanticPackageFact {
            id: GoSemanticPackageId(0),
            stable_key: interner.intern("pkg|example.com/pkg"),
            package_id: "example.com/pkg".to_string(),
            package_path: "example.com/pkg".to_string(),
            package_name: "pkg".to_string(),
            module_path: "example.com".to_string(),
            files: vec!["pkg/file.go".to_string()],
        };
        let output = GoSemanticFactsOutput {
            packages: vec![package.clone(), package],
            functions: vec![valid_function(&interner)],
            ..GoSemanticFactsOutput::default()
        };
        let store = GoSemanticStore::from_output(output, &interner)
            .expect("a duplicate package key must NOT zero the whole Go fact set");
        assert_eq!(store.output().packages.len(), 1);
        assert_eq!(store.output().functions.len(), 1);
        assert_eq!(store.structural_duplicates().packages, 1);
        assert!(!store.structural_duplicates().conflicting);
    }

    #[test]
    fn from_output_flags_conflicting_duplicate_function_key_but_keeps_facts() {
        let interner = StableKeyInterner::default();
        // FIX-08: the "conflicting" case — two rows sharing a stable_key but DIFFERING (here a
        // different span). This can only arise from a stable-key-recipe bug (official Go
        // identity is unique). Keep-first still proceeds (far better than zeroing all Go RTA),
        // but the conflict must be flagged LOUDLY so the recipe bug surfaces.
        let mut a = valid_function(&interner);
        let mut b = valid_function(&interner);
        a.stable_key = interner.intern("dup");
        b.stable_key = interner.intern("dup");
        // Same stable_key, different span → conflicting duplicate.
        a.span = Some(Span::new(FileId::from_raw(0), 10, 20, 1, 1, 1, 11));
        b.span = Some(Span::new(FileId::from_raw(0), 30, 40, 2, 1, 2, 11));
        let output = GoSemanticFactsOutput {
            functions: vec![a, b],
            ..GoSemanticFactsOutput::default()
        };
        let store = GoSemanticStore::from_output(output, &interner)
            .expect("a conflicting duplicate must keep-first, NOT zero the Go fact set");
        // Keep-first: exactly one function survives (the first, span 10..20).
        assert_eq!(store.output().functions.len(), 1);
        assert_eq!(
            store.output().functions[0]
                .span
                .as_ref()
                .map(|s| s.start_byte),
            Some(10)
        );
        // One duplicate was collapsed AND it was conflicting (rows differed).
        assert_eq!(store.structural_duplicates().functions, 1);
        assert!(
            store.structural_duplicates().conflicting,
            "a same-key/differing-row duplicate must be flagged as conflicting"
        );
    }

    #[test]
    fn from_output_reports_no_structural_duplicates_when_all_unique() {
        let interner = StableKeyInterner::default();
        // The resilience pass is a no-op on a clean run: zero collapsed, not conflicting.
        let output = GoSemanticFactsOutput {
            functions: vec![valid_function(&interner)],
            callsites: vec![valid_callsite(&interner)],
            ..GoSemanticFactsOutput::default()
        };
        let store = GoSemanticStore::from_output(output, &interner).expect("valid output stores");
        let report = store.structural_duplicates();
        assert_eq!(report.packages, 0);
        assert_eq!(report.functions, 0);
        assert_eq!(report.method_sets, 0);
        assert!(!report.conflicting);
    }

    #[test]
    fn normalized_dedups_identity_duplicate_set_facts_keeping_first() {
        let interner = StableKeyInterner::default();
        use crate::go::semantic::facts::{GoSemanticAddressTakenFact, GoSemanticAddressTakenId};

        // The whole-reachable-program harvests (address-taken / instantiated / callsite /
        // dynamic-dispatch) legitimately produce the SAME official identity more than once:
        // a function whose address is GENUINELY taken as a value from two distinct sites
        // (e.g. `f := handler` in two places, or `handler` stored into two slices) yields
        // two address-taken rows for the same `ssa.Function.String()`. These are SET facts
        // keyed by official identity, so an identity-duplicate row is the SAME member and
        // must dedup (keep first) — NOT be rejected by `validate_unique`. Verification
        // surfaced this. (We deliberately use a genuine value-use function, not
        // a statically-called one like `fmt.Println`: FINDING 2 establishes that a
        // statically-called function is NOT address-taken at all.)
        let output = GoSemanticFactsOutput {
            address_taken: vec![
                GoSemanticAddressTakenFact {
                    id: GoSemanticAddressTakenId(7),
                    stable_key: interner.intern("at|example.com/pkg.handler"),
                    package_id: "pkg".to_string(),
                    package_path: "example.com/pkg".to_string(),
                    function: "example.com/pkg.handler".to_string(),
                },
                GoSemanticAddressTakenFact {
                    id: GoSemanticAddressTakenId(9),
                    stable_key: interner.intern("at|example.com/pkg.handler"),
                    package_id: "pkg".to_string(),
                    package_path: "example.com/pkg".to_string(),
                    function: "example.com/pkg.handler".to_string(),
                },
            ],
            ..GoSemanticFactsOutput::default()
        };

        // Without dedup this would fail `validate_unique`; with dedup it stores one row.
        let store =
            GoSemanticStore::from_output(output, &interner).expect("dedup keeps the set valid");
        assert_eq!(store.output().address_taken.len(), 1);
        assert_eq!(
            store.output().address_taken[0].function,
            "example.com/pkg.handler"
        );
        assert_eq!(
            store.output().address_taken[0].id,
            GoSemanticAddressTakenId(0)
        );
    }
}
