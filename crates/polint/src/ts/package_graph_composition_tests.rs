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
use crate::analysis_neutral::semantic_graph::facts::NodeKind;
use crate::analysis_neutral::solver::budget::SolverBudget;
use crate::analysis_plan::AnalysisPlan;
use crate::cache::Cache;
use crate::config::load_config;
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
    let mut loaded = load_config(root).expect("default config loads");
    loaded.config.workspace.exclude = vec!["**/.git/**".to_string()];
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

/// The package a repository-relative path belongs to and the path inside
/// it: the innermost `node_modules/<name>` (or `node_modules/@scope/<name>`)
/// names the package, and a path outside every `node_modules` belongs to the
/// root package, named `.`.
fn package_of(path: &str) -> (String, String) {
    let parts = path.split('/').collect::<Vec<_>>();
    let Some(at) = parts.iter().rposition(|part| *part == "node_modules") else {
        return (".".to_string(), path.to_string());
    };
    let width = if parts.get(at + 1).is_some_and(|name| name.starts_with('@')) {
        2
    } else {
        1
    };
    let name = parts[at + 1..at + 1 + width].join("/");
    let inner = parts[at + 1 + width..].join("/");
    (name, inner)
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
            .map(|file| package_of(&file.relative_path))
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

/// The answers of building one package graph per package and stitching them.
fn stitched_answers(root: &Path) -> Result<BTreeMap<String, PortableAnswer>, String> {
    let graphs = package_roots(root)
        .into_iter()
        .map(|(name, package_root)| package_graph_of(&name, &package_root))
        .collect::<Result<Vec<_>, _>>()?;
    match graphs.as_slice() {
        [] => Ok(BTreeMap::new()),
        _ => Err("no stitch".to_string()),
    }
}

fn package_graph_of(_name: &str, _root: &Path) -> Result<(), String> {
    Err("no package graph export".to_string())
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
fn a_file_belongs_to_its_innermost_node_modules_package() {
    assert_eq!(package_of("main.js"), (".".into(), "main.js".into()));
    assert_eq!(
        package_of("node_modules/liba/index.js"),
        ("liba".into(), "index.js".into())
    );
    assert_eq!(
        package_of("node_modules/@scope/pkg/lib/a.js"),
        ("@scope/pkg".into(), "lib/a.js".into())
    );
    assert_eq!(
        package_of("node_modules/a/node_modules/b/x.js"),
        ("b".into(), "x.js".into())
    );
}
