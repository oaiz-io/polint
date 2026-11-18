//! The composition contract of package graphs: stitching the graphs of the
//! packages of a program gives the answers of analysing the program whole.
//!
//! The fixture is `tests/js-analysis-bench/composition`: an application and
//! two dependency packages under `node_modules`, with callbacks, factories,
//! prototype methods and a re-export crossing the package boundaries. Each
//! call site is compared by a portable key, the package and the path inside
//! it, so the two sides agree on names without sharing run-local ids.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::analysis_kernel::{AnalysisKernel, KernelInput, KernelOutput};
use crate::analysis_neutral::package_graph::{PackageGraph, package_of};
use crate::analysis_neutral::points_to::facts::PointsToBudgetStatus;
use crate::analysis_neutral::semantic_graph::facts::NodeKind;
use crate::analysis_neutral::solver::budget::SolverBudget;
use crate::analysis_plan::AnalysisPlan;
use crate::cache::Cache;
use crate::config::load_config;
use crate::ts::points_to::package_graph::{StitchError, Stitched, Tree, export, stitch};
use crate::ts::points_to::{SiteAnswer, TsPointsToInputs, site_answers, solve_ts_points_to};

const REFINED: &[&str] = &[
    "syntax",
    "calls",
    "control_flow",
    "dataflow",
    "call_graph",
    "symbols",
    "references",
];

/// The answer at one call site, with every node named portably.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PortableAnswer {
    claim: String,
    targets: Vec<String>,
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/js-analysis-bench/composition")
}

/// Copies the fixture out of the repository, so the repository's ignore
/// files do not hide its `node_modules`.
fn stage(into: &Path) {
    copy_tree(&fixture(), into);
}

fn copy_tree(from: &Path, into: &Path) {
    std::fs::create_dir_all(into).expect("create staged directory");
    let mut entries = std::fs::read_dir(from)
        .expect("read fixture directory")
        .map(|entry| entry.expect("fixture entry").path())
        .collect::<Vec<_>>();
    entries.sort();
    for entry in entries {
        let target = into.join(entry.file_name().expect("entry name"));
        if entry.is_dir() {
            copy_tree(&entry, &target);
        } else {
            std::fs::copy(&entry, &target).expect("copy fixture file");
        }
    }
}

/// Runs the kernel over `root` with the refined plan, analysing the files
/// under `node_modules` as well.
fn run(root: &Path) -> KernelOutput {
    run_excluding(root, &["**/.git/**"])
}

fn run_excluding(root: &Path, exclude: &[&str]) -> KernelOutput {
    let mut loaded = load_config(root).expect("default config loads");
    loaded.config.workspace.exclude = exclude.iter().map(|glob| glob.to_string()).collect();
    loaded.respect_gitignore = false;
    let plan = AnalysisPlan::from_capability_names_for_test(REFINED);
    AnalysisKernel::run(KernelInput {
        loaded: &loaded,
        cache: &Cache::new("", false),
        config_digest: "config",
        rule_digest: "rules",
        plan: &plan,
        parallel: false,
    })
    .expect("kernel should run")
}

/// The answers of analysing `root` as one program, keyed by call site.
fn whole_program_answers(root: &Path) -> BTreeMap<String, PortableAnswer> {
    let output = run(root);
    let db = &output.db;
    let interner = db.stable_key_interner();
    let inputs = TsPointsToInputs::from_db(db);
    let result = solve_ts_points_to(&interner, &inputs, &SolverBudget::default());

    let path_of = |file| {
        db.file(file)
            .map(|file| {
                let (package, path) = package_of(&file.relative_path);
                (package.to_string(), path.to_string())
            })
            .expect("every fact names an analysed file")
    };
    let functions = db
        .functions()
        .iter()
        .map(|function| (function.id, function))
        .collect::<BTreeMap<_, _>>();
    let callsites = db
        .call_sites()
        .iter()
        .map(|site| (site.id, site))
        .collect::<BTreeMap<_, _>>();
    let node_key = db
        .semantic_nodes()
        .iter()
        .filter_map(|node| {
            let key = match node.kind {
                NodeKind::Function(id) => {
                    let function = functions.get(&id)?;
                    let (package, path) = path_of(function.file);
                    format!(
                        "{package}:{path}@{}..{}#{}",
                        function.span.start_byte, function.span.end_byte, function.name
                    )
                }
                NodeKind::Callsite(id) => {
                    let site = callsites.get(&id)?;
                    let (package, path) = path_of(site.file);
                    format!(
                        "{package}:{path}@{}..{}",
                        site.span.start_byte, site.span.end_byte
                    )
                }
                _ => return None,
            };
            Some((node.id, key))
        })
        .collect::<BTreeMap<_, _>>();
    portable(&site_answers(&inputs, &result), &node_key)
}

fn portable(
    answers: &[SiteAnswer],
    node_key: &BTreeMap<crate::analysis_neutral::ids::SemanticNodeId, String>,
) -> BTreeMap<String, PortableAnswer> {
    answers
        .iter()
        .map(|answer| {
            let site = node_key[&answer.callsite].clone();
            let mut targets = answer
                .targets
                .iter()
                .map(|target| node_key[target].clone())
                .collect::<Vec<_>>();
            targets.sort();
            (
                site,
                PortableAnswer {
                    claim: format!("{:?} {:?}", answer.status, answer.claim),
                    targets,
                },
            )
        })
        .collect()
}

/// The package roots of the staged fixture, the root package first.
fn package_roots(root: &Path) -> Vec<(String, PathBuf)> {
    vec![
        (".".to_string(), root.to_path_buf()),
        ("liba".to_string(), root.join("node_modules/liba")),
        ("libb".to_string(), root.join("node_modules/libb")),
    ]
}

/// One package graph per package of the staged fixture, named and rooted.
fn fixture_graphs(root: &Path) -> Result<Vec<(String, PathBuf, PackageGraph)>, String> {
    package_roots(root)
        .into_iter()
        .map(|(name, package_root)| {
            let graph = package_graph_of(&package_root)?;
            Ok((name, package_root, graph))
        })
        .collect()
}

fn tree_of(packages: &[(String, PathBuf, PackageGraph)]) -> Tree {
    let on_disk = packages
        .iter()
        .map(|(name, root, graph)| (name.as_str(), root.as_path(), graph))
        .collect::<Vec<_>>();
    Tree::on_disk(&on_disk)
}

fn named(packages: &[(String, PathBuf, PackageGraph)]) -> Vec<(&str, &PackageGraph)> {
    packages
        .iter()
        .map(|(name, _, graph)| (name.as_str(), graph))
        .collect()
}

/// The answers of building one package graph per package and stitching them.
fn stitched_answers(root: &Path) -> Result<BTreeMap<String, PortableAnswer>, String> {
    let packages = fixture_graphs(root)?;
    let graphs = named(&packages);
    let stitched = stitch(&graphs, &tree_of(&packages)).map_err(|error| error.to_string())?;
    if !stitched.open_imports.is_empty() {
        return Err(format!("imports left open: {:?}", stitched.open_imports));
    }
    Ok(stitched_portable(&stitched, &graphs))
}

/// The package graph of the package rooted at `root`, analysed alone: the
/// packages under its `node_modules` are other graphs.
fn package_graph_of(root: &Path) -> Result<PackageGraph, String> {
    let output = run_excluding(root, &["**/.git/**", "**/node_modules/**"]);
    export(&output.db, root, None).map_err(|error| error.to_string())
}

/// The stitched answers keyed like [`whole_program_answers`]: a node's
/// portable key is its package with the key its own graph gives it.
fn stitched_portable(
    stitched: &Stitched,
    graphs: &[(&str, &PackageGraph)],
) -> BTreeMap<String, PortableAnswer> {
    let mut key_of = BTreeMap::new();
    for (name, graph) in graphs {
        let text = |node: u32| format!("{name}\0{}", graph.nodes[node as usize]);
        let mut functions = BTreeMap::new();
        for function in &graph.functions {
            let identity = &function.identity;
            functions.insert(function.node, identity);
            key_of.insert(
                text(function.node),
                format!(
                    "{name}:{}@{}..{}#{}",
                    identity.path, identity.start, identity.end, function.name
                ),
            );
        }
        for site in &graph.callsites {
            let caller = functions[&site.caller];
            key_of.insert(
                text(site.node),
                format!(
                    "{name}:{}@{}..{}",
                    caller.path,
                    caller.start + site.identity.start,
                    caller.start + site.identity.end
                ),
            );
        }
    }
    let key = |node: u32| key_of[&stitched.nodes[node as usize]].clone();
    stitched
        .sites
        .iter()
        .map(|site| {
            let mut targets = site
                .targets
                .iter()
                .map(|target| key(*target))
                .collect::<Vec<_>>();
            targets.sort();
            (
                key(site.callsite),
                PortableAnswer {
                    claim: format!("{:?} {:?}", site.status, site.claim),
                    targets,
                },
            )
        })
        .collect()
}

#[test]
fn stitched_package_graphs_answer_as_the_whole_program() {
    let temp = tempfile::tempdir().expect("tempdir");
    stage(temp.path());
    let whole = whole_program_answers(temp.path());
    assert!(
        !whole.is_empty(),
        "the whole program has call sites to compare"
    );
    let stitched = stitched_answers(temp.path()).expect("stitch the package graphs");
    assert_eq!(stitched, whole);
}

#[test]
fn the_stitch_does_not_depend_on_the_order_of_the_graphs() {
    let temp = tempfile::tempdir().expect("tempdir");
    stage(temp.path());
    let packages = fixture_graphs(temp.path()).expect("package graphs");
    let tree = tree_of(&packages);
    let graphs = named(&packages);
    let mut reversed = graphs.clone();
    reversed.reverse();
    let rotated = [&graphs[1..], &graphs[..1]].concat();
    let stitched = stitch(&graphs, &tree).expect("stitch");
    assert_eq!(stitch(&reversed, &tree).expect("stitch"), stitched);
    assert_eq!(stitch(&rotated, &tree).expect("stitch"), stitched);
}

/// A program whose packages each make calls of their own, and which imports
/// one package both by `require` and by `import`.
fn write_local_calls(root: &Path) {
    let files: [(&str, &str); 5] = [
        ("package.json", r#"{"name":"app","version":"1.0.0"}"#),
        (
            "main.js",
            "const lib = require(\"libx\");\nfunction local() { return 1; }\nlocal();\nlib.run();\n",
        ),
        (
            "esm.mjs",
            "import { run } from \"libx\";\nfunction twice() { run(); run(); }\ntwice();\n",
        ),
        (
            "node_modules/libx/package.json",
            r#"{"name":"libx","version":"1.0.0","exports":{"import":"./index.mjs","require":"./index.js"}}"#,
        ),
        (
            "node_modules/libx/index.js",
            "function helper() { return 2; }\nfunction run() { return helper(); }\nmodule.exports = { run };\n",
        ),
    ];
    for (path, text) in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("create dir");
        std::fs::write(path, text).expect("write file");
    }
    std::fs::write(
        root.join("node_modules/libx/index.mjs"),
        "function helper() { return 2; }\nexport function run() { return helper(); }\n",
    )
    .expect("write file");
}

#[test]
fn the_stitch_keeps_every_edge_a_graph_found_alone() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_local_calls(temp.path());
    let packages = [
        (".", temp.path().to_path_buf()),
        ("libx", temp.path().join("node_modules/libx")),
    ]
    .into_iter()
    .map(|(name, root)| {
        let graph = package_graph_of(&root).expect("package graph");
        (name.to_string(), root, graph)
    })
    .collect::<Vec<_>>();
    let graphs = named(&packages);
    let stitched = stitch(&graphs, &tree_of(&packages)).expect("stitch");
    assert_eq!(stitched.open_imports, []);
    assert_eq!(
        stitched_portable(&stitched, &graphs),
        whole_program_answers(temp.path())
    );

    let number = stitched
        .nodes
        .iter()
        .enumerate()
        .map(|(at, text)| (text.as_str(), at as u32))
        .collect::<BTreeMap<_, _>>();
    let answers = stitched
        .sites
        .iter()
        .map(|site| (site.callsite, &site.targets))
        .collect::<BTreeMap<_, _>>();
    let mut checked = 0;
    for (name, graph) in &graphs {
        let global = |node: u32| number[format!("{name}\0{}", graph.nodes[node as usize]).as_str()];
        for site in &graph.sites {
            let targets = answers[&global(site.callsite)];
            for target in &site.targets {
                assert!(
                    targets.contains(&global(*target)),
                    "{name}: the stitch lost an edge of the graph alone"
                );
                checked += 1;
            }
        }
    }
    assert!(checked >= 2, "each package found an edge of its own");
}

#[test]
fn the_stitch_refuses_a_graph_its_budget_cut_short() {
    let temp = tempfile::tempdir().expect("tempdir");
    stage(temp.path());
    let mut packages = fixture_graphs(temp.path()).expect("package graphs");
    let tree = tree_of(&packages);
    packages[1].2.budget.status = PointsToBudgetStatus::BudgetExceeded;
    packages[1].2.budget.events = vec!["iterations".to_string()];
    assert_eq!(
        stitch(&named(&packages), &tree),
        Err(StitchError::BudgetExceeded {
            package: "liba".to_string(),
            events: vec!["iterations".to_string()],
        })
    );
}

#[test]
fn the_stitch_refuses_two_graphs_of_one_package() {
    let temp = tempfile::tempdir().expect("tempdir");
    stage(temp.path());
    let packages = fixture_graphs(temp.path()).expect("package graphs");
    let tree = tree_of(&packages);
    let mut graphs = named(&packages);
    graphs.push(graphs[1]);
    assert_eq!(
        stitch(&graphs, &tree),
        Err(StitchError::DuplicatePackage("liba".to_string()))
    );
}

#[test]
fn the_stitch_reports_the_imports_it_cannot_link() {
    let temp = tempfile::tempdir().expect("tempdir");
    stage(temp.path());
    let packages = fixture_graphs(temp.path()).expect("package graphs");
    let tree = tree_of(&packages);
    let graphs = named(&packages);

    let alone = stitch(&graphs, &Tree::default()).expect("stitch");
    let mut reasons = alone
        .open_imports
        .iter()
        .map(|open| (open.package.as_str(), open.specifier.as_str(), open.reason))
        .collect::<Vec<_>>();
    reasons.dedup();
    assert_eq!(
        reasons,
        [
            (".", "liba", "unlinked_import"),
            (".", "libb", "unlinked_import"),
            ("libb", "liba", "unlinked_import"),
        ]
    );

    let without_liba = [graphs[0], graphs[2]];
    let missing = stitch(&without_liba, &tree).expect("stitch");
    assert!(
        missing
            .open_imports
            .iter()
            .filter(|open| open.specifier == "liba")
            .all(|open| open.reason == "missing_package_graph")
    );
    assert!(
        missing
            .open_imports
            .iter()
            .any(|open| open.specifier == "liba")
    );
}
