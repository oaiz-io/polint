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
    /// The checker typed the callee, but the binding it read may hold another
    /// value when the call runs.
    Inferred,
}

/// What the checker typed a call's receiver as, for a method call, or its
/// callee expression, for a value call.
///
/// Only a receiver named by one declaration lets the member the checker picks
/// be the member that runs. A union's member is one of several; an
/// intersection's and an object literal's are matched by structure, so another
/// value of the same shape can carry another function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TsTypeReceiverShape {
    Named,
    Union,
    Intersection,
    ObjectLiteral,
}

/// What kind of declaration a callee row names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TsTypeDeclarationKind {
    /// A function declaration, function expression or arrow function.
    Function,
    Method,
    Constructor,
    Class,
    /// A getter or setter: it runs on a property read or write, never through a
    /// call to its name.
    Accessor,
    /// A declaration outside the scan.
    External,
    /// A callee whose declaration row the sidecar did not send.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsTypeCallsiteInput {
    pub stable_key: StableKeyId,
    pub file: Option<FileId>,
    pub span: Option<Span>,
    pub status: TsTypeSiteStatus,
    pub receiver_shape: TsTypeReceiverShape,
    /// The receiver type the checker printed, carried onto the edge as
    /// evidence so a reader can see why the tier answered as it did.
    pub receiver_printed: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsTypeCalleeInput {
    pub stable_key: StableKeyId,
    pub callsite_stable_key: StableKeyId,
    pub dispatch: TsTypeDispatchKind,
    pub kind: TsTypeDeclarationKind,
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
        // A type checker's answer is a frontend fact, not a model: the Go
        // semantic tier records its compiler-derived edges the same way, and
        // `Model` is reserved for framework and extension recognizers, which
        // may never claim exact precision.
        provenance: CallProvenance::Native,
        precision: precision_for(TypedAnswer::new(core_site, site, callee), degraded),
        validation: RefinedCallValidation::ReferentiallyValidated,
        confidence: confidence_for(TypedAnswer::new(core_site, site, callee), degraded),
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
        assumptions: Vec::new(),
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
        provenance: CallProvenance::Native,
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
        assumptions: Vec::new(),
    }
}

fn dispatch_label(dispatch: TsTypeDispatchKind) -> &'static str {
    match dispatch {
        TsTypeDispatchKind::Declared => "declared",
        TsTypeDispatchKind::DeclaredSignature => "declared_signature",
        TsTypeDispatchKind::Implementation => "implementation",
        TsTypeDispatchKind::UnionMember => "union_member",
        TsTypeDispatchKind::Inferred => "inferred",
    }
}

/// The parts of a typed answer that decide whether it names the function the
/// call runs.
#[derive(Debug, Clone, Copy)]
struct TypedAnswer {
    dispatch: TsTypeDispatchKind,
    status: TsTypeSiteStatus,
    shape: TsTypeReceiverShape,
    kind: TsTypeDeclarationKind,
    constructs: bool,
}

impl TypedAnswer {
    fn new(
        core_site: &CallSiteFact,
        site: &TsTypeCallsiteInput,
        callee: &TsTypeCalleeInput,
    ) -> Self {
        Self {
            dispatch: callee.dispatch,
            status: site.status,
            shape: site.receiver_shape,
            kind: callee.kind,
            constructs: matches!(
                core_site.kind,
                CallSyntaxKind::Constructor | CallSyntaxKind::New
            ),
        }
    }

    /// Whether the checker's answer is the function the call runs.
    ///
    /// The checker resolved the call to one declaration through a binding that
    /// holds only it, the declaration is a function the scan owns and a call
    /// runs (a class only when the call constructs it), and the receiver is
    /// named by one declaration. A union site, a structural receiver, an
    /// accessor, a class called without `new` and a declaration outside the
    /// scan may each run something else.
    fn names_the_target(self) -> bool {
        let runs = match self.kind {
            TsTypeDeclarationKind::Function
            | TsTypeDeclarationKind::Method
            | TsTypeDeclarationKind::Constructor => true,
            TsTypeDeclarationKind::Class => self.constructs,
            TsTypeDeclarationKind::Accessor
            | TsTypeDeclarationKind::External
            | TsTypeDeclarationKind::Unknown => false,
        };
        self.dispatch == TsTypeDispatchKind::Declared
            && self.status != TsTypeSiteStatus::Union
            && self.shape == TsTypeReceiverShape::Named
            && runs
    }
}

/// An answer that names the target is exact in a file whose receivers the
/// checker could type; in a degraded file the same answer rests on types the
/// checker only partly had, so it may reach its target but is not claimed as
/// the only one. Every other answer may reach its target without being the
/// only one: a declared target among several at a union site, a candidate
/// narrowed by types but not chosen by them, a member picked by shape, and a
/// target read through a binding that can hold another value.
fn precision_for(answer: TypedAnswer, degraded: bool) -> CallPrecision {
    if answer.names_the_target() && !degraded {
        CallPrecision::Exact
    } else {
        CallPrecision::Conservative
    }
}

fn confidence_for(answer: TypedAnswer, degraded: bool) -> RefinedCallConfidence {
    match (answer.names_the_target(), degraded) {
        (true, false) => RefinedCallConfidence::High,
        (true, true) | (false, false) => RefinedCallConfidence::Medium,
        (false, true) => RefinedCallConfidence::Low,
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
/// TypeScript parser includes a declaration's modifiers and a call's
/// parenthesised callee where Oxc does not. Those differences only ever move
/// the start, so the join tries exact equality first and then a span that ends
/// at the same byte and starts inside the sidecar's. A row that matches
/// neither describes something the native scan did not record, and is counted
/// unmatched rather than given the call or function around it.
struct TsSiteIndex {
    sites_by_file: BTreeMap<FileId, Vec<usize>>,
    functions_by_file: BTreeMap<FileId, Vec<usize>>,
}

impl TsSiteIndex {
    fn new(db: &impl AnalysisHost) -> Self {
        let mut sites_by_file: BTreeMap<FileId, Vec<usize>> = BTreeMap::new();
        for (position, site) in db.call_sites().iter().enumerate() {
            if covers_language(site.language) {
                sites_by_file.entry(site.file).or_default().push(position);
            }
        }
        let mut functions_by_file: BTreeMap<FileId, Vec<usize>> = BTreeMap::new();
        for (position, function) in db.functions().iter().enumerate() {
            if covers_language(function.language) {
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
        // `(function () {})()` starts at the parenthesis for the type checker
        // and at the function for the syntax frontend. No other call ends at
        // the same byte inside it, so the widest such site is the same call.
        // Neither a shared start nor containment is enough: `chain().greet()`
        // and `outer(inner())` are two calls each.
        candidates
            .iter()
            .copied()
            .filter(|candidate| same_end_inside(&candidate.span, span))
            .min_by_key(|candidate| {
                (
                    candidate.span.start_byte,
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
        if let Some(same_end) = candidates
            .iter()
            .copied()
            .filter(|candidate| same_end_inside(&candidate.span, span))
            .min_by_key(|candidate| (candidate.span.start_byte, candidate.id))
        {
            return Some(same_end.id);
        }
        // Names disagree between the two parsers as well: a class method is
        // `greet` to the type checker and `Loud.greet` to the syntax frontend,
        // so a suffix match on the declared name is the strongest name signal
        // available. A declaration no native function is named for gets no
        // target: the function around it is a different function. A class is
        // the native row of the same name, never a member that shares its
        // name, and an accessor is joined only by its span: the native
        // function named like it can be a method of the same name.
        if callee.kind == TsTypeDeclarationKind::Accessor {
            return None;
        }
        let anchor = callee
            .name_span
            .as_ref()
            .map(|name_span| name_span.start_byte)
            .unwrap_or(span.start_byte);
        candidates
            .iter()
            .copied()
            .filter(|candidate| match callee.kind {
                TsTypeDeclarationKind::Class => candidate.name == callee.name,
                _ => name_matches(&candidate.name, &callee.name),
            })
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

/// Whether the native `candidate` ends where the sidecar's `span` does and
/// starts inside it.
fn same_end_inside(candidate: &Span, span: &Span) -> bool {
    candidate.file == span.file
        && candidate.end_byte == span.end_byte
        && span.start_byte <= candidate.start_byte
        && candidate.start_byte < span.end_byte
}

fn same_byte_span(left: &Span, right: &Span) -> bool {
    left.file == right.file
        && left.start_byte == right.start_byte
        && left.end_byte == right.end_byte
}

/// Languages the tier can answer for.
///
/// The join asks this rather than testing the language inline, so a frontend
/// added later inherits typed edges only by being named here.
pub fn covers_language(language: Language) -> bool {
    language.is_ts_family()
}

/// Distinct call sites the typed tier produced at least one runnable target
/// for.
///
/// Reported by the measurement harness next to the whole-pipeline recall
/// proxy, which counts a site resolved by any tier: the difference between the
/// two is what this tier contributed on its own.
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
    use crate::internal_core::{FileId, Span, StableKeyInterner};

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
        fixture_with_site(0, 19)
    }

    /// The same file with its one native call site at `start..end`.
    fn fixture_with_site(start: u32, end: u32) -> Fixture {
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
                span: span(file, start, end),
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
            receiver_shape: TsTypeReceiverShape::Named,
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
            kind: TsTypeDeclarationKind::Method,
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
        // A candidate the checker narrowed but did not choose may reach its
        // target; it never claims that the call reaches only that target.
        assert!(
            output
                .edges
                .iter()
                .all(|edge| edge.precision == CallPrecision::Conservative)
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
        assert_eq!(output.edges[0].precision, CallPrecision::Conservative);
    }

    /// The precision and confidence of the one edge a declared target at a
    /// resolved site gets, with the receiver shape and declaration kind given.
    fn declared_answer(
        shape: TsTypeReceiverShape,
        kind: TsTypeDeclarationKind,
    ) -> (CallPrecision, RefinedCallConfidence) {
        let fixture = fixture();
        let mut typed_site = site(&fixture.db, fixture.file, TsTypeSiteStatus::Resolved, 0, 19);
        typed_site.receiver_shape = shape;
        let mut target = callee(
            &fixture.db,
            fixture.file,
            "callee:one",
            TsTypeDispatchKind::Declared,
            100,
            160,
        );
        target.kind = kind;

        let (output, _) = derive_ts_type_refinements(&fixture.db, &[typed_site], &[target], &[]);

        assert_eq!(output.edges.len(), 1, "{shape:?} {kind:?}");
        (output.edges[0].precision, output.edges[0].confidence)
    }

    #[test]
    fn only_a_receiver_named_by_one_declaration_is_claimed_exactly() {
        assert_eq!(
            declared_answer(TsTypeReceiverShape::Named, TsTypeDeclarationKind::Method),
            (CallPrecision::Exact, RefinedCallConfidence::High)
        );
        for shape in [
            TsTypeReceiverShape::Union,
            TsTypeReceiverShape::Intersection,
            TsTypeReceiverShape::ObjectLiteral,
        ] {
            assert_eq!(
                declared_answer(shape, TsTypeDeclarationKind::Method),
                (CallPrecision::Conservative, RefinedCallConfidence::Medium),
                "{shape:?}"
            );
        }
    }

    #[test]
    fn only_a_declaration_a_call_runs_is_claimed_exactly() {
        for kind in [
            TsTypeDeclarationKind::Function,
            TsTypeDeclarationKind::Method,
            TsTypeDeclarationKind::Constructor,
        ] {
            assert_eq!(
                declared_answer(TsTypeReceiverShape::Named, kind).0,
                CallPrecision::Exact,
                "{kind:?}"
            );
        }
        // The fixture's site is a method call: a class it names is not what
        // the call constructs.
        for kind in [
            TsTypeDeclarationKind::Class,
            TsTypeDeclarationKind::Accessor,
            TsTypeDeclarationKind::Unknown,
        ] {
            assert_eq!(
                declared_answer(TsTypeReceiverShape::Named, kind).0,
                CallPrecision::Conservative,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn a_class_is_claimed_exactly_only_where_the_call_constructs_it() {
        let answer = |kind| TypedAnswer {
            dispatch: TsTypeDispatchKind::Declared,
            status: TsTypeSiteStatus::Resolved,
            shape: TsTypeReceiverShape::Named,
            kind: TsTypeDeclarationKind::Class,
            constructs: kind,
        };
        assert_eq!(precision_for(answer(true), false), CallPrecision::Exact);
        assert_eq!(
            precision_for(answer(false), false),
            CallPrecision::Conservative
        );
        assert_eq!(
            precision_for(answer(true), true),
            CallPrecision::Conservative
        );
    }

    #[test]
    fn an_accessor_joins_only_through_its_span() {
        // `greet` is a getter whose span the native scan does not have; the
        // native `greet` function around its name is not the getter.
        let fixture = fixture();
        let sites = vec![site(
            &fixture.db,
            fixture.file,
            TsTypeSiteStatus::Resolved,
            0,
            19,
        )];
        let mut getter = callee(
            &fixture.db,
            fixture.file,
            "callee:getter",
            TsTypeDispatchKind::Declared,
            101,
            150,
        );
        getter.kind = TsTypeDeclarationKind::Accessor;

        let (output, report) = derive_ts_type_refinements(&fixture.db, &sites, &[getter], &[]);

        assert!(output.edges.is_empty());
        assert_eq!(report.unmatched_callees, 1);
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
    fn a_declared_target_at_a_union_site_is_one_candidate_not_the_answer() {
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
                TsTypeDispatchKind::Declared,
                100,
                160,
            ),
            callee(
                &fixture.db,
                fixture.file,
                "callee:two",
                TsTypeDispatchKind::Declared,
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
                .all(|edge| edge.precision == CallPrecision::Conservative),
            "a site with several declared targets has no single exact one"
        );
    }

    #[test]
    fn an_inferred_target_may_be_reached_but_is_never_claimed_exactly() {
        // The checker typed the callee, but the binding it read can hold
        // another value at run time: it is reassigned, or its declared type
        // comes from somewhere other than the value it holds.
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
            TsTypeDispatchKind::Inferred,
            100,
            160,
        )];

        let (output, _) = derive_ts_type_refinements(&fixture.db, &sites, &callees, &[]);

        assert_eq!(output.edges.len(), 1);
        let edge = &output.edges[0];
        assert_eq!(edge.target_function, Some(fixture.callee_one));
        assert_eq!(edge.precision, CallPrecision::Conservative);
        assert_eq!(edge.confidence, RefinedCallConfidence::Medium);
        assert!(edge.evidence.iter().any(|item| item == "dispatch=inferred"));
    }

    #[test]
    fn a_call_the_native_scan_does_not_have_is_not_given_the_call_around_it() {
        // `outer(inner())` where the syntax frontend recorded only `outer`:
        // the typed answer for `inner` describes a different call, and joining
        // it to `outer` would claim a target `outer` never reaches.
        let fixture = fixture();
        let sites = vec![site(
            &fixture.db,
            fixture.file,
            TsTypeSiteStatus::Resolved,
            8,
            15,
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

        assert!(output.edges.is_empty());
        assert_eq!(report.unmatched_callsites, 1);
        assert_eq!(report.matched_callsites, 0);
    }

    #[test]
    fn a_call_that_only_shares_its_start_with_a_native_call_does_not_join_it() {
        // `chain().greet()` recorded natively as the outer call only: the
        // sidecar's inner `chain()` begins at the same byte but ends earlier.
        let fixture = fixture();
        let sites = vec![site(
            &fixture.db,
            fixture.file,
            TsTypeSiteStatus::Resolved,
            0,
            9,
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

        assert!(output.edges.is_empty());
        assert_eq!(report.unmatched_callsites, 1);
    }

    #[test]
    fn a_parenthesised_callee_joins_the_native_call_with_the_same_end() {
        // `(function () {})()`: the type checker's call starts at the
        // parenthesis and the syntax frontend's at the function inside it.
        let fixture = fixture_with_site(1, 19);
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

        assert_eq!(report.matched_callsites, 1);
        assert_eq!(output.edges.len(), 1);
        assert_eq!(output.edges[0].site, CallSiteId(0));
    }

    #[test]
    fn a_declaration_no_native_function_is_named_for_gets_no_enclosing_function() {
        // The callee lies inside `run`, but nothing native is named for it:
        // the function around a declaration is not the declaration.
        let fixture = fixture();
        let sites = vec![site(
            &fixture.db,
            fixture.file,
            TsTypeSiteStatus::Resolved,
            0,
            19,
        )];
        let mut inner = callee(
            &fixture.db,
            fixture.file,
            "callee:inner",
            TsTypeDispatchKind::Declared,
            5,
            12,
        );
        inner.name = "trackStream".to_string();
        inner.name_span = Some(span(fixture.file, 5, 10));

        let (output, report) = derive_ts_type_refinements(&fixture.db, &sites, &[inner], &[]);

        assert!(output.edges.is_empty());
        assert_eq!(report.unmatched_callees, 1);
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
        external.kind = TsTypeDeclarationKind::External;
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
        // Code outside the scan is not seen: another value can stand in for it.
        assert_eq!(output.edges[0].precision, CallPrecision::Conservative);
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
    fn an_exactly_resolved_edge_survives_refined_call_validation() {
        // A typed edge claims exact precision, and the refined-call validator
        // refuses exact precision from a modelled provenance. Debug builds run
        // that validator, and a violation there does not only add a
        // `polint/internal` diagnostic: it marks `polint.refined_calls`
        // validation-rejected and blocks every rule that requested calls.
        let mut fixture = fixture();
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

        let (output, _) = derive_ts_type_refinements(&fixture.db, &sites, &callees, &[]);
        assert_eq!(output.edges[0].precision, CallPrecision::Exact);
        fixture
            .db
            .replace_refined_call_facts(output)
            .expect("typed edges install");

        let mut diagnostics = Vec::new();
        crate::analysis_neutral::refined_calls::validate::validate_refined_calls(
            &fixture.db,
            &mut diagnostics,
        );

        assert!(
            diagnostics.is_empty(),
            "typed edges must pass refined-call validation: {:?}",
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn two_calls_that_begin_at_the_same_offset_keep_their_own_targets() {
        // `greeter.chain().greet(name)` and its inner `greeter.chain()` start
        // at the same byte. Joining on the start alone would hand the outer
        // call the inner call's target, which is a fabricated call-graph edge
        // rather than a missing one.
        let mut db = LocalAnalysisDb::new();
        let file = db.add_file(
            "src/app.ts".into(),
            "src/app.ts".to_string(),
            "greeter.chain().greet(name);".to_string(),
        );
        let caller = db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "run".to_string(),
            span(file, 0, 28),
            Language::TypeScript,
            false,
            true,
            1,
            Vec::new(),
        ));
        let greet = db.push_function(FunctionFact::new(
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
        let chain = db.push_function(FunctionFact::new(
            FunctionId::from_raw(0),
            file,
            "Loud.chain".to_string(),
            span(file, 200, 260),
            Language::TypeScript,
            false,
            false,
            1,
            Vec::new(),
        ));
        let native = |id: u64, end: u32, key: &str, interner: &StableKeyInterner| CallSiteFact {
            id: CallSiteId(id),
            language: Language::TypeScript,
            file,
            caller,
            owner_symbol: None,
            body: MirBodyId(0),
            operation: MirOpId(id),
            span: span(file, 0, end),
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
            stable_key: interner.intern(key),
        };
        let interner = db.stable_key_interner();
        db.replace_call_facts(CallOutput {
            sites: vec![
                native(0, 27, "call:outer", &interner),
                native(1, 15, "call:inner", &interner),
            ],
            targets: Vec::new(),
            unresolved: Vec::new(),
        })
        .expect("valid call facts");

        let typed_site = |key: &str, end: u32| TsTypeCallsiteInput {
            stable_key: interner.intern(key),
            file: Some(file),
            span: Some(span(file, 0, end)),
            status: TsTypeSiteStatus::Resolved,
            receiver_shape: TsTypeReceiverShape::Named,
            receiver_printed: Some("Loud".to_string()),
        };
        let typed_callee = |key: &str, site: &str, start: u32, end: u32| TsTypeCalleeInput {
            stable_key: interner.intern(key),
            callsite_stable_key: interner.intern(site),
            dispatch: TsTypeDispatchKind::Declared,
            kind: TsTypeDeclarationKind::Method,
            name: "greet".to_string(),
            external: None,
            file: Some(file),
            span: Some(span(file, start, end)),
            name_span: Some(span(file, start, start + 5)),
        };

        let (output, report) = derive_ts_type_refinements(
            &db,
            &[typed_site("ts:outer", 27), typed_site("ts:inner", 15)],
            &[
                typed_callee("callee:greet", "ts:outer", 100, 160),
                typed_callee("callee:chain", "ts:inner", 200, 260),
            ],
            &[],
        );

        assert_eq!(report.matched_callsites, 2);
        let targets = output
            .edges
            .iter()
            .map(|edge| (edge.site, edge.target_function))
            .collect::<BTreeSet<_>>();
        assert_eq!(
            targets,
            BTreeSet::from([(CallSiteId(0), Some(greet)), (CallSiteId(1), Some(chain))]),
            "each call must keep only its own target"
        );
    }

    #[test]
    fn only_the_typescript_family_is_covered() {
        assert!(covers_language(Language::TypeScript));
        assert!(covers_language(Language::JavaScript));
        assert!(!covers_language(Language::Go));
    }
}
