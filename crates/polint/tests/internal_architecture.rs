use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const REMOVED_PACKAGES: &[&str] = &[
    "polint-core",
    "polint-ir",
    "polint-analysis-api",
    "polint-frontend-api",
    "polint-analysis",
    "polint-go",
    "polint-ts",
];

#[test]
fn workspace_has_only_two_publishable_product_packages() {
    let root = repo_root();
    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("read workspace manifest");
    let lock = fs::read_to_string(root.join("Cargo.lock")).expect("read workspace lockfile");
    for package in REMOVED_PACKAGES {
        assert!(
            !manifest.contains(&format!("crates/{package}")),
            "removed internal package remains a workspace member: {package}"
        );
        assert!(
            !lock.contains(&format!("name = \"{package}\"")),
            "removed internal package remains in Cargo.lock: {package}"
        );
        assert!(
            !root.join("crates").join(package).exists(),
            "removed internal package directory remains: {package}"
        );
    }
}

#[test]
fn internal_dependency_directions_are_acyclic() {
    let src = repo_root().join("crates/polint/src");
    assert_tree_excludes(
        &src.join("internal_core"),
        &[
            "crate::ir",
            "crate::analysis_api",
            "crate::analysis_neutral",
            "crate::frontend_api",
            "crate::go",
            "crate::ts",
        ],
    );
    assert_tree_excludes(
        &src.join("ir"),
        &[
            "crate::analysis_api",
            "crate::analysis_neutral",
            "crate::frontend_api",
            "crate::go",
            "crate::ts",
        ],
    );
    assert_tree_excludes(
        &src.join("analysis_api"),
        &[
            "crate::analysis_neutral",
            "crate::frontend_api",
            "crate::go",
            "crate::ts",
        ],
    );
    assert_tree_excludes(
        &src.join("frontend_api"),
        &["crate::analysis_neutral", "crate::go", "crate::ts"],
    );
    assert_tree_excludes(
        &src.join("analysis_neutral"),
        &["crate::frontend_api", "crate::go", "crate::ts"],
    );
    assert_tree_excludes(&src.join("go"), &["crate::ts"]);
    assert_tree_excludes(
        &src.join("ts"),
        &["crate::go", "crate::analysis::", "crate::core::AnalysisDb"],
    );
}

#[test]
fn semantic_graph_facade_delegates_ts_builder_without_ts_implementation_imports() {
    let src = repo_root().join("crates/polint/src");
    let facade_dir = src.join("analysis/semantic_graph");
    assert!(
        !facade_dir.join("build.rs").exists(),
        "the composition facade must not own the TypeScript semantic-graph builder"
    );

    let facade =
        fs::read_to_string(facade_dir.join("mod.rs")).expect("read semantic-graph facade module");
    assert!(
        facade.contains("crate::ts::semantic_graph_build"),
        "the composition facade must narrowly re-export the TypeScript-owned builder"
    );
    assert_tree_excludes(
        &facade_dir,
        &[
            "crate::ts::binding::direct",
            "crate::ts::binding::facts",
            "crate::ts::inventory",
            "crate::ts::object_model",
            "crate::ts::parse",
            "crate::ts::scope",
            "crate::ts::semantic_graph::",
            "crate::ts::token_flow",
        ],
    );

    let ts_builder_path = src.join("ts/semantic_graph_build.rs");
    assert!(
        ts_builder_path.is_file(),
        "the TypeScript frontend must own semantic-graph projection"
    );
    let ts_builder = fs::read_to_string(ts_builder_path).expect("read TypeScript graph builder");
    for forbidden in ["crate::analysis::", "crate::core::AnalysisDb"] {
        assert!(
            !ts_builder.contains(forbidden),
            "the TypeScript graph builder must use neutral host contracts, not `{forbidden}`"
        );
    }
}

#[test]
fn go_rta_projection_is_separate_from_the_neutral_engine() {
    let src = repo_root().join("crates/polint/src");
    let neutral = src.join("analysis_neutral/solver/go_rta");
    let go = src.join("go/rta");

    for file in ["snapshot.rs", "dispatch.rs", "fixpoint.rs"] {
        assert!(
            neutral.join(file).is_file(),
            "the frontend-neutral RTA engine must own {file}"
        );
        assert!(
            !go.join(file).exists(),
            "the Go frontend must not own RTA algorithm file {file}"
        );
    }

    let mut go_files = Vec::new();
    collect_rs_files(&go, &mut go_files);
    let mut go_file_names = go_files
        .iter()
        .map(|path| {
            path.file_name()
                .expect("RTA file name")
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    go_file_names.sort();
    assert_eq!(
        go_file_names,
        ["inputs.rs", "mod.rs"],
        "the Go RTA directory is a projection adapter and facade only"
    );

    let snapshot = fs::read_to_string(neutral.join("snapshot.rs")).expect("read RTA snapshot");
    assert!(snapshot.contains("struct RtaInputs"));
    assert!(!snapshot.contains("struct GoRtaInputs"));

    let adapter = fs::read_to_string(go.join("inputs.rs")).expect("read Go RTA input adapter");
    assert!(adapter.contains("fn from_db"));
    assert!(adapter.contains("crate::go::semantic::facts"));
    assert!(!adapter.contains("crate::analysis::"));
    for algorithm in [
        "fn resolve_callsite",
        "fn solve_rta",
        "while !frontier.is_empty()",
    ] {
        assert!(
            !adapter.contains(algorithm),
            "the Go input adapter contains neutral algorithm logic: {algorithm}"
        );
    }

    let facade = fs::read_to_string(go.join("mod.rs")).expect("read Go RTA facade");
    assert!(facade.contains("analysis_neutral::solver::go_rta::solve_rta"));
    assert_tree_excludes(
        &neutral,
        &[
            "crate::go",
            "crate::core",
            "crate::analysis::",
            "AnalysisDb",
            "GoSemantic",
        ],
    );
    assert!(
        !src.join("analysis/solver/go_rta").exists(),
        "the composition-root solver must not duplicate the neutral RTA engine"
    );
}

fn assert_tree_excludes(root: &Path, forbidden: &[&str]) {
    let mut files = Vec::new();
    collect_rs_files(root, &mut files);
    for file in files {
        let source = fs::read_to_string(&file).expect("read Rust source");
        for needle in forbidden {
            assert!(
                !source.contains(needle),
                "{} crosses internal dependency direction with `{needle}`",
                file.display()
            );
        }
    }
}

fn collect_rs_files(root: &Path, files: &mut Vec<PathBuf>) {
    for entry in
        fs::read_dir(root).unwrap_or_else(|error| panic!("read {}: {error}", root.display()))
    {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            collect_rs_files(&path, files);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonical repo root")
}

#[test]
fn language_features_are_isolated_and_default_to_both() {
    let root = repo_root();
    let manifest =
        fs::read_to_string(root.join("crates/polint/Cargo.toml")).expect("read polint manifest");
    let parsed: toml::Value = toml::from_str(&manifest).expect("parse polint manifest");
    let features = parsed["features"].as_table().expect("features table");
    assert_eq!(
        features["default"],
        toml::Value::Array(vec!["lang-go".into(), "lang-typescript".into()])
    );
    assert_eq!(
        features["all-languages"],
        toml::Value::Array(vec!["lang-go".into(), "lang-typescript".into()])
    );

    let dependencies = parsed["dependencies"]
        .as_table()
        .expect("dependencies table");
    assert_eq!(
        dependencies["tree-sitter"]["optional"].as_bool(),
        Some(true)
    );
    assert!(
        features["lang-go"]
            .as_array()
            .unwrap()
            .contains(&"dep:tree-sitter".into())
    );
    assert!(
        !features["lang-go"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value.as_str() == Some("dep:tree-sitter-go")),
        "Go parsing uses the vendored grammar, not crates.io tree-sitter-go"
    );
    assert!(
        repo_root()
            .join("crates/polint/vendor/tree-sitter-go/src/parser.c")
            .is_file(),
        "vendored tree-sitter-go parser.c must be present for lang-go builds"
    );
    for dependency in [
        "oxc_allocator",
        "oxc_ast",
        "oxc_parser",
        "oxc_resolver",
        "oxc_semantic",
        "oxc_span",
    ] {
        assert_eq!(dependencies[dependency]["optional"].as_bool(), Some(true));
        assert!(
            features["lang-typescript"]
                .as_array()
                .unwrap()
                .contains(&format!("dep:{dependency}").into())
        );
    }
}

#[test]
fn ci_uses_supported_supply_chain_and_existing_go_cache_inputs() {
    let root = repo_root();
    let workflow =
        fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("read CI workflow");

    assert!(workflow.contains("command: check"));
    assert!(workflow.contains("command-arguments: all"));
    assert!(workflow.contains("arguments: \"\""));
    assert!(!workflow.contains("arguments: --all-features --locked"));
    assert!(workflow.contains("crates/polint/src/go-sidecar/polint-go-frontend/go.sum"));
    assert!(workflow.contains("crates/polint/src/go-sidecar/polint-go-symbols/go.sum"));
    assert!(!workflow.contains("crates/polint/go-sidecar/"));

    for path in [
        "crates/polint/src/go-sidecar/polint-go-frontend/go.sum",
        "crates/polint/src/go-sidecar/polint-go-symbols/go.sum",
    ] {
        assert!(
            root.join(path).is_file(),
            "CI cache input is missing: {path}"
        );
    }
}

// ---------------------------------------------------------------------------
// Dense-id sweep (plan section 2.1 standing rule)
// ---------------------------------------------------------------------------
//
// Four greps over `crates/polint/src`, implemented natively here so they run in
// every `cargo test -p polint --tests` and as `gate.sh`'s first step:
//
//   dot0         a `.0` field access formatted into a string
//   debug        a `{:?}` / `{name:?}` placeholder or a `DigestBuilder::debug_part`
//   raw          a `.raw()` rendering of an id
//   other-debug  every other `{...?}` placeholder form (`{:#?}`, widths, fills)
//
// Every `XId(pub u32|u64)` newtype in the crate exposes its integer through one
// of those four channels, and a dense id is a run-set property: it follows the
// file set a scan covers. A dense id that reaches persisted key text or a payload
// digest therefore makes a fact row depend on the scope it was produced in, which
// is what W3 commit 4 removes and what W6's packed ids would otherwise
// reintroduce. The sweep does not judge a hit; it pins the hit *set*, so a new
// one cannot appear without a reviewer classifying it against section 2.1's
// disposition tables.
//
// The baseline is keyed by signature, never by line: the SHA-256 of the
// repository-relative path, the name of the enclosing `fn` or `impl` item, and
// the matched line with runs of whitespace collapsed. Editing lines above a hit
// moves nothing; adding, removing or rewording one fails this test until the
// commit re-baselines with
//
//   POLINT_UPDATE_DENSE_ID_BASELINE=1 \
//     cargo test -p polint --test internal_architecture dense_id_sweep --locked
//
// and states the added and removed signatures, with their disposition rows, in
// its commit message. `POLINT_DENSE_ID_SWEEP_DUMP=<path>` writes every hit as
// `<signature> <command> <scope> <path>:<line>` so a removed signature can be
// traced back to the line it stood for.

const BASELINE_PATH: &str = "crates/polint/tests/fixtures/dense_id_sweep.txt";
const UPDATE_ENV: &str = "POLINT_UPDATE_DENSE_ID_BASELINE";
const DUMP_ENV: &str = "POLINT_DENSE_ID_SWEEP_DUMP";

/// The four commands, in the order section 2.1 states them.
const COMMANDS: [&str; 4] = ["dot0", "debug", "raw", "other-debug"];

#[test]
fn dense_id_sweep_matches_baseline() {
    let root = repo_root();
    let hits = sweep(&root);

    if let Some(path) = std::env::var_os(DUMP_ENV) {
        let mut body = String::new();
        for hit in &hits {
            body.push_str(&format!(
                "{} {} {} {}:{}\n",
                hit.signature, hit.command, hit.scope, hit.path, hit.line
            ));
        }
        fs::write(path, body).expect("write the dense-id sweep dump");
    }

    let observed = fold(&hits);
    let rendered = render(&observed, &hits);

    let baseline_path = root.join(BASELINE_PATH);
    if std::env::var_os(UPDATE_ENV).is_some() {
        fs::create_dir_all(baseline_path.parent().expect("fixtures directory"))
            .expect("create the fixtures directory");
        fs::write(&baseline_path, &rendered).expect("write the dense-id sweep baseline");
        eprintln!("dense-id sweep baseline rewritten: {BASELINE_PATH}");
        return;
    }

    let baseline = fs::read_to_string(&baseline_path).unwrap_or_else(|error| {
        panic!("read {BASELINE_PATH}: {error}; regenerate it with {UPDATE_ENV}=1")
    });
    if baseline == rendered {
        return;
    }

    let expected = parse_baseline(&baseline);
    let mut added = Vec::new();
    let mut removed = Vec::new();
    for (key, count) in &observed {
        if expected.get(key) != Some(count) {
            let where_from = hits
                .iter()
                .find(|hit| hit.signature == key.0 && hit.command == key.1)
                .map(|hit| format!("{}:{}", hit.path, hit.line))
                .unwrap_or_else(|| "?".to_string());
            added.push(format!(
                "  + {} {} {} x{} at {where_from}",
                key.0, key.1, key.2, count
            ));
        }
    }
    for (key, count) in &expected {
        if observed.get(key) != Some(count) {
            removed.push(format!("  - {} {} {} x{}", key.0, key.1, key.2, count));
        }
    }
    added.sort();
    removed.sort();
    panic!(
        "the dense-id sweep no longer matches {BASELINE_PATH}.\n\
         Classify every added hit against the disposition tables in section 2.1 of \
         research/strategy/plans/2026-09-19_full-app-deep-capability_plan.md, then \
         re-baseline with `{UPDATE_ENV}=1 cargo test -p polint --test \
         internal_architecture dense_id_sweep --locked` and state the signatures below \
         in the commit message. `{DUMP_ENV}=<path>` maps a signature back to its line.\n\
         added ({}):\n{}\nremoved ({}):\n{}",
        added.len(),
        added.join("\n"),
        removed.len(),
        removed.join("\n")
    );
}

#[derive(Debug)]
struct Hit {
    signature: String,
    command: &'static str,
    scope: &'static str,
    path: String,
    line: usize,
}

/// (signature, command, scope) -> how many times it occurs in one item.
type HitCounts = BTreeMap<(String, &'static str, &'static str), usize>;

fn fold(hits: &[Hit]) -> HitCounts {
    let mut counts = HitCounts::new();
    for hit in hits {
        *counts
            .entry((hit.signature.clone(), hit.command, hit.scope))
            .or_insert(0) += 1;
    }
    counts
}

fn render(counts: &HitCounts, hits: &[Hit]) -> String {
    let mut out = String::new();
    out.push_str("# Dense-id sweep baseline. Plan section 2.1 standing rule.\n");
    out.push_str(
        "# Regenerate: POLINT_UPDATE_DENSE_ID_BASELINE=1 cargo test -p polint --test internal_architecture dense_id_sweep --locked\n",
    );
    out.push_str(
        "# Entry: <sha256 of (repo-relative path, enclosing fn/impl item, whitespace-collapsed line)>  <command>  <scope>  <count>\n",
    );
    for command in COMMANDS {
        let total = hits.iter().filter(|hit| hit.command == command).count();
        let test_only = hits
            .iter()
            .filter(|hit| hit.command == command && hit.scope == "test")
            .count();
        out.push_str(&format!(
            "# {command}: {total} hits, {test_only} test-only, {} non-test\n",
            total - test_only
        ));
    }
    for ((signature, command, scope), count) in counts {
        out.push_str(&format!("{signature}  {command}  {scope}  {count}\n"));
    }
    out
}

fn parse_baseline(text: &str) -> HitCounts {
    let mut counts = HitCounts::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(fields.len(), 4, "malformed baseline entry: {line}");
        let command = COMMANDS
            .iter()
            .find(|name| **name == fields[1])
            .unwrap_or_else(|| panic!("unknown command in the baseline: {line}"));
        let scope = match fields[2] {
            "test" => "test",
            "non-test" => "non-test",
            other => panic!("unknown scope {other} in the baseline: {line}"),
        };
        counts.insert(
            (fields[0].to_string(), *command, scope),
            fields[3].parse().expect("a baseline count"),
        );
    }
    counts
}

fn sweep(root: &Path) -> Vec<Hit> {
    let source_root = root.join("crates/polint/src");
    let mut files = Vec::new();
    collect_rs_files(&source_root, &mut files);
    files.sort();

    let parsed: BTreeMap<PathBuf, ParsedFile> = files
        .iter()
        .map(|path| {
            let text = fs::read_to_string(path).expect("read Rust source");
            (path.clone(), ParsedFile::parse(&text))
        })
        .collect();
    let test_only_files = test_only_files(&parsed);

    let mut hits = Vec::new();
    for path in &files {
        let parsed = &parsed[path];
        let relative = path
            .strip_prefix(root)
            .expect("source file under the repo root")
            .to_string_lossy()
            .replace('\\', "/");
        let whole_file_is_test = test_only_files.contains(path);
        for (index, line) in parsed.lines.iter().enumerate() {
            for command in COMMANDS {
                if !matches(command, &line.raw) {
                    continue;
                }
                let scope = if whole_file_is_test || parsed.in_cfg_test_item(index) {
                    "test"
                } else {
                    "non-test"
                };
                hits.push(Hit {
                    signature: signature(&relative, &line.item, &line.raw),
                    command,
                    scope,
                    path: relative.clone(),
                    line: index + 1,
                });
            }
        }
    }
    hits
}

fn signature(path: &str, item: &str, line: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(path.as_bytes());
    hasher.update([0u8]);
    hasher.update(item.as_bytes());
    hasher.update([0u8]);
    hasher.update(collapse_whitespace(line).as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

fn collapse_whitespace(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_space = false;
    for character in line.trim().chars() {
        if character.is_whitespace() {
            in_space = true;
            continue;
        }
        if in_space && !out.is_empty() {
            out.push(' ');
        }
        in_space = false;
        out.push(character);
    }
    out
}

// --- the four commands, matched on the raw line exactly as grep sees it ------
//
// Byte-level throughout: the patterns are ASCII, the sources are not, and a
// `&str` slice at an arbitrary byte offset would panic mid-codepoint.

fn matches(command: &str, line: &str) -> bool {
    let bytes = line.as_bytes();
    match command {
        // grep -rnE -e '(format!|write!|writeln!)\(.*\.0\b' -e '\.0\.to_string\(\)'
        //           -e '\{[a-z_]+\.0\}' | grep -vE '[0-9]\.0\b|\.0\.[0-9]'
        "dot0" => {
            if excluded_dot_zero(bytes) {
                return false;
            }
            macro_then_dot_zero(bytes)
                || contains(bytes, b".0.to_string()")
                || brace_suffix(bytes, b".0}", is_lower_or_underscore)
        }
        // grep -rnE -e '\{:\?\}' -e '\{[a-z_.]+:\?\}' -e 'debug_part\(' | grep -v 'fn debug_part'
        "debug" => {
            !contains(bytes, b"fn debug_part")
                && (contains(bytes, b"{:?}")
                    || brace_suffix(bytes, b":?}", is_lower_underscore_or_dot)
                    || contains(bytes, b"debug_part("))
        }
        // grep -rnE -e '(format!|write!|writeln!)\(.*\.raw\(\)' -e '\{[a-z_]+\.raw\(\)\}'
        //           -e '\.raw\(\)\.to_string\(\)'
        "raw" => {
            macro_then(bytes, b".raw()")
                || contains(bytes, b".raw().to_string()")
                || brace_suffix(bytes, b".raw()}", is_lower_or_underscore)
        }
        // grep -rnE '\{[A-Za-z_.0-9]*:[#0-9<>^+ -]*\?\}' | grep -vE '\{:\?\}|\{[a-z_.]+:\?\}'
        "other-debug" => {
            other_debug_placeholder(bytes)
                && !contains(bytes, b"{:?}")
                && !brace_suffix(bytes, b":?}", is_lower_underscore_or_dot)
        }
        other => panic!("unknown sweep command {other}"),
    }
}

fn is_lower_or_underscore(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte == b'_'
}

fn is_lower_underscore_or_dot(byte: u8) -> bool {
    is_lower_or_underscore(byte) || byte == b'.'
}

fn starts_at(bytes: &[u8], index: usize, needle: &[u8]) -> bool {
    bytes.len() >= index + needle.len() && &bytes[index..index + needle.len()] == needle
}

fn contains(bytes: &[u8], needle: &[u8]) -> bool {
    positions(bytes, needle).next().is_some()
}

fn positions<'a>(bytes: &'a [u8], needle: &'a [u8]) -> impl Iterator<Item = usize> + 'a {
    (0..bytes.len()).filter(move |index| starts_at(bytes, *index, needle))
}

/// `\.0\b` at `index`: `.0` whose next byte is not a word byte.
fn dot_zero_boundary(bytes: &[u8], index: usize) -> bool {
    starts_at(bytes, index, b".0")
        && bytes
            .get(index + 2)
            .is_none_or(|byte| !(byte.is_ascii_alphanumeric() || *byte == b'_'))
}

/// The `grep -vE '[0-9]\.0\b|\.0\.[0-9]'` filter: a numeric literal, not a field.
fn excluded_dot_zero(bytes: &[u8]) -> bool {
    let digit_before = (1..bytes.len())
        .any(|index| dot_zero_boundary(bytes, index) && bytes[index - 1].is_ascii_digit());
    let digit_after =
        positions(bytes, b".0.").any(|index| bytes.get(index + 3).is_some_and(u8::is_ascii_digit));
    digit_before || digit_after
}

/// `(format!|write!|writeln!)\(.*<needle>`.
fn macro_then(bytes: &[u8], needle: &[u8]) -> bool {
    match first_macro_open(bytes) {
        Some(first) => positions(bytes, needle).any(|at| at >= first),
        None => false,
    }
}

/// `macro_then` with `\.0\b` in place of a literal needle.
fn macro_then_dot_zero(bytes: &[u8]) -> bool {
    match first_macro_open(bytes) {
        Some(first) => (first..bytes.len()).any(|at| dot_zero_boundary(bytes, at)),
        None => false,
    }
}

/// Offset just past the earliest `format!(`, `write!(` or `writeln!(` on the line.
fn first_macro_open(bytes: &[u8]) -> Option<usize> {
    [
        b"format!(".as_slice(),
        b"write!(".as_slice(),
        b"writeln!(".as_slice(),
    ]
    .iter()
    .filter_map(|name| positions(bytes, name).next().map(|at| at + name.len()))
    .min()
}

/// `\{<class>+<suffix>`. Every class here excludes the suffix's first byte, so a
/// greedy run needs no backtracking.
fn brace_suffix(bytes: &[u8], suffix: &[u8], class: impl Fn(u8) -> bool) -> bool {
    positions(bytes, b"{").any(|open| {
        let mut cursor = open + 1;
        while cursor < bytes.len() && class(bytes[cursor]) {
            cursor += 1;
        }
        cursor > open + 1 && starts_at(bytes, cursor, suffix)
    })
}

/// `\{[A-Za-z_.0-9]*:[#0-9<>^+ -]*\?\}`.
fn other_debug_placeholder(bytes: &[u8]) -> bool {
    const SPEC: &[u8] = b"#0123456789<>^+ -";
    positions(bytes, b"{").any(|open| {
        let mut cursor = open + 1;
        while cursor < bytes.len()
            && (bytes[cursor].is_ascii_alphanumeric()
                || bytes[cursor] == b'_'
                || bytes[cursor] == b'.')
        {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b':') {
            return false;
        }
        cursor += 1;
        while cursor < bytes.len() && SPEC.contains(&bytes[cursor]) {
            cursor += 1;
        }
        starts_at(bytes, cursor, b"?}")
    })
}

// --- source structure: enclosing item, cfg(test) spans, module declarations ---

struct ParsedLine {
    /// The line as it is on disk; the four commands match this, as grep does.
    raw: String,
    /// The line with string literals, char literals and comments removed, which
    /// is what brace counting and item detection read.
    code: String,
    /// Innermost enclosing `fn` or `impl` item open at the start of this line,
    /// or `<none>` at file scope.
    item: String,
}

struct ParsedFile {
    lines: Vec<ParsedLine>,
    /// Line ranges (inclusive, zero-based) of the items a `#[cfg(test)]` or
    /// `#[cfg(all(test, ...))]` attribute introduces.
    cfg_test_spans: Vec<(usize, usize)>,
    /// `(line, module name)` for every `mod <name>;` declaration.
    module_declarations: Vec<(usize, String)>,
    /// The file's first inner attribute, if it has one.
    first_inner_attribute: Option<String>,
}

impl ParsedFile {
    fn in_cfg_test_item(&self, line: usize) -> bool {
        self.cfg_test_spans
            .iter()
            .any(|(start, end)| line >= *start && line <= *end)
    }

    fn parse(text: &str) -> Self {
        let raws: Vec<String> = text.lines().map(str::to_string).collect();
        let codes = strip_literals_and_comments(text, raws.len());

        let mut items = Vec::with_capacity(raws.len());
        let mut stack: Vec<Option<String>> = Vec::new();
        let mut header = String::new();
        for code in &codes {
            items.push(
                stack
                    .iter()
                    .rev()
                    .find_map(|frame| frame.clone())
                    .unwrap_or_else(|| "<none>".to_string()),
            );
            for character in code.chars() {
                match character {
                    '{' => {
                        stack.push(item_name(&header));
                        header.clear();
                    }
                    '}' => {
                        stack.pop();
                        header.clear();
                    }
                    ';' => header.clear(),
                    other => header.push(other),
                }
            }
            header.push(' ');
        }

        let lines: Vec<ParsedLine> = raws
            .into_iter()
            .zip(codes.iter().cloned())
            .zip(items)
            .map(|((raw, code), item)| ParsedLine { raw, code, item })
            .collect();

        let first_inner_attribute = lines
            .iter()
            .map(|line| line.code.trim())
            .find(|code| code.starts_with("#!["))
            .map(str::to_string);

        let mut parsed = Self {
            lines,
            cfg_test_spans: Vec::new(),
            module_declarations: Vec::new(),
            first_inner_attribute,
        };
        parsed.cfg_test_spans = parsed.find_cfg_test_spans();
        parsed.module_declarations = parsed.find_module_declarations();
        parsed
    }

    /// Rule (1): the brace-balanced item that follows a `#[cfg(test)]` or
    /// `#[cfg(all(test, ...))]` attribute. Intervening attributes and doc
    /// comments belong to the item; an item that ends in `;` before any `{` is
    /// that one line.
    fn find_cfg_test_spans(&self) -> Vec<(usize, usize)> {
        let mut spans = Vec::new();
        for (index, line) in self.lines.iter().enumerate() {
            let code = line.code.trim_start();
            if !(code.starts_with("#[cfg(test)]") || code.starts_with("#[cfg(all(test")) {
                continue;
            }
            let after_attribute = strip_leading_attributes(code);
            let mut start = index;
            if after_attribute.trim().is_empty() {
                // The item is on a later line; attributes and doc comments (which
                // `code` has already emptied) are skipped.
                start += 1;
                while start < self.lines.len() {
                    let next = self.lines[start].code.trim();
                    if next.is_empty() || next.starts_with("#[") {
                        start += 1;
                        continue;
                    }
                    break;
                }
                if start >= self.lines.len() {
                    continue;
                }
            }
            spans.push((index, self.item_end(start, after_attribute)));
        }
        spans
    }

    /// Last line of the item starting at `start`, whose first line's code after
    /// any leading attributes is `first`.
    fn item_end(&self, start: usize, first: &str) -> usize {
        let mut depth = 0usize;
        let mut opened = false;
        for index in start..self.lines.len() {
            let code: &str = if index == start && !first.trim().is_empty() {
                first
            } else {
                &self.lines[index].code
            };
            for character in code.chars() {
                match character {
                    '{' => {
                        depth += 1;
                        opened = true;
                    }
                    '}' => {
                        depth = depth.saturating_sub(1);
                        if opened && depth == 0 {
                            return index;
                        }
                    }
                    ';' if !opened => return index,
                    _ => {}
                }
            }
        }
        self.lines.len().saturating_sub(1)
    }

    fn find_module_declarations(&self) -> Vec<(usize, String)> {
        let mut declarations = Vec::new();
        for (index, line) in self.lines.iter().enumerate() {
            if let Some(name) = module_declaration(strip_leading_attributes(line.code.trim())) {
                declarations.push((index, name));
            }
        }
        declarations
    }
}

/// `mod <name>;` with an optional visibility, after any leading attributes.
fn module_declaration(code: &str) -> Option<String> {
    let code = code.trim();
    let rest = code
        .strip_prefix("pub ")
        .or_else(|| {
            code.strip_prefix("pub(")
                .and_then(|after| after.split_once(')'))
                .map(|(_, after)| after)
        })
        .unwrap_or(code)
        .trim_start();
    let rest = rest.strip_prefix("mod ")?;
    let name: String = rest
        .chars()
        .take_while(|character| character.is_alphanumeric() || *character == '_')
        .collect();
    if name.is_empty() {
        return None;
    }
    rest[name.len()..]
        .trim_start()
        .starts_with(';')
        .then_some(name)
}

/// Drop every leading `#[...]` attribute group, returning what follows on the line.
fn strip_leading_attributes(code: &str) -> &str {
    let mut rest = code.trim_start();
    while rest.starts_with("#[") {
        let mut depth = 0usize;
        let mut end = None;
        for (offset, character) in rest.char_indices() {
            match character {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(offset + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        match end {
            Some(offset) => rest = rest[offset..].trim_start(),
            None => return "",
        }
    }
    rest
}

/// The name a `{` opens an item under, or `None` for a block that is neither a
/// `fn` nor an `impl` (a module, a type, a `match`, a closure body).
fn item_name(header: &str) -> Option<String> {
    let header = collapse_whitespace(header);
    if let Some(at) = token_position(&header, "fn") {
        let rest = header[at + 2..].trim_start();
        let name: String = rest
            .chars()
            .take_while(|character| character.is_alphanumeric() || *character == '_')
            .collect();
        if !name.is_empty() {
            return Some(format!("fn {name}"));
        }
    }
    token_position(&header, "impl").map(|at| header[at..].trim_end().to_string())
}

/// Byte offset of `token` in `text` with word boundaries on both sides.
fn token_position(text: &str, token: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    positions(bytes, token.as_bytes()).find(|at| {
        let before = *at == 0 || !word(bytes[*at - 1]);
        let after = bytes.get(*at + token.len()).is_none_or(|byte| !word(*byte));
        before && after
    })
}

/// Per-line source with string literals, char literals and comments removed, so
/// a brace inside one is never counted. Raw strings (`r#"..."#`) included: the
/// test suites in this crate are full of them and they carry braces.
fn strip_literals_and_comments(text: &str, line_count: usize) -> Vec<String> {
    #[derive(PartialEq)]
    enum State {
        Code,
        LineComment,
        BlockComment(usize),
        Str,
        RawStr(usize),
        Char,
    }

    let mut out = vec![String::new(); line_count];
    let mut line = 0usize;
    let mut state = State::Code;
    let bytes = text.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'\n' {
            line += 1;
            if state == State::LineComment {
                state = State::Code;
            }
            index += 1;
            continue;
        }
        let push = |out: &mut Vec<String>, line: usize, byte: u8| {
            if line < out.len() {
                out[line].push(char::from(byte));
            }
        };
        match state {
            State::LineComment => index += 1,
            State::BlockComment(depth) => {
                if starts_at(bytes, index, b"/*") {
                    state = State::BlockComment(depth + 1);
                    index += 2;
                } else if starts_at(bytes, index, b"*/") {
                    state = if depth == 1 {
                        State::Code
                    } else {
                        State::BlockComment(depth - 1)
                    };
                    index += 2;
                } else {
                    index += 1;
                }
            }
            State::Str => {
                if byte == b'\\' {
                    index += 2;
                } else {
                    if byte == b'"' {
                        state = State::Code;
                    }
                    index += 1;
                }
            }
            State::Char => {
                if byte == b'\\' {
                    index += 2;
                } else {
                    if byte == b'\'' {
                        state = State::Code;
                    }
                    index += 1;
                }
            }
            State::RawStr(hashes) => {
                if byte == b'"' && bytes[index + 1..].iter().take(hashes).all(|b| *b == b'#') {
                    state = State::Code;
                    index += 1 + hashes;
                } else {
                    index += 1;
                }
            }
            State::Code => {
                if starts_at(bytes, index, b"//") {
                    state = State::LineComment;
                    index += 2;
                } else if starts_at(bytes, index, b"/*") {
                    state = State::BlockComment(1);
                    index += 2;
                } else if let Some((skip, hashes)) = raw_string_open(bytes, index) {
                    state = State::RawStr(hashes);
                    index += skip;
                } else if byte == b'"' {
                    state = State::Str;
                    index += 1;
                } else if byte == b'\'' && char_literal_open(bytes, index) {
                    state = State::Char;
                    index += 1;
                } else {
                    push(&mut out, line, byte);
                    index += 1;
                }
            }
        }
    }
    out
}

/// `r"`, `r#"`, `br##"` ... : the byte length of the opener and its hash count.
fn raw_string_open(bytes: &[u8], index: usize) -> Option<(usize, usize)> {
    let mut cursor = index;
    if bytes.get(cursor) == Some(&b'b') {
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b'r') {
        return None;
    }
    // `r` must start a token, or this is the tail of an identifier such as `for`.
    if index > 0 && (bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'_') {
        return None;
    }
    cursor += 1;
    let mut hashes = 0usize;
    while bytes.get(cursor) == Some(&b'#') {
        hashes += 1;
        cursor += 1;
    }
    (bytes.get(cursor) == Some(&b'"')).then_some((cursor - index + 1, hashes))
}

/// A `'` that opens a char literal rather than a lifetime: `'x'` or `'\n'`.
fn char_literal_open(bytes: &[u8], index: usize) -> bool {
    match bytes.get(index + 1) {
        Some(b'\\') => true,
        Some(_) => bytes.get(index + 2) == Some(&b'\''),
        None => false,
    }
}

#[test]
fn dense_id_sweep_commands_reproduce_their_grep_filters() {
    // dot0: a field access hits, a numeric literal does not.
    assert!(matches("dot0", r#"format!("{}", body.id.0)"#));
    assert!(matches("dot0", "let text = id.0.to_string();"));
    assert!(matches("dot0", r#"format!("{site.0}")"#));
    // The `grep -vE '[0-9]\.0\b|\.0\.[0-9]'` filter drops a whole line that
    // carries a numeric literal, field access on it or not.
    assert!(!matches("dot0", "let ratio = value / 3.0;"));
    assert!(!matches("dot0", r#"format!("{} {}", 3.0, node.block.0)"#));
    assert!(!matches("dot0", "let version = pin.0.1;"));
    // A width spec is a `.0` the filter keeps, which is why every hit is
    // classified by hand rather than trusted.
    assert!(matches("dot0", r#"format!("{:.0}", 105)"#));
    // The `.0` has to follow the macro open, not precede it.
    assert!(!matches("dot0", r#"let x = id.0; format!("plain")"#));

    // debug: the three alternatives, and the one exclusion.
    assert!(matches("debug", r#"format!("{:?}", fact)"#));
    assert!(matches("debug", r#"format!("{fact.inner:?}")"#));
    assert!(matches("debug", "builder.debug_part(\"kind\", kind);"));
    assert!(!matches("debug", "fn debug_part(&mut self, label: &str) {"));
    assert!(!matches("debug", r#"format!("{Fact:?}")"#));

    // raw: nothing in the tree matches it today, so its branches are only
    // exercised here.
    assert!(matches("raw", r#"format!("{}", id.raw())"#));
    assert!(matches("raw", "let text = id.raw().to_string();"));
    assert!(matches("raw", r#"format!("{id.raw()}")"#));
    assert!(!matches("raw", "let value = id.raw();"));

    // other-debug: the complement of the debug command over `{...?}` forms.
    assert!(matches("other-debug", r#"panic!("{fact:#?}")"#));
    assert!(matches("other-debug", r#"panic!("{:#?}", fact)"#));
    assert!(!matches("other-debug", r#"panic!("{:?}", fact)"#));
    assert!(!matches("other-debug", r#"panic!("{fact:?}")"#));
}

#[test]
fn dense_id_sweep_reads_structure_the_way_the_test_rule_states_it() {
    let parsed = ParsedFile::parse(
        "#![cfg(test)]\n\
         impl Foo {\n\
         fn bar(&self) -> String {\n\
         let brace = \"{ not code }\";\n\
         format!(\"{:?}\", self)\n\
         }\n\
         }\n",
    );
    assert_eq!(parsed.lines[4].item, "fn bar");
    assert_eq!(parsed.lines[1].item, "<none>");
    assert_eq!(
        parsed.first_inner_attribute.as_deref(),
        Some("#![cfg(test)]")
    );
    // The brace inside the string literal must not have moved the depth.
    assert_eq!(parsed.lines[5].item, "fn bar");

    let parsed = ParsedFile::parse(
        "#[cfg(test)]\n\
         mod tests {\n\
         fn helper() {}\n\
         }\n\
         fn production() {}\n",
    );
    assert!(parsed.in_cfg_test_item(2));
    assert!(!parsed.in_cfg_test_item(4));

    // An attribute and its item on one line, ending in `;`, is that one line.
    let parsed = ParsedFile::parse("#[rustfmt::skip]\n#[cfg(test)] mod debug;\nmod other;\n");
    assert!(parsed.in_cfg_test_item(1));
    assert!(!parsed.in_cfg_test_item(2));
    assert_eq!(
        parsed.module_declarations,
        vec![(1, "debug".to_string()), (2, "other".to_string())]
    );
}

#[test]
fn dense_id_sweep_resolves_module_declarations_both_ways() {
    let from_mod = Path::new("crates/polint/src/analysis_kernel/mod.rs");
    assert_eq!(
        module_files(from_mod, "debug"),
        vec![
            PathBuf::from("crates/polint/src/analysis_kernel/debug.rs"),
            PathBuf::from("crates/polint/src/analysis_kernel/debug/mod.rs"),
        ]
    );
    let from_file = Path::new("crates/polint/src/analysis_kernel/store.rs");
    assert_eq!(
        module_files(from_file, "scale_tests"),
        vec![
            PathBuf::from("crates/polint/src/analysis_kernel/store/scale_tests.rs"),
            PathBuf::from("crates/polint/src/analysis_kernel/store/scale_tests/mod.rs"),
        ]
    );
}

/// Files every hit in which is test-only, under rules (2), (3) and (4) of the
/// test rule in section 2.1:
///
///   (2) the file's first inner attribute is `#![cfg(test)]` or `#![cfg(all(test, ...))]`;
///   (3) the file is declared by a `mod name;` item rule (1) classifies test-only,
///       applied transitively;
///   (4) the file is named `tests.rs` or lies under a `tests/` directory.
fn test_only_files(parsed: &BTreeMap<PathBuf, ParsedFile>) -> BTreeSet<PathBuf> {
    let mut test_only = BTreeSet::new();
    for (path, file) in parsed {
        let inner_is_test = file.first_inner_attribute.as_deref().is_some_and(|code| {
            code.starts_with("#![cfg(test)]") || code.starts_with("#![cfg(all(test")
        });
        let named_tests = path.file_name().is_some_and(|name| name == "tests.rs")
            || path
                .components()
                .any(|component| component.as_os_str() == "tests");
        if inner_is_test || named_tests {
            test_only.insert(path.clone());
        }
    }

    // Rule (3), to a fixpoint: a test-only `mod name;` makes the file it declares
    // test-only, and every `mod name;` in a wholly test-only file is itself
    // test-only.
    loop {
        let mut grew = false;
        for (path, file) in parsed {
            let whole_file = test_only.contains(path);
            for (line, name) in &file.module_declarations {
                if !(whole_file || file.in_cfg_test_item(*line)) {
                    continue;
                }
                for candidate in module_files(path, name) {
                    if parsed.contains_key(&candidate) && test_only.insert(candidate) {
                        grew = true;
                    }
                }
            }
        }
        if !grew {
            break;
        }
    }
    test_only
}

/// The two paths `mod <name>;` in `declaring` can resolve to.
fn module_files(declaring: &Path, name: &str) -> Vec<PathBuf> {
    let Some(parent) = declaring.parent() else {
        return Vec::new();
    };
    let stem = declaring.file_stem().and_then(|stem| stem.to_str());
    let directory = match stem {
        Some("mod") | Some("lib") | Some("main") | None => parent.to_path_buf(),
        Some(stem) => parent.join(stem),
    };
    vec![
        directory.join(format!("{name}.rs")),
        directory.join(name).join("mod.rs"),
    ]
}
