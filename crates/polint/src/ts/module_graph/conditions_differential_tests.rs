//! polint's resolution of `exports` and `imports` maps against Node's.
//!
//! The corpus `tests/js-analysis-bench/conditions/corpus.json` holds generated
//! packages and, for every request, the file Node loaded under each of its
//! condition sets in import and require mode. polint must name every file
//! Node loads, and may name one file as the exact answer only where every
//! condition set loads that file. No file or a declaration-only result
//! cannot be used to claim one runtime file exactly.

use super::{DiskResolver, DiskTarget};
use crate::analysis_api::ImportSyntax;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/js-analysis-bench/conditions/corpus.json"
);

#[derive(Deserialize)]
struct Corpus {
    header: Header,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Header {
    condition_sets: Vec<Vec<String>>,
    importers: Vec<String>,
    shared: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    kind: String,
    directory: String,
    manifest: String,
    files: Vec<String>,
    requests: Vec<Request>,
}

#[derive(Deserialize)]
struct Request {
    importer: String,
    specifier: String,
    mode: String,
    loads: BTreeMap<String, Vec<usize>>,
}

fn write(root: &Path, relative: &str, text: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("a corpus file has a directory"))
        .expect("create a corpus directory");
    fs::write(path, text).expect("write a corpus file");
}

fn materialise(root: &Path, corpus: &Corpus) {
    for importer in &corpus.header.importers {
        write(root, importer, "");
    }
    for (path, text) in &corpus.header.shared {
        write(root, path, text);
    }
    for case in &corpus.cases {
        write(
            root,
            &format!("{}/package.json", case.directory),
            &case.manifest,
        );
        for file in &case.files {
            write(root, file, "");
        }
        if case.kind == "imports" {
            write(root, &format!("{}/main.mjs", case.directory), "");
            write(root, &format!("{}/main.cjs", case.directory), "");
        }
    }
}

fn is_declaration(path: &str) -> bool {
    [".d.ts", ".d.mts", ".d.cts"]
        .iter()
        .any(|extension| path.ends_with(extension))
}

#[test]
fn every_file_node_loads_through_a_condition_map_is_named_and_one_file_only_where_node_agrees() {
    let corpus: Corpus =
        serde_json::from_str(&fs::read_to_string(CORPUS).expect("read the condition-map corpus"))
            .expect("parse the condition-map corpus");
    assert!(
        corpus.cases.len() >= 2_000,
        "the corpus holds at least 2,000 maps"
    );
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    materialise(root, &corpus);
    let resolver = DiskResolver::new();
    let relative = |path: &Path| path.strip_prefix(root).unwrap_or(path).to_path_buf();

    let mut failures = Vec::new();
    let mut requests = 0;
    for case in &corpus.cases {
        for request in &case.requests {
            requests += 1;
            let syntax = match request.mode.as_str() {
                "require" => ImportSyntax::Require,
                _ => ImportSyntax::Import,
            };
            let answer =
                resolver.resolve(&root.join(&request.importer), &request.specifier, syntax);
            let (named, exact): (BTreeSet<PathBuf>, bool) = match &answer {
                Some(DiskTarget::File(path)) => ([relative(path)].into(), true),
                Some(DiskTarget::Alternatives(paths)) => {
                    (paths.iter().map(|path| relative(path)).collect(), false)
                }
                None => (BTreeSet::new(), false),
            };
            let mut picks = vec![None; corpus.header.condition_sets.len()];
            for (path, sets) in &request.loads {
                for &set in sets {
                    assert!(picks[set].replace(path.as_str()).is_none());
                }
            }
            let failures_by_profile = picks
                .iter()
                .zip(&corpus.header.condition_sets)
                .filter_map(|(pick, profile)| {
                    let runtime_file = pick.filter(|path| !is_declaration(path)).map(Path::new);
                    let named_here = runtime_file.is_some_and(|path| named.contains(path));
                    let wrong = if exact {
                        !named_here
                    } else {
                        runtime_file.is_some() && !named_here
                    };
                    wrong.then(|| format!("{profile:?}: {pick:?}"))
                })
                .collect::<Vec<_>>();
            if failures_by_profile.is_empty() {
                continue;
            }
            failures.push(format!(
                "{} {} `{}` from {}:\n  polint: {:?} (exact: {exact})\n  Node disagreements: {}\n  manifest: {}",
                case.id,
                request.mode,
                request.specifier,
                request.importer,
                named,
                failures_by_profile.join("; "),
                case.manifest
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {requests} requests disagree with Node:\n{}",
        failures.len(),
        failures
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
