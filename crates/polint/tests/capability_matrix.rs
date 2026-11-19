//! Capability matrix: one assertion per (prelude fact view × language).
//!
//! Pins that each supported SDK fact view still returns non-empty, well-formed
//! data for languages that claim support. Distinct from the golden corpus.

use assert_cmd::Command;
use serde::Deserialize;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;

fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|err| panic!("create parent dir for {}: {err}", path.display()));
    }
    let contents = unique_rule_pack_manifest_contents(path, contents);
    fs::write(path, contents.as_ref())
        .unwrap_or_else(|err| panic!("write fixture file {}: {err}", path.display()));
}

fn uniquify_rule_pack_manifest(path: &Path) {
    let contents = fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("read fixture manifest {}: {err}", path.display()));
    let rewritten = unique_rule_pack_manifest_contents(path, &contents);
    fs::write(path, rewritten.as_ref())
        .unwrap_or_else(|err| panic!("rewrite fixture manifest {}: {err}", path.display()));
}

fn unique_rule_pack_manifest_contents<'a>(path: &Path, contents: &'a str) -> Cow<'a, str> {
    if !path
        .to_string_lossy()
        .replace('\\', "/")
        .ends_with("/.polint/rules/Cargo.toml")
    {
        return Cow::Borrowed(contents);
    }

    let suffix = rule_pack_suffix(path);
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

/// The suffix that names the rule pack at `path` in the shared rules target
/// directory.
///
/// It is the same for the same test, test binary, and fixture layout on every
/// run, so a re-run rebuilds that test's rule host in place rather than adding
/// another one per temporary directory; tests that run at the same time get
/// different names, so they never build over each other's host. A path outside
/// a temporary directory, or code not running on a named test thread, falls
/// back to a suffix of the whole path.
fn rule_pack_suffix(path: &Path) -> String {
    let thread = std::thread::current();
    let test = thread.name().filter(|name| *name != "main");
    let mut components = path.components();
    let in_temp_dir = components
        .by_ref()
        .any(|component| component.as_os_str().to_string_lossy().starts_with(".tmp"));
    let fixture = components.as_path();
    let identity = match test {
        Some(test) if in_temp_dir => {
            let binary = std::env::current_exe()
                .ok()
                .and_then(|exe| {
                    exe.file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                })
                .unwrap_or_default();
            format!("{binary}\0{test}\0{}", fixture.to_string_lossy())
        }
        _ => path.to_string_lossy().into_owned(),
    };
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in identity.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn stdout_json(assert: assert_cmd::assert::Assert) -> serde_json::Value {
    let stdout = String::from_utf8(assert.get_output().stdout.clone())
        .expect("polint stdout should be valid UTF-8");
    serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("stdout was not parseable JSON: {error}\nstdout:\n{stdout}"))
}

fn diagnostic_has_evidence(diagnostic: &serde_json::Value, label: &str, value: &str) -> bool {
    diagnostic["evidence"].as_array().is_some_and(|evidence| {
        evidence.iter().any(|item| {
            item["label"] == label
                && item["value"]
                    .as_str()
                    .is_some_and(|actual| actual.contains(value))
        })
    })
}

fn diagnostics_for_rule<'a>(
    value: &'a serde_json::Value,
    rule_id: &str,
) -> Vec<&'a serde_json::Value> {
    value["diagnostics"]
        .as_array()
        .unwrap_or_else(|| panic!("expected diagnostics array: {value:?}"))
        .iter()
        .filter(|diagnostic| diagnostic["rule_id"] == rule_id)
        .collect()
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("polint crate should live under crates/")
        .to_path_buf()
}

fn polint_cmd() -> Command {
    let mut command = Command::cargo_bin("polint").unwrap();
    command.env(
        "CARGO_TARGET_DIR",
        repo_root().join("target/polint-cli-test-cargo"),
    );
    command.env(
        "POLINT_RULES_TARGET_DIR",
        repo_root().join("target/polint-cli-test-rules"),
    );
    command.env("POLINT_RULES_PROFILE", "dev");
    command.env("POLINT_CACHE_STORE", "off");
    command
}

const MATRIX_REL: &str = "tests/capability-matrix/matrix.toml";
const FIXTURES_REL: &str = "tests/capability-matrix/fixtures";
const EXPECTED_SCHEMA: &str = "polint-capability-matrix-1";
const PRESENT_MESSAGE: &str = "capability present";

/// Every typed fact view re-exported by `polint::sdk::prelude`.
const PRELUDE_FACT_VIEWS: &[&str] = &[
    "BranchObligations",
    "CallGraph",
    "Calls",
    "Cfg",
    "ChangedFiles",
    "ComplexityMetrics",
    "ControlFlow",
    "CoverageFacts",
    "DataFlow",
    "Events",
    "FileMetrics",
    "FunctionMetrics",
    "Functions",
    "GoTests",
    "Imports",
    "JsxAttributes",
    "ModuleGraphFacts",
    "Packages",
    "References",
    "ResolvedImports",
    "SourceFiles",
    "StringLiterals",
    "Symbols",
    "TestSuiteMetrics",
    "TsClasses",
    "TsComponents",
];

#[derive(Debug, Deserialize)]
struct MatrixFile {
    schema: String,
    languages: Vec<String>,
    cell: Vec<MatrixCell>,
}

#[derive(Debug, Deserialize, Clone)]
struct MatrixCell {
    view: String,
    language: String,
    status: String,
    #[serde(default)]
    fixture: Option<String>,
}

fn matrix_path() -> PathBuf {
    repo_root().join(MATRIX_REL)
}

fn load_matrix() -> MatrixFile {
    let path = matrix_path();
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    let matrix: MatrixFile = toml::from_str(&raw)
        .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()));
    assert_eq!(
        matrix.schema,
        EXPECTED_SCHEMA,
        "{} schema must be {EXPECTED_SCHEMA}",
        path.display()
    );
    matrix
}

fn copy_dir(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap_or_else(|err| {
        panic!("create {}: {err}", destination.display());
    });
    for entry in
        fs::read_dir(source).unwrap_or_else(|err| panic!("read {}: {err}", source.display()))
    {
        let entry = entry.expect("read fixture entry");
        let from = entry.path();
        let to = destination.join(entry.file_name());
        if from.is_dir() {
            copy_dir(&from, &to);
        } else {
            fs::copy(&from, &to).unwrap_or_else(|err| {
                panic!("copy {} -> {}: {err}", from.display(), to.display());
            });
        }
    }
}

fn point_rule_pack_at_local_polint(root: &Path) {
    let manifest_path = root.join(".polint/rules/Cargo.toml");
    let manifest = fs::read_to_string(&manifest_path).unwrap();
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    let dep_line = format!(r#"polint = {{ path = "{polint_path}" }}"#);
    let rewritten = manifest
        .lines()
        .map(|line| {
            if line.starts_with("polint = ") {
                dep_line.clone()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&manifest_path, format!("{rewritten}\n")).unwrap();
    uniquify_rule_pack_manifest(&manifest_path);
}

fn write_probe_rule_pack(root: &Path) {
    let rules_dir = root.join(".polint/rules");
    fs::create_dir_all(rules_dir.join("src")).unwrap();
    write_file(
        &rules_dir.join("Cargo.toml"),
        r#"[package]
name = "polint-local-rules"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
polint = "0.0.0"

[workspace]
"#,
    );
    write_file(
        &rules_dir.join("src/main.rs"),
        r#"use std::process::ExitCode;

use polint::runner;

mod probes;

fn main() -> ExitCode {
    runner::run_cli(probes::all_rules())
}
"#,
    );
    write_file(&rules_dir.join("src/probes.rs"), PROBE_RULES_RS);
    point_rule_pack_at_local_polint(root);
}

const PROBE_RULES_RS: &str = r#"use polint::sdk::prelude::*;

fn present(ctx: &mut RuleCtx<'_>, view: &str, count: usize) {
    if count == 0 {
        return;
    }
    ctx.report(
        Diagnostic::info(
            ctx.rule_id(),
            "<workspace>",
            DiagnosticRange::point(1, 1),
            "capability present",
        )
        .with_evidence("view", view.to_string())
        .with_evidence("count", count.to_string()),
    );
}

#[polint::rule(id = "matrix/SourceFiles", description = "Probe SourceFiles", severity = "info")]
fn probe_source_files(ctx: &mut RuleCtx<'_>, files: SourceFiles<'_>) -> RuleResult {
    let count = files
        .iter()
        .filter(|file| !file.relative_path.is_empty())
        .count();
    present(ctx, "SourceFiles", count);
    Ok(())
}

#[polint::rule(id = "matrix/Packages", description = "Probe Packages", severity = "info")]
fn probe_packages(ctx: &mut RuleCtx<'_>, packages: Packages<'_>) -> RuleResult {
    let count = packages
        .iter()
        .filter(|package| !package.name.is_empty())
        .count();
    present(ctx, "Packages", count);
    Ok(())
}

#[polint::rule(id = "matrix/Functions", description = "Probe Functions", severity = "info")]
fn probe_functions(ctx: &mut RuleCtx<'_>, functions: Functions<'_>) -> RuleResult {
    let count = functions
        .iter()
        .filter(|function| !function.name.is_empty())
        .count();
    present(ctx, "Functions", count);
    Ok(())
}

#[polint::rule(id = "matrix/FileMetrics", description = "Probe FileMetrics", severity = "info")]
fn probe_file_metrics(ctx: &mut RuleCtx<'_>, metrics: FileMetrics<'_>) -> RuleResult {
    let count = metrics.iter().filter(|metric| metric.line_count > 0).count();
    present(ctx, "FileMetrics", count);
    Ok(())
}

#[polint::rule(
    id = "matrix/FunctionMetrics",
    description = "Probe FunctionMetrics",
    severity = "info"
)]
fn probe_function_metrics(ctx: &mut RuleCtx<'_>, metrics: FunctionMetrics<'_>) -> RuleResult {
    let count = metrics
        .iter()
        .filter(|metric| metric.line_count > 0 && !metric.name.is_empty())
        .count();
    present(ctx, "FunctionMetrics", count);
    Ok(())
}

#[polint::rule(
    id = "matrix/ComplexityMetrics",
    description = "Probe ComplexityMetrics",
    severity = "info"
)]
fn probe_complexity_metrics(ctx: &mut RuleCtx<'_>, metrics: ComplexityMetrics<'_>) -> RuleResult {
    let count = metrics
        .iter()
        .filter(|metric| metric.cyclomatic_complexity >= 1 && !metric.name.is_empty())
        .count();
    present(ctx, "ComplexityMetrics", count);
    Ok(())
}

#[polint::rule(id = "matrix/Imports", description = "Probe Imports", severity = "info")]
fn probe_imports(ctx: &mut RuleCtx<'_>, imports: Imports<'_>) -> RuleResult {
    let count = imports.iter().filter(|import| !import.path.is_empty()).count();
    present(ctx, "Imports", count);
    Ok(())
}

#[polint::rule(
    id = "matrix/ResolvedImports",
    description = "Probe ResolvedImports",
    severity = "info"
)]
fn probe_resolved_imports(ctx: &mut RuleCtx<'_>, imports: ResolvedImports<'_>) -> RuleResult {
    present(ctx, "ResolvedImports", imports.iter().count());
    Ok(())
}

#[polint::rule(
    id = "matrix/ModuleGraphFacts",
    description = "Probe ModuleGraphFacts",
    severity = "info"
)]
fn probe_module_graph(ctx: &mut RuleCtx<'_>, graph: ModuleGraphFacts<'_>) -> RuleResult {
    let count = graph.nodes().len();
    present(ctx, "ModuleGraphFacts", count);
    Ok(())
}

#[polint::rule(id = "matrix/Symbols", description = "Probe Symbols", severity = "info")]
fn probe_symbols(ctx: &mut RuleCtx<'_>, symbols: Symbols<'_>) -> RuleResult {
    let count = symbols.iter().filter(|symbol| !symbol.name.is_empty()).count();
    present(ctx, "Symbols", count);
    Ok(())
}

#[polint::rule(id = "matrix/References", description = "Probe References", severity = "info")]
fn probe_references(ctx: &mut RuleCtx<'_>, references: References<'_>) -> RuleResult {
    present(ctx, "References", references.iter().count());
    Ok(())
}

#[polint::rule(
    id = "matrix/BranchObligations",
    description = "Probe BranchObligations",
    severity = "info"
)]
fn probe_branches(ctx: &mut RuleCtx<'_>, branches: BranchObligations<'_>) -> RuleResult {
    present(ctx, "BranchObligations", branches.iter().count());
    Ok(())
}

#[polint::rule(id = "matrix/GoTests", description = "Probe GoTests", severity = "info")]
fn probe_go_tests(ctx: &mut RuleCtx<'_>, tests: GoTests<'_>) -> RuleResult {
    let count = tests.iter().filter(|test| !test.name.is_empty()).count();
    present(ctx, "GoTests", count);
    Ok(())
}

#[polint::rule(id = "matrix/TsComponents", description = "Probe TsComponents", severity = "info")]
fn probe_ts_components(ctx: &mut RuleCtx<'_>, components: TsComponents<'_>) -> RuleResult {
    let count = components
        .iter()
        .filter(|component| !component.name.is_empty())
        .count();
    present(ctx, "TsComponents", count);
    Ok(())
}

#[polint::rule(id = "matrix/TsClasses", description = "Probe TsClasses", severity = "info")]
fn probe_ts_classes(ctx: &mut RuleCtx<'_>, classes: TsClasses<'_>) -> RuleResult {
    let count = classes.iter().filter(|class| !class.name.is_empty()).count();
    present(ctx, "TsClasses", count);
    Ok(())
}

#[polint::rule(
    id = "matrix/StringLiterals",
    description = "Probe StringLiterals",
    severity = "info"
)]
fn probe_string_literals(ctx: &mut RuleCtx<'_>, literals: StringLiterals<'_>) -> RuleResult {
    present(ctx, "StringLiterals", literals.iter().count());
    Ok(())
}

#[polint::rule(
    id = "matrix/JsxAttributes",
    description = "Probe JsxAttributes",
    severity = "info"
)]
fn probe_jsx_attributes(ctx: &mut RuleCtx<'_>, attrs: JsxAttributes<'_>) -> RuleResult {
    let count = attrs.iter().filter(|attr| !attr.name.is_empty()).count();
    present(ctx, "JsxAttributes", count);
    Ok(())
}

#[polint::rule(id = "matrix/Events", description = "Probe Events", severity = "info")]
fn probe_events(ctx: &mut RuleCtx<'_>, events: Events<'_>) -> RuleResult {
    let count = events.matching(EventPattern::call("dangerous")).len();
    present(ctx, "Events", count);
    Ok(())
}

#[polint::rule(id = "matrix/Calls", description = "Probe Calls", severity = "info")]
fn probe_calls(ctx: &mut RuleCtx<'_>, calls: Calls<'_>) -> RuleResult {
    let mut query = ReachQuery::new(EventPattern::call("dangerous"));
    query.roots = vec![EventPattern::call("main")];
    query.max_depth = 8;
    query.max_paths = 8;
    let count = calls.forbidden_reachable(query).len();
    present(ctx, "Calls", count);
    Ok(())
}

#[polint::rule(id = "matrix/ControlFlow", description = "Probe ControlFlow", severity = "info")]
fn probe_control_flow(ctx: &mut RuleCtx<'_>, control: ControlFlow<'_>) -> RuleResult {
    let mut guard = GuardQuery::new(
        EventPattern::call("writeBalance"),
        GuardPattern::call_any(["authorize"]),
    );
    guard.max_paths = 8;
    let count = control.missing_guard(guard).len();
    present(ctx, "ControlFlow", count);
    Ok(())
}

#[polint::rule(id = "matrix/DataFlow", description = "Probe DataFlow", severity = "info")]
fn probe_data_flow(ctx: &mut RuleCtx<'_>, flow: DataFlow<'_>) -> RuleResult {
    let mut query = FlowQuery::new(
        SourcePattern::secret_like(["token"]),
        SinkPattern::logger(),
    );
    query.barriers = BarrierPattern::call_any(["redact"]);
    query.minimum_precision = PolicyPrecision::Heuristic;
    query.max_paths = 8;
    let count = flow.forbidden(query).len();
    present(ctx, "DataFlow", count);
    Ok(())
}

pub(crate) fn all_rules() -> Vec<Rule> {
    vec![
        probe_source_files(),
        probe_packages(),
        probe_functions(),
        probe_file_metrics(),
        probe_function_metrics(),
        probe_complexity_metrics(),
        probe_imports(),
        probe_resolved_imports(),
        probe_module_graph(),
        probe_symbols(),
        probe_references(),
        probe_branches(),
        probe_go_tests(),
        probe_ts_components(),
        probe_ts_classes(),
        probe_string_literals(),
        probe_jsx_attributes(),
        probe_events(),
        probe_calls(),
        probe_control_flow(),
        probe_data_flow(),
    ]
}
"#;

fn write_language_workspace(root: &Path, language: &str, fixture_name: &str) {
    let fixture = repo_root().join(FIXTURES_REL).join(fixture_name);
    assert!(
        fixture.is_dir(),
        "missing fixture directory {}",
        fixture.display()
    );

    let include = match language {
        "go" => r#"["**/*.go"]"#,
        "typescript" => r#"["**/*.ts", "**/*.tsx"]"#,
        other => panic!("unsupported matrix language `{other}`"),
    };
    // GoTests is syntax-extracted from `_test.go`. Keep include_tests=false so the
    // Go symbol sidecar does not load synthetic `.test` packages whose CompiledGoFiles
    // point into the go-build cache (absolute paths rejected by path validation).
    let language_section = match language {
        "go" => "\n[languages.go]\ninclude_tests = false\n",
        _ => "",
    };
    write_file(
        &root.join(".polint.toml"),
        &format!(
            r#"
[workspace]
include = {include}
exclude = []
{language_section}
[rules]
paths = [".polint/rules"]
"#
        ),
    );

    let src = root.join("src");
    copy_dir(&fixture, &src);
    if language == "go" {
        let go_mod_src = src.join("go.mod");
        if go_mod_src.is_file() {
            fs::rename(&go_mod_src, root.join("go.mod")).expect("move go.mod to root");
        }
        // Fixture util/ must live at the module root for example.com/.../util imports.
        let util_src = src.join("util");
        if util_src.is_dir() {
            copy_dir(&util_src, &root.join("util"));
            fs::remove_dir_all(&util_src).expect("remove nested util");
        }
    }

    write_probe_rule_pack(root);
}

fn present_views(report: &serde_json::Value) -> BTreeSet<String> {
    let mut views = BTreeSet::new();
    let diagnostics = report["diagnostics"]
        .as_array()
        .unwrap_or_else(|| panic!("expected diagnostics array: {report:#?}"));
    for diagnostic in diagnostics {
        let rule_id = diagnostic["rule_id"].as_str().unwrap_or_default();
        if !rule_id.starts_with("matrix/") {
            continue;
        }
        if diagnostic["message"] != PRESENT_MESSAGE {
            continue;
        }
        let Some(evidence) = diagnostic["evidence"].as_array() else {
            continue;
        };
        for item in evidence {
            if item["label"] == "view"
                && let Some(view) = item["value"].as_str()
            {
                views.insert(view.to_string());
            }
        }
    }
    views
}

fn run_language_matrix(language: &str, cells: &[MatrixCell]) {
    let supported: Vec<_> = cells
        .iter()
        .filter(|cell| cell.language == language && cell.status == "supported")
        .collect();
    if supported.is_empty() {
        return;
    }

    let fixture_name = supported[0]
        .fixture
        .as_deref()
        .unwrap_or_else(|| panic!("supported {language} cells require fixture"));
    for cell in &supported {
        assert_eq!(
            cell.fixture.as_deref(),
            Some(fixture_name),
            "all supported {language} cells must share one language fixture"
        );
    }

    let temp = tempfile::tempdir().expect("tempdir");
    write_language_workspace(temp.path(), language, fixture_name);

    let report = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );

    let found = present_views(&report);
    let mut missing = Vec::new();
    for cell in supported {
        if !found.contains(&cell.view) {
            missing.push(cell.view.clone());
        }
    }
    assert!(
        missing.is_empty(),
        "capability matrix missing non-empty well-formed data for {language}: {missing:?}\nfound={found:?}\nreport={report:#?}"
    );

    let capability_diags = diagnostics_for_rule(&report, "polint/capability");
    for cell in cells
        .iter()
        .filter(|cell| cell.language == language && cell.status == "supported")
    {
        let blocked = capability_diags.iter().any(|diagnostic| {
            diagnostic_has_evidence(diagnostic, "view", &cell.view)
                || diagnostic_has_evidence(diagnostic, "rule", &format!("matrix/{}", cell.view))
        });
        assert!(
            !blocked,
            "supported cell {}×{} must not be blocked by polint/capability: {report:#?}",
            cell.view, language
        );
    }
}

fn review_git_available() -> bool {
    StdCommand::new("git")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

fn review_run_git(dir: &Path, args: &[&str]) {
    let out = StdCommand::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap_or_else(|err| panic!("git {args:?}: {err}"));
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn review_git_init_identity(dir: &Path) {
    review_run_git(dir, &["init", "--quiet"]);
    review_run_git(dir, &["config", "user.email", "matrix@example.com"]);
    review_run_git(dir, &["config", "user.name", "Matrix"]);
    review_run_git(dir, &["config", "commit.gpgsign", "false"]);
}

fn review_git_commit_all(dir: &Path, message: &str) -> String {
    review_run_git(dir, &["add", "-A"]);
    review_run_git(dir, &["commit", "--quiet", "-m", message]);
    let out = StdCommand::new("git")
        .current_dir(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("git rev-parse");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write_review_probe_repo(root: &Path, language: &str) {
    let (include, source_path, source_body, changed_body) = match language {
        "go" => (
            r#"["**/*.go"]"#,
            "src/main.go",
            "package app\n\nfunc main() {}\n",
            "package app\n\nfunc main() {\n\t_ = 1\n}\n",
        ),
        "typescript" => (
            r#"["**/*.ts"]"#,
            "src/main.ts",
            "export function main(): void {}\n",
            "export function main(): void {\n  const x = 1;\n  void x;\n}\n",
        ),
        other => panic!("unsupported review language `{other}`"),
    };

    write_file(
        &root.join(".polint.toml"),
        &format!(
            r#"
[workspace]
include = {include}
exclude = []

[rules]
paths = [".polint/rules"]
"#
        ),
    );

    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#
        ),
    );
    uniquify_rule_pack_manifest(&root.join(".polint/rules/Cargo.toml"));
    write_file(
        &root.join(".polint/rules/src/main.rs"),
        r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "matrix/ChangedFiles",
    description = "Probe ChangedFiles",
    severity = "info",
    kind = "review"
)]
fn probe_changed_files(ctx: &mut RuleCtx<'_>, changes: ChangedFiles<'_>) -> RuleResult {
    let Some(changed) = changes.iter().next() else {
        return Ok(());
    };
    // Anchor on a changed path/line so default review diff-gating keeps the probe.
    let line = changed
        .lines()
        .first()
        .map(|(start, _)| *start)
        .unwrap_or(1)
        .max(1);
    ctx.report(
        Diagnostic::info(
            ctx.rule_id(),
            changed.path(),
            DiagnosticRange::point(line, 1),
            "capability present",
        )
        .with_evidence("view", "ChangedFiles")
        .with_evidence("count", changes.iter().count().to_string()),
    );
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![probe_changed_files()])
}
"#,
    );

    write_file(&root.join(source_path), source_body);
    review_git_init_identity(root);
    let base = review_git_commit_all(root, "base");
    write_file(&root.join(source_path), changed_body);
    review_git_commit_all(root, "change");

    let report = stdout_json(
        polint_cmd()
            .current_dir(root)
            .args(["review", &base, "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    let found = present_views(&report);
    assert!(
        found.contains("ChangedFiles"),
        "ChangedFiles×{language} must report non-empty changeset under review: {report:#?}"
    );
}

#[test]
fn capability_matrix_inventory_covers_full_prelude() {
    let matrix = load_matrix();
    let languages: BTreeSet<_> = matrix.languages.iter().cloned().collect();
    assert_eq!(
        languages,
        BTreeSet::from(["go".to_string(), "typescript".to_string()]),
        "matrix languages must be go and typescript"
    );

    let mut by_view: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for cell in &matrix.cell {
        assert!(
            languages.contains(&cell.language),
            "cell language `{}` not declared in languages",
            cell.language
        );
        assert!(
            matches!(
                cell.status.as_str(),
                "supported" | "unsupported" | "reserved" | "review_only"
            ),
            "unknown status `{}` for {}×{}",
            cell.status,
            cell.view,
            cell.language
        );
        if cell.status == "supported" || cell.status == "review_only" {
            assert!(
                cell.fixture
                    .as_ref()
                    .is_some_and(|fixture| !fixture.is_empty()),
                "supported/review_only cell {}×{} requires fixture",
                cell.view,
                cell.language
            );
        }
        by_view
            .entry(cell.view.clone())
            .or_default()
            .insert(cell.language.clone());
    }

    let expected: BTreeSet<_> = PRELUDE_FACT_VIEWS
        .iter()
        .map(|view| (*view).to_string())
        .collect();
    let declared: BTreeSet<_> = by_view.keys().cloned().collect();
    let missing: Vec<_> = expected.difference(&declared).cloned().collect();
    let extra: Vec<_> = declared.difference(&expected).cloned().collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "prelude fact-view inventory mismatch\n  missing from matrix: {missing:?}\n  extra in matrix: {extra:?}"
    );

    for view in PRELUDE_FACT_VIEWS {
        let langs = by_view
            .get(*view)
            .unwrap_or_else(|| panic!("missing view {view}"));
        assert_eq!(
            langs, &languages,
            "view `{view}` must declare a cell for every matrix language"
        );
    }
}

#[test]
fn capability_matrix_go_supported_views_are_non_empty() {
    let matrix = load_matrix();
    run_language_matrix("go", &matrix.cell);
}

#[test]
fn capability_matrix_typescript_supported_views_are_non_empty() {
    let matrix = load_matrix();
    run_language_matrix("typescript", &matrix.cell);
}

#[test]
fn capability_matrix_changed_files_review_only() {
    if !review_git_available() {
        eprintln!("skipping ChangedFiles matrix cell; git not on PATH");
        return;
    }
    let matrix = load_matrix();
    let review_cells: Vec<_> = matrix
        .cell
        .iter()
        .filter(|cell| cell.view == "ChangedFiles" && cell.status == "review_only")
        .collect();
    assert_eq!(
        review_cells.len(),
        2,
        "ChangedFiles must be review_only for both languages"
    );

    for cell in review_cells {
        let temp = tempfile::tempdir().expect("tempdir");
        write_review_probe_repo(temp.path(), &cell.language);
    }
}
