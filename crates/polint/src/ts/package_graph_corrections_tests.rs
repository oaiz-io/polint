//! The correction layer over real package graphs: the edges a recorded run
//! saw, added beside the engine's answers, anchored by identity, and stale
//! once the text they were written against changes.
//!
//! The fixture is written here: a package whose one dynamic call the engine
//! cannot follow. The proof over commander reads its fetched source and its
//! recording; it is ignored by default and run explicitly.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::analysis_kernel::{AnalysisKernel, KernelInput};
use crate::analysis_neutral::package_graph::corrections::{
    Anchor, ApplyReport, CorrectedGraph, Correction, CorrectionEvidence, Loaded, Op, apply,
    observed_corrections, recording_id,
};
use crate::analysis_neutral::package_graph::{FunctionIdentity, PackageGraph};
use crate::analysis_neutral::points_to::tiers::EdgeClaim;
use crate::analysis_plan::AnalysisPlan;
use crate::cache::Cache;
use crate::config::load_config;
use crate::ts::points_to::package_graph::export;

const REFINED: &[&str] = &[
    "syntax",
    "calls",
    "control_flow",
    "dataflow",
    "call_graph",
    "symbols",
    "references",
];

const FIXTURE: &str = "\
function a() { return 1; }
function b() { return 2; }
function pick(name) { return globalThis[name]; }
function run(name) { return pick(name)(); }
function direct() { return a(); }
module.exports = { run, direct };
";

const RUN: &str = "fixture-recording";
const DRIVER: &str = "drivers/fixture.mjs";

/// The package graph of the package rooted at `root`, analysed alone.
fn package_graph_of(root: &Path) -> PackageGraph {
    let mut loaded = load_config(root).expect("config loads");
    loaded.config.workspace.exclude = vec!["**/.git/**".into(), "**/node_modules/**".into()];
    loaded.respect_gitignore = false;
    let plan = AnalysisPlan::from_capability_names_for_test(REFINED);
    let output = AnalysisKernel::run(KernelInput {
        loaded: &loaded,
        cache: &Cache::new("", false),
        config_digest: "config",
        rule_digest: "rules",
        plan: &plan,
        parallel: false,
    })
    .expect("kernel should run");
    export(&output.db, root, None).expect("export the package graph")
}

fn write_fixture(root: &Path, source: &str) {
    std::fs::write(
        root.join("package.json"),
        r#"{"name":"fixture","version":"1.0.0","main":"index.js"}"#,
    )
    .expect("write package.json");
    std::fs::write(root.join("index.js"), source).expect("write index.js");
}

/// `index.js@start..end` of `part`, found inside the first `context`.
fn key_in(source: &str, context: &str, part: &str) -> String {
    let start = source.find(context).expect("context in source")
        + context.find(part).expect("part in context");
    format!("index.js@{start}..{}", start + part.len())
}

fn fixture_calls(source: &str) -> String {
    let edge = |kind: &str, site: String, callee: String, name: &str| serde_json::json!({"kind": kind, "site": site, "callee": callee, "callee_name": name});
    let function = |text: &str| key_in(source, text, text);
    serde_json::json!({
        "schema": "polint-js-bench-observed-calls-2",
        "edges": [
            edge(
                "call",
                key_in(source, "return pick(name)();", "pick(name)()"),
                function("function b() { return 2; }"),
                "b",
            ),
            edge(
                "call",
                key_in(source, "return pick(name)();", "pick(name)"),
                function("function pick(name) { return globalThis[name]; }"),
                "pick",
            ),
            edge(
                "call",
                key_in(source, "return a();", "a()"),
                function("function a() { return 1; }"),
                "a",
            ),
            edge(
                "implicit",
                key_in(source, "return a();", "a()"),
                function("function b() { return 2; }"),
                "b",
            ),
            edge(
                "call",
                "index.js@0..1".to_string(),
                function("function b() { return 2; }"),
                "b",
            ),
        ],
    })
    .to_string()
}

/// The call site node a correction names in `graph`, found by its exact
/// identity, and its target.
fn anchored(graph: &PackageGraph, correction: &Correction) -> (u32, u32) {
    let (Anchor::CallSite { function, site }, Op::AddEdge { target }) =
        (&correction.anchor, &correction.op)
    else {
        panic!("a recorded edge is a call site and a target: {correction:?}");
    };
    let node_of = |identity| {
        graph
            .functions
            .iter()
            .find(|entry| &entry.identity == identity)
            .map(|entry| entry.node)
            .expect("the function of a correction is in the graph it was written against")
    };
    let caller = node_of(function);
    let callsite = graph
        .callsites
        .iter()
        .find(|entry| entry.caller == caller && &entry.identity == site)
        .map(|entry| entry.node)
        .expect("the call site of a correction is in the graph it was written against");
    (callsite, node_of(target))
}

/// The corrections whose edge the graph does not hold, with the layer
/// applied: either the engine's answer claims it or the layer adds it.
fn missed<'a>(
    graph: &PackageGraph,
    corrected: &CorrectedGraph<'_>,
    corrections: &'a [Correction],
) -> Vec<&'a Correction> {
    let added = corrected
        .corrections
        .iter()
        .map(|edge| (edge.callsite, edge.target))
        .collect::<BTreeSet<_>>();
    let answers = graph
        .sites
        .iter()
        .map(|site| (site.callsite, site))
        .collect::<BTreeMap<_, _>>();
    corrections
        .iter()
        .filter(|correction| {
            let (callsite, target) = anchored(graph, correction);
            let derived = answers.get(&callsite).is_some_and(|answer| {
                !matches!(answer.claim, EdgeClaim::Unassigned) && answer.targets.contains(&target)
            });
            !derived && !added.contains(&(callsite, target))
        })
        .collect()
}

fn assert_observed_conservative(corrected: &CorrectedGraph<'_>, run: &str, driver: &str) {
    let json = serde_json::to_value(corrected).expect("serialize the corrected graph");
    for edge in json["corrections"].as_array().expect("corrections") {
        assert_eq!(edge["tier"], "conservative", "{edge}");
    }
    for edge in &corrected.corrections {
        assert_eq!(
            edge.evidence,
            CorrectionEvidence::Observed {
                run: run.to_string(),
                driver: driver.to_string(),
            }
        );
    }
}

/// The corrections an edit to `edited` must make stale: those anchored to,
/// or targeting, a function whose text contains it, or a function the edit
/// moved whose text another function of its file shares, which is found
/// only at its own span.
fn expected_stale(
    graph: &PackageGraph,
    corrections: &[Correction],
    edited: &FunctionIdentity,
) -> Vec<usize> {
    let encloses = |function: &FunctionIdentity| {
        function.path == edited.path && function.start <= edited.start && edited.end <= function.end
    };
    let moved =
        |function: &FunctionIdentity| function.path == edited.path && function.start >= edited.end;
    let shared = |function: &FunctionIdentity| {
        graph
            .functions
            .iter()
            .filter(|other| {
                other.identity.path == function.path && other.identity.body == function.body
            })
            .count()
            > 1
    };
    let lost =
        |function: &FunctionIdentity| encloses(function) || (moved(function) && shared(function));
    corrections
        .iter()
        .enumerate()
        .filter(|(_, correction)| {
            let (Anchor::CallSite { function, .. }, Op::AddEdge { target }) =
                (&correction.anchor, &correction.op)
            else {
                return false;
            };
            lost(function) || lost(target)
        })
        .map(|(index, _)| index)
        .collect()
}

/// Appends `void 0;` to the body of `function` in the package at `root`.
fn append_statement(root: &Path, function: &FunctionIdentity) {
    let path = root.join(&function.path);
    let mut source = std::fs::read_to_string(&path).expect("read the edited file");
    let close = function.end as usize - 1;
    assert_eq!(
        &source[close..=close],
        "}",
        "the function's body ends in a brace"
    );
    source.insert_str(close, " void 0; ");
    std::fs::write(&path, source).expect("write the edited file");
}

fn load_fixture(graph: &PackageGraph) -> Loaded {
    observed_corrections(graph, &fixture_calls(FIXTURE), RUN, DRIVER).expect("the recording loads")
}

#[test]
fn recorded_edges_complete_the_graph_beside_the_engine_answers() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_fixture(temp.path(), FIXTURE);
    let graph = package_graph_of(temp.path());
    let loaded = load_fixture(&graph);
    let reasons = loaded
        .unanchored
        .iter()
        .map(|(_, _, reason)| *reason)
        .collect::<Vec<_>>();
    assert_eq!(reasons, ["no call expression", "no call site"]);
    assert_eq!(loaded.corrections.len(), 3);

    let (corrected, report) = apply(&graph, &loaded.corrections);
    assert_eq!((report.applied, report.derived), (1, 2), "{report:?}");
    assert!(report.stale.is_empty(), "{report:?}");
    assert!(missed(&graph, &corrected, &loaded.corrections).is_empty());
    assert_observed_conservative(&corrected, RUN, DRIVER);
}

#[test]
fn applying_corrections_leaves_the_engine_graph_unchanged() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_fixture(temp.path(), FIXTURE);
    let graph = package_graph_of(temp.path());
    let before = serde_json::to_vec(&graph).expect("serialize");
    let loaded = load_fixture(&graph);
    let (corrected, _) = apply(&graph, &loaded.corrections);
    assert!(!corrected.corrections.is_empty());
    assert_eq!(
        serde_json::to_vec(corrected.graph).expect("serialize"),
        before
    );
    let again = package_graph_of(temp.path());
    assert_eq!(serde_json::to_vec(&again).expect("serialize"), before);
}

#[test]
fn an_edit_stales_exactly_the_corrections_of_the_edited_function() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_fixture(temp.path(), FIXTURE);
    let graph = package_graph_of(temp.path());
    let loaded = load_fixture(&graph);
    let run = graph
        .functions
        .iter()
        .find(|function| function.name == "run")
        .map(|function| function.identity.clone())
        .expect("the fixture's run function");

    append_statement(temp.path(), &run);
    let edited = package_graph_of(temp.path());
    let (_, report) = apply(&edited, &loaded.corrections);
    assert_eq!(
        report.stale,
        expected_stale(&graph, &loaded.corrections, &run)
    );
    assert_eq!(report.stale.len(), 2, "{report:?}");
    assert_eq!((report.applied, report.derived), (0, 1), "{report:?}");
}

/// An edit that keeps every span still changes the text: a correction aimed
/// at the edited function is stale, though a function sits at its span.
#[test]
fn an_edit_of_the_same_length_stales_the_corrections_aimed_at_it() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_fixture(temp.path(), FIXTURE);
    let graph = package_graph_of(temp.path());
    let loaded = load_fixture(&graph);
    let b = graph
        .functions
        .iter()
        .find(|function| function.name == "b")
        .map(|function| function.identity.clone())
        .expect("the fixture's b function");

    write_fixture(
        temp.path(),
        &FIXTURE.replace("function b() { return 2; }", "function b() { return 3; }"),
    );
    let edited = package_graph_of(temp.path());
    assert!(
        edited
            .functions
            .iter()
            .any(|function| function.identity.path == b.path
                && function.identity.start == b.start
                && function.identity.end == b.end)
    );
    let (corrected, report) = apply(&edited, &loaded.corrections);
    assert_eq!(
        report.stale,
        expected_stale(&graph, &loaded.corrections, &b)
    );
    assert_eq!(report.stale.len(), 1, "{report:?}");
    assert!(corrected.corrections.is_empty(), "{report:?}");
}

#[test]
fn corrections_read_back_as_written() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_fixture(temp.path(), FIXTURE);
    let graph = package_graph_of(temp.path());
    let loaded = load_fixture(&graph);
    let json = serde_json::to_string(&loaded.corrections).expect("serialize");
    let read: Vec<Correction> = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(read, loaded.corrections);
}

#[test]
fn a_correction_for_another_package_is_not_applied() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_fixture(temp.path(), FIXTURE);
    let graph = package_graph_of(temp.path());
    let mut corrections = load_fixture(&graph).corrections;
    for correction in &mut corrections {
        correction.graph.name = Some("another".to_string());
    }
    let (corrected, report) = apply(&graph, &corrections);
    assert!(corrected.corrections.is_empty());
    assert_eq!(report.other_package, corrections.len());
}

/// The engine never reads the correction layer: no source of the crate
/// outside the layer and its tests names it.
#[test]
fn the_engine_never_reads_the_correction_layer() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let names = [
        "corrections::",
        "observed_corrections",
        "CorrectedGraph",
        "CorrectedEdge",
        "ApplyReport",
    ];
    let mut readers = Vec::new();
    let mut pending = vec![src.clone()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).expect("read source directory") {
            let path = entry.expect("source entry").path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if name == "corrections.rs" || name == "tests.rs" || name.ends_with("_tests.rs") {
                continue;
            }
            if path.extension().is_some_and(|extension| extension == "rs") {
                let text = std::fs::read_to_string(&path).expect("read source file");
                if names.iter().any(|needle| text.contains(needle)) {
                    readers.push(path.strip_prefix(&src).unwrap_or(&path).to_path_buf());
                }
            }
        }
    }
    readers.sort();
    assert_eq!(readers, Vec::<PathBuf>::new());
}

fn bench_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/js-analysis-bench")
}

/// Copies commander's analysed files, as the benchmark stages them.
fn stage_commander(into: &Path) {
    let fetched = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../research/evaluation-harness/repos/js-analysis-bench/commander-14.0.3");
    assert!(
        fetched.is_dir(),
        "commander is not fetched: run `make js-bench` once"
    );
    std::fs::create_dir_all(into.join("lib")).expect("create lib");
    for file in ["index.js", "package.json"] {
        std::fs::copy(fetched.join(file), into.join(file)).expect("copy commander file");
    }
    let mut lib = std::fs::read_dir(fetched.join("lib"))
        .expect("read commander lib")
        .map(|entry| entry.expect("lib entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "js"))
        .collect::<Vec<_>>();
    lib.sort();
    for path in lib {
        let name = path.file_name().expect("file name");
        std::fs::copy(&path, into.join("lib").join(name)).expect("copy commander lib file");
    }
    std::fs::write(
        into.join(".polint.toml"),
        "[workspace]\ninclude = [\"index.js\", \"lib/*.js\"]\nexclude = [\"**/node_modules/**\", \"**/.git/**\"]\n",
    )
    .expect("write .polint.toml");
}

fn print_report(label: &str, report: &ApplyReport) {
    println!(
        "{label}: applied {}, derived {}, stale {}, not applied {}, other package {}",
        report.applied,
        report.derived,
        report.stale.len(),
        report.not_applied,
        report.other_package
    );
}

#[test]
#[ignore = "reads commander's fetched source and recording: run explicitly"]
fn commander_recording_completes_its_package_graph() {
    let recording = bench_dir().join("observed/packages/commander.calls.json");
    let driver = "drivers/commander.mjs";
    let run = recording_id(&recording).expect("hash the recording");
    let calls = std::fs::read_to_string(&recording).expect("read the recording");

    let temp = tempfile::tempdir().expect("tempdir");
    stage_commander(temp.path());
    let graph = package_graph_of(temp.path());
    let before = serde_json::to_vec(&graph).expect("serialize");
    let loaded = observed_corrections(&graph, &calls, &run, driver).expect("the recording loads");
    let mut reasons = BTreeMap::<&str, usize>::new();
    for (_, _, reason) in &loaded.unanchored {
        *reasons.entry(reason).or_default() += 1;
    }
    println!(
        "commander: {} corrections, unanchored {reasons:?}",
        loaded.corrections.len()
    );
    let unanchored_calls = loaded
        .unanchored
        .iter()
        .filter(|(_, _, reason)| *reason != "no call expression")
        .collect::<Vec<_>>();
    assert!(
        unanchored_calls.is_empty(),
        "recorded call and callback edges with no anchor: {unanchored_calls:?}"
    );

    let (corrected, report) = apply(&graph, &loaded.corrections);
    print_report("commander", &report);
    assert!(report.stale.is_empty(), "{report:?}");
    assert_eq!(
        report.applied + report.derived,
        loaded.corrections.len(),
        "{report:?}"
    );
    assert!(missed(&graph, &corrected, &loaded.corrections).is_empty());
    assert_observed_conservative(&corrected, &run, driver);
    assert_eq!(
        serde_json::to_vec(corrected.graph).expect("serialize"),
        before
    );
    assert_eq!(
        serde_json::to_vec(&package_graph_of(temp.path())).expect("serialize"),
        before
    );

    // Edit the innermost function with the most corrections anchored in it.
    let mut anchored_in = BTreeMap::<&FunctionIdentity, usize>::new();
    for correction in &loaded.corrections {
        if let Anchor::CallSite { function, .. } = &correction.anchor {
            *anchored_in.entry(function).or_default() += 1;
        }
    }
    let innermost = |function: &FunctionIdentity| {
        !graph.functions.iter().any(|other| {
            other.identity != *function
                && other.identity.path == function.path
                && function.start <= other.identity.start
                && other.identity.end <= function.end
        })
    };
    let (edited, _) = anchored_in
        .iter()
        .filter(|(function, _)| innermost(function))
        .max_by_key(|(function, count)| (**count, std::cmp::Reverse(*function)))
        .expect("a function with corrections");
    let edited = (*edited).clone();
    println!("edited {}@{}..{}", edited.path, edited.start, edited.end);
    append_statement(temp.path(), &edited);
    let changed = package_graph_of(temp.path());
    let (_, report) = apply(&changed, &loaded.corrections);
    print_report("commander after the edit", &report);
    let expected = expected_stale(&graph, &loaded.corrections, &edited);
    assert!(!expected.is_empty());
    assert_eq!(report.stale, expected);
    assert_eq!(
        report.applied + report.derived + report.stale.len(),
        loaded.corrections.len(),
        "{report:?}"
    );
}
