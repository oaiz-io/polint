//! Inventory gate for the golden-corpus input surface.
//!
//! Asserts that `tests/golden-corpus/inputs.toml` is a complete, exact listing
//! of example rule packs and eval-fixture trees, and that scale-repo pins are
//! full commit SHAs reachable through `make fetch-scale-repos` (never floating).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use toml::Value;

const INPUTS_REL: &str = "tests/golden-corpus/inputs.toml";
const FETCH_SCRIPT_REL: &str = "scripts/fetch-scale-repos.py";
const EXPECTED_SCHEMA: &str = "polint-golden-corpus-inputs-1";
const FULL_SHA_LEN: usize = 40;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/polint has a workspace root two levels up")
        .to_path_buf()
}

fn load_toml(path: &Path) -> Value {
    let raw = fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    toml::from_str(&raw).unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()))
}

fn string_array<'a>(table: &'a Value, key: &str, path: &Path) -> Vec<&'a str> {
    let Some(Value::Array(items)) = table.get(key) else {
        panic!("{}: missing string array `{key}`", path.display());
    };
    items
        .iter()
        .map(|item| {
            item.as_str()
                .unwrap_or_else(|| panic!("{}: `{key}` entries must be strings", path.display()))
        })
        .collect()
}

fn discover_example_rule_packs(root: &Path) -> BTreeSet<String> {
    let examples = root.join("examples");
    let mut packs = BTreeSet::new();
    let entries = fs::read_dir(&examples).unwrap_or_else(|err| {
        panic!("failed to read {}: {err}", examples.display());
    });
    for entry in entries {
        let entry = entry.unwrap();
        if !entry.file_type().unwrap().is_dir() {
            continue;
        }
        let rules = entry.path().join(".polint/rules");
        if rules.is_dir() && rules.join("Cargo.toml").is_file() {
            packs.insert(
                rules
                    .strip_prefix(root)
                    .expect("rules path under repo root")
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    packs
}

fn discover_eval_fixture_trees(root: &Path) -> BTreeSet<String> {
    let fixtures = root.join("tests/eval-fixtures");
    let mut trees = BTreeSet::new();
    let entries = fs::read_dir(&fixtures).unwrap_or_else(|err| {
        panic!("failed to read {}: {err}", fixtures.display());
    });
    for entry in entries {
        let entry = entry.unwrap();
        if !entry.file_type().unwrap().is_dir() {
            continue;
        }
        trees.insert(
            entry
                .path()
                .strip_prefix(root)
                .expect("fixture path under repo root")
                .to_string_lossy()
                .replace('\\', "/"),
        );
    }
    trees
}

fn assert_sets_equal(label: &str, declared: &BTreeSet<String>, discovered: &BTreeSet<String>) {
    let missing: Vec<_> = discovered.difference(declared).cloned().collect();
    let extra: Vec<_> = declared.difference(discovered).cloned().collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "{label} inventory mismatch\n  present on disk but missing from {INPUTS_REL}: {missing:?}\n  declared in {INPUTS_REL} but missing on disk: {extra:?}"
    );
}

fn parse_scale_pin(root: &Path, relative_manifest: &str) -> (String, String, String, String) {
    let path = root.join(relative_manifest);
    let suite = load_toml(&path);
    let id = suite
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{}: missing id", path.display()))
        .to_string();
    let source_url = suite
        .get("source_url")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{}: missing source_url", path.display()))
        .to_string();
    let source_commit = suite
        .get("source_commit")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{}: missing source_commit", path.display()))
        .to_string();
    assert_eq!(
        source_commit.len(),
        FULL_SHA_LEN,
        "{}: source_commit must be a full {FULL_SHA_LEN}-char SHA, got {source_commit}",
        path.display()
    );
    assert!(
        source_commit.chars().all(|ch| ch.is_ascii_hexdigit()),
        "{}: source_commit is not hex: {source_commit}",
        path.display()
    );
    let checkout_path = suite
        .get("checkout")
        .and_then(Value::as_table)
        .and_then(|table| table.get("path"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{}: missing checkout.path", path.display()))
        .to_string();
    assert!(
        !Path::new(&checkout_path).is_absolute(),
        "{}: checkout.path must be repo-relative",
        path.display()
    );
    (
        id,
        source_commit.to_ascii_lowercase(),
        source_url,
        checkout_path,
    )
}

#[test]
fn golden_corpus_inputs_match_on_disk_targets() {
    let root = repo_root();
    let inputs_path = root.join(INPUTS_REL);
    let inputs = load_toml(&inputs_path);

    let schema = inputs
        .get("schema_version")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{}: missing schema_version", inputs_path.display()));
    assert_eq!(schema, EXPECTED_SCHEMA);

    let declared_packs: BTreeSet<String> =
        string_array(&inputs, "example_rule_packs", &inputs_path)
            .into_iter()
            .map(str::to_string)
            .collect();
    let declared_fixtures: BTreeSet<String> =
        string_array(&inputs, "eval_fixture_trees", &inputs_path)
            .into_iter()
            .map(str::to_string)
            .collect();
    let scale_manifests = string_array(&inputs, "scale_suite_manifests", &inputs_path);

    assert_eq!(
        declared_packs.len(),
        18,
        "expected 18 example rule packs, found {}",
        declared_packs.len()
    );
    assert_eq!(
        declared_fixtures.len(),
        26,
        "expected 26 eval-fixture trees, found {}",
        declared_fixtures.len()
    );
    assert_eq!(
        scale_manifests.len(),
        3,
        "expected 3 scale suite manifests, found {}",
        scale_manifests.len()
    );

    assert_sets_equal(
        "example rule packs",
        &declared_packs,
        &discover_example_rule_packs(&root),
    );
    assert_sets_equal(
        "eval fixture trees",
        &declared_fixtures,
        &discover_eval_fixture_trees(&root),
    );

    for relative in &declared_packs {
        let cargo_toml = root.join(relative).join("Cargo.toml");
        assert!(
            cargo_toml.is_file(),
            "rule pack missing Cargo.toml: {}",
            cargo_toml.display()
        );
    }
    for relative in &declared_fixtures {
        let path = root.join(relative);
        assert!(path.is_dir(), "fixture tree missing: {}", path.display());
    }

    let mut seen_ids = BTreeSet::new();
    for relative in &scale_manifests {
        let path = root.join(relative);
        assert!(
            path.is_file(),
            "scale suite manifest missing: {}",
            path.display()
        );
        let (id, commit, url, checkout) = parse_scale_pin(&root, relative);
        assert!(
            seen_ids.insert(id.clone()),
            "duplicate scale suite id `{id}`"
        );
        assert!(
            url.starts_with("https://"),
            "{relative}: source_url must be https, got {url}"
        );
        assert!(
            checkout.starts_with("research/evaluation-harness/repos/"),
            "{relative}: checkout.path must stay under research/evaluation-harness/repos/, got {checkout}"
        );
        assert_eq!(commit.len(), FULL_SHA_LEN);
    }
}

#[test]
fn fetch_scale_repos_make_target_prints_pinned_shas() {
    let root = repo_root();
    let script = root.join(FETCH_SCRIPT_REL);
    assert!(script.is_file(), "missing {}", script.display());

    let makefile = fs::read_to_string(root.join("Makefile"))
        .unwrap_or_else(|err| panic!("failed to read Makefile: {err}"));
    assert!(
        makefile.contains("fetch-scale-repos:"),
        "Makefile must declare a fetch-scale-repos target"
    );
    assert!(
        makefile.contains(FETCH_SCRIPT_REL),
        "Makefile fetch-scale-repos must invoke {FETCH_SCRIPT_REL}"
    );

    let output = Command::new(std::env::var("PYTHON").unwrap_or_else(|_| "python3".to_string()))
        .arg(&script)
        .arg("--repo-root")
        .arg(&root)
        .arg("--print-pins")
        .output()
        .unwrap_or_else(|err| panic!("failed to run {}: {err}", script.display()));
    assert!(
        output.status.success(),
        "fetch-scale-repos --print-pins failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut rows = 0usize;
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let cols: Vec<_> = line.split('\t').collect();
        assert_eq!(
            cols.len(),
            5,
            "unexpected pin row (want id, sha, url, path, digest): {line}"
        );
        assert_eq!(
            cols[1].len(),
            FULL_SHA_LEN,
            "pin SHA must be full length: {line}"
        );
        assert!(
            cols[1].chars().all(|ch| ch.is_ascii_hexdigit()),
            "pin SHA must be hex: {line}"
        );
        assert!(
            cols[2].starts_with("https://"),
            "pin URL must be https: {line}"
        );
        assert!(
            cols[3].starts_with("research/evaluation-harness/repos/"),
            "pin checkout must be under repos/: {line}"
        );
        rows += 1;
    }
    assert_eq!(rows, 3, "expected three scale-repo pin rows, got {rows}");
}

/// If the inventory file is removed or emptied, characterization has no input surface.
#[test]
fn golden_corpus_inputs_file_is_non_empty_inventory() {
    let root = repo_root();
    let path = root.join(INPUTS_REL);
    let meta = fs::metadata(&path)
        .unwrap_or_else(|err| panic!("golden corpus inputs missing at {}: {err}", path.display()));
    assert!(meta.len() > 200, "{} is unexpectedly small", path.display());
}
