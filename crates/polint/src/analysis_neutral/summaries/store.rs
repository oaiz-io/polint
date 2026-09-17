use std::collections::BTreeMap;

use super::facts::{SummaryDomainKind, SummaryEventFact, SummaryFact};
use crate::analysis_neutral::error::AnalysisError;
use crate::analysis_neutral::ids::{SummaryEventId, SummaryId};
use crate::internal_core::{FunctionId, StableKeyId, StableKeyInterner};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SummaryOutput {
    pub summaries: Vec<SummaryFact>,
    pub events: Vec<SummaryEventFact>,
}

impl SummaryOutput {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn normalized(mut self, interner: &StableKeyInterner) -> Self {
        self.summaries.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.id.cmp(&right.id))
        });
        self.events.sort_by(|left, right| {
            interner
                .compare_canonical(left.stable_key, right.stable_key)
                .then_with(|| left.id.cmp(&right.id))
        });
        for (index, fact) in self.summaries.iter_mut().enumerate() {
            fact.id = SummaryId(index as u64);
        }
        for (index, fact) in self.events.iter_mut().enumerate() {
            fact.id = SummaryEventId(index as u64);
        }
        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct SummaryStore {
    output: SummaryOutput,
    summaries_by_callable: BTreeMap<StableKeyId, Vec<usize>>,
    summaries_by_domain: BTreeMap<SummaryDomainKind, Vec<usize>>,
    summaries_by_function: BTreeMap<FunctionId, Vec<usize>>,
    summary_by_function_domain: BTreeMap<(FunctionId, SummaryDomainKind), usize>,
}

impl SummaryStore {
    pub fn from_output(
        output: SummaryOutput,
        interner: &StableKeyInterner,
    ) -> Result<Self, AnalysisError> {
        Self::from_normalized_output(output.normalized(interner))
    }

    pub fn from_normalized_output(output: SummaryOutput) -> Result<Self, AnalysisError> {
        let mut store = Self {
            output,
            ..Self::default()
        };

        store.rebuild_summary_indexes();

        Ok(store)
    }

    pub fn merge_updates(
        &mut self,
        updated: &[SummaryFact],
        events: &[SummaryEventFact],
        interner: &StableKeyInterner,
    ) {
        let mut rebuild_summary_indexes = false;

        for fact in updated {
            let key = (fact.function, fact.domain);
            let Some(&index) = self.summary_by_function_domain.get(&key) else {
                self.output.summaries.push(fact.clone());
                rebuild_summary_indexes = true;
                continue;
            };

            let existing = &self.output.summaries[index];
            let stable_index_fields_changed = existing.stable_key != fact.stable_key
                || existing.callable_stable_key != fact.callable_stable_key
                || existing.function != fact.function
                || existing.domain != fact.domain;

            let mut replacement = fact.clone();
            replacement.id = existing.id;
            self.output.summaries[index] = replacement;

            if stable_index_fields_changed {
                rebuild_summary_indexes = true;
            }
        }

        if rebuild_summary_indexes {
            self.output.summaries = SummaryOutput {
                summaries: std::mem::take(&mut self.output.summaries),
                events: Vec::new(),
            }
            .normalized(interner)
            .summaries;
            self.rebuild_summary_indexes();
        }

        if !events.is_empty() {
            self.output.events.extend(events.iter().cloned());
            self.output.events.sort_by(|left, right| {
                interner
                    .compare_canonical(left.stable_key, right.stable_key)
                    .then_with(|| left.id.cmp(&right.id))
            });
            for (index, fact) in self.output.events.iter_mut().enumerate() {
                fact.id = SummaryEventId(index as u64);
            }
        }
    }

    pub fn summaries_by_callable(&self, callable_key: StableKeyId) -> Vec<&SummaryFact> {
        self.summary_refs(self.summaries_by_callable.get(&callable_key))
    }

    pub fn summaries_by_domain(&self, domain: SummaryDomainKind) -> Vec<&SummaryFact> {
        self.summary_refs(self.summaries_by_domain.get(&domain))
    }

    pub fn summaries_by_function(&self, function: FunctionId) -> Vec<&SummaryFact> {
        self.summary_refs(self.summaries_by_function.get(&function))
    }

    pub fn all_summaries(&self) -> &[SummaryFact] {
        &self.output.summaries
    }

    pub fn all_events(&self) -> &[SummaryEventFact] {
        &self.output.events
    }

    pub fn summary_count(&self) -> usize {
        self.output.summaries.len()
    }

    pub fn event_count(&self) -> usize {
        self.output.events.len()
    }

    fn summary_refs(&self, indexes: Option<&Vec<usize>>) -> Vec<&SummaryFact> {
        indexes.map_or_else(Vec::new, |indexes| {
            indexes
                .iter()
                .map(|&index| &self.output.summaries[index])
                .collect()
        })
    }

    fn rebuild_summary_indexes(&mut self) {
        self.summaries_by_callable.clear();
        self.summaries_by_domain.clear();
        self.summaries_by_function.clear();
        self.summary_by_function_domain.clear();

        for (index, fact) in self.output.summaries.iter().enumerate() {
            self.summaries_by_callable
                .entry(fact.callable_stable_key)
                .or_default()
                .push(index);
            self.summaries_by_domain
                .entry(fact.domain)
                .or_default()
                .push(index);
            self.summaries_by_function
                .entry(fact.function)
                .or_default()
                .push(index);
            self.summary_by_function_domain
                .insert((fact.function, fact.domain), index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::summaries::facts::{
        SummaryDomainKind, SummaryPrecision, SummaryProvenance, SummaryStatus,
    };
    use crate::internal_core::{stable_key_for_test, test_stable_key_interner};

    fn summary_fact(
        id: u64,
        callable_key: &str,
        function: u64,
        domain: SummaryDomainKind,
        stable_key: &str,
    ) -> SummaryFact {
        SummaryFact {
            id: SummaryId(id),
            callable_stable_key: stable_key_for_test(callable_key),
            function: FunctionId::from_raw(function),
            domain,
            status: SummaryStatus::Present,
            precision: SummaryPrecision::Local,
            provenance: SummaryProvenance::NativeLocal,
            payload_digest: format!("digest:{stable_key}"),
            tito_flows: Vec::new(),
            stable_key: stable_key_for_test(stable_key),
        }
    }

    fn event_fact(
        id: u64,
        callable_key: &str,
        function: u64,
        stable_key: &str,
    ) -> SummaryEventFact {
        SummaryEventFact {
            id: SummaryEventId(id),
            callable_stable_key: stable_key_for_test(callable_key),
            function: FunctionId::from_raw(function),
            domain: SummaryDomainKind::CallEffects,
            event_kind: "unresolved_callee".to_string(),
            reason: "dynamic".to_string(),
            status: SummaryStatus::Unknown,
            precision: SummaryPrecision::UnknownTop,
            stable_key: stable_key_for_test(stable_key),
        }
    }

    #[test]
    fn normalized_sorts_summaries_by_stable_key_and_reassigns_ids() {
        let interner = test_stable_key_interner();
        let output = SummaryOutput {
            summaries: vec![
                summary_fact(
                    99,
                    "func::b",
                    2,
                    SummaryDomainKind::ControlEffects,
                    "summary:z",
                ),
                summary_fact(
                    50,
                    "func::a",
                    1,
                    SummaryDomainKind::MemoryEffects,
                    "summary:a",
                ),
                summary_fact(
                    10,
                    "func::a",
                    1,
                    SummaryDomainKind::CallEffects,
                    "summary:m",
                ),
            ],
            events: vec![
                event_fact(5, "func::b", 2, "event:z"),
                event_fact(1, "func::a", 1, "event:a"),
            ],
        }
        .normalized(&interner);

        assert_eq!(
            output
                .summaries
                .iter()
                .map(|f| (f.id.0, interner.resolve(f.stable_key).to_string()))
                .collect::<Vec<_>>(),
            vec![
                (0, "summary:a".to_string()),
                (1, "summary:m".to_string()),
                (2, "summary:z".to_string())
            ]
        );
        assert_eq!(
            output
                .events
                .iter()
                .map(|f| (f.id.0, interner.resolve(f.stable_key).to_string()))
                .collect::<Vec<_>>(),
            vec![(0, "event:a".to_string()), (1, "event:z".to_string())]
        );
    }

    #[test]
    fn from_output_builds_deterministic_indexes() {
        let interner = test_stable_key_interner();
        let store = SummaryStore::from_output(
            SummaryOutput {
                summaries: vec![
                    summary_fact(
                        2,
                        "func::b",
                        2,
                        SummaryDomainKind::ControlEffects,
                        "summary:b-control",
                    ),
                    summary_fact(
                        1,
                        "func::a",
                        1,
                        SummaryDomainKind::ControlEffects,
                        "summary:a-control",
                    ),
                    summary_fact(
                        3,
                        "func::a",
                        1,
                        SummaryDomainKind::MemoryEffects,
                        "summary:a-memory",
                    ),
                ],
                events: vec![event_fact(1, "func::a", 1, "event:a")],
            },
            &interner,
        )
        .expect("valid output");

        assert_eq!(store.summary_count(), 3);
        assert_eq!(store.event_count(), 1);

        // IDs are reassigned sequentially
        assert_eq!(store.all_summaries()[0].id.0, 0);
        assert_eq!(store.all_summaries()[1].id.0, 1);
        assert_eq!(store.all_summaries()[2].id.0, 2);

        // Sorted by stable key
        assert_eq!(
            interner
                .resolve(store.all_summaries()[0].stable_key)
                .as_ref(),
            "summary:a-control"
        );
        assert_eq!(
            interner
                .resolve(store.all_summaries()[1].stable_key)
                .as_ref(),
            "summary:a-memory"
        );
        assert_eq!(
            interner
                .resolve(store.all_summaries()[2].stable_key)
                .as_ref(),
            "summary:b-control"
        );
    }

    #[test]
    fn empty_output_builds_empty_store() {
        let interner = test_stable_key_interner();
        let store =
            SummaryStore::from_output(SummaryOutput::empty(), &interner).expect("empty is valid");

        assert!(store.all_summaries().is_empty());
        assert!(store.all_events().is_empty());
        assert_eq!(store.summary_count(), 0);
        assert_eq!(store.event_count(), 0);
    }

    #[test]
    fn summaries_indexed_by_callable_and_domain() {
        let interner = test_stable_key_interner();
        let store = SummaryStore::from_output(
            SummaryOutput {
                summaries: vec![
                    summary_fact(
                        1,
                        "func::a",
                        1,
                        SummaryDomainKind::ControlEffects,
                        "summary:a-control",
                    ),
                    summary_fact(
                        2,
                        "func::a",
                        1,
                        SummaryDomainKind::MemoryEffects,
                        "summary:a-memory",
                    ),
                    summary_fact(
                        3,
                        "func::b",
                        2,
                        SummaryDomainKind::ControlEffects,
                        "summary:b-control",
                    ),
                ],
                events: Vec::new(),
            },
            &interner,
        )
        .expect("valid output");

        // by callable
        let a_key = stable_key_for_test("func::a");
        let b_key = stable_key_for_test("func::b");
        let a_summaries = store.summaries_by_callable(a_key);
        assert_eq!(a_summaries.len(), 2);
        assert!(a_summaries.iter().all(|f| f.callable_stable_key == a_key));

        let b_summaries = store.summaries_by_callable(b_key);
        assert_eq!(b_summaries.len(), 1);
        assert_eq!(b_summaries[0].callable_stable_key, b_key);

        let none = store.summaries_by_callable(stable_key_for_test("func::nonexistent"));
        assert!(none.is_empty());

        // by domain
        let control = store.summaries_by_domain(SummaryDomainKind::ControlEffects);
        assert_eq!(control.len(), 2);

        let memory = store.summaries_by_domain(SummaryDomainKind::MemoryEffects);
        assert_eq!(memory.len(), 1);

        let tito = store.summaries_by_domain(SummaryDomainKind::DataFlowTito);
        assert!(tito.is_empty());

        // by function
        let func1 = store.summaries_by_function(FunctionId::from_raw(1));
        assert_eq!(func1.len(), 2);

        let func2 = store.summaries_by_function(FunctionId::from_raw(2));
        assert_eq!(func2.len(), 1);

        let func99 = store.summaries_by_function(FunctionId::from_raw(99));
        assert!(func99.is_empty());
    }

    #[test]
    fn merge_updates_replaces_existing_rows_without_reordering() {
        let interner = test_stable_key_interner();
        let mut store = SummaryStore::from_output(
            SummaryOutput {
                summaries: vec![
                    summary_fact(
                        1,
                        "func::a",
                        1,
                        SummaryDomainKind::ControlEffects,
                        "summary:a-control",
                    ),
                    summary_fact(
                        2,
                        "func::b",
                        2,
                        SummaryDomainKind::MemoryEffects,
                        "summary:b-memory",
                    ),
                ],
                events: Vec::new(),
            },
            &interner,
        )
        .expect("valid output");

        let mut updated = summary_fact(
            99,
            "func::a",
            1,
            SummaryDomainKind::ControlEffects,
            "summary:a-control",
        );
        updated.payload_digest = "updated".to_string();

        store.merge_updates(&[updated], &[], &interner);

        assert_eq!(store.all_summaries()[0].id.0, 0);
        assert_eq!(store.all_summaries()[0].payload_digest, "updated");
        assert_eq!(store.all_summaries()[1].id.0, 1);
        assert_eq!(
            store.summaries_by_function(FunctionId::from_raw(1))[0].payload_digest,
            "updated"
        );
    }

    // -----------------------------------------------------------------------
    // LocalAnalysisDb integration tests (summary_fact_storage)
    // -----------------------------------------------------------------------

    #[test]
    fn summary_fact_storage_replace_removes_stale_rows_from_previous_run() {
        use crate::analysis_neutral::LocalAnalysisDb;

        let mut db = LocalAnalysisDb::new();
        db.replace_summary_facts(SummaryOutput {
            summaries: vec![summary_fact(
                1,
                "func::first",
                1,
                SummaryDomainKind::ControlEffects,
                "summary:first",
            )],
            events: vec![event_fact(1, "func::first", 1, "event:first")],
        });
        assert_eq!(db.summary_facts().len(), 1);
        assert_eq!(db.summary_events().len(), 1);

        db.replace_summary_facts(SummaryOutput {
            summaries: vec![summary_fact(
                2,
                "func::second",
                2,
                SummaryDomainKind::MemoryEffects,
                "summary:second",
            )],
            events: Vec::new(),
        });
        assert_eq!(db.summary_facts().len(), 1);
        assert_eq!(
            db.resolve_stable_key(db.summary_facts()[0].stable_key)
                .as_ref(),
            "summary:second"
        );
        assert!(db.summary_events().is_empty());
    }

    #[test]
    fn summary_fact_storage_records_metadata_provider_and_family_labels() {
        use crate::analysis_api::{FactFamily, FactRef};
        use crate::analysis_neutral::LocalAnalysisDb;

        let mut db = LocalAnalysisDb::new();
        db.replace_summary_facts(SummaryOutput {
            summaries: vec![
                summary_fact(
                    1,
                    "func::a",
                    1,
                    SummaryDomainKind::ControlEffects,
                    "summary:control",
                ),
                summary_fact(
                    2,
                    "func::a",
                    1,
                    SummaryDomainKind::CallEffects,
                    "summary:call",
                ),
                summary_fact(
                    3,
                    "func::a",
                    1,
                    SummaryDomainKind::MemoryEffects,
                    "summary:memory",
                ),
                summary_fact(
                    4,
                    "func::a",
                    1,
                    SummaryDomainKind::DataFlowTito,
                    "summary:tito",
                ),
            ],
            events: vec![event_fact(1, "func::a", 1, "event:a")],
        });

        // After normalization, stable_key order is: call(0), control(1), memory(2), tito(3)
        // Verify each fact family has metadata with correct producer
        let facts = db.summary_facts();
        for fact in facts {
            let family = match fact.domain {
                SummaryDomainKind::ControlEffects => FactFamily::SummaryControl,
                SummaryDomainKind::CallEffects => FactFamily::SummaryCall,
                SummaryDomainKind::MemoryEffects => FactFamily::SummaryMemory,
                SummaryDomainKind::DataFlowTito => FactFamily::SummaryTito,
            };
            let meta = db.fact_meta().get(FactRef::new(family, fact.id.0));
            assert!(
                meta.is_some(),
                "metadata missing for {:?} id={}",
                family,
                fact.id.0
            );
            assert_eq!(meta.unwrap().producer_id, "polint.direct_summaries");
        }

        // Verify all four summary families have at least one metadata row
        let mut families_found = std::collections::BTreeSet::new();
        for fact in facts {
            families_found.insert(fact.domain);
        }
        assert!(families_found.contains(&SummaryDomainKind::ControlEffects));
        assert!(families_found.contains(&SummaryDomainKind::CallEffects));
        assert!(families_found.contains(&SummaryDomainKind::MemoryEffects));
        assert!(families_found.contains(&SummaryDomainKind::DataFlowTito));

        // Check event family metadata exists
        let event_meta = db
            .fact_meta()
            .get(FactRef::new(FactFamily::SummaryEvent, 0));
        assert!(event_meta.is_some());
        assert_eq!(event_meta.unwrap().producer_id, "polint.direct_summaries");
    }

    #[test]
    fn summary_fact_storage_accessors_return_empty_before_replace() {
        let db = crate::analysis_neutral::LocalAnalysisDb::new();

        assert!(db.summary_facts().is_empty());
        assert!(db.summary_events().is_empty());
        assert!(db.summary_store().is_some());
        assert!(db.summary_store().unwrap().all_summaries().is_empty());
    }
}
