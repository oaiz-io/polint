//! The machine-global rule-host store, end to end.
//!
//! The store's whole claim is that a rule host compiled once on a machine can be
//! run from another checkout without compiling it again, and that this changes
//! nothing about what `polint check` reports. Both halves are asserted here
//! against a real build: a run that publishes, a run that restores into an empty
//! cargo target directory with a `cargo` that refuses to compile anything, a run
//! that answers from the stamp that restore left behind, and a run whose rule
//! sources changed by one byte and therefore has to compile.
//!
//! Unix only, like the neighboring rule-host tests: the stand-in cargo is a
//! shell script.
#![cfg(unix)]

use assert_cmd::Command;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("polint crate should live under crates/")
        .to_path_buf()
}

/// The cargo target directory the whole CLI test suite shares, so a rule host
/// built here reuses the dependency units the other tests already compiled.
fn shared_rules_target_dir() -> PathBuf {
    repo_root().join("target/polint-cli-test-rules")
}

fn polint_cmd() -> Command {
    let mut command = Command::cargo_bin("polint").unwrap();
    command.env(
        "CARGO_TARGET_DIR",
        repo_root().join("target/polint-cli-test-cargo"),
    );
    command.env("POLINT_RULES_TARGET_DIR", shared_rules_target_dir());
    command.env("POLINT_RULES_PROFILE", "dev");
    command
}

fn stdout_string(assert: assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone())
        .expect("polint stdout should be valid UTF-8")
}

fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|error| panic!("create parent dir for {}: {error}", path.display()));
    }
    fs::write(path, contents)
        .unwrap_or_else(|error| panic!("write fixture file {}: {error}", path.display()));
}

/// Replace the generated pack with a dependency-free host that emits a valid
/// empty report. The store behavior under test does not depend on polint's SDK,
/// and keeping every build input inside the package makes the sharing proof
/// explicit.
fn replace_generated_rule_pack_with_static_host(root: &Path) {
    let manifest_path = root.join(".polint/rules/Cargo.toml");
    let suffix = path_hash_suffix(&manifest_path);
    write_file(
        &manifest_path,
        &format!(
            "[package]\nname = \"polint-store-host-{suffix}\"\nversion = \"0.0.0\"\n\
             edition = \"2024\"\n\n[workspace]\n"
        ),
    );
    write_file(
        &root.join(".polint/rules/src/main.rs"),
        "fn main() {\n    if std::env::var_os(\"POLINT_STORE_HOST_FAIL\").is_some() {\n        \
         eprintln!(\"intentional host failure\");\n        std::process::exit(17);\n    }\n    \
         println!(\"{{\\\"diagnostics\\\":[]}}\");\n}\n",
    );
}

fn path_hash_suffix(path: &Path) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in path.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

/// A repository with a generated rule pack, ready to check.
fn fixture_workspace() -> tempfile::TempDir {
    let workspace = tempfile::tempdir().expect("create a fixture workspace");
    polint_cmd()
        .current_dir(workspace.path())
        .arg("init")
        .assert()
        .success();
    polint_cmd()
        .current_dir(workspace.path())
        .args(["new-rule", "ts", "no-raw-colors"])
        .assert()
        .success();
    replace_generated_rule_pack_with_static_host(workspace.path());
    write_file(
        &workspace.path().join("src/theme.ts"),
        "export const color = \"#ff0000\";\n",
    );
    workspace
}

/// A `cargo` that reports its version and refuses to do anything else.
///
/// polint reads the cargo version into every build fingerprint, so a stand-in
/// that could not answer `-V` would change the key rather than prove anything
/// about it. Every other invocation is recorded and fails, so "polint compiled
/// nothing" is asserted from the absence of a record rather than inferred from a
/// run that happened to succeed.
fn cargo_that_refuses_to_compile(directory: &Path) -> (PathBuf, PathBuf) {
    let script = directory.join("cargo");
    let record = directory.join("invocations");
    let real = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    write_file(
        &script,
        &format!(
            "#!/bin/sh\n\
             if [ \"$1\" = \"-V\" ]; then exec {real} \"$@\"; fi\n\
             echo \"$@\" >> {}\n\
             echo \"fake cargo refused: $*\" >&2\n\
             exit 42\n",
            record.display()
        ),
    );
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    (script, record)
}

/// Every rule-host entry the store holds.
fn store_entries(store: &Path) -> Vec<PathBuf> {
    fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect(&path, out);
            } else {
                out.push(path);
            }
        }
    }

    let mut out = Vec::new();
    collect(&store.join("rule-hosts"), &mut out);
    out.sort();
    out
}

#[test]
fn a_rule_host_compiled_once_is_shared_with_every_other_checkout() {
    let workspace = fixture_workspace();
    let root = workspace.path();
    let store = tempfile::tempdir().expect("create a store directory");
    let scratch = tempfile::tempdir().expect("create a scratch directory");
    let (cargo, invocations) = cargo_that_refuses_to_compile(scratch.path());

    // A first check compiles the rule host and publishes it under the
    // fingerprint of the inputs that produced it.
    let published = stdout_string(
        polint_cmd()
            .current_dir(root)
            .env("POLINT_CACHE_STORE", store.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    let entries = store_entries(store.path());
    assert_eq!(
        entries.len(),
        1,
        "one build publishes one entry: {entries:?}"
    );
    let entry: serde_json::Value =
        serde_json::from_slice(&fs::read(&entries[0]).expect("read the published entry"))
            .expect("the entry is JSON");
    assert_eq!(entry["schema"], "polint-rule-host-store-v1");
    assert_eq!(
        entry["polint_version"],
        env!("CARGO_PKG_VERSION"),
        "an entry records the polint that wrote it"
    );
    assert!(
        entry["target_relative_path"]
            .as_str()
            .is_some_and(|path| path.starts_with("debug/")),
        "the entry places the host under the profile it was built with: {entry}"
    );

    // A checkout with an empty cargo target directory restores that host and
    // runs it. `cargo` here refuses to compile, so a successful run can only
    // mean nothing was compiled.
    let fresh_target = tempfile::tempdir().expect("create an empty target directory");
    let restored = stdout_string(
        polint_cmd()
            .current_dir(root)
            .env("POLINT_CACHE_STORE", store.path())
            .env("POLINT_RULES_TARGET_DIR", fresh_target.path())
            .env("POLINT_CARGO", &cargo)
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    assert!(
        !invocations.exists(),
        "a restored host is never compiled: {}",
        fs::read_to_string(&invocations).unwrap_or_default()
    );
    assert_eq!(
        restored, published,
        "the store changes how the host is obtained, never what it reports"
    );

    // That restore stamped the checkout, so the next run needs neither a Cargo
    // build/run nor the store (the version probe still protects toolchain identity).
    let stamped = stdout_string(
        polint_cmd()
            .current_dir(root)
            .env("POLINT_CACHE_STORE", "off")
            .env("POLINT_RULES_TARGET_DIR", fresh_target.path())
            .env("POLINT_CARGO", &cargo)
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    assert!(!invocations.exists(), "a stamped host is never compiled");
    assert_eq!(stamped, published);

    // Cargo appends its own diagnostic when the host exits nonzero. A direct
    // execution that fails therefore re-enters Cargo so the public failure is
    // byte-identical to the original path.
    let stamp_path = fresh_target.path().join("polint-store-stamp.json");
    let stamp_bytes = fs::read(&stamp_path).expect("read stamp bytes");
    let direct_nonzero = polint_cmd()
        .current_dir(root)
        .env("POLINT_CACHE_STORE", "off")
        .env("POLINT_RULES_TARGET_DIR", fresh_target.path())
        .env("POLINT_STORE_HOST_FAIL", "1")
        .args(["check", "--format", "json", "--fail-on", "none"])
        .output()
        .expect("run failing stamped host");
    assert!(!direct_nonzero.status.success());
    fs::remove_file(&stamp_path).expect("remove stamp for nonzero baseline");
    let cargo_nonzero = polint_cmd()
        .current_dir(root)
        .env("POLINT_CACHE_STORE", "off")
        .env("POLINT_RULES_TARGET_DIR", fresh_target.path())
        .env("POLINT_STORE_HOST_FAIL", "1")
        .args(["check", "--format", "json", "--fail-on", "none"])
        .output()
        .expect("run failing host through cargo");
    assert_eq!(direct_nonzero.status, cargo_nonzero.status);
    assert_eq!(direct_nonzero.stdout, cargo_nonzero.stdout);
    assert_eq!(direct_nonzero.stderr, cargo_nonzero.stderr);
    fs::write(&stamp_path, &stamp_bytes).expect("restore stamp for spawn test");

    // A stamp can still identify the right bytes when the executable bit was
    // lost. Failure to spawn that direct host is a cache miss, and the exact
    // original cargo-run failure is preserved.
    let stamp: serde_json::Value = serde_json::from_slice(&stamp_bytes).expect("stamp is JSON");
    let stamped_binary = fresh_target.path().join(
        stamp["target_relative_path"]
            .as_str()
            .expect("stamp records target path"),
    );
    fs::set_permissions(&stamped_binary, fs::Permissions::from_mode(0o644))
        .expect("remove executable bit");
    let direct_spawn_miss = polint_cmd()
        .current_dir(root)
        .env("POLINT_CACHE_STORE", "off")
        .env("POLINT_RULES_TARGET_DIR", fresh_target.path())
        .env("POLINT_CARGO", &cargo)
        .args(["check", "--format", "json", "--fail-on", "none"])
        .output()
        .expect("run with unusable stamped binary");
    assert!(!direct_spawn_miss.status.success());
    fs::remove_file(&stamp_path).expect("remove stamp for cargo-only baseline");
    let cargo_only_failure = polint_cmd()
        .current_dir(root)
        .env("POLINT_CACHE_STORE", "off")
        .env("POLINT_RULES_TARGET_DIR", fresh_target.path())
        .env("POLINT_CARGO", &cargo)
        .args(["check", "--format", "json", "--fail-on", "none"])
        .output()
        .expect("run cargo-only failure baseline");
    assert_eq!(direct_spawn_miss.status, cargo_only_failure.status);
    assert_eq!(direct_spawn_miss.stdout, cargo_only_failure.stdout);
    assert_eq!(direct_spawn_miss.stderr, cargo_only_failure.stderr);
    fs::remove_file(&invocations).expect("clear direct-spawn fallback invocations");

    // One changed byte of rule source is a different build, so neither the stamp
    // nor the store answers and cargo is asked to compile again.
    let rule = root.join(".polint/rules/src/main.rs");
    let source = fs::read_to_string(&rule).expect("read the rule source");
    fs::write(&rule, format!("{source}\n// one more byte\n")).expect("edit the rule source");
    let speculative_failure = polint_cmd()
        .current_dir(root)
        .env("POLINT_CACHE_STORE", store.path())
        .env("POLINT_RULES_TARGET_DIR", fresh_target.path())
        .env("POLINT_CARGO", &cargo)
        .args(["check", "--format", "json", "--fail-on", "none"])
        .output()
        .expect("run changed-source failure");
    assert!(!speculative_failure.status.success());
    assert!(
        invocations.exists(),
        "changed rule sources are compiled rather than restored"
    );
    let attempted = fs::read_to_string(&invocations).expect("read cargo attempts");
    assert!(attempted.lines().any(|line| line.starts_with("build ")));
    assert!(attempted.lines().any(|line| line.starts_with("run ")));

    fs::remove_file(&invocations).expect("clear speculative attempts");
    let original_failure = polint_cmd()
        .current_dir(root)
        .env("POLINT_CACHE_STORE", "off")
        .env("POLINT_RULES_TARGET_DIR", fresh_target.path())
        .env("POLINT_CARGO", &cargo)
        .args(["check", "--format", "json", "--fail-on", "none"])
        .output()
        .expect("run original cargo path");
    assert_eq!(speculative_failure.status, original_failure.status);
    assert_eq!(speculative_failure.stdout, original_failure.stdout);
    assert_eq!(speculative_failure.stderr, original_failure.stderr);

    // The first build stamped the target directory the whole suite shares.
    // Leave it as it was found: every other test compiles for itself.
    let _ = fs::remove_file(shared_rules_target_dir().join("polint-store-stamp.json"));
}

/// A host published from one cargo home is restored from another one.
///
/// The key hashes cargo configuration by content, never the directory it was
/// read from, so a fresh container whose cargo home sits somewhere else restores
/// the binary instead of paying for the compile a second time. Both homes here
/// are empty, which is the same configuration from two paths; `cargo` in the
/// second run refuses to compile, so the restore is asserted from the absence of
/// any invocation rather than inferred from a run that happened to succeed.
#[test]
fn a_rule_host_is_shared_between_cargo_homes_at_different_paths() {
    let workspace = fixture_workspace();
    let root = workspace.path();
    let store = tempfile::tempdir().expect("create a store directory");
    let scratch = tempfile::tempdir().expect("create a scratch directory");
    let (cargo, invocations) = cargo_that_refuses_to_compile(scratch.path());
    let publishing_home = tempfile::tempdir().expect("create the publishing cargo home");
    let restoring_home = tempfile::tempdir().expect("create the restoring cargo home");
    assert_ne!(
        publishing_home.path(),
        restoring_home.path(),
        "the two cargo homes must differ for this test to say anything"
    );

    let published = stdout_string(
        polint_cmd()
            .current_dir(root)
            .env("POLINT_CACHE_STORE", store.path())
            .env("CARGO_HOME", publishing_home.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    let entries = store_entries(store.path());
    assert_eq!(
        entries.len(),
        1,
        "one build publishes one entry: {entries:?}"
    );

    let fresh_target = tempfile::tempdir().expect("create an empty target directory");
    let restored = stdout_string(
        polint_cmd()
            .current_dir(root)
            .env("POLINT_CACHE_STORE", store.path())
            .env("POLINT_RULES_TARGET_DIR", fresh_target.path())
            .env("POLINT_CARGO", &cargo)
            .env("CARGO_HOME", restoring_home.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    assert!(
        !invocations.exists(),
        "a cargo home at another path is still a store hit: {}",
        fs::read_to_string(&invocations).unwrap_or_default()
    );
    assert_eq!(
        restored, published,
        "the cargo home a host was compiled under never changes what it reports"
    );
    assert_eq!(
        store_entries(store.path()),
        entries,
        "a restore republishes nothing under a second key"
    );

    // As above: the first build stamped the target directory the suite shares.
    let _ = fs::remove_file(shared_rules_target_dir().join("polint-store-stamp.json"));
}
