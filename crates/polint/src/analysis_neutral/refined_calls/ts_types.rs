use std::collections::{BTreeMap, BTreeSet};

use super::facts::{
    RefinedCallConfidence, RefinedCallEdgeFact, RefinedCallTier, RefinedCallValidation,
};
use super::store::RefinedCallOutput;
use crate::analysis_api::{FactFamily, stable_key_from_parts};
use crate::analysis_neutral::AnalysisHost;
use crate::analysis_neutral::calls::facts::{
    CallAlgorithm, CallEdgeKind, CallPrecision, CallProvenance, CallSiteFact, CallSyntaxKind,
    CallTargetStatus, UnresolvedCallReason,
};
use crate::analysis_neutral::ids::{CallSiteId, RefinedCallEdgeId};
use crate::internal_core::{FileId, FunctionId, Language, Span, StableKeyId};

/// Share of `any`/`unknown` receivers above which a file's typed answers are
/// reported as degraded rather than exact.
///
/// A quarter of a file's receivers being untyped says its type information is
/// partial, so the answers that remain are still used but no longer claim full
/// confidence.
pub const ANY_DENSITY_DEGRADED_PERCENT: u64 = 25;

/// Share of `any`/`unknown` receivers above which a file's inexact sites are
/// left to the field and heap tiers.
///
/// At half untyped, a typed candidate for a site the checker could not resolve
/// exactly is a guess dressed as a type answer. Exact sites in the same file
/// still produce edges: the gate decides which answers to trust, not whether to
/// keep the file.
pub const ANY_DENSITY_DEFER_PERCENT: u64 = 50;

/// How much a type checker could say about one call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TsTypeSiteStatus {
    Resolved,
    Union,
    External,
    AnyReceiver,
    Unresolved,
}

/// Why a callee is a candidate for its call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TsTypeDispatchKind {
    /// The checker resolved the call to this declaration.
    Declared,
    /// The checker answered with a declaration that cannot run.
    DeclaredSignature,
    /// A rapid-type candidate: an instantiated implementation of a declared
    /// target.
    Implementation,
    /// One constituent of a union receiver.
    UnionMember,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsTypeCallsiteInput {
    pub stable_key: StableKeyId,
    pub file: Option<FileId>,
    pub span: Option<Span>,
    pub status: TsTypeSiteStatus,
    /// The receiver type the checker printed, carried onto the edge as
    /// evidence so a reader can see why the tier answered as it did.
    pub receiver_printed: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsTypeCalleeInput {
    pub stable_key: StableKeyId,
    pub callsite_stable_key: StableKeyId,
    pub dispatch: TsTypeDispatchKind,
    /// Declaration name, used when the declaration spans disagree.
    pub name: String,
    /// Moniker for a declaration the scan does not own.
    pub external: Option<String>,
    pub file: Option<FileId>,
    pub span: Option<Span>,
    pub name_span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsTypeFileDensityInput {
    pub file: Option<FileId>,
    pub any_percent: u64,
}

/// Counters describing how well the sidecar's rows joined to native facts.
///
/// A typed tier that cannot find the call site a row describes produces no
/// edges and no error, which looks exactly like a repository with no typed
/// calls. Counting the misses is what makes the difference visible.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TsTypeJoinReport {
    pub matched_callsites: usize,
    pub unmatched_callsites: usize,
    pub matched_callees: usize,
    pub unmatched_callees: usize,
    pub deferred_callsites: usize,
}

pub fn derive_ts_type_refinements(
    db: &impl AnalysisHost,
    callsites: &[TsTypeCallsiteInput],
    callees: &[TsTypeCalleeInput],
    densities: &[TsTypeFileDensityInput],
) -> (RefinedCallOutput, TsTypeJoinReport) {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let mut report = TsTypeJoinReport::default();
    let mut edges = Vec::new();

    if callsites.is_empty() {
        return (RefinedCallOutput { edges }, report);
    }

    let density_by_file = densities
        .iter()
        .filter_map(|density| density.file.map(|file| (file, density.any_percent)))
        .collect::<BTreeMap<_, _>>();
    let index = TsSiteIndex::new(db);
    let mut callees_by_site: BTreeMap<StableKeyId, Vec<&TsTypeCalleeInput>> = BTreeMap::new();
    for callee in callees {
        callees_by_site
            .entry(callee.callsite_stable_key)
            .or_default()
            .push(callee);
    }

    for site in callsites {
        let Some(core_site) = index.core_site_for(db, site) else {
            report.unmatched_callsites += 1;
            continue;
        };
        report.matched_callsites += 1;
        let any_percent = site
            .file
            .and_then(|file| density_by_file.get(&file).copied())
            .unwrap_or(0);
        let exact = site.status == TsTypeSiteStatus::Resolved;

        if site.status == TsTypeSiteStatus::AnyReceiver {
            edges.push(untyped_site_edge(db, core_site, site, edges.len()));
            continue;
        }
        if any_percent >= ANY_DENSITY_DEFER_PERCENT && !exact {
            report.deferred_callsites += 1;
            continue;
        }

        for callee in callees_by_site
            .get(&site.stable_key)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            // A declaration that cannot run is not a call-graph target. It is
            // kept on the wire so a site with no runnable target stays
            // distinguishable from one the sidecar never saw, and dropped here.
            if callee.dispatch == TsTypeDispatchKind::DeclaredSignature {
                continue;
            }
            let target = index.core_function_for(db, callee);
            if target.is_none() && callee.external.is_none() {
                report.unmatched_callees += 1;
                continue;
            }
            report.matched_callees += 1;
            edges.push(typed_edge(
                db,
                core_site,
                site,
                callee,
                target,
                any_percent,
                edges.len(),
            ));
        }
    }

    (RefinedCallOutput { edges }.normalized(interner), report)
}

fn typed_edge(
    db: &impl AnalysisHost,
    core_site: &CallSiteFact,
    site: &TsTypeCallsiteInput,
    callee: &TsTypeCalleeInput,
    target: Option<FunctionId>,
    any_percent: u64,
    index: usize,
) -> RefinedCallEdgeFact {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let site_key = db.resolve_stable_key(site.stable_key).to_string();
    let callee_key = db.resolve_stable_key(callee.stable_key).to_string();
    let degraded = any_percent >= ANY_DENSITY_DEGRADED_PERCENT;
    let mut evidence = vec![
        "ts_type_checker".to_string(),
        format!("dispatch={}", dispatch_label(callee.dispatch)),
    ];
    if degraded {
        evidence.push(format!("any_density_percent={any_percent}"));
    }
    if let Some(external) = &callee.external {
        evidence.push(format!("external={external}"));
    }
    if let Some(printed) = &site.receiver_printed {
        evidence.push(format!("receiver_type={printed}"));
    }

    RefinedCallEdgeFact {
        id: RefinedCallEdgeId(index as u64),
        site: core_site.id,
        base_target: None,
        caller: core_site.caller,
        target_function: target,
        target_symbol: None,
        synthetic_target: callee
            .external
            .as_ref()
            .map(|external| format!("ts-types:external:{external}")),
        language: core_site.language,
        edge_kind: edge_kind_for_site(core_site),
        algorithm: CallAlgorithm::TypeHierarchy,
        tier: RefinedCallTier::TypeDirected,
        status: CallTargetStatus::Resolved,
        reason: None,
        provenance: CallProvenance::Model,
        precision: precision_for(callee.dispatch, degraded),
        validation: RefinedCallValidation::ReferentiallyValidated,
        confidence: confidence_for(callee.dispatch, degraded),
        evidence,
        input_stable_keys: vec![site_key.clone(), callee_key.clone()],
        stable_key: stable_key_from_parts(
            interner,
            FactFamily::RefinedCallEdge,
            &[
                ("tier", "ts_type_directed".to_string()),
                ("dispatch", dispatch_label(callee.dispatch).to_string()),
                ("callsite", site_key),
                ("callee", callee_key),
            ],
        ),
    }
}

/// An honest row for a site whose receiver the checker typed as `any` or
/// `unknown`.
///
/// This is the Q22 rule made visible rather than silent: the tier saw the site,
/// could not type it, and says so, so `polint unknowns` can attribute the gap
/// instead of the site simply not appearing.
fn untyped_site_edge(
    db: &impl AnalysisHost,
    core_site: &CallSiteFact,
    site: &TsTypeCallsiteInput,
    index: usize,
) -> RefinedCallEdgeFact {
    let interner_handle = db.stable_key_interner();
    let interner = &interner_handle;
    let site_key = db.resolve_stable_key(site.stable_key).to_string();
    RefinedCallEdgeFact {
        id: RefinedCallEdgeId(index as u64),
        site: core_site.id,
        base_target: None,
        caller: core_site.caller,
        target_function: None,
        target_symbol: None,
        synthetic_target: None,
        language: core_site.language,
        edge_kind: edge_kind_for_site(core_site),
        algorithm: CallAlgorithm::TypeHierarchy,
        tier: RefinedCallTier::TypeDirected,
        status: CallTargetStatus::Unresolved,
        reason: Some(UnresolvedCallReason::UnknownCallee),
        provenance: CallProvenance::Model,
        precision: CallPrecision::Unknown,
        validation: RefinedCallValidation::ReferentiallyValidated,
        confidence: RefinedCallConfidence::Low,
        evidence: {
            let mut evidence = vec![
                "ts_type_checker".to_string(),
                "receiver=any_or_unknown".to_string(),
            ];
            if let Some(printed) = &site.receiver_printed {
                evidence.push(format!("receiver_type={printed}"));
            }
            evidence
        },
        input_stable_keys: vec![site_key.clone()],
        stable_key: stable_key_from_parts(
            interner,
            FactFamily::RefinedCallEdge,
            &[
                ("tier", "ts_type_directed".to_string()),
                ("dispatch", "any_receiver".to_string()),
                ("callsite", site_key),
            ],
        ),
    }
}

fn dispatch_label(dispatch: TsTypeDispatchKind) -> &'static str {
    match dispatch {
        TsTypeDispatchKind::Declared => "declared",
        TsTypeDispatchKind::DeclaredSignature => "declared_signature",
        TsTypeDispatchKind::Implementation => "implementation",
        TsTypeDispatchKind::UnionMember => "union_member",
    }
}

/// A declared target is what the checker resolved, so it is exact. An expanded
/// candidate is narrowed by types but not chosen by them, so it is
/// setup-aware; in a degraded file it drops to conservative.
fn precision_for(dispatch: TsTypeDispatchKind, degraded: bool) -> CallPrecision {
    match (dispatch, degraded) {
        (TsTypeDispatchKind::Declared, false) => CallPrecision::Exact,
        (TsTypeDispatchKind::Declared, true) => CallPrecision::SetupAware,
        (_, false) => CallPrecision::SetupAware,
        (_, true) => CallPrecision::Conservative,
    }
}

fn confidence_for(dispatch: TsTypeDispatchKind, degraded: bool) -> RefinedCallConfidence {
    match (dispatch, degraded) {
        (TsTypeDispatchKind::Declared, false) => RefinedCallConfidence::High,
        (TsTypeDispatchKind::Declared, true) => RefinedCallConfidence::Medium,
        (_, false) => RefinedCallConfidence::Medium,
        (_, true) => RefinedCallConfidence::Low,
    }
}

fn edge_kind_for_site(site: &CallSiteFact) -> CallEdgeKind {
    match site.kind {
        CallSyntaxKind::Method | CallSyntaxKind::Member => CallEdgeKind::Method,
        CallSyntaxKind::StaticMember => CallEdgeKind::StaticMember,
        CallSyntaxKind::Constructor | CallSyntaxKind::New => CallEdgeKind::Constructor,
        CallSyntaxKind::Function => CallEdgeKind::Direct,
        CallSyntaxKind::FunctionValue => CallEdgeKind::FunctionValue,
        _ => CallEdgeKind::Unknown,
    }
}

/// Byte-span index over the native TS/JS call sites and functions.
///
/// Both parsers record byte offsets, but they do not record the same spans: the
/// TypeScript parser includes a declaration's modifiers where Oxc does not, so
/// the join tries exact equality first and falls back to containment anchored
/// on the declaration's own name.
struct TsSiteIndex {
    sites_by_file: BTreeMap<FileId, Vec<usize>>,
    functions_by_file: BTreeMap<FileId, Vec<usize>>,
}

impl TsSiteIndex {
    fn new(db: &impl AnalysisHost) -> Self {
        let mut sites_by_file: BTreeMap<FileId, Vec<usize>> = BTreeMap::new();
        for (position, site) in db.call_sites().iter().enumerate() {
            if site.language.is_ts_family() {
                sites_by_file.entry(site.file).or_default().push(position);
            }
        }
        let mut functions_by_file: BTreeMap<FileId, Vec<usize>> = BTreeMap::new();
        for (position, function) in db.functions().iter().enumerate() {
            if function.language.is_ts_family() {
                functions_by_file
                    .entry(function.file)
                    .or_default()
                    .push(position);
            }
        }
        Self {
            sites_by_file,
            functions_by_file,
        }
    }

    fn core_site_for<'a>(
        &self,
        db: &'a impl AnalysisHost,
        site: &TsTypeCallsiteInput,
    ) -> Option<&'a CallSiteFact> {
        let file = site.file?;
        let span = site.span.as_ref()?;
        let all = db.call_sites();
        let candidates = self
            .sites_by_file
            .get(&file)?
            .iter()
            .map(|position| &all[*position])
            .collect::<Vec<_>>();
        if let Some(exact) = candidates
            .iter()
            .copied()
            .filter(|candidate| same_byte_span(&candidate.span, span))
            .min_by_key(|candidate| db.resolve_stable_key(candidate.stable_key))
        {
            return Some(exact);
        }
        if let Some(same_start) = candidates
            .iter()
            .copied()
            .filter(|candidate| candidate.span.start_byte == span.start_byte)
            .min_by_key(|candidate| candidate.span.end_byte)
        {
            return Some(same_start);
        }
        // A call the sidecar sees as one expression can be several MIR
        // operations, so the narrowest native site containing the sidecar's
        // start offset is the one that describes the same call.
        candidates
            .iter()
            .copied()
            .filter(|candidate| {
                // A native site recorded as a point still covers its own
                // offset, so the end is widened by one for that case only —
                // never by the probe's offset, which would make every earlier
                // site contain every later one.
                let end = candidate
                    .span
                    .end_byte
                    .max(candidate.span.start_byte.saturating_add(1));
                candidate.span.start_byte <= span.start_byte && span.start_byte < end
            })
            .min_by_key(|candidate| {
                (
                    candidate
                        .span
                        .end_byte
                        .saturating_sub(candidate.span.start_byte),
                    db.resolve_stable_key(candidate.stable_key),
                )
            })
    }

    fn core_function_for(
        &self,
        db: &impl AnalysisHost,
        callee: &TsTypeCalleeInput,
    ) -> Option<FunctionId> {
        let file = callee.file?;
        let span = callee.span.as_ref()?;
        let all = db.functions();
        let candidates = self
            .functions_by_file
            .get(&file)?
            .iter()
            .map(|position| &all[*position])
            .collect::<Vec<_>>();
        if let Some(exact) = candidates
            .iter()
            .copied()
            .find(|candidate| same_byte_span(&candidate.span, span))
        {
            return Some(exact.id);
        }
        // Names disagree between the two parsers as well: a class method is
        // `greet` to the type checker and `Loud.greet` to the syntax frontend,
        // so a suffix match on the declared name is the strongest name signal
        // available.
        let named = candidates
            .iter()
            .copied()
            .filter(|candidate| name_matches(&candidate.name, &callee.name))
            .collect::<Vec<_>>();
        let anchor = callee
            .name_span
            .as_ref()
            .map(|name_span| name_span.start_byte)
            .unwrap_or(span.start_byte);
        let pool: &[&crate::analysis_api::FunctionFact] = if named.is_empty() {
            &candidates
        } else {
            &named
        };
        pool.iter()
            .copied()
            .filter(|candidate| {
                candidate.span.start_byte <= anchor && anchor < candidate.span.end_byte
            })
            // The narrowest containing declaration is the one that declares the
            // callee; the id breaks ties in the frontend's deterministic
            // source order.
            .min_by_key(|candidate| {
                (
                    candidate
                        .span
                        .end_byte
                        .saturating_sub(candidate.span.start_byte),
                    candidate.id,
                )
            })
            .map(|candidate| candidate.id)
    }
}

fn name_matches(core_name: &str, declared: &str) -> bool {
    if declared.is_empty() {
        return false;
    }
    core_name == declared
        || core_name
            .rsplit('.')
            .next()
            .is_some_and(|tail| tail == declared)
}

fn same_byte_span(left: &Span, right: &Span) -> bool {
    left.file == right.file
        && left.start_byte == right.start_byte
        && left.end_byte == right.end_byte
}

/// Languages the tier can answer for. Kept explicit so a future frontend does
/// not inherit typed edges by accident.
pub fn covers_language(language: Language) -> bool {
    language.is_ts_family()
}

/// Distinct call sites the typed tier produced at least one runnable target
/// for, used by the evaluation harness to attribute tier contribution.
pub fn typed_tier_site_count(output: &RefinedCallOutput) -> usize {
    output
        .edges
        .iter()
        .filter(|edge| {
            edge.tier == RefinedCallTier::TypeDirected && edge.status == CallTargetStatus::Resolved
        })
        .map(|edge| edge.site)
        .collect::<BTreeSet<_>>()
        .len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_api::FunctionFact;
    use crate::analysis_neutral::LocalAnalysisDb;
    use crate::analysis_neutral::calls::facts::{CallCallee, CallPrecision, CallSiteFact};
    use crate::analysis_neutral::calls::store::CallOutput;
    use crate::analysis_neutral::ids::{MirBodyId, MirOpId};
    use crate::internal_core::{FileId, Span};

    struct Fixture {
        db: LocalAnalysisDb,
        file: FileId,
        caller: FunctionId,
        callee_one: FunctionId,
        callee_two: FunctionId,
    }

    /// One file with a caller, two callable targets, and one call site whose
    /// byte span the sidecar rows are written against.
    fn fixture() -> Fixture {
        let mut db = LocalAnalysisDb::new();
        let file = db.add_file(
            "src/app.ts".into(),
            "src/app.ts".to_string(),
            "greeter.greet(name);".to_string(),
        );
        let caller = db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "run".to_string(),
            span(file, 0, 20),
            Language::TypeScript,
            false,
            true,
            1,
            Vec::new(),
        ));
        let callee_one = db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "Loud.greet".to_string(),
            span(file, 100, 160),
            Language::TypeScript,
            false,
            false,
            1,
            Vec::new(),
        ));
        let callee_two = db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "Quiet.greet".to_string(),
            span(file, 200, 260),
            Language::TypeScript,
            false,
            false,
            1,
            Vec::new(),
        ));
        db.replace_call_facts(CallOutput {
            sites: vec![CallSiteFact {
                id: CallSiteId(0),
                language: Language::TypeScript,
                file,
                caller,
                owner_symbol: None,
                body: MirBodyId(0),
                operation: MirOpId(0),
                span: span(file, 0, 19),
                kind: CallSyntaxKind::Method,
                callee: CallCallee::Member {
                    base: crate::analysis_neutral::ids::PlaceId(0),
                    property: "greet".to_string(),
                },
                receiver: None,
                arguments: Vec::new(),
                result: None,
                status: CallTargetStatus::Unresolved,
                precision: CallPrecision::Unknown,
                in_throw: false,
                stable_key: db.stable_key_interner().intern("call:site"),
            }],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("valid call facts");

        Fixture {
            db,
            file,
            caller,
            callee_one,
            callee_two,
        }
    }

    fn span(file: FileId, start: u32, end: u32) -> Span {
        Span::new(file, start, end, 1, start + 1, 1, end + 1)
    }

    fn site(
        db: &LocalAnalysisDb,
        file: FileId,
        status: TsTypeSiteStatus,
        start: u32,
        end: u32,
    ) -> TsTypeCallsiteInput {
        TsTypeCallsiteInput {
            stable_key: db.stable_key_interner().intern("ts-type:site"),
            file: Some(file),
            span: Some(span(file, start, end)),
            status,
            receiver_printed: Some("Greeter".to_string()),
        }
    }

    fn callee(
        db: &LocalAnalysisDb,
        file: FileId,
        key: &str,
        dispatch: TsTypeDispatchKind,
        start: u32,
        end: u32,
    ) -> TsTypeCalleeInput {
        TsTypeCalleeInput {
            stable_key: db.stable_key_interner().intern(key),
            callsite_stable_key: db.stable_key_interner().intern("ts-type:site"),
            dispatch,
            name: "greet".to_string(),
            external: None,
            file: Some(file),
            span: Some(span(file, start, end)),
            name_span: Some(span(file, start, start + 5)),
        }
    }

    #[test]
    fn a_declared_target_becomes_a_high_confidence_type_directed_edge() {
        let fixture = fixture();
        let sites = vec![site(
            &fixture.db,
            fixture.file,
            TsTypeSiteStatus::Resolved,
            0,
            19,
        )];
        let callees = vec![callee(
            &fixture.db,
            fixture.file,
            "callee:one",
            TsTypeDispatchKind::Declared,
            100,
            160,
        )];

        let (output, report) = derive_ts_type_refinements(&fixture.db, &sites, &callees, &[]);

        assert_eq!(output.edges.len(), 1);
        let edge = &output.edges[0];
        assert_eq!(edge.tier, RefinedCallTier::TypeDirected);
        assert_eq!(edge.confidence, RefinedCallConfidence::High);
        assert_eq!(edge.precision, CallPrecision::Exact);
        assert_eq!(edge.algorithm, CallAlgorithm::TypeHierarchy);
        assert_eq!(edge.target_function, Some(fixture.callee_one));
        assert_eq!(edge.caller, fixture.caller);
        assert!(edge.evidence.iter().any(|item| item == "ts_type_checker"));
        assert!(
            edge.evidence
                .iter()
                .any(|item| item == "receiver_type=Greeter")
        );
        assert_eq!(report.matched_callsites, 1);
        assert_eq!(report.matched_callees, 1);
    }

    #[test]
    fn rapid_type_candidates_are_medium_confidence_and_one_edge_each() {
        let fixture = fixture();
        let sites = vec![site(
            &fixture.db,
            fixture.file,
            TsTypeSiteStatus::Union,
            0,
            19,
        )];
        let callees = vec![
            callee(
                &fixture.db,
                fixture.file,
                "callee:one",
                TsTypeDispatchKind::Implementation,
                100,
                160,
            ),
            callee(
                &fixture.db,
                fixture.file,
                "callee:two",
                TsTypeDispatchKind::Implementation,
                200,
                260,
            ),
        ];

        let (output, _) = derive_ts_type_refinements(&fixture.db, &sites, &callees, &[]);

        assert_eq!(output.edges.len(), 2);
        assert!(
            output
                .edges
                .iter()
                .all(|edge| edge.confidence == RefinedCallConfidence::Medium)
        );
        let targets = output
            .edges
            .iter()
            .filter_map(|edge| edge.target_function)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            targets,
            BTreeSet::from([fixture.callee_one, fixture.callee_two])
        );
    }

    #[test]
    fn a_declaration_that_cannot_run_produces_no_edge() {
        let fixture = fixture();
        let sites = vec![site(
            &fixture.db,
            fixture.file,
            TsTypeSiteStatus::Resolved,
            0,
            19,
        )];
        let callees = vec![callee(
            &fixture.db,
            fixture.file,
            "callee:signature",
            TsTypeDispatchKind::DeclaredSignature,
            100,
            160,
        )];

        let (output, report) = derive_ts_type_refinements(&fixture.db, &sites, &callees, &[]);

        assert!(output.edges.is_empty());
        assert_eq!(report.matched_callsites, 1);
        assert_eq!(report.matched_callees, 0);
        assert_eq!(report.unmatched_callees, 0);
    }

    #[test]
    fn an_any_receiver_site_reports_the_gap_instead_of_inventing_a_target() {
        let fixture = fixture();
        let sites = vec![site(
            &fixture.db,
            fixture.file,
            TsTypeSiteStatus::AnyReceiver,
            0,
            19,
        )];
        let callees = vec![callee(
            &fixture.db,
            fixture.file,
            "callee:one",
            TsTypeDispatchKind::Declared,
            100,
            160,
        )];

        let (output, _) = derive_ts_type_refinements(&fixture.db, &sites, &callees, &[]);

        assert_eq!(output.edges.len(), 1);
        let edge = &output.edges[0];
        assert_eq!(edge.status, CallTargetStatus::Unresolved);
        assert_eq!(edge.reason, Some(UnresolvedCallReason::UnknownCallee));
        assert_eq!(edge.confidence, RefinedCallConfidence::Low);
        assert!(edge.target_function.is_none());
    }

    #[test]
    fn a_degraded_file_lowers_confidence_without_dropping_the_edge() {
        let fixture = fixture();
        let sites = vec![site(
            &fixture.db,
            fixture.file,
            TsTypeSiteStatus::Resolved,
            0,
            19,
        )];
        let callees = vec![callee(
            &fixture.db,
            fixture.file,
            "callee:one",
            TsTypeDispatchKind::Declared,
            100,
            160,
        )];
        let densities = vec![TsTypeFileDensityInput {
            file: Some(fixture.file),
            any_percent: ANY_DENSITY_DEGRADED_PERCENT,
        }];

        let (output, _) = derive_ts_type_refinements(&fixture.db, &sites, &callees, &densities);

        assert_eq!(output.edges.len(), 1);
        assert_eq!(output.edges[0].confidence, RefinedCallConfidence::Medium);
        assert_eq!(output.edges[0].precision, CallPrecision::SetupAware);
    }

    #[test]
    fn a_mostly_untyped_file_defers_its_inexact_sites_to_the_heap_tier() {
        let fixture = fixture();
        let sites = vec![site(
            &fixture.db,
            fixture.file,
            TsTypeSiteStatus::Union,
            0,
            19,
        )];
        let callees = vec![callee(
            &fixture.db,
            fixture.file,
            "callee:one",
            TsTypeDispatchKind::Implementation,
            100,
            160,
        )];
        let densities = vec![TsTypeFileDensityInput {
            file: Some(fixture.file),
            any_percent: ANY_DENSITY_DEFER_PERCENT,
        }];

        let (output, report) =
            derive_ts_type_refinements(&fixture.db, &sites, &callees, &densities);

        assert!(output.edges.is_empty());
        assert_eq!(report.deferred_callsites, 1);
    }

    #[test]
    fn a_mostly_untyped_file_still_emits_its_exactly_resolved_sites() {
        let fixture = fixture();
        let sites = vec![site(
            &fixture.db,
            fixture.file,
            TsTypeSiteStatus::Resolved,
            0,
            19,
        )];
        let callees = vec![callee(
            &fixture.db,
            fixture.file,
            "callee:one",
            TsTypeDispatchKind::Declared,
            100,
            160,
        )];
        let densities = vec![TsTypeFileDensityInput {
            file: Some(fixture.file),
            any_percent: 90,
        }];

        let (output, report) =
            derive_ts_type_refinements(&fixture.db, &sites, &callees, &densities);

        assert_eq!(output.edges.len(), 1);
        assert_eq!(report.deferred_callsites, 0);
    }

    #[test]
    fn a_declaration_span_that_disagrees_still_joins_through_the_name() {
        let fixture = fixture();
        let sites = vec![site(
            &fixture.db,
            fixture.file,
            TsTypeSiteStatus::Resolved,
            0,
            19,
        )];
        // The type checker counts an `export` modifier as part of the
        // declaration and the syntax frontend does not, so the spans differ by
        // the modifier's width while both still contain the name.
        let mut callee = callee(
            &fixture.db,
            fixture.file,
            "callee:one",
            TsTypeDispatchKind::Declared,
            93,
            160,
        );
        callee.name_span = Some(span(fixture.file, 110, 115));

        let (output, report) = derive_ts_type_refinements(&fixture.db, &sites, &[callee], &[]);

        assert_eq!(output.edges.len(), 1);
        assert_eq!(output.edges[0].target_function, Some(fixture.callee_one));
        assert_eq!(report.unmatched_callees, 0);
    }

    #[test]
    fn an_external_target_is_named_rather_than_dropped() {
        let fixture = fixture();
        let sites = vec![site(
            &fixture.db,
            fixture.file,
            TsTypeSiteStatus::External,
            0,
            19,
        )];
        let mut external = callee(
            &fixture.db,
            fixture.file,
            "callee:external",
            TsTypeDispatchKind::Declared,
            0,
            0,
        );
        external.external = Some("node_modules:express/index.d.ts#Router".to_string());
        external.file = None;
        external.span = None;
        external.name_span = None;

        let (output, report) = derive_ts_type_refinements(&fixture.db, &sites, &[external], &[]);

        assert_eq!(output.edges.len(), 1);
        assert_eq!(
            output.edges[0].synthetic_target.as_deref(),
            Some("ts-types:external:node_modules:express/index.d.ts#Router")
        );
        assert!(output.edges[0].target_function.is_none());
        assert_eq!(report.matched_callees, 1);
    }

    #[test]
    fn a_row_whose_call_site_this_scan_does_not_have_is_counted_not_dropped_silently() {
        let fixture = fixture();
        let mut orphan = site(
            &fixture.db,
            fixture.file,
            TsTypeSiteStatus::Resolved,
            900,
            920,
        );
        orphan.stable_key = fixture.db.stable_key_interner().intern("ts-type:orphan");

        let (output, report) = derive_ts_type_refinements(&fixture.db, &[orphan], &[], &[]);

        assert!(output.edges.is_empty());
        assert_eq!(report.unmatched_callsites, 1);
        assert_eq!(report.matched_callsites, 0);
    }

    #[test]
    fn an_empty_sidecar_result_produces_no_edges_and_no_work() {
        let fixture = fixture();

        let (output, report) = derive_ts_type_refinements(&fixture.db, &[], &[], &[]);

        assert!(output.edges.is_empty());
        assert_eq!(report, TsTypeJoinReport::default());
    }

    #[test]
    fn the_typed_tier_ranks_above_the_token_and_points_to_tiers() {
        assert!(RefinedCallTier::TypeDirected < RefinedCallTier::TypeValueFunctionToken);
        assert!(RefinedCallTier::TypeDirected < RefinedCallTier::PointsToAssisted);
        assert!(RefinedCallTier::DirectOnly < RefinedCallTier::TypeDirected);
    }

    #[test]
    fn typed_tier_site_count_counts_distinct_resolved_sites_only() {
        let fixture = fixture();
        let sites = vec![site(
            &fixture.db,
            fixture.file,
            TsTypeSiteStatus::Union,
            0,
            19,
        )];
        let callees = vec![
            callee(
                &fixture.db,
                fixture.file,
                "callee:one",
                TsTypeDispatchKind::Implementation,
                100,
                160,
            ),
            callee(
                &fixture.db,
                fixture.file,
                "callee:two",
                TsTypeDispatchKind::Implementation,
                200,
                260,
            ),
        ];

        let (output, _) = derive_ts_type_refinements(&fixture.db, &sites, &callees, &[]);

        assert_eq!(typed_tier_site_count(&output), 1);
    }

    #[test]
    fn only_the_typescript_family_is_covered() {
        assert!(covers_language(Language::TypeScript));
        assert!(covers_language(Language::JavaScript));
        assert!(!covers_language(Language::Go));
    }
}
