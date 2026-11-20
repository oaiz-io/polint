use std::path::{Path, PathBuf};

use super::export;
use crate::analysis_kernel::{AnalysisKernel, KernelInput, KernelOutput};
use crate::analysis_plan::AnalysisPlan;
use crate::cache::Cache;
use crate::config::load_config;

const REFINED: &[&str] = &[
    "syntax",
    "calls",
    "control_flow",
    "dataflow",
    "call_graph",
    "symbols",
    "references",
];

/// A package of three modules joined by a re-exporting barrel.
fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/js-analysis-bench/constructs/esm-reexport/named-barrel")
}

fn stage(into: &Path) {
    std::fs::create_dir_all(into).expect("create staged directory");
    for entry in std::fs::read_dir(fixture()).expect("read fixture directory") {
        let entry = entry.expect("fixture entry").path();
        if entry.is_file() {
            let name = entry.file_name().expect("entry name");
            std::fs::copy(&entry, into.join(name)).expect("copy fixture file");
        }
    }
}

fn run(root: &Path, parallel: bool) -> KernelOutput {
    let loaded = load_config(root).expect("default config loads");
    let plan = AnalysisPlan::from_capability_names_for_test(REFINED);
    AnalysisKernel::run(KernelInput {
        loaded: &loaded,
        cache: &Cache::new("", false),
        config_digest: "config",
        rule_digest: "rules",
        plan: &plan,
        parallel,
    })
    .expect("kernel should run")
}

/// The serialized package graph of the package staged at `root`, analysed
/// on a pool of `threads` threads.
fn exported(root: &Path, threads: usize) -> String {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .expect("thread pool builds");
    pool.install(|| {
        let output = run(root, threads > 1);
        let graph = export(&output.db, root, None).expect("package graph exports");
        serde_json::to_string_pretty(&graph).expect("package graph serializes")
    })
}

#[test]
fn the_graph_does_not_depend_on_the_thread_count() {
    let dir = tempfile::tempdir().expect("temp dir");
    stage(dir.path());
    let one = exported(dir.path(), 1);
    let eight = exported(dir.path(), 8);
    assert!(one.contains("\"sites\""), "the graph holds its answers");
    assert_eq!(one, eight);
}

#[test]
fn the_graph_does_not_depend_on_where_the_package_is_checked_out() {
    let first = tempfile::tempdir().expect("temp dir");
    let second = tempfile::tempdir().expect("temp dir");
    let moved = second.path().join("deeper").join("checkout");
    stage(first.path());
    stage(&moved);
    assert_eq!(exported(first.path(), 1), exported(&moved, 1));
}

#[test]
fn the_graph_names_no_run_local_id() {
    let dir = tempfile::tempdir().expect("temp dir");
    stage(dir.path());
    let graph = exported(dir.path(), 1);
    for id in [
        "FileId",
        "FunctionId",
        "CallSiteId",
        "SemanticNodeId",
        "SymbolId",
        "ObjectTokenId",
        "PtVarId",
        "TypeFactId",
        "StableKeyId",
    ] {
        assert!(!graph.contains(id), "the graph names a {id}");
    }
    let root = dir.path().to_string_lossy();
    assert!(
        !graph.contains(root.as_ref()),
        "the graph names its checkout"
    );
}

#[test]
fn the_graph_answers_every_call_site_it_holds() {
    let dir = tempfile::tempdir().expect("temp dir");
    stage(dir.path());
    let output = run(dir.path(), false);
    let graph = export(&output.db, dir.path(), None).expect("package graph exports");
    assert!(!graph.functions.is_empty());
    assert!(!graph.constraints.is_empty());
    let sites = graph
        .sites
        .iter()
        .map(|site| site.callsite)
        .collect::<Vec<_>>();
    let callsites = graph
        .callsites
        .iter()
        .map(|site| site.node)
        .collect::<Vec<_>>();
    assert_eq!(sites, callsites);
    let count = u32::try_from(graph.nodes.len()).expect("node count fits u32");
    for site in &graph.sites {
        assert!(site.targets.iter().all(|target| *target < count));
    }
}

/// Eight factories, one per file, each returning an object whose methods come
/// from a prototype module. The calls in `main.js` reach those methods only
/// through the summary of what each factory returns.
fn stage_factories(into: &Path) {
    std::fs::write(
        into.join("package.json"),
        r#"{"name":"factories","main":"main.js"}"#,
    )
    .expect("write package.json");
    let mut main = String::new();
    for i in 0..8 {
        std::fs::write(
            into.join(format!("proto{i}.js")),
            format!("exports.handle{i} = function handle{i}() {{ return {i}; }};\n"),
        )
        .expect("write prototype");
        std::fs::write(
            into.join(format!("factory{i}.js")),
            format!(
                "var proto = require(\"./proto{i}\");\n\
                 module.exports = function make{i}() {{\n  var app = {{}};\n  \
                 Object.assign(app, proto);\n  return app;\n}};\n"
            ),
        )
        .expect("write factory");
        main.push_str(&format!(
            "var make{i} = require(\"./factory{i}\");\nvar app{i} = make{i}();\napp{i}.handle{i}();\n"
        ));
    }
    std::fs::write(into.join("main.js"), main).expect("write main.js");
}

/// The functions `main.js` calls in the package graph exported from `root`
/// on `threads` threads, as `(name, file)` pairs, with the graph itself.
fn targets_called_from_main(root: &Path, threads: usize) -> (Vec<(String, String)>, String) {
    let text = exported(root, threads);
    let graph: serde_json::Value = serde_json::from_str(&text).expect("graph parses");
    let function = |node: &serde_json::Value| {
        graph["functions"]
            .as_array()
            .expect("functions")
            .iter()
            .find(|function| &function["node"] == node)
            .map(|function| {
                (
                    function["name"].as_str().expect("name").to_string(),
                    function["identity"]["path"]
                        .as_str()
                        .expect("path")
                        .to_string(),
                )
            })
            .expect("target is a function of the graph")
    };
    let mut targets = Vec::new();
    for site in graph["sites"].as_array().expect("sites") {
        let callsite = graph["callsites"]
            .as_array()
            .expect("callsites")
            .iter()
            .find(|callsite| callsite["node"] == site["callsite"])
            .expect("site names a callsite");
        if function(&callsite["caller"]).1 != "main.js" {
            continue;
        }
        for target in site["targets"].as_array().expect("targets") {
            targets.push(function(target));
        }
    }
    targets.sort();
    (targets, text)
}

#[test]
fn return_summaries_across_files_do_not_depend_on_the_thread_count() {
    let dir = tempfile::tempdir().expect("temp dir");
    stage_factories(dir.path());
    let (one, one_graph) = targets_called_from_main(dir.path(), 1);
    let handlers = one
        .iter()
        .filter(|(name, _)| name.starts_with("handle"))
        .cloned()
        .collect::<Vec<_>>();
    let expected = (0..8)
        .map(|i| (format!("handle{i}"), format!("proto{i}.js")))
        .collect::<Vec<_>>();
    assert_eq!(
        handlers, expected,
        "each call reaches the method its factory returns"
    );
    let (eight, eight_graph) = targets_called_from_main(dir.path(), 8);
    assert_eq!(one, eight);
    assert_eq!(one_graph, eight_graph);
}
