use std::collections::{BTreeMap, BTreeSet};

use crate::analysis_neutral::calls::facts::{
    CallSiteFact, CallTargetFact, CallTargetStatus, UnresolvedCallFact, UnresolvedCallReason,
};
use crate::analysis_neutral::error::AnalysisError;
use crate::analysis_neutral::ids::CallSiteId;
use crate::internal_core::{FunctionId, StableKeyInterner, SymbolId};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallOutput {
    pub sites: Vec<CallSiteFact>,
    pub targets: Vec<CallTargetFact>,
    pub unresolved: Vec<UnresolvedCallFact>,
}

impl CallOutput {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn normalized(mut self, interner: &StableKeyInterner) -> Self {
        self.sites.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.id.cmp(&right.id))
        });
        self.targets.sort_by(|left, right| {
            left.site
                .cmp(&right.site)
                .then_with(|| interner.compare_canonical(left.stable_key, right.stable_key))
                .then_with(|| left.id.cmp(&right.id))
        });
        self.unresolved.sort_by(|left, right| {
            left.site
                .cmp(&right.site)
                .then_with(|| left.reason.cmp(&right.reason))
                .then_with(|| left.status.cmp(&right.status))
                .then_with(|| interner.compare_canonical(left.stable_key, right.stable_key))
        });
        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct CallStore {
    output: CallOutput,
    sites_by_caller: BTreeMap<FunctionId, Vec<usize>>,
    targets_by_site: BTreeMap<CallSiteId, Vec<usize>>,
    outgoing_by_function: BTreeMap<FunctionId, Vec<usize>>,
    outgoing_by_symbol: BTreeMap<SymbolId, Vec<usize>>,
    incoming_by_symbol: BTreeMap<SymbolId, Vec<usize>>,
    incoming_by_function: BTreeMap<FunctionId, Vec<usize>>,
    unresolved_by_reason: BTreeMap<UnresolvedCallReason, Vec<usize>>,
    unresolved_by_status: BTreeMap<CallTargetStatus, Vec<usize>>,
}

impl CallStore {
    pub fn from_output(
        output: CallOutput,
        interner: &StableKeyInterner,
    ) -> Result<Self, AnalysisError> {
        let output = output.normalized(interner);
        let mut site_ids = BTreeSet::new();
        let mut site_owner_by_id = BTreeMap::new();

        for site in &output.sites {
            if !site_ids.insert(site.id) {
                return Err(AnalysisError::InvalidFact {
                    provider: "polint.calls",
                    reason: format!(
                        "duplicate call site {:?} for `{}`",
                        site.id,
                        interner.resolve(site.stable_key)
                    ),
                });
            }
            if let Some(owner_symbol) = site.owner_symbol {
                site_owner_by_id.insert(site.id, owner_symbol);
            }
        }

        for target in &output.targets {
            if !site_ids.contains(&target.site) {
                return Err(AnalysisError::InvalidFact {
                    provider: "polint.calls",
                    reason: format!(
                        "dangling call site {:?} for target `{}`",
                        target.site,
                        interner.resolve(target.stable_key)
                    ),
                });
            }
        }

        for unresolved in &output.unresolved {
            if !site_ids.contains(&unresolved.site) {
                return Err(AnalysisError::InvalidFact {
                    provider: "polint.calls",
                    reason: format!(
                        "dangling call site {:?} for unresolved call `{}`",
                        unresolved.site,
                        interner.resolve(unresolved.stable_key)
                    ),
                });
            }
        }

        let mut store = Self {
            output,
            ..Self::default()
        };

        for (index, site) in store.output.sites.iter().enumerate() {
            store
                .sites_by_caller
                .entry(site.caller)
                .or_default()
                .push(index);
        }

        for (index, target) in store.output.targets.iter().enumerate() {
            store
                .targets_by_site
                .entry(target.site)
                .or_default()
                .push(index);
            store
                .outgoing_by_function
                .entry(target.caller)
                .or_default()
                .push(index);
            if let Some(owner_symbol) = site_owner_by_id.get(&target.site).copied() {
                store
                    .outgoing_by_symbol
                    .entry(owner_symbol)
                    .or_default()
                    .push(index);
            }
            if let Some(target_symbol) = target.target_symbol {
                store
                    .incoming_by_symbol
                    .entry(target_symbol)
                    .or_default()
                    .push(index);
            }
            if let Some(target_function) = target.target_function {
                store
                    .incoming_by_function
                    .entry(target_function)
                    .or_default()
                    .push(index);
            }
        }

        for (index, unresolved) in store.output.unresolved.iter().enumerate() {
            store
                .unresolved_by_reason
                .entry(unresolved.reason)
                .or_default()
                .push(index);
            store
                .unresolved_by_status
                .entry(unresolved.status)
                .or_default()
                .push(index);
        }

        Ok(store)
    }

    pub fn sites(&self) -> &[CallSiteFact] {
        &self.output.sites
    }

    pub fn targets(&self) -> &[CallTargetFact] {
        &self.output.targets
    }

    pub fn unresolved(&self) -> &[UnresolvedCallFact] {
        &self.output.unresolved
    }

    pub fn sites_by_caller(&self, caller: FunctionId) -> Vec<&CallSiteFact> {
        self.site_refs(self.sites_by_caller.get(&caller))
    }

    pub fn targets_by_site(&self, site: CallSiteId) -> Vec<&CallTargetFact> {
        self.target_refs(self.targets_by_site.get(&site))
    }

    pub fn outgoing_by_function(&self, caller: FunctionId) -> Vec<&CallTargetFact> {
        self.target_refs(self.outgoing_by_function.get(&caller))
    }

    pub fn outgoing_by_symbol(&self, caller_symbol: SymbolId) -> Vec<&CallTargetFact> {
        self.target_refs(self.outgoing_by_symbol.get(&caller_symbol))
    }

    pub fn incoming_by_symbol(&self, target_symbol: SymbolId) -> Vec<&CallTargetFact> {
        self.target_refs(self.incoming_by_symbol.get(&target_symbol))
    }

    pub fn incoming_by_function(&self, target_function: FunctionId) -> Vec<&CallTargetFact> {
        self.target_refs(self.incoming_by_function.get(&target_function))
    }

    pub fn unresolved_by_reason(&self, reason: UnresolvedCallReason) -> Vec<&UnresolvedCallFact> {
        self.unresolved_refs(self.unresolved_by_reason.get(&reason))
    }

    pub fn unresolved_by_status(&self, status: CallTargetStatus) -> Vec<&UnresolvedCallFact> {
        self.unresolved_refs(self.unresolved_by_status.get(&status))
    }

    fn site_refs(&self, indexes: Option<&Vec<usize>>) -> Vec<&CallSiteFact> {
        indexes.map_or_else(Vec::new, |indexes| {
            indexes
                .iter()
                .map(|&index| &self.output.sites[index])
                .collect()
        })
    }

    fn target_refs(&self, indexes: Option<&Vec<usize>>) -> Vec<&CallTargetFact> {
        indexes.map_or_else(Vec::new, |indexes| {
            indexes
                .iter()
                .map(|&index| &self.output.targets[index])
                .collect()
        })
    }

    fn unresolved_refs(&self, indexes: Option<&Vec<usize>>) -> Vec<&UnresolvedCallFact> {
        indexes.map_or_else(Vec::new, |indexes| {
            indexes
                .iter()
                .map(|&index| &self.output.unresolved[index])
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::calls::facts::{
        CallAlgorithm, CallCallee, CallEdgeKind, CallPrecision, CallProvenance, CallSiteFact,
        CallSyntaxKind, CallTargetFact, CallTargetStatus, UnresolvedCallFact, UnresolvedCallReason,
    };
    use crate::analysis_neutral::ids::{CallSiteId, CallTargetId, MirBodyId, MirOpId};
    use crate::internal_core::{FileId, FunctionId, Language, Span, StableKeyInterner, SymbolId};

    fn span() -> Span {
        Span::point(FileId::from_raw(1), 1, 1)
    }

    fn site(interner: &StableKeyInterner, id: u64, caller: u64, stable_key: &str) -> CallSiteFact {
        CallSiteFact {
            in_throw: false,
            id: CallSiteId(id),
            language: Language::TypeScript,
            file: FileId::from_raw(1),
            caller: FunctionId::from_raw(caller),
            owner_symbol: Some(SymbolId::from_raw(caller + 100)),
            body: MirBodyId(caller),
            operation: MirOpId(id),
            span: span(),
            kind: CallSyntaxKind::Function,
            callee: CallCallee::Identifier {
                reference: None,
                name: stable_key.to_string(),
            },
            receiver: None,
            arguments: Vec::new(),
            result: None,
            status: CallTargetStatus::Resolved,
            precision: CallPrecision::Exact,
            stable_key: interner.intern(stable_key.to_string()),
        }
    }

    fn target(
        interner: &StableKeyInterner,
        id: u64,
        site: u64,
        caller: u64,
        stable_key: &str,
    ) -> CallTargetFact {
        CallTargetFact {
            id: CallTargetId(id),
            site: CallSiteId(site),
            caller: FunctionId::from_raw(caller),
            target_function: Some(FunctionId::from_raw(id + 10)),
            target_symbol: Some(SymbolId::from_raw(id + 20)),
            edge_kind: CallEdgeKind::Direct,
            algorithm: CallAlgorithm::DirectReference,
            status: CallTargetStatus::Resolved,
            reason: None,
            provenance: CallProvenance::Native,
            precision: CallPrecision::Exact,
            stable_key: interner.intern(stable_key.to_string()),
        }
    }

    fn unresolved(
        interner: &StableKeyInterner,
        site: u64,
        caller: u64,
        reason: UnresolvedCallReason,
        stable_key: &str,
    ) -> UnresolvedCallFact {
        UnresolvedCallFact {
            site: CallSiteId(site),
            caller: FunctionId::from_raw(caller),
            status: CallTargetStatus::Unresolved,
            reason,
            algorithm: CallAlgorithm::SyntaxOnly,
            provenance: CallProvenance::MirShape,
            precision: CallPrecision::Unknown,
            stable_key: interner.intern(stable_key.to_string()),
        }
    }

    #[test]
    fn normalized_sorts_call_rows_without_dropping_duplicates() {
        let db = LocalAnalysisDb::new();
        let interner = db.stable_key_interner();
        let output = CallOutput {
            sites: vec![
                site(&interner, 2, 1, "b"),
                site(&interner, 1, 1, "a"),
                site(&interner, 3, 1, "a"),
            ],
            targets: vec![
                target(&interner, 2, 2, 1, "target-b"),
                target(&interner, 1, 1, 1, "target-a"),
            ],
            unresolved: vec![
                unresolved(
                    &interner,
                    2,
                    1,
                    UnresolvedCallReason::DynamicProperty,
                    "unresolved-b",
                ),
                unresolved(
                    &interner,
                    1,
                    1,
                    UnresolvedCallReason::FunctionValue,
                    "unresolved-a",
                ),
            ],
        }
        .normalized(&interner);

        assert_eq!(
            output
                .sites
                .iter()
                .map(|fact| interner.resolve(fact.stable_key).to_string())
                .collect::<Vec<_>>(),
            vec!["a", "a", "b"]
        );
        assert_eq!(
            output
                .targets
                .iter()
                .map(|fact| interner.resolve(fact.stable_key).to_string())
                .collect::<Vec<_>>(),
            vec!["target-a", "target-b"]
        );
        assert_eq!(
            output
                .unresolved
                .iter()
                .map(|fact| interner.resolve(fact.stable_key).to_string())
                .collect::<Vec<_>>(),
            vec!["unresolved-a", "unresolved-b"]
        );
    }

    #[test]
    fn from_output_builds_deterministic_call_indexes() {
        let db = LocalAnalysisDb::new();
        let interner = db.stable_key_interner();
        let store = CallStore::from_output(
            CallOutput {
                sites: vec![
                    site(&interner, 2, 1, "site-b"),
                    site(&interner, 1, 1, "site-a"),
                ],
                targets: vec![
                    target(&interner, 2, 2, 1, "target-b"),
                    target(&interner, 1, 1, 1, "target-a"),
                ],
                unresolved: vec![unresolved(
                    &interner,
                    2,
                    1,
                    UnresolvedCallReason::DynamicProperty,
                    "unresolved-b",
                )],
            },
            &interner,
        )
        .expect("call output should be valid");

        assert_eq!(store.sites().len(), 2);
        assert_eq!(store.targets().len(), 2);
        assert_eq!(store.unresolved().len(), 1);
        assert_eq!(store.sites_by_caller(FunctionId::from_raw(1)).len(), 2);
        assert_eq!(
            interner
                .resolve(store.targets_by_site(CallSiteId(1))[0].stable_key)
                .as_ref(),
            "target-a"
        );
        assert_eq!(store.outgoing_by_function(FunctionId::from_raw(1)).len(), 2);
        assert_eq!(store.outgoing_by_symbol(SymbolId::from_raw(101)).len(), 2);
        assert_eq!(
            interner
                .resolve(store.incoming_by_symbol(SymbolId::from_raw(21))[0].stable_key)
                .as_ref(),
            "target-a"
        );
        assert_eq!(
            interner
                .resolve(store.incoming_by_function(FunctionId::from_raw(11))[0].stable_key)
                .as_ref(),
            "target-a"
        );
        assert_eq!(
            interner
                .resolve(
                    store.unresolved_by_reason(UnresolvedCallReason::DynamicProperty)[0].stable_key
                )
                .as_ref(),
            "unresolved-b"
        );
        assert_eq!(
            interner
                .resolve(store.unresolved_by_status(CallTargetStatus::Unresolved)[0].stable_key)
                .as_ref(),
            "unresolved-b"
        );
    }

    #[test]
    fn from_output_rejects_targets_without_matching_sites() {
        let db = LocalAnalysisDb::new();
        let interner = db.stable_key_interner();
        let error = CallStore::from_output(
            CallOutput {
                sites: vec![site(&interner, 1, 1, "site-a")],
                targets: vec![target(&interner, 2, 99, 1, "dangling-target")],
                unresolved: Vec::new(),
            },
            &interner,
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("dangling call site CallSiteId(99)")
        );
    }

    #[test]
    fn from_output_rejects_duplicate_site_ids() {
        let db = LocalAnalysisDb::new();
        let interner = db.stable_key_interner();
        let error = CallStore::from_output(
            CallOutput {
                sites: vec![
                    site(&interner, 7, 1, "site-first"),
                    site(&interner, 7, 2, "site-second"),
                ],
                targets: Vec::new(),
                unresolved: Vec::new(),
            },
            &interner,
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("duplicate call site CallSiteId(7)")
        );
    }

    #[test]
    fn empty_output_builds_empty_store() {
        let db = LocalAnalysisDb::new();
        let interner = db.stable_key_interner();
        let store =
            CallStore::from_output(CallOutput::empty(), &interner).expect("empty output is valid");

        assert!(store.sites().is_empty());
        assert!(store.targets().is_empty());
        assert!(store.unresolved().is_empty());
    }
}
