//! Shared helpers for integration tests (`tests/*.rs`).

use assert_cmd::Command;
use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub(crate) fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|e| panic!("create parent dir for {}: {e}", path.display()));
    }
    let contents = unique_rule_pack_manifest_contents(path, contents);
    fs::write(path, contents.as_ref())
        .unwrap_or_else(|e| panic!("write fixture file {}: {e}", path.display()));
}

pub(crate) fn uniquify_rule_pack_manifest(path: &Path) {
    let contents = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read fixture manifest {}: {e}", path.display()));
    let rewritten = unique_rule_pack_manifest_contents(path, &contents);
    fs::write(path, rewritten.as_ref())
        .unwrap_or_else(|e| panic!("rewrite fixture manifest {}: {e}", path.display()));
}

fn unique_rule_pack_manifest_contents<'a>(path: &Path, contents: &'a str) -> Cow<'a, str> {
    if !is_rule_pack_manifest(path) {
        return Cow::Borrowed(contents);
    }

    let suffix = path_hash_suffix(path);
    let mut changed = false;
    let rewritten = contents
        .lines()
        .map(|line| {
            for prefix in [r#"name = "polint-local-rules"#, r#"name = "polint-rules"#] {
                if let Some(rest) = line.strip_prefix(prefix)
                    && let Some(rest) = rest.strip_prefix('"')
                {
                    changed = true;
                    return format!("{prefix}-{suffix}\"{rest}");
                }
            }
            line.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");

    if changed {
        Cow::Owned(format!("{rewritten}\n"))
    } else {
        Cow::Borrowed(contents)
    }
}

fn is_rule_pack_manifest(path: &Path) -> bool {
    path.to_string_lossy()
        .replace('\\', "/")
        .ends_with("/.polint/rules/Cargo.toml")
}

fn path_hash_suffix(path: &Path) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in path.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

pub(crate) fn stdout_json(assert: assert_cmd::assert::Assert) -> serde_json::Value {
    let stdout = String::from_utf8(assert.get_output().stdout.clone())
        .expect("polint stdout should be valid UTF-8");
    serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("stdout was not parseable JSON: {error}\nstdout:\n{stdout}"))
}

pub(crate) fn stdout_string(assert: assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone())
        .expect("polint stdout should be valid UTF-8")
}

pub(crate) fn cache_json_count(root: &Path) -> usize {
    let cache_dir = root.join(".polint/cache");
    if !cache_dir.exists() {
        return 0;
    }
    cache_json_count_in(&cache_dir)
}

fn cache_json_count_in(path: &Path) -> usize {
    fs::read_dir(path)
        .unwrap_or_else(|e| panic!("read cache dir {}: {e}", path.display()))
        .map(|entry| {
            let entry = entry.unwrap_or_else(|e| panic!("read cache entry: {e}"));
            let path = entry.path();
            if path.is_dir() {
                cache_json_count_in(&path)
            } else if path.extension().is_some_and(|ext| ext == "json") {
                1
            } else {
                0
            }
        })
        .sum()
}

pub(crate) fn diagnostic_files(value: &serde_json::Value, rule_id: &str) -> Vec<String> {
    diagnostics(value)
        .iter()
        .filter(|diagnostic| diagnostic["rule_id"] == rule_id)
        .map(|diagnostic| {
            diagnostic["file"]
                .as_str()
                .unwrap_or_else(|| {
                    panic!("expected diagnostic.file to be a string, got: {diagnostic:#?}")
                })
                .to_string()
        })
        .collect()
}

pub(crate) fn diagnostics(value: &serde_json::Value) -> &[serde_json::Value] {
    value["diagnostics"].as_array().unwrap_or_else(|| {
        panic!("expected polint JSON report with diagnostics array, got: {value:?}")
    })
}

pub(crate) fn diagnostic_has_evidence(
    diagnostic: &serde_json::Value,
    label: &str,
    value: &str,
) -> bool {
    diagnostic["evidence"].as_array().is_some_and(|evidence| {
        evidence.iter().any(|item| {
            item["label"] == label
                && item["value"]
                    .as_str()
                    .is_some_and(|actual| actual.contains(value))
        })
    })
}

pub(crate) fn diagnostics_for_rule<'a>(
    value: &'a serde_json::Value,
    rule_id: &str,
) -> Vec<&'a serde_json::Value> {
    diagnostics(value)
        .iter()
        .filter(|diagnostic| diagnostic["rule_id"] == rule_id)
        .collect()
}

pub(crate) fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("polint crate should live under crates/")
        .to_path_buf()
}

fn shared_cargo_target_dir() -> PathBuf {
    repo_root().join("target/polint-cli-test-cargo")
}

fn shared_rules_target_dir() -> PathBuf {
    repo_root().join("target/polint-cli-test-rules")
}

pub(crate) fn cargo_cmd() -> Command {
    let mut command = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string()));
    command.env("CARGO_TARGET_DIR", shared_cargo_target_dir());
    command.env("POLINT_RULES_TARGET_DIR", shared_rules_target_dir());
    command.env("POLINT_RULES_PROFILE", "dev");
    command.env("POLINT_CACHE_STORE", "off");
    command
}

pub(crate) fn polint_cmd() -> Command {
    let mut command = Command::cargo_bin("polint").unwrap();
    command.env("CARGO_TARGET_DIR", shared_cargo_target_dir());
    command.env("POLINT_RULES_TARGET_DIR", shared_rules_target_dir());
    command.env("POLINT_RULES_PROFILE", "dev");
    // A test never reaches the developer's machine-global rule-host store: the
    // suite would fill it with hosts nothing else will ask for, and a run's
    // result must not depend on what another checkout left there. The tests
    // that cover sharing point this at a directory of their own.
    command.env("POLINT_CACHE_STORE", "off");
    command.env_remove("POLINT_JOBS");
    command
}

pub(crate) fn polint_help(args: &[&str]) -> String {
    static TOP_LEVEL: OnceLock<String> = OnceLock::new();
    static CHECK: OnceLock<String> = OnceLock::new();
    static INSPECT: OnceLock<String> = OnceLock::new();
    static INSPECT_RULE: OnceLock<String> = OnceLock::new();
    static TEST: OnceLock<String> = OnceLock::new();
    static CACHE: OnceLock<String> = OnceLock::new();
    static CACHE_STATUS: OnceLock<String> = OnceLock::new();

    let cache = match args {
        ["--help"] => &TOP_LEVEL,
        ["check", "--help"] => &CHECK,
        ["inspect", "--help"] => &INSPECT,
        ["inspect", "rule", "--help"] => &INSPECT_RULE,
        ["test", "--help"] => &TEST,
        ["cache", "--help"] => &CACHE,
        ["cache", "status", "--help"] => &CACHE_STATUS,
        _ => panic!("unsupported cached polint help command: {args:?}"),
    };

    cache
        .get_or_init(|| stdout_string(polint_cmd().args(args.iter().copied()).assert().success()))
        .clone()
}

pub(crate) fn shared_fixture_root() -> &'static Path {
    static SHARED_FIXTURE: OnceLock<tempfile::TempDir> = OnceLock::new();

    SHARED_FIXTURE
        .get_or_init(|| {
            let fixture = tempfile::tempdir().expect("create shared CLI fixture");
            polint_cmd()
                .current_dir(fixture.path())
                .arg("init")
                .assert()
                .success();
            polint_cmd()
                .current_dir(fixture.path())
                .args(["new-rule", "ts", "no-raw-colors"])
                .assert()
                .success();
            fixture
        })
        .path()
}

pub(crate) fn fixture_workspace() -> tempfile::TempDir {
    let workspace = tempfile::tempdir().expect("create CLI fixture workspace");
    copy_dir_contents(shared_fixture_root(), workspace.path());
    workspace
}

fn copy_dir_contents(source: &Path, destination: &Path) {
    for entry in fs::read_dir(source)
        .unwrap_or_else(|error| panic!("read shared fixture {}: {error}", source.display()))
    {
        let entry = entry.expect("read shared fixture entry");
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            fs::create_dir_all(&destination_path).unwrap_or_else(|error| {
                panic!(
                    "create fixture directory {}: {error}",
                    destination_path.display()
                )
            });
            copy_dir_contents(&source_path, &destination_path);
        } else {
            fs::copy(&source_path, &destination_path).unwrap_or_else(|error| {
                panic!(
                    "copy fixture file {} to {}: {error}",
                    source_path.display(),
                    destination_path.display()
                )
            });
        }
    }
}

fn example_rule_cmd(package: &'static str) -> Command {
    static EXAMPLE_RULES_BUILT: OnceLock<()> = OnceLock::new();

    EXAMPLE_RULES_BUILT.get_or_init(|| {
        cargo_cmd()
            .current_dir(repo_root())
            .args([
                "build",
                "--quiet",
                "--package",
                "polint-example-ts-design-tokens-rule",
                "--package",
                "polint-example-ts-complexity-rule",
                "--package",
                "polint-example-go-import-boundaries-rule",
                "--package",
                "polint-example-go-branch-obligations-rule",
            ])
            .assert()
            .success();
    });

    let executable = shared_cargo_target_dir()
        .join("debug")
        .join(format!("{package}{}", std::env::consts::EXE_SUFFIX));
    let mut command = Command::new(executable);
    command.env("CARGO_TARGET_DIR", shared_cargo_target_dir());
    command.env("POLINT_RULES_TARGET_DIR", shared_rules_target_dir());
    command.env("POLINT_RULES_PROFILE", "dev");
    command.env("POLINT_CACHE_STORE", "off");
    command
}

pub(crate) fn raw_color_rule_cmd() -> Command {
    example_rule_cmd("polint-example-ts-design-tokens-rule")
}

pub(crate) fn ts_complexity_rule_cmd() -> Command {
    example_rule_cmd("polint-example-ts-complexity-rule")
}

pub(crate) fn go_import_boundaries_rule_cmd() -> Command {
    example_rule_cmd("polint-example-go-import-boundaries-rule")
}

pub(crate) fn go_branch_obligations_rule_cmd() -> Command {
    example_rule_cmd("polint-example-go-branch-obligations-rule")
}

pub(crate) fn write_phase8_raw_color_fixture(root: &Path, severity: &str) {
    write_file(
        &root.join(".polint.toml"),
        &format!(
            r##"
[profiles.phase8]
rules = ["local/no-raw-colors"]

[[rules.config]]
id = "local/no-raw-colors"
severity = "{severity}"
files = ["**/*.tsx"]
"##
        ),
    );
    write_file(
        &root.join("component.tsx"),
        "export function Button() { return <button style={{ color: \"#ff00aa\" }}>Pay</button>; }\n",
    );
}
