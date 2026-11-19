//! The correction layer: records kept beside a package graph that add edges
//! the engine did not derive, each anchored by stable identity and carrying
//! the evidence for it.
//!
//! The engine never reads this layer. [`apply`] runs on a finished package
//! graph, before stitching, and leaves the graph as the engine wrote it: the
//! edges it adds sit beside the graph's answers, each with its evidence.
//!
//! An anchor holds the identity of a function with the hash of its source
//! text. A correction whose function's text changed no longer applies; it is
//! reported stale. A function's text includes the functions nested in it, so
//! an edit to a function also stales the corrections anchored to every
//! function that encloses it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::format::{PackageGraph, SiteEntry};
use super::identity::{CallsiteIdentity, FunctionIdentity, PackageIdentity};
use crate::analysis_api::TS_JS_MODULE_FUNCTION_NAME;
use crate::analysis_neutral::points_to::tiers::EdgeClaim;

/// The recorder formats the loader reads.
const OBSERVED_CALLS_SCHEMAS: &[&str] = &[
    "polint-js-bench-observed-calls-1",
    "polint-js-bench-observed-calls-2",
];
/// Recorded edges with a call expression. An `implicit` edge (an accessor, a
/// coercion, a protocol call) has no call site to anchor to.
const OBSERVED_KINDS: &[&str] = &["call", "callback"];

/// One correction to the graph of a package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Correction {
    /// The graph the correction was written against. It applies to any graph
    /// of the package with the same name; its anchor decides whether it still
    /// holds.
    pub(crate) graph: PackageIdentity,
    pub(crate) anchor: Anchor,
    pub(crate) op: Op,
    pub(crate) evidence: CorrectionEvidence,
    pub(crate) stage: Stage,
    /// The engine version that built the graph the correction was written
    /// against.
    pub(crate) engine: String,
}

/// What a correction is about.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Anchor {
    Function(FunctionIdentity),
    CallSite {
        function: FunctionIdentity,
        site: CallsiteIdentity,
    },
}

/// What a correction changes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Op {
    /// The call site reaches `target`.
    AddEdge { target: FunctionIdentity },
    /// The call site never reaches `target`.
    RejectEdge { target: FunctionIdentity },
    /// The call site reaches no function other than those its answer holds.
    MarkComplete,
    /// A fact about the package's boundary, in words.
    AddBoundaryFact { fact: String },
}

/// Why a correction holds.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CorrectionEvidence {
    /// A run saw the call: `run` names the recording, `driver` the program
    /// that ran the package.
    Observed { run: String, driver: String },
    /// A test shows it: where the test is and how to run it.
    Test { path: String, command: String },
    /// A review found it: who reviewed and what they read.
    Review { who: String, read: Vec<String> },
}

/// The stage that wrote a correction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Stage {
    RecordedExecution,
    AgentReview,
}

/// Recorded edges as corrections, and the edges that could not be anchored.
#[derive(Debug, Default)]
pub(crate) struct Loaded {
    pub(crate) corrections: Vec<Correction>,
    /// `(site, callee, reason)` of each recorded edge with no correction.
    pub(crate) unanchored: Vec<(String, String, &'static str)>,
}

#[derive(Deserialize)]
struct ObservedCalls {
    schema: String,
    edges: Vec<ObservedEdge>,
}

#[derive(Deserialize)]
struct ObservedEdge {
    kind: String,
    site: String,
    callee: String,
    callee_name: Option<String>,
}

/// The edges a recorded run of the package saw, as corrections to `graph`.
///
/// `calls` is the recorder's output for the package, with paths relative to
/// the package root. `run` names the recording and `driver` the program that
/// ran it. Each edge is anchored to a call site and a function of the graph
/// by the benchmark's matching rule: see [`Spans`].
pub(crate) fn observed_corrections(
    graph: &PackageGraph,
    calls: &str,
    run: &str,
    driver: &str,
) -> anyhow::Result<Loaded> {
    let calls: ObservedCalls = serde_json::from_str(calls)?;
    anyhow::ensure!(
        OBSERVED_CALLS_SCHEMAS.contains(&calls.schema.as_str()),
        "unknown recorder schema {:?}",
        calls.schema
    );
    let spans = Spans::new(graph);
    let mut loaded = Loaded::default();
    let mut seen = BTreeSet::new();
    for edge in &calls.edges {
        if !OBSERVED_KINDS.contains(&edge.kind.as_str()) {
            loaded
                .unanchored
                .push((edge.site.clone(), edge.callee.clone(), "no call expression"));
            continue;
        }
        let (Some(site), Some(callee)) = (parse_key(&edge.site), parse_key(&edge.callee)) else {
            loaded
                .unanchored
                .push((edge.site.clone(), edge.callee.clone(), "unreadable span"));
            continue;
        };
        let Some(site) = spans.site_for(site) else {
            loaded
                .unanchored
                .push((edge.site.clone(), edge.callee.clone(), "no call site"));
            continue;
        };
        let Some(target) = spans.function_for(callee, edge.callee_name.as_deref()) else {
            loaded
                .unanchored
                .push((edge.site.clone(), edge.callee.clone(), "no function"));
            continue;
        };
        let anchor = Anchor::CallSite {
            function: spans.functions[&site.caller].identity.clone(),
            site: site.identity.clone(),
        };
        let op = Op::AddEdge {
            target: target.identity.clone(),
        };
        if seen.insert((anchor.clone(), op.clone())) {
            loaded.corrections.push(Correction {
                graph: graph.identity.clone(),
                anchor,
                op,
                evidence: CorrectionEvidence::Observed {
                    run: run.to_string(),
                    driver: driver.to_string(),
                },
                stage: Stage::RecordedExecution,
                engine: graph.identity.engine.clone(),
            });
        }
    }
    Ok(loaded)
}

/// The run id of a recording: the hash of its bytes.
pub(crate) fn recording_id(path: &Path) -> anyhow::Result<String> {
    use sha2::{Digest as _, Sha256};
    let bytes = std::fs::read(path)?;
    let digest = Sha256::digest(&bytes);
    Ok(format!(
        "sha256:{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}

/// A path and a byte span, as `path@start..end`.
type Key<'a> = (&'a str, u32, u32);

fn parse_key(key: &str) -> Option<Key<'_>> {
    let (path, span) = key.rsplit_once('@')?;
    let (start, end) = span.split_once("..")?;
    Some((path, start.parse().ok()?, end.parse().ok()?))
}

/// A function of the graph with its identity.
struct Function<'a> {
    node: u32,
    name: &'a str,
    identity: &'a FunctionIdentity,
}

/// A call site of the graph at its place in its file.
struct Site<'a> {
    node: u32,
    caller: u32,
    identity: &'a CallsiteIdentity,
    path: &'a str,
    start: u32,
    end: u32,
}

/// The call sites and functions of a graph by span, and the benchmark's two
/// matching rules (`Spans` in `tests/js-analysis-bench/report.py`): the
/// loader anchors a recorded edge to the site and function the benchmark
/// scores it against.
struct Spans<'a> {
    functions: BTreeMap<u32, Function<'a>>,
    sites: Vec<Site<'a>>,
}

impl<'a> Spans<'a> {
    fn new(graph: &'a PackageGraph) -> Self {
        let functions = graph
            .functions
            .iter()
            .map(|function| {
                let entry = Function {
                    node: function.node,
                    name: &function.name,
                    identity: &function.identity,
                };
                (function.node, entry)
            })
            .collect::<BTreeMap<_, _>>();
        let sites = graph
            .callsites
            .iter()
            .filter_map(|site| {
                let caller = functions.get(&site.caller)?.identity;
                Some(Site {
                    node: site.node,
                    caller: site.caller,
                    identity: &site.identity,
                    path: &caller.path,
                    start: caller.start + site.identity.start,
                    end: caller.start + site.identity.end,
                })
            })
            .collect();
        Self { functions, sites }
    }

    /// The site with the same span, else the smallest site in the same file
    /// ending at the same byte.
    fn site_for(&self, (path, start, end): Key<'_>) -> Option<&Site<'a>> {
        let mut candidates = self
            .sites
            .iter()
            .filter(|site| site.path == path && site.end == end);
        let candidates = candidates.by_ref().collect::<Vec<_>>();
        candidates
            .iter()
            .find(|site| site.start == start)
            .or_else(|| candidates.iter().min_by_key(|site| site.end - site.start))
            .copied()
    }

    /// The function with the same span, else the innermost one in the same
    /// file ending at the same byte. A recorded class `constructor` with no
    /// function ending where it does maps to the innermost function enclosing
    /// it: the class. A recorded TypeScript arrow ends before a trailing type
    /// assertion its stripped text lacks; with no function ending where it
    /// does, the function in the same file starting where it starts and
    /// ending nearest after it is the one called.
    fn function_for(
        &self,
        (path, start, end): Key<'_>,
        name: Option<&str>,
    ) -> Option<&Function<'a>> {
        let in_file = || {
            self.functions.values().filter(move |function| {
                function.identity.path == path && function.name != TS_JS_MODULE_FUNCTION_NAME
            })
        };
        let candidates = in_file()
            .filter(|function| function.identity.end == end)
            .collect::<Vec<_>>();
        if let Some(exact) = candidates.iter().find(|f| f.identity.start == start) {
            return Some(exact);
        }
        if let Some(enclosing) = candidates
            .iter()
            .filter(|f| f.identity.start <= start)
            .max_by_key(|f| f.identity.start)
        {
            return Some(enclosing);
        }
        if candidates.is_empty() && is_typescript(path) {
            let stripped = in_file()
                .filter(|f| f.identity.start == start && f.identity.end > end)
                .min_by_key(|f| f.identity.end);
            if stripped.is_some() {
                return stripped;
            }
        }
        if candidates.is_empty() && name == Some("constructor") {
            return in_file()
                .filter(|f| f.identity.start <= start && end <= f.identity.end)
                .min_by_key(|f| f.identity.end - f.identity.start);
        }
        candidates
            .into_iter()
            .min_by_key(|f| f.identity.end - f.identity.start)
    }
}

fn is_typescript(path: &str) -> bool {
    [".ts", ".tsx", ".mts", ".cts"]
        .iter()
        .any(|suffix| path.ends_with(suffix))
}

/// An edge the correction layer adds to a call site.
///
/// It is conservative: a recorded run proves that the site reaches the
/// function, never that the site reaches nothing else.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) struct CorrectedEdge {
    pub(crate) callsite: u32,
    pub(crate) target: u32,
    pub(crate) tier: CorrectedTier,
    pub(crate) evidence: CorrectionEvidence,
}

/// The tier of an edge the correction layer adds: always conservative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CorrectedTier {
    Conservative,
}

/// A package graph as the engine wrote it, with the edges the correction
/// layer adds beside its answers.
#[derive(Debug, Serialize)]
pub(crate) struct CorrectedGraph<'a> {
    pub(crate) graph: &'a PackageGraph,
    pub(crate) corrections: Vec<CorrectedEdge>,
}

/// What became of each correction.
#[derive(Debug, Default, Serialize)]
pub(crate) struct ApplyReport {
    /// Added an edge the engine's answer does not hold, by correction index.
    pub(crate) applied: Vec<usize>,
    /// The engine's answer already claims the edge: the correction can be
    /// retired and kept as a fixture. By correction index.
    pub(crate) derived: Vec<usize>,
    /// Its function, call site or target is no longer in the graph with the
    /// same text; the correction is not applied. By correction index.
    pub(crate) stale: Vec<usize>,
    /// An operation this layer records but does not carry out: an edge is
    /// only ever added, never removed or closed.
    pub(crate) not_applied: usize,
    /// Written for another package.
    pub(crate) other_package: usize,
}

/// The graph with the edges of `corrections` beside it.
///
/// The graph itself is not changed. A correction applies to a graph of the
/// package it names; it is stale when its anchor or its target is not in
/// the graph with the same text.
pub(crate) fn apply<'a>(
    graph: &'a PackageGraph,
    corrections: &[Correction],
) -> (CorrectedGraph<'a>, ApplyReport) {
    let functions = FunctionIndex::new(graph);
    let sites = graph
        .callsites
        .iter()
        .map(|site| ((site.caller, &site.identity), site.node))
        .collect::<BTreeMap<_, _>>();
    let answers = graph
        .sites
        .iter()
        .map(|site| (site.callsite, site))
        .collect::<BTreeMap<_, _>>();

    let mut report = ApplyReport::default();
    let mut edges = BTreeSet::new();
    for (index, correction) in corrections.iter().enumerate() {
        if correction.graph.name != graph.identity.name {
            report.other_package += 1;
            continue;
        }
        let (Anchor::CallSite { function, site }, Op::AddEdge { target }) =
            (&correction.anchor, &correction.op)
        else {
            report.not_applied += 1;
            continue;
        };
        let callsite = functions
            .find(function)
            .and_then(|caller| sites.get(&(caller, site)).copied());
        let (Some(callsite), Some(target)) = (callsite, functions.find(target)) else {
            report.stale.push(index);
            continue;
        };
        if answers
            .get(&callsite)
            .is_some_and(|answer| claims(answer, target))
        {
            report.derived.push(index);
            continue;
        }
        report.applied.push(index);
        edges.insert(CorrectedEdge {
            callsite,
            target,
            tier: CorrectedTier::Conservative,
            evidence: correction.evidence.clone(),
        });
    }
    let corrected = CorrectedGraph {
        graph,
        corrections: edges.into_iter().collect(),
    };
    (corrected, report)
}

/// Whether the engine's answer at a site claims `target`, in any tier.
fn claims(answer: &SiteEntry, target: u32) -> bool {
    !matches!(answer.claim, EdgeClaim::Unassigned) && answer.targets.binary_search(&target).is_ok()
}

/// The functions of a graph by identity. A function whose span moved but
/// whose text did not is still found: by its text, when no other function
/// of its file has the same text.
struct FunctionIndex<'a> {
    exact: BTreeMap<&'a FunctionIdentity, u32>,
    by_text: BTreeMap<(&'a str, &'a str), Vec<u32>>,
}

impl<'a> FunctionIndex<'a> {
    fn new(graph: &'a PackageGraph) -> Self {
        let mut index = Self {
            exact: BTreeMap::new(),
            by_text: BTreeMap::new(),
        };
        for function in &graph.functions {
            index.exact.insert(&function.identity, function.node);
            index
                .by_text
                .entry((
                    function.identity.path.as_str(),
                    function.identity.body.as_str(),
                ))
                .or_default()
                .push(function.node);
        }
        index
    }

    fn find(&self, identity: &FunctionIdentity) -> Option<u32> {
        if let Some(node) = self.exact.get(identity) {
            return Some(*node);
        }
        match self
            .by_text
            .get(&(identity.path.as_str(), identity.body.as_str()))?
            .as_slice()
        {
            [node] => Some(*node),
            _ => None,
        }
    }
}
