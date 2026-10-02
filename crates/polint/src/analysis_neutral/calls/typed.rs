//! Call targets a language frontend resolved with its own type checker.
//!
//! A typed frontend (for Go, the semantic sidecar's SSA program) knows, per call
//! site, the exact static callee, or the candidate callees of a dynamic call from
//! a call-graph algorithm, or that the call expression is not a call at all (a type
//! conversion, a builtin). This module joins those answers to polint's own call
//! sites by `(file, start byte, end byte)` of the call expression, which both
//! sides take from the same source text, and turns them into call targets.
//!
//! The answers are authoritative for the sites they cover: a site with typed
//! facts takes its targets from them alone, never from the name-based
//! resolution, because the type checker decides what a selector or a dynamic
//! call means and a same-named symbol does not. Sites the frontend did not
//! cover — another language, a file the frontend did not load, a frontend that
//! could not run — keep the name-based resolution unchanged.

use std::collections::{BTreeMap, BTreeSet};

use crate::analysis_api::FactFamily;
use crate::analysis_neutral::calls::facts::{
    CallAlgorithm, CallEdgeKind, CallPrecision, CallProvenance, CallSiteFact, CallTargetFact,
    CallTargetStatus,
};
use crate::analysis_neutral::ids::{CallSiteId, CallTargetId};
use crate::analysis_neutral::stable_key::semantic_stable_key;
use crate::internal_core::{FileId, FunctionId, Language, StableKeyId, StableKeyInterner};

/// The typed answers for one language's call sites, keyed by the call
/// expression's `(file, start byte, end byte)`.
#[derive(Debug, Clone, Default)]
pub struct TypedCallInputs {
    language: Option<Language>,
    sites: BTreeMap<(FileId, u32, u32), TypedCallSite>,
}

/// What the frontend knows about one call expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypedCallSite {
    /// A call with these candidate callees. One target is a static call; several
    /// are the candidates of a dynamic call.
    Targets(Vec<TypedCallTarget>),
    /// A call of a builtin such as `len` or `make`, named by the frontend's
    /// label for it.
    Builtin(String),
    /// A type conversion written with call syntax, such as `Kind(raw)`, named by
    /// the frontend's label for the target type.
    Conversion(String),
}

/// One candidate callee of a call.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TypedCallTarget {
    /// The in-repository function the callee is, when it has a declaration
    /// polint parsed. A callee without one (a dependency's function, a
    /// standard-library method) is named by `name` alone.
    pub function: Option<FunctionId>,
    /// The frontend's label for the callee, such as `go:func:fmt.Println` or
    /// `go:func:(*example.com/app.Store).Save`: a `scope:`-prefixed name that
    /// identifies the callee in the target's stable key, and names a callee that
    /// has no in-repository function.
    pub name: String,
    pub algorithm: CallAlgorithm,
    pub precision: CallPrecision,
    pub edge_kind: CallEdgeKind,
}

impl TypedCallInputs {
    pub fn new(language: Language) -> Self {
        Self {
            language: Some(language),
            sites: BTreeMap::new(),
        }
    }

    /// Records what the frontend knows about the call expression at `span`.
    /// Two answers for one expression (a package and its test variant) merge:
    /// their targets are united, and a call outranks a builtin or conversion
    /// label, which outranks nothing.
    pub fn insert(&mut self, file: FileId, start_byte: u32, end_byte: u32, site: TypedCallSite) {
        use std::collections::btree_map::Entry;
        match self.sites.entry((file, start_byte, end_byte)) {
            Entry::Vacant(entry) => {
                entry.insert(site);
            }
            Entry::Occupied(mut entry) => match (entry.get_mut(), site) {
                (TypedCallSite::Targets(existing), TypedCallSite::Targets(more)) => {
                    existing.extend(more);
                    existing.sort();
                    existing.dedup();
                }
                (TypedCallSite::Targets(_), _) => {}
                (current, TypedCallSite::Targets(more)) => *current = TypedCallSite::Targets(more),
                _ => {}
            },
        }
    }

    pub fn is_empty(&self) -> bool {
        self.sites.is_empty()
    }

    pub fn len(&self) -> usize {
        self.sites.len()
    }

    /// What the frontend said about the call expression at `span`, if anything.
    #[cfg(test)]
    pub(crate) fn answer_at(
        &self,
        file: FileId,
        start_byte: u32,
        end_byte: u32,
    ) -> Option<&TypedCallSite> {
        self.sites.get(&(file, start_byte, end_byte))
    }

    fn site_for(&self, site: &CallSiteFact) -> Option<&TypedCallSite> {
        if Some(site.language) != self.language {
            return None;
        }
        self.sites
            .get(&(site.span.file, site.span.start_byte, site.span.end_byte))
    }
}

/// How many of a language's call sites typed facts covered, for the run report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TypedCallJoinReport {
    pub sites: usize,
    pub joined: usize,
    pub targets: usize,
}

/// Replaces the targets of every site the typed inputs cover with typed
/// targets, and returns the sites they cover so the caller can drop the
/// name-based targets and unresolved rows of those sites.
pub fn typed_call_targets(
    interner: &StableKeyInterner,
    sites: &[CallSiteFact],
    typed: &TypedCallInputs,
) -> (
    Vec<CallTargetFact>,
    BTreeSet<CallSiteId>,
    TypedCallJoinReport,
) {
    let mut rows = Vec::new();
    let mut covered = BTreeSet::new();
    let mut report = TypedCallJoinReport::default();
    for site in sites {
        if Some(site.language) != typed.language {
            continue;
        }
        report.sites += 1;
        let Some(answer) = typed.site_for(site) else {
            continue;
        };
        report.joined += 1;
        match answer {
            TypedCallSite::Targets(targets) => {
                if targets.is_empty() {
                    continue;
                }
                covered.insert(site.id);
                for target in targets {
                    rows.push(CallTargetFact {
                        id: CallTargetId(0),
                        site: site.id,
                        caller: site.caller,
                        target_function: target.function,
                        target_symbol: None,
                        synthetic_target: target.function.is_none().then(|| target.name.clone()),
                        edge_kind: target.edge_kind,
                        algorithm: target.algorithm,
                        status: CallTargetStatus::Resolved,
                        reason: None,
                        provenance: CallProvenance::Native,
                        precision: target.precision,
                        stable_key: typed_target_stable_key(
                            interner,
                            site,
                            algorithm_label(target.algorithm),
                            &target.name,
                        ),
                    });
                }
            }
            TypedCallSite::Builtin(label) => {
                covered.insert(site.id);
                rows.push(labelled_target(
                    interner,
                    site,
                    label.clone(),
                    CallEdgeKind::Direct,
                ));
            }
            TypedCallSite::Conversion(label) => {
                covered.insert(site.id);
                rows.push(labelled_target(
                    interner,
                    site,
                    label.clone(),
                    CallEdgeKind::Synthetic,
                ));
            }
        }
    }
    report.targets = rows.len();
    (rows, covered, report)
}

/// The target of a call expression that is a builtin or a conversion: exact,
/// and named by a label no user pattern matches.
fn labelled_target(
    interner: &StableKeyInterner,
    site: &CallSiteFact,
    label: String,
    edge_kind: CallEdgeKind,
) -> CallTargetFact {
    CallTargetFact {
        id: CallTargetId(0),
        site: site.id,
        caller: site.caller,
        target_function: None,
        target_symbol: None,
        stable_key: typed_target_stable_key(
            interner,
            site,
            algorithm_label(CallAlgorithm::GoStatic),
            &label,
        ),
        synthetic_target: Some(label),
        edge_kind,
        algorithm: CallAlgorithm::GoStatic,
        status: CallTargetStatus::Resolved,
        reason: None,
        provenance: CallProvenance::Native,
        precision: CallPrecision::Exact,
    }
}

fn algorithm_label(algorithm: CallAlgorithm) -> &'static str {
    match algorithm {
        CallAlgorithm::GoStatic => "go_static",
        CallAlgorithm::GoVta => "go_vta",
        CallAlgorithm::GoCha => "go_cha",
        CallAlgorithm::TypeHierarchy => "type_hierarchy",
        _ => "typed",
    }
}

fn typed_target_stable_key(
    interner: &StableKeyInterner,
    site: &CallSiteFact,
    algorithm: &str,
    target: &str,
) -> StableKeyId {
    interner.intern(
        semantic_stable_key(
            FactFamily::CallTarget,
            &[
                ("site", interner.resolve(site.stable_key).to_string()),
                ("algorithm", algorithm.to_string()),
                ("target", target.to_string()),
                ("provider", "polint.calls".to_string()),
                ("schema", "calls-facts-1:1".to_string()),
                ("model", "typed".to_string()),
            ],
        )
        .into_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_neutral::calls::facts::{CallCallee, CallSyntaxKind};
    use crate::analysis_neutral::ids::{MirBodyId, MirOpId};
    use crate::internal_core::Span;

    const FILE: FileId = FileId::from_raw(1);

    fn site(
        interner: &StableKeyInterner,
        id: u64,
        language: Language,
        start: u32,
        end: u32,
    ) -> CallSiteFact {
        CallSiteFact {
            in_throw: false,
            id: CallSiteId(id),
            language,
            file: FILE,
            caller: FunctionId::from_raw(1),
            owner_symbol: None,
            body: MirBodyId(1),
            operation: MirOpId(id),
            span: Span::new(FILE, start, end, 1, start + 1, 1, end + 1),
            kind: CallSyntaxKind::Member,
            callee: CallCallee::Identifier {
                reference: None,
                name: "callee".to_string(),
            },
            receiver: None,
            arguments: Vec::new(),
            result: None,
            status: CallTargetStatus::Unresolved,
            precision: CallPrecision::Unknown,
            stable_key: interner.intern(format!("site-{id}")),
        }
    }

    fn candidate(
        name: &str,
        function: Option<FunctionId>,
        algorithm: CallAlgorithm,
        precision: CallPrecision,
    ) -> TypedCallTarget {
        TypedCallTarget {
            function,
            name: name.to_string(),
            algorithm,
            precision,
            edge_kind: CallEdgeKind::Method,
        }
    }

    #[test]
    fn a_covered_site_takes_every_typed_candidate_and_others_are_left_alone() {
        let interner = StableKeyInterner::default();
        let sites = [
            site(&interner, 0, Language::Go, 10, 20),
            site(&interner, 1, Language::Go, 30, 40),
        ];
        let mut typed = TypedCallInputs::new(Language::Go);
        typed.insert(
            FILE,
            10,
            20,
            TypedCallSite::Targets(vec![
                candidate(
                    "go:func:a.F",
                    None,
                    CallAlgorithm::GoVta,
                    CallPrecision::SetupAware,
                ),
                candidate(
                    "go:func:a.G",
                    None,
                    CallAlgorithm::GoVta,
                    CallPrecision::SetupAware,
                ),
            ]),
        );

        let (rows, covered, report) = typed_call_targets(&interner, &sites, &typed);

        assert_eq!(covered, BTreeSet::from([CallSiteId(0)]));
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row.site == CallSiteId(0)
            && row.status == CallTargetStatus::Resolved
            && row.algorithm == CallAlgorithm::GoVta));
        assert_eq!(
            report,
            TypedCallJoinReport {
                sites: 2,
                joined: 1,
                targets: 2
            }
        );
    }

    #[test]
    fn another_language_or_another_span_is_never_joined() {
        let interner = StableKeyInterner::default();
        let sites = [
            site(&interner, 0, Language::TypeScript, 10, 20),
            site(&interner, 1, Language::Go, 10, 21),
        ];
        let mut typed = TypedCallInputs::new(Language::Go);
        typed.insert(
            FILE,
            10,
            20,
            TypedCallSite::Targets(vec![candidate(
                "go:func:a.F",
                None,
                CallAlgorithm::GoStatic,
                CallPrecision::Exact,
            )]),
        );

        let (rows, covered, report) = typed_call_targets(&interner, &sites, &typed);

        assert!(rows.is_empty());
        assert!(covered.is_empty());
        assert_eq!(report.sites, 1, "only the Go site is counted");
        assert_eq!(report.joined, 0);
    }

    #[test]
    fn every_callee_is_keyed_by_its_label_and_only_a_dependency_is_synthetic() {
        let interner = StableKeyInterner::default();
        let sites = [site(&interner, 0, Language::Go, 10, 20)];
        let local = FunctionId::from_raw(7);
        let mut typed = TypedCallInputs::new(Language::Go);
        typed.insert(
            FILE,
            10,
            20,
            TypedCallSite::Targets(vec![
                candidate(
                    "go:func:a.Local",
                    Some(local),
                    CallAlgorithm::GoCha,
                    CallPrecision::Conservative,
                ),
                candidate(
                    "go:func:fmt.Println",
                    None,
                    CallAlgorithm::GoCha,
                    CallPrecision::Conservative,
                ),
            ]),
        );
        let (rows, _, _) = typed_call_targets(&interner, &sites, &typed);

        let local_row = rows
            .iter()
            .find(|row| row.target_function == Some(local))
            .expect("the in-repository candidate");
        assert_eq!(local_row.synthetic_target, None);
        assert!(
            interner
                .resolve(local_row.stable_key)
                .contains("go:func:a.Local")
        );
        let dependency = rows
            .iter()
            .find(|row| row.target_function.is_none())
            .expect("the dependency candidate");
        assert_eq!(
            dependency.synthetic_target.as_deref(),
            Some("go:func:fmt.Println")
        );
    }

    #[test]
    fn builtins_and_conversions_are_exact_labelled_targets() {
        let interner = StableKeyInterner::default();
        let sites = [
            site(&interner, 0, Language::Go, 10, 20),
            site(&interner, 1, Language::Go, 30, 40),
        ];
        let mut typed = TypedCallInputs::new(Language::Go);
        typed.insert(
            FILE,
            10,
            20,
            TypedCallSite::Builtin("go:builtin:len".to_string()),
        );
        typed.insert(
            FILE,
            30,
            40,
            TypedCallSite::Conversion("go:conversion:a.Kind".to_string()),
        );

        let (rows, covered, _) = typed_call_targets(&interner, &sites, &typed);

        assert_eq!(covered.len(), 2);
        let builtin = rows.iter().find(|row| row.site == CallSiteId(0)).unwrap();
        assert_eq!(builtin.synthetic_target.as_deref(), Some("go:builtin:len"));
        assert_eq!(builtin.edge_kind, CallEdgeKind::Direct);
        assert_eq!(builtin.precision, CallPrecision::Exact);
        let conversion = rows.iter().find(|row| row.site == CallSiteId(1)).unwrap();
        assert_eq!(
            conversion.synthetic_target.as_deref(),
            Some("go:conversion:a.Kind")
        );
        assert_eq!(conversion.edge_kind, CallEdgeKind::Synthetic);
    }

    #[test]
    fn an_empty_candidate_list_covers_nothing() {
        let interner = StableKeyInterner::default();
        let sites = [site(&interner, 0, Language::Go, 10, 20)];
        let mut typed = TypedCallInputs::new(Language::Go);
        typed.insert(FILE, 10, 20, TypedCallSite::Targets(Vec::new()));

        let (rows, covered, report) = typed_call_targets(&interner, &sites, &typed);

        assert!(rows.is_empty());
        assert!(
            covered.is_empty(),
            "the name-based resolution keeps the site"
        );
        assert_eq!(report.joined, 1);
    }

    #[test]
    fn answers_for_one_expression_merge_with_calls_outranking_labels() {
        let a = candidate(
            "go:func:a.A",
            None,
            CallAlgorithm::GoVta,
            CallPrecision::SetupAware,
        );
        let b = candidate(
            "go:func:a.B",
            None,
            CallAlgorithm::GoVta,
            CallPrecision::SetupAware,
        );
        let mut typed = TypedCallInputs::new(Language::Go);
        typed.insert(FILE, 1, 2, TypedCallSite::Targets(vec![b.clone()]));
        typed.insert(
            FILE,
            1,
            2,
            TypedCallSite::Targets(vec![a.clone(), b.clone()]),
        );
        typed.insert(
            FILE,
            1,
            2,
            TypedCallSite::Builtin("go:builtin:len".to_string()),
        );
        typed.insert(
            FILE,
            3,
            4,
            TypedCallSite::Conversion("go:conversion:a.K".to_string()),
        );
        typed.insert(FILE, 3, 4, TypedCallSite::Targets(vec![a.clone()]));

        assert_eq!(typed.len(), 2);
        assert_eq!(
            typed.sites.get(&(FILE, 1, 2)),
            Some(&TypedCallSite::Targets(vec![a.clone(), b]))
        );
        assert_eq!(
            typed.sites.get(&(FILE, 3, 4)),
            Some(&TypedCallSite::Targets(vec![a]))
        );
    }
}
