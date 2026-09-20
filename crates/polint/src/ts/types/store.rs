use std::any::Any;
use std::collections::BTreeSet;

use crate::analysis_api::{FactFamily, FactStore};
use crate::internal_core::{StableKeyId, StableKeyInterner};
use crate::ts::error::AnalysisError;
use crate::ts::types::facts::{
    TsTypeCallableFact, TsTypeCallableId, TsTypeCalleeFact, TsTypeCalleeId, TsTypeCallsiteFact,
    TsTypeCallsiteId, TsTypeFileDensityFact, TsTypeFileDensityId, TsTypeProjectErrorFact,
    TsTypeProjectErrorId, TsTypeProjectFact, TsTypeProjectId, TsTypeReceiverFact, TsTypeReceiverId,
};
use crate::ts::types::validate::validate_ts_types_output;

pub(crate) const TS_TYPES_PROVIDER_ID: &str = "polint.ts.types";

/// Registry key for [`TsTypesStore`] in the host fact-store map.
pub(crate) const TS_TYPES_STORE_FAMILY: FactFamily = FactFamily::TsTypes;

/// Rows dropped while building the store, by reason.
///
/// A malformed row is dropped and counted rather than rejecting the whole
/// output: one bad row from an emitter regression must not zero every typed
/// edge in the repository, which is exactly the failure the Go tier had to fix
/// twice. The counts are surfaced as provider diagnostics so the regression is
/// loud instead of catastrophic.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TsTypesStoreReport {
    /// Rows whose stable key was missing or duplicated.
    pub(crate) dropped_rows: usize,
    /// Callee rows whose call site is not in this output.
    pub(crate) dangling_callees: usize,
}

impl TsTypesStoreReport {
    pub(crate) fn total(&self) -> usize {
        self.dropped_rows + self.dangling_callees
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TsTypesFactsOutput {
    pub(crate) projects: Vec<TsTypeProjectFact>,
    pub(crate) callables: Vec<TsTypeCallableFact>,
    pub(crate) callsites: Vec<TsTypeCallsiteFact>,
    pub(crate) callees: Vec<TsTypeCalleeFact>,
    pub(crate) receivers: Vec<TsTypeReceiverFact>,
    pub(crate) file_densities: Vec<TsTypeFileDensityFact>,
    pub(crate) project_errors: Vec<TsTypeProjectErrorFact>,
}

impl TsTypesFactsOutput {
    pub(crate) fn is_empty(&self) -> bool {
        self.projects.is_empty()
            && self.callables.is_empty()
            && self.callsites.is_empty()
            && self.callees.is_empty()
            && self.receivers.is_empty()
            && self.file_densities.is_empty()
            && self.project_errors.is_empty()
    }

    /// Sorts every family by stable key and reassigns dense ids.
    ///
    /// Row order on the wire follows the sidecar's walk, which is stable for a
    /// given program but is not the order the rest of the engine keys on.
    pub(crate) fn normalized(mut self, interner: &StableKeyInterner) -> Self {
        let key = |id: StableKeyId| interner.resolve(id);
        self.projects.sort_by(|left, right| {
            key(left.stable_key)
                .cmp(&key(right.stable_key))
                .then_with(|| left.project.cmp(&right.project))
        });
        self.callables.sort_by_key(|left| key(left.stable_key));
        self.callsites.sort_by_key(|left| key(left.stable_key));
        self.callees.sort_by_key(|left| key(left.stable_key));
        self.receivers.sort_by_key(|left| key(left.stable_key));
        self.file_densities.sort_by_key(|left| key(left.stable_key));
        self.project_errors.sort_by_key(|left| key(left.stable_key));

        for (index, row) in self.projects.iter_mut().enumerate() {
            row.id = TsTypeProjectId(index as u64);
        }
        for (index, row) in self.callables.iter_mut().enumerate() {
            row.id = TsTypeCallableId(index as u64);
        }
        for (index, row) in self.callsites.iter_mut().enumerate() {
            row.id = TsTypeCallsiteId(index as u64);
        }
        for (index, row) in self.callees.iter_mut().enumerate() {
            row.id = TsTypeCalleeId(index as u64);
        }
        for (index, row) in self.receivers.iter_mut().enumerate() {
            row.id = TsTypeReceiverId(index as u64);
        }
        for (index, row) in self.file_densities.iter_mut().enumerate() {
            row.id = TsTypeFileDensityId(index as u64);
        }
        for (index, row) in self.project_errors.iter_mut().enumerate() {
            row.id = TsTypeProjectErrorId(index as u64);
        }
        self
    }

    /// Drops rows that cannot be keyed or joined, counting each drop.
    fn drop_unusable_rows(mut self, interner: &StableKeyInterner) -> (Self, TsTypesStoreReport) {
        let mut report = TsTypesStoreReport::default();
        let empty = interner.intern("");

        let keep_keyed = |seen: &mut BTreeSet<String>, stable_key: StableKeyId| -> bool {
            if stable_key == empty {
                return false;
            }
            seen.insert(interner.resolve(stable_key).to_string())
        };

        let mut project_keys = BTreeSet::new();
        let before = self.projects.len();
        self.projects
            .retain(|row| keep_keyed(&mut project_keys, row.stable_key));
        report.dropped_rows += before - self.projects.len();

        let mut callable_keys = BTreeSet::new();
        let before = self.callables.len();
        self.callables
            .retain(|row| keep_keyed(&mut callable_keys, row.stable_key));
        report.dropped_rows += before - self.callables.len();

        let mut callsite_keys = BTreeSet::new();
        let before = self.callsites.len();
        self.callsites
            .retain(|row| keep_keyed(&mut callsite_keys, row.stable_key));
        report.dropped_rows += before - self.callsites.len();

        let mut callee_keys = BTreeSet::new();
        let before = self.callees.len();
        self.callees
            .retain(|row| keep_keyed(&mut callee_keys, row.stable_key));
        report.dropped_rows += before - self.callees.len();

        // A callee whose call site was dropped, or that names a site the
        // sidecar never emitted, has nothing to attach an edge to.
        let live_callsites = self
            .callsites
            .iter()
            .map(|row| interner.resolve(row.stable_key).to_string())
            .collect::<BTreeSet<_>>();
        let before = self.callees.len();
        self.callees.retain(|row| {
            live_callsites.contains(interner.resolve(row.callsite_stable_key).as_ref())
        });
        report.dangling_callees += before - self.callees.len();

        let mut receiver_keys = BTreeSet::new();
        let before = self.receivers.len();
        self.receivers
            .retain(|row| keep_keyed(&mut receiver_keys, row.stable_key));
        report.dropped_rows += before - self.receivers.len();

        let mut density_keys = BTreeSet::new();
        let before = self.file_densities.len();
        self.file_densities
            .retain(|row| keep_keyed(&mut density_keys, row.stable_key));
        report.dropped_rows += before - self.file_densities.len();

        let mut error_keys = BTreeSet::new();
        let before = self.project_errors.len();
        self.project_errors
            .retain(|row| keep_keyed(&mut error_keys, row.stable_key));
        report.dropped_rows += before - self.project_errors.len();

        (self, report)
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TsTypesStore {
    output: TsTypesFactsOutput,
    report: TsTypesStoreReport,
}

impl TsTypesStore {
    pub(crate) fn from_output(
        output: TsTypesFactsOutput,
        interner: &StableKeyInterner,
    ) -> Result<Self, AnalysisError> {
        let (output, report) = output.drop_unusable_rows(interner);
        let output = output.normalized(interner);
        validate_ts_types_output(&output, interner)?;
        Ok(Self { output, report })
    }

    pub(crate) fn output(&self) -> &TsTypesFactsOutput {
        &self.output
    }

    pub(crate) fn report(&self) -> TsTypesStoreReport {
        self.report
    }
}

impl FactStore for TsTypesStore {
    fn family(&self) -> FactFamily {
        FactFamily::TsTypes
    }

    fn clear(&mut self) {
        *self = TsTypesStore::default();
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
    use crate::ts::types::facts::{TsTypeCallStatus, TsTypeDispatch};

    fn callsite(interner: &StableKeyInterner, key: &str) -> TsTypeCallsiteFact {
        TsTypeCallsiteFact {
            id: TsTypeCallsiteId(0),
            stable_key: interner.intern(key),
            project: "tsconfig.json".to_string(),
            callsite: key.to_string(),
            enclosing: None,
            call_kind: "call".to_string(),
            status: TsTypeCallStatus::Resolved,
            reason: None,
            relative_file: Some("src/app.ts".to_string()),
            file: None,
            span: None,
        }
    }

    fn callee(interner: &StableKeyInterner, key: &str, site: &str) -> TsTypeCalleeFact {
        TsTypeCalleeFact {
            id: TsTypeCalleeId(0),
            stable_key: interner.intern(key),
            project: "tsconfig.json".to_string(),
            callsite_stable_key: interner.intern(site),
            callable: Some("src/app.ts:10:run".to_string()),
            external: None,
            dispatch: TsTypeDispatch::Declared,
            relative_file: Some("src/app.ts".to_string()),
            file: None,
            span: None,
        }
    }

    #[test]
    fn rows_are_sorted_by_stable_key_and_given_dense_ids() {
        let interner = StableKeyInterner::default();
        let output = TsTypesFactsOutput {
            callsites: vec![callsite(&interner, "b"), callsite(&interner, "a")],
            ..TsTypesFactsOutput::default()
        };

        let store = TsTypesStore::from_output(output, &interner).expect("valid output");

        assert_eq!(
            store
                .output()
                .callsites
                .iter()
                .map(|row| interner.resolve(row.stable_key).to_string())
                .collect::<Vec<_>>(),
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(store.output().callsites[0].id, TsTypeCallsiteId(0));
        assert_eq!(store.output().callsites[1].id, TsTypeCallsiteId(1));
    }

    #[test]
    fn a_duplicate_stable_key_is_collapsed_rather_than_rejecting_every_row() {
        let interner = StableKeyInterner::default();
        let output = TsTypesFactsOutput {
            callsites: vec![callsite(&interner, "a"), callsite(&interner, "a")],
            ..TsTypesFactsOutput::default()
        };

        let store = TsTypesStore::from_output(output, &interner).expect("duplicates collapse");

        assert_eq!(store.output().callsites.len(), 1);
        assert_eq!(store.report().dropped_rows, 1);
    }

    #[test]
    fn a_row_with_no_stable_key_is_dropped_and_counted() {
        let interner = StableKeyInterner::default();
        let output = TsTypesFactsOutput {
            callsites: vec![callsite(&interner, ""), callsite(&interner, "a")],
            ..TsTypesFactsOutput::default()
        };

        let store = TsTypesStore::from_output(output, &interner).expect("keyless row drops");

        assert_eq!(store.output().callsites.len(), 1);
        assert_eq!(store.report().dropped_rows, 1);
    }

    #[test]
    fn a_callee_naming_an_absent_call_site_is_dropped_and_counted_separately() {
        let interner = StableKeyInterner::default();
        let output = TsTypesFactsOutput {
            callsites: vec![callsite(&interner, "site")],
            callees: vec![
                callee(&interner, "kept", "site"),
                callee(&interner, "dangling", "missing"),
            ],
            ..TsTypesFactsOutput::default()
        };

        let store = TsTypesStore::from_output(output, &interner).expect("dangling callee drops");

        assert_eq!(store.output().callees.len(), 1);
        assert_eq!(store.report().dangling_callees, 1);
        assert_eq!(store.report().dropped_rows, 0);
    }

    #[test]
    fn a_clean_output_reports_nothing_dropped() {
        let interner = StableKeyInterner::default();
        let output = TsTypesFactsOutput {
            callsites: vec![callsite(&interner, "site")],
            callees: vec![callee(&interner, "kept", "site")],
            ..TsTypesFactsOutput::default()
        };

        let store = TsTypesStore::from_output(output, &interner).expect("clean output");

        assert_eq!(store.report().total(), 0);
    }

    #[test]
    fn clearing_the_store_leaves_no_rows_behind() {
        let interner = StableKeyInterner::default();
        let mut store = TsTypesStore::from_output(
            TsTypesFactsOutput {
                callsites: vec![callsite(&interner, "site")],
                ..TsTypesFactsOutput::default()
            },
            &interner,
        )
        .expect("valid output");

        FactStore::clear(&mut store);

        assert!(store.output().is_empty());
    }
}
