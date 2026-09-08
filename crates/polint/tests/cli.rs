mod common;

use assert_cmd::assert::Assert;
use common::*;
use predicates::prelude::*;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn point_generated_rule_pack_at_local_polint(root: &Path) {
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

fn write_phase41_rule_pack(root: &Path, main_rs: &str, rule_rs: &str) {
    let rules_dir = root.join(".polint/rules");
    fs::create_dir_all(rules_dir.join("src")).unwrap();
    write_file(&rules_dir.join("src/main.rs"), main_rs);
    write_file(&rules_dir.join("src/rule.rs"), rule_rs);
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
    point_generated_rule_pack_at_local_polint(root);
}

fn path_str_ends_with(path: &str, suffix: &str) -> bool {
    path.replace('\\', "/").ends_with(suffix)
}

fn assert_schema_object_is_closed(value: &serde_json::Value, label: &str) {
    assert_eq!(
        value.get("additionalProperties"),
        Some(&serde_json::Value::Bool(false)),
        "{label} schema object must reject unknown properties"
    );
}

#[test]
fn top_level_help_only_lists_supported_public_commands() {
    let help = polint_help(&["--help"]);

    for command in [
        "init",
        "add-skill",
        "new-rule",
        "baseline",
        "cache",
        "inspect",
        "facts",
        "unknowns",
        "explain",
        "test",
        "check",
        "review",
        "ignores",
    ] {
        assert!(help.contains(command), "help should list {command}: {help}");
    }
    for command in ["test-rules", "profile-rules", "graph"] {
        assert!(
            !help.contains(command),
            "help should not expose internal command {command}: {help}"
        );
    }
    assert!(
        help.contains("polint check --format ai-friendly --fail-on none"),
        "top-level help should guide AI agents to compact output: {help}"
    );
}

#[test]
fn facts_list_json_is_stable_and_public_only() {
    let run = || {
        stdout_string(
            polint_cmd()
                .args(["facts", "list", "--format", "json"])
                .assert()
                .success(),
        )
    };
    let first = run();
    let second = run();
    assert_eq!(first, second);

    let value: serde_json::Value = serde_json::from_str(&first)
        .unwrap_or_else(|error| panic!("facts list stdout was not JSON: {error}\n{first}"));
    assert_eq!(value["version"], 1);
    assert_eq!(value["tool"]["name"], "polint");
    let views = value["views"].as_array().expect("views should be an array");
    assert!(views.iter().any(|view| {
        view["capability"] == "resolved_imports"
            && view["stability"] == "stable"
            && view["unknowns"] == true
    }));
    assert!(views.iter().any(|view| {
        view["capability"] == "file_metrics"
            && view["stability"] == "stable"
            && view["sampling"] == true
            && view["unknowns"] == false
    }));
    let view_for = |capability: &str| {
        views
            .iter()
            .find(|view| view["capability"] == capability)
            .unwrap_or_else(|| panic!("missing facts list view for {capability}: {views:#?}"))
    };
    let stability_for = |capability: &str| {
        view_for(capability)["stability"]
            .as_str()
            .unwrap_or_else(|| panic!("missing stability for {capability}: {views:#?}"))
            .to_string()
    };
    for capability in ["events", "calls", "control_flow", "dataflow"] {
        assert_eq!(stability_for(capability), "preview");
        assert_eq!(view_for(capability)["unknowns"], true);
    }
    for capability in ["cfg", "call_graph"] {
        assert_eq!(stability_for(capability), "reserved");
    }

    for marker in [
        "polint.data_flow",
        "polint.evidence",
        "polint.refined_calls",
        "analysis_kernel",
        "output_hash",
        "/Users/",
    ] {
        assert!(
            !first.contains(marker),
            "facts list JSON leaked private marker `{marker}`:\n{first}"
        );
    }
}

#[test]
fn facts_sample_requires_or_applies_bounded_limit() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []
"#,
    );
    write_file(
        &temp.path().join("src/app.ts"),
        "export function answer(): number { return 42; }\n",
    );

    let value = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args([
                "facts",
                "sample",
                "--cap",
                "file_metrics",
                "--limit",
                "1",
                "--format",
                "json",
            ])
            .assert()
            .success(),
    );

    assert_eq!(value["version"], 1);
    assert_eq!(value["capability"], "file_metrics");
    assert_eq!(value["limit"], 1);
    assert_eq!(value["sampled"], 1);
    assert_eq!(value["rows"][0]["file"], "src/app.ts");

    let unsupported = output_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["facts", "sample", "--cap", "dataflow", "--format", "json"])
            .assert()
            .failure(),
    );
    assert!(
        unsupported.contains("does not support sampling yet"),
        "{unsupported}"
    );
    assert!(
        unsupported.contains("docs/facts/data-flow.md"),
        "{unsupported}"
    );
}

#[test]
fn unknowns_json_reports_public_setup_and_resolution_gaps() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []
"#,
    );
    write_file(
        &temp.path().join("src/app.ts"),
        "import missing from './missing';\nexport const answer = missing;\n",
    );

    let value = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["unknowns", "--cap", "resolved_imports", "--format", "json"])
            .assert()
            .success(),
    );
    assert_eq!(value["version"], 1);
    assert_eq!(value["capability"], "resolved_imports");
    assert_eq!(value["rows"][0]["reason"], "not_found");
    assert_eq!(value["rows"][0]["precision"], "none");

    let unsupported_stable = output_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["unknowns", "--cap", "file_metrics", "--format", "json"])
            .assert()
            .code(2),
    );
    assert!(
        unsupported_stable.contains("\"status\": \"unsupported\""),
        "{unsupported_stable}"
    );
    assert!(
        unsupported_stable.contains("docs/facts/metrics.md"),
        "{unsupported_stable}"
    );

    let dataflow = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["unknowns", "--cap", "dataflow", "--format", "json"])
            .assert()
            .success(),
    );
    assert_eq!(dataflow["capability"], "dataflow");
    assert_eq!(
        dataflow["rows"].as_array().map(Vec::len),
        Some(0),
        "{dataflow:#?}"
    );
}

#[test]
fn inspect_unknowns_json_reports_consolidated_and_cap_filtered_rows() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []
"#,
    );
    write_file(
        &temp.path().join("src/app.ts"),
        "import missing from './missing';\nexport const answer = missing;\n",
    );

    let consolidated = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["inspect", "unknowns", "--format", "json"])
            .assert()
            .success(),
    );
    assert_eq!(consolidated["version"], 1);
    assert_eq!(consolidated["capability"], "all");
    let rows = consolidated["rows"].as_array().expect("unknown rows");
    let import_row = rows
        .iter()
        .find(|row| row["reason"] == "not_found")
        .unwrap_or_else(|| panic!("missing import-resolution unknown row: {consolidated:#?}"));
    assert_eq!(import_row["category"], "missing_fact");
    assert!(import_row.get("provider").is_none());
    assert!(import_row.get("family").is_none());
    assert!(import_row.get("source_stable_key").is_none());

    let filtered = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args([
                "inspect",
                "unknowns",
                "--cap",
                "resolved_imports",
                "--format",
                "json",
            ])
            .assert()
            .success(),
    );
    assert_eq!(filtered["capability"], "resolved_imports");
    assert_eq!(filtered["rows"][0]["category"], "missing_fact");

    let dataflow = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args([
                "inspect", "unknowns", "--cap", "dataflow", "--format", "json",
            ])
            .assert()
            .success(),
    );
    assert_eq!(dataflow["capability"], "dataflow");
    assert_eq!(
        dataflow["rows"].as_array().map(Vec::len),
        Some(0),
        "{dataflow:#?}"
    );
}

#[test]
fn inspect_unknowns_default_requests_preview_policy_unknowns() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []
"#,
    );
    write_file(
        &temp.path().join("src/app.ts"),
        r#"export function handler(cb: () => void) {
  cb();
}
"#,
    );

    let value = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["inspect", "unknowns", "--format", "json"])
            .assert()
            .success(),
    );
    let rows = value["rows"].as_array().expect("unknown rows");

    assert!(
        rows.iter().any(|row| {
            row["docs_path"] == "docs/facts/data-flow.md"
                && row["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.contains("unresolved_calls"))
        }),
        "unfiltered inspect unknowns should request preview policy/dataflow facts: {value:#?}"
    );
}

#[test]
fn unknowns_json_does_not_report_external_imports_as_unknowns() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []
"#,
    );
    write_file(
        &temp.path().join("src/app.ts"),
        "import React from 'react';\nexport const element = React.createElement('div');\n",
    );

    let value = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["unknowns", "--cap", "resolved_imports", "--format", "json"])
            .assert()
            .success(),
    );

    assert_eq!(value["capability"], "resolved_imports");
    assert!(value["rows"].as_array().expect("rows").is_empty());
}

#[test]
fn unknowns_json_schema_rejects_internal_extra_fields() {
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/schemas/polint-unknowns-v1.json"
    ))
    .expect("unknowns schema parses");
    assert_schema_object_is_closed(&schema, "root");

    let tool = schema
        .pointer("/properties/tool")
        .expect("schema has tool object");
    assert_schema_object_is_closed(tool, "tool");

    let row = schema
        .pointer("/properties/rows/items")
        .expect("schema has row object");
    assert_schema_object_is_closed(row, "row");
    let row_properties = row
        .get("properties")
        .and_then(serde_json::Value::as_object)
        .expect("row has properties");
    for internal in ["provider", "family", "source_stable_key"] {
        assert!(
            !row_properties.contains_key(internal),
            "unknowns row schema must not expose internal field `{internal}`"
        );
    }

    let span = schema
        .pointer("/properties/rows/items/properties/span")
        .expect("schema has span object");
    assert_schema_object_is_closed(span, "span");
}

#[test]
fn inspect_unknowns_json_reports_go_provider_diagnostics() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []
"#,
    );
    write_file(
        &temp.path().join("go.mod"),
        "module example.com/app\n\ngo 1.24\n",
    );
    write_file(
        &temp.path().join("src/main.go"),
        "package main\n\nfunc main() {}\n",
    );
    let missing_frontend = temp.path().join("missing-polint-go-frontend");

    let value = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .env("POLINT_GO_FRONTEND", &missing_frontend)
            .args(["inspect", "unknowns", "--format", "json"])
            .assert()
            .success(),
    );
    let rows = value["rows"].as_array().expect("unknown rows");

    assert!(rows.iter().any(|row| {
        row["category"] == "go_packages_load_failed"
            && row["suggested_artifact"] == "go_setup"
            && row.get("provider").is_none()
            && row.get("family").is_none()
            && row.get("source_stable_key").is_none()
    }));
    let rendered = serde_json::to_string(&value).unwrap();
    assert!(!rendered.contains("polint.go.semantic"));
    assert!(!rendered.contains("GoSemanticDiagnostic"));
    assert!(!rendered.contains("source_stable_key"));
}

#[test]
fn explain_json_reports_rule_capability_plan() {
    let temp = fixture_workspace();
    point_generated_rule_pack_at_local_polint(temp.path());

    let first = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args([
                "explain",
                "--rule",
                "custom/no-raw-colors",
                "--format",
                "json",
            ])
            .assert()
            .success(),
    );
    let second = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args([
                "explain",
                "--rule",
                "custom/no-raw-colors",
                "--format",
                "json",
            ])
            .assert()
            .success(),
    );
    assert_eq!(first, second);
    let value: serde_json::Value = serde_json::from_str(&first)
        .unwrap_or_else(|error| panic!("explain stdout was not JSON: {error}\n{first}"));
    assert_eq!(value["version"], 1);
    assert_eq!(value["scope"], "custom/no-raw-colors");
    assert_eq!(value["rules"][0]["rule_id"], "custom/no-raw-colors");
    assert!(
        value["public_capabilities"]
            .as_array()
            .is_some_and(|rows| { rows.iter().any(|row| row["capability"] == "references") })
    );
}

#[test]
fn phase41_public_promotion_baseline_no_leak() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []
"#,
    );
    write_file(
        &temp.path().join("src/app.ts"),
        "export function answer(): number { return 42; }\n",
    );

    let mut surface = String::new();
    surface.push_str(&output_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    ));
    surface.push_str(&polint_help(&["--help"]));
    surface.push_str(&source_tree_text(
        &repo_root().join("crates/polint/src/sdk"),
    ));
    surface.push_str(&source_tree_text(
        &repo_root().join("crates/polint/src/runner"),
    ));
    surface
        .push_str(&fs::read_to_string(repo_root().join("README.md")).expect("README should exist"));

    for marker in [
        "polint.data_flow",
        "polint.evidence",
        "polint.refined_calls",
        "polint.direct_calls",
        "polint.summary",
        "polint.eval",
        "EvidenceStore",
        "DemandQueryEngine",
        "RefinedCall",
        "TypeValueAlias",
        "BenchmarkAdapter",
    ] {
        assert!(
            !surface.contains(marker),
            "stable public surface leaked private implementation marker `{marker}`"
        );
    }

    let data_flow = fs::read_to_string(repo_root().join("docs/facts/data-flow.md")).unwrap();
    let evidence = fs::read_to_string(repo_root().join("docs/facts/evidence.md")).unwrap();
    assert!(data_flow.contains("v1.4 preview SDK view"));
    assert!(data_flow.contains("raw data-flow graph APIs remain"));
    assert!(evidence.contains("not a public SDK fact view"));
}

#[test]
fn generated_skills_describe_phase41_public_surface() {
    for path in [
        ".claude/skills/polint/SKILL.md",
        ".agents/skills/polint/SKILL.md",
        "crates/polint/src/cli/skill.rs",
    ] {
        let source = fs::read_to_string(repo_root().join(path)).unwrap();
        for required in [
            "use polint::sdk::prelude::*",
            "polint inspect rule --format json",
            "polint test --format json",
            "polint facts list --format json",
            "polint unknowns --cap references --format json",
            "polint explain --rule",
        ] {
            assert!(
                source.contains(required),
                "{path} should mention `{required}`"
            );
        }
        assert!(
            !source.contains("use polint::core"),
            "{path} should not promote direct core imports"
        );
        assert!(
            !source.contains("analysis_kernel::"),
            "{path} should not promote analysis kernel imports"
        );
        assert!(source.contains("DataFlow<'_>"));
        assert!(source.contains("reserved"));
    }
}

#[test]
fn phase41_public_json_contracts_are_stable() {
    let temp = fixture_workspace();
    point_generated_rule_pack_at_local_polint(temp.path());

    let commands: Vec<Vec<&str>> = vec![
        vec!["inspect", "rule", "--format", "json"],
        vec!["test", "--format", "json"],
        vec!["facts", "list", "--format", "json"],
        vec![
            "facts",
            "sample",
            "--cap",
            "file_metrics",
            "--limit",
            "2",
            "--format",
            "json",
        ],
        vec!["unknowns", "--cap", "references", "--format", "json"],
        vec![
            "explain",
            "--rule",
            "custom/no-raw-colors",
            "--format",
            "json",
        ],
    ];

    for command in commands {
        let first = stdout_string(
            polint_cmd()
                .current_dir(temp.path())
                .args(&command)
                .assert()
                .success(),
        );
        let second = stdout_string(
            polint_cmd()
                .current_dir(temp.path())
                .args(&command)
                .assert()
                .success(),
        );
        assert_eq!(first, second, "{command:?} should be deterministic");
        let value: serde_json::Value = serde_json::from_str(&first)
            .unwrap_or_else(|error| panic!("{command:?} stdout was not JSON: {error}\n{first}"));
        assert_eq!(value["version"], 1, "{command:?}");
        assert_eq!(value["tool"]["name"], "polint", "{command:?}");
        assert!(value["schema"].as_str().is_some(), "{command:?}");

        for marker in [
            "polint.data_flow",
            "polint.evidence",
            "polint.refined_calls",
            "analysis_kernel",
            "output_hash",
            "/Users/",
        ] {
            assert!(
                !first.contains(marker),
                "{command:?} JSON leaked private marker `{marker}`:\n{first}"
            );
        }
    }
}

#[test]
fn inspect_rule_json_matches_schema_v1() {
    let temp = fixture_workspace();
    point_generated_rule_pack_at_local_polint(temp.path());

    let value = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["inspect", "rule", "--format", "json"])
            .assert()
            .success(),
    );
    assert_eq!(value["version"], 1);
    assert_eq!(value["tool"]["name"], "polint");
    assert_eq!(
        value["schema"],
        "https://raw.githubusercontent.com/oaiz-io/polint/main/docs/schemas/polint-rule-inspect-v1.json"
    );
}

#[test]
fn polint_test_json_matches_schema_v1() {
    let temp = fixture_workspace();
    point_generated_rule_pack_at_local_polint(temp.path());

    let value = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["test", "--format", "json"])
            .assert()
            .success(),
    );
    assert_eq!(value["version"], 1);
    assert_eq!(value["tool"]["name"], "polint");
    assert_eq!(
        value["schema"],
        "https://raw.githubusercontent.com/oaiz-io/polint/main/docs/schemas/polint-test-report-v1.json"
    );
}

#[test]
fn new_rule_generates_clean_and_violating_agent_fixtures() {
    let temp = tempfile::tempdir().unwrap();
    polint_cmd()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    polint_cmd()
        .current_dir(temp.path())
        .args(["new-rule", "ts", "no-raw-colors"])
        .assert()
        .success();

    assert!(
        temp.path()
            .join(".polint/tests/rules/no_raw_colors/clean/polint-test.toml")
            .is_file()
    );
    assert!(
        temp.path()
            .join(".polint/tests/rules/no_raw_colors/violating/polint-test.toml")
            .is_file()
    );
}

#[test]
fn new_rule_review_scaffolds_review_kind_with_changed_files() {
    let temp = tempfile::tempdir().unwrap();
    polint_cmd()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    polint_cmd()
        .current_dir(temp.path())
        .args(["new-rule", "generic", "db-migration-review", "--review"])
        .assert()
        .success();

    let module = fs::read_to_string(temp.path().join(".polint/rules/src/db_migration_review.rs"))
        .expect("review rule module should exist");
    assert!(
        module.contains("kind = \"review\""),
        "review scaffold must set kind = review: {module}"
    );
    assert!(
        module.contains("ChangedFiles<'_>"),
        "review scaffold must request the ChangedFiles fact view: {module}"
    );
    assert!(
        module.contains("changed.lines().first()"),
        "review scaffold must anchor diagnostics on a changed line: {module}"
    );

    let main_rs = fs::read_to_string(temp.path().join(".polint/rules/src/main.rs"))
        .expect("main.rs should exist");
    assert!(
        main_rs.contains("db_migration_review::db_migration_review()")
            && main_rs.contains("polint::runner::run_cli"),
        "main.rs should register the review rule via run_cli: {main_rs}"
    );

    // Review rules are exercised via `polint review` against a diff, so no
    // static `polint check` fixtures are generated for them.
    assert!(
        !temp
            .path()
            .join(".polint/tests/rules/db_migration_review")
            .exists(),
        "review scaffold should not generate diff-independent check fixtures"
    );
}

#[test]
fn phase41_metric_query_helpers_external_rule() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_phase41_rule_pack(
        temp.path(),
        r#"use std::process::ExitCode;
use polint::runner;
mod rule;
fn main() -> ExitCode { runner::run_cli(vec![rule::rule()]) }
"#,
        r#"use polint::sdk::prelude::*;

#[polint::rule(id = "local/metric-helper", description = "Metric helper", severity = "warn")]
pub(crate) fn rule(ctx: &mut RuleCtx<'_>, files: FileMetrics<'_>, functions: FunctionMetrics<'_>, complexity: ComplexityMetrics<'_>) -> RuleResult {
    if let Some(file) = files.over_line_count(0).next() {
        ctx.warn(&Span::point(file.file, 1, 1), "file metric helper matched");
    }
    let _ = functions.over_byte_count(1).count();
    let _ = complexity.over_complexity(0).count();
    Ok(())
}
"#,
    );
    write_file(
        &temp.path().join("src/app.ts"),
        "export function answer(): number { return 42; }\n",
    );

    let json = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    assert_eq!(diagnostics_for_rule(&json, "local/metric-helper").len(), 1);
}

#[test]
fn phase41_relationship_query_helpers_external_rule() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_phase41_rule_pack(
        temp.path(),
        r#"use std::process::ExitCode;
use polint::runner;
mod rule;
fn main() -> ExitCode { runner::run_cli(vec![rule::rule()]) }
"#,
        r#"use polint::sdk::prelude::*;

#[polint::rule(id = "local/relationship-helper", description = "Relationship helper", severity = "warn")]
pub(crate) fn rule(ctx: &mut RuleCtx<'_>, imports: ResolvedImports<'_>, graph: ModuleGraphFacts<'_>) -> RuleResult {
    for unresolved in imports.unresolved() {
        let _ = imports.by_specifier("./missing").count();
        let _ = imports.dynamic().count();
        let _ = imports.unsupported().count();
        let _ = graph.edges_from_file(unresolved.from_file).count();
        ctx.warn(&Span::point(unresolved.from_file, 1, 1), "relationship helper matched unresolved import");
    }
    Ok(())
}
"#,
    );
    write_file(
        &temp.path().join("src/app.ts"),
        "import missing from './missing';\nexport const answer = missing;\n",
    );

    let json = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    assert_eq!(
        diagnostics_for_rule(&json, "local/relationship-helper").len(),
        1
    );
}

#[test]
fn phase41_symbol_reference_query_helpers_external_rule() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_phase41_rule_pack(
        temp.path(),
        r#"use std::process::ExitCode;
use polint::runner;
mod rule;
fn main() -> ExitCode { runner::run_cli(vec![rule::rule()]) }
"#,
        r#"use polint::sdk::prelude::*;

#[polint::rule(id = "local/symbol-helper", description = "Symbol helper", severity = "warn")]
pub(crate) fn rule(ctx: &mut RuleCtx<'_>, symbols: Symbols<'_>, references: References<'_>) -> RuleResult {
    let exported = symbols.exported_by_name("forbidden").collect::<Vec<_>>();
    let _ = symbols.by_kind(SymbolKind::Constant).count();
    for reference in references.to_any(exported.iter().copied()) {
        if reference.status == SymbolResolutionStatus::Resolved && !matches!(reference.precision, SymbolPrecision::Unsupported | SymbolPrecision::SetupMissing) {
            let Some(span) = reference.primary_span.as_ref() else { continue; };
            ctx.warn(span, "symbol helper matched forbidden reference");
        }
    }
    let _ = references.resolved().count();
    let _ = references.by_name("forbidden").count();
    let _ = references.unresolved_by_name("missing").count();
    Ok(())
}
"#,
    );
    write_file(
        &temp.path().join("src/app.ts"),
        "export const forbidden = 1;\nexport const useIt = forbidden;\n",
    );

    let json = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    assert_eq!(diagnostics_for_rule(&json, "local/symbol-helper").len(), 1);
}

#[test]
fn phase55_preview_rule_syntax_compiles_and_executes_supported_views() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_phase41_rule_pack(
        temp.path(),
        r#"use std::process::ExitCode;
use polint::runner;
mod rule;
fn main() -> ExitCode { runner::run_cli(vec![rule::rule()]) }
"#,
        r#"use polint::sdk::prelude::*;

#[polint::rule(id = "local/policy-preview", description = "Policy preview syntax", severity = "warn")]
pub(crate) fn rule(
    ctx: &mut RuleCtx<'_>,
    events: Events<'_>,
    calls: Calls<'_>,
    control: ControlFlow<'_>,
    flow: DataFlow<'_>,
) -> RuleResult {
    let mut flow_query = FlowQuery::new(
        SourcePattern::secret_like(["token", "password", "apiKey"]),
        SinkPattern::logger(),
    );
    flow_query.barriers = BarrierPattern::call_any(["redact", "mask_secret"]);
    flow_query.minimum_precision = PolicyPrecision::Heuristic;
    flow_query.max_paths = 20;

    for violation in flow.forbidden(flow_query) {
        ctx.report(violation.diagnostic(
            ctx.rule_id(),
            "secret-like value reaches logging without redaction",
        ));
    }

    let _ = events.matching(EventPattern::call("handler"));
    let _ = calls.forbidden_reachable(ReachQuery::new(EventPattern::call("dangerous_exec")));
    let _ = control.missing_guard(GuardQuery::new(
        EventPattern::write_field("account.balance"),
        GuardPattern::call_any(["authorize", "require_admin"]),
    ));
    let mut cleanup_query = LifecycleQuery::new(
        EventPattern::call("begin_transaction"),
        EventPattern::call("rollback"),
    );
    cleanup_query.minimum_precision = PolicyPrecision::SetupAware;
    let _ = control.missing_cleanup(cleanup_query);

    Ok(())
}
"#,
    );
    write_file(
        &temp.path().join("src/app.ts"),
        r#"export function handler(token: string) {
  console.log(token);
}
"#,
    );

    let json = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );

    let diagnostics = diagnostics_for_rule(&json, "local/policy-preview");
    assert_eq!(
        diagnostics.len(),
        1,
        "preview rule should execute and report the secret-log data-flow policy: {json:#?}"
    );
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic["message"] == "secret-like value reaches logging without redaction"
                && diagnostic_has_evidence(diagnostic, "policy", "forbidden_flow")
                && diagnostic_has_evidence(diagnostic, "sink", "log")
                && diagnostic_has_evidence(diagnostic, "barrier_status", "uncovered")
        }),
        "missing data-flow diagnostic evidence: {json:#?}"
    );
    let capability_diagnostics = diagnostics_for_rule(&json, "polint/capability");
    for capability in ["events", "calls", "control_flow", "dataflow"] {
        assert!(
            capability_diagnostics.iter().all(|diagnostic| {
                !diagnostic_has_evidence(diagnostic, "rule", "local/policy-preview")
                    || !diagnostic_has_evidence(diagnostic, "capability", capability)
            }),
            "events/calls/control_flow should be supported by the analysis engine: {json:#?}"
        );
    }

    let manifest = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["inspect", "rule", "--format", "json"])
            .assert()
            .success(),
    );
    let rule = manifest["rules"]
        .as_array()
        .and_then(|rules| {
            rules
                .iter()
                .find(|rule| rule["rule_id"] == "local/policy-preview")
        })
        .unwrap_or_else(|| panic!("missing preview rule manifest row: {manifest:#?}"));
    let capabilities = rule["capabilities"]
        .as_array()
        .unwrap_or_else(|| panic!("rule capabilities should be an array: {rule:#?}"));
    let capability_names = capabilities
        .iter()
        .map(|capability| capability["name"].as_str().unwrap_or_default())
        .collect::<BTreeSet<_>>();
    for capability in ["events", "calls", "control_flow", "dataflow"] {
        assert!(
            capability_names.contains(capability),
            "preview rule manifest should include {capability}: {rule:#?}"
        );
    }
    let fact_views = rule["fact_views"]
        .as_array()
        .unwrap_or_else(|| panic!("rule fact_views should be an array: {rule:#?}"));
    for (view_type, capability) in [
        ("Events", "events"),
        ("Calls", "calls"),
        ("ControlFlow", "control_flow"),
        ("DataFlow", "dataflow"),
    ] {
        assert!(
            fact_views
                .iter()
                .any(|view| view["view_type"] == view_type && view["capability"] == capability),
            "preview fact view metadata should include {view_type}/{capability}: {rule:#?}"
        );
    }
}

#[test]
fn phase56_events_and_calls_rule_reports_json() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_file(
        &temp.path().join("go.mod"),
        "module example.com/policyquery\n\ngo 1.22\n",
    );
    write_phase41_rule_pack(
        temp.path(),
        r#"use std::process::ExitCode;
use polint::runner;
mod rule;
fn main() -> ExitCode { runner::run_cli(vec![rule::rule()]) }
"#,
        r#"use polint::sdk::prelude::*;

#[polint::rule(id = "local/events-calls", description = "Events and calls", severity = "error")]
pub(crate) fn rule(
    ctx: &mut RuleCtx<'_>,
    events: Events<'_>,
    calls: Calls<'_>,
) -> RuleResult {
    for violation in events.matching(EventPattern::call("dangerous")) {
        ctx.report(violation.diagnostic(
            ctx.rule_id(),
            "dangerous call event matched",
        ));
    }

    let mut query = ReachQuery::new(EventPattern::call("dangerous"));
    query.roots = vec![EventPattern::call("main")];
    query.max_depth = 4;
    query.max_paths = 4;
    query.minimum_precision = PolicyPrecision::Conservative;
    query.minimum_confidence = PolicyConfidence::Low;

    for violation in calls.forbidden_reachable(query) {
        ctx.report(violation.diagnostic(
            ctx.rule_id(),
            "dangerous call is reachable from main",
        ));
    }

    Ok(())
}
"#,
    );
    write_file(
        &temp.path().join("src/main.go"),
        r#"package main

func main() {
	handler()
}

func handler() {
	dangerous()
}

func dangerous() {}
"#,
    );

    let json = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );

    let diagnostics = diagnostics_for_rule(&json, "local/events-calls");
    assert_eq!(
        diagnostics.len(),
        2,
        "expected event and reach diagnostics: {json:#?}"
    );
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic["message"] == "dangerous call event matched"
                && diagnostic_has_evidence(
                    diagnostic,
                    "target",
                    "example.com/policyquery/src.dangerous",
                )
                && diagnostic_has_evidence(diagnostic, "policy_precision", "setup_aware")
        }),
        "missing event diagnostic evidence: {json:#?}"
    );
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic["message"] == "dangerous call is reachable from main"
                && diagnostic_has_evidence(diagnostic, "root", "main")
                && diagnostic_has_evidence(
                    diagnostic,
                    "target",
                    "example.com/policyquery/src.dangerous",
                )
                && diagnostic_has_evidence(diagnostic, "confidence", "high")
        }),
        "missing reachable-call diagnostic evidence: {json:#?}"
    );
    assert!(
        diagnostics_for_rule(&json, "polint/capability")
            .iter()
            .all(|diagnostic| {
                !diagnostic_has_evidence(diagnostic, "capability", "events")
                    && !diagnostic_has_evidence(diagnostic, "capability", "calls")
            }),
        "events/calls should execute without capability diagnostics: {json:#?}"
    );
}

#[test]
fn phase57_control_flow_rule_reports_json() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_file(
        &temp.path().join("go.mod"),
        "module example.com/controlflowquery\n\ngo 1.22\n",
    );
    write_phase41_rule_pack(
        temp.path(),
        r#"use std::process::ExitCode;
use polint::runner;
mod rule;
fn main() -> ExitCode { runner::run_cli(vec![rule::rule()]) }
"#,
        r#"use polint::sdk::prelude::*;

#[polint::rule(id = "local/control-flow", description = "Control-flow", severity = "error")]
pub(crate) fn rule(ctx: &mut RuleCtx<'_>, control: ControlFlow<'_>) -> RuleResult {
    let mut guard = GuardQuery::new(
        EventPattern::call("dangerous"),
        GuardPattern::call_any(["authorize"]),
    );
    guard.max_paths = 4;

    for violation in control.missing_guard(guard) {
        ctx.report(violation.diagnostic(
            ctx.rule_id(),
            "dangerous call requires authorization first",
        ));
    }

    let mut cleanup = LifecycleQuery::new(
        EventPattern::call("Begin"),
        EventPattern::call("Rollback"),
    );
    cleanup.max_paths = 4;

    for violation in control.missing_cleanup(cleanup) {
        ctx.report(violation.diagnostic(
            ctx.rule_id(),
            "transaction begin requires cleanup",
        ));
    }

    Ok(())
}
"#,
    );
    write_file(
        &temp.path().join("src/main.go"),
        r#"package main

func main() {
	handler()
}

func handler() {
	dangerous()
	authorize()
	tx := Begin()
	_ = tx
}

type Tx struct{}

func dangerous() {}
func authorize() {}
func Begin() *Tx { return &Tx{} }
func Rollback() {}
"#,
    );

    let json = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );

    let diagnostics = diagnostics_for_rule(&json, "local/control-flow");
    assert_eq!(
        diagnostics.len(),
        2,
        "expected guard and cleanup diagnostics: {json:#?}"
    );
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic["message"] == "dangerous call requires authorization first"
                && diagnostic_has_evidence(diagnostic, "policy", "missing_guard")
                && diagnostic_has_evidence(diagnostic, "required_guard", "authorize")
                && diagnostic_has_evidence(diagnostic, "control_scope", "same_function")
                && diagnostic_has_evidence(diagnostic, "policy_precision", "conservative")
        }),
        "missing guard diagnostic evidence: {json:#?}"
    );
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic["message"] == "transaction begin requires cleanup"
                && diagnostic_has_evidence(diagnostic, "policy", "missing_cleanup")
                && diagnostic_has_evidence(diagnostic, "required_cleanup", "Rollback")
                && diagnostic_has_evidence(diagnostic, "control_scope", "same_function")
        }),
        "missing cleanup diagnostic evidence: {json:#?}"
    );
    assert!(
        diagnostics_for_rule(&json, "polint/capability")
            .iter()
            .all(|diagnostic| {
                !diagnostic_has_evidence(diagnostic, "capability", "control_flow")
            }),
        "control_flow should execute without capability diagnostics: {json:#?}"
    );
}

#[test]
fn phase58_data_flow_rule_reports_json() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_phase41_rule_pack(
        temp.path(),
        r#"use std::process::ExitCode;
use polint::runner;
mod rule;
fn main() -> ExitCode { runner::run_cli(vec![rule::rule()]) }
"#,
        r#"use polint::sdk::prelude::*;

#[polint::rule(id = "local/data-flow", description = "Data-flow", severity = "error")]
pub(crate) fn rule(ctx: &mut RuleCtx<'_>, flow: DataFlow<'_>) -> RuleResult {
    let mut query = FlowQuery::new(
        SourcePattern::secret_like(["token", "password"]),
        SinkPattern::logger(),
    );
    query.barriers = BarrierPattern::call_any(["redact", "mask_secret"]);
    query.max_depth = 8;
    query.minimum_precision = PolicyPrecision::Heuristic;
    query.max_paths = 4;

    for violation in flow.forbidden(query) {
        ctx.report(violation.diagnostic(
            ctx.rule_id(),
            "secret-like value reaches logging without redaction",
        ));
    }

    Ok(())
}
"#,
    );
    write_file(
        &temp.path().join("src/app.ts"),
        r#"export function handler(token: string) {
  console.log(token);
}
"#,
    );

    let json = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );

    let diagnostics = diagnostics_for_rule(&json, "local/data-flow");
    assert_eq!(
        diagnostics.len(),
        1,
        "expected one data-flow diagnostic: {json:#?}"
    );
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic["message"] == "secret-like value reaches logging without redaction"
                && diagnostic_has_evidence(diagnostic, "policy", "forbidden_flow")
                && diagnostic_has_evidence(diagnostic, "source", "token")
                && diagnostic_has_evidence(diagnostic, "sink", "log")
                && diagnostic_has_evidence(diagnostic, "required_barrier", "redact,mask_secret")
                && diagnostic_has_evidence(diagnostic, "barrier_status", "uncovered")
                && diagnostic_has_evidence(diagnostic, "path_status", "found")
        }),
        "missing data-flow diagnostic evidence: {json:#?}"
    );
    assert!(
        diagnostics_for_rule(&json, "polint/capability")
            .iter()
            .all(|diagnostic| { !diagnostic_has_evidence(diagnostic, "capability", "dataflow") }),
        "dataflow should execute without capability diagnostics: {json:#?}"
    );
}

#[test]
fn phase61_policy_query_docs_cover_preview_contract() {
    let policy = fs::read_to_string(repo_root().join("docs/facts/policy-queries.md")).unwrap();
    for required in [
        "The public style has one shape",
        "Events<'_>",
        "Calls<'_>",
        "ControlFlow<'_>",
        "DataFlow<'_>",
        "ReachQuery::new",
        "GuardQuery::new",
        "LifecycleQuery::new",
        "FlowQuery::new",
        "EventPattern::call",
        "SourcePattern::http_request",
        "SourcePattern::secret_like",
        "SinkPattern::logger",
        "BarrierPattern::call_any",
        "policy_query_version",
        "query_digest",
        "policy_status",
        "policy_precision",
        "budget_exceeded",
        "unknowns --cap events|calls|control_flow|dataflow",
        "request-to-shell",
        "unsafe-deserialization",
        "not built-in rules enabled",
    ] {
        assert!(
            policy.contains(required),
            "missing `{required}` in policy docs"
        );
    }

    for page in [
        "docs/facts/README.md",
        "docs/facts/events.md",
        "docs/facts/calls.md",
        "docs/facts/control-flow.md",
        "docs/facts/data-flow.md",
        "docs/facts/evidence.md",
        "docs/facts/capability-plans.md",
    ] {
        let source = fs::read_to_string(repo_root().join(page)).unwrap();
        assert!(
            source.contains("policy-queries.md"),
            "{page} should link the shared policy query reference"
        );
    }
}

#[test]
fn phase61_policy_preview_external_sdk_matrix() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_file(
        &temp.path().join("go.mod"),
        "module example.com/policyquerymatrix\n\ngo 1.22\n",
    );
    write_phase41_rule_pack(
        temp.path(),
        r#"use std::process::ExitCode;
use polint::runner;
mod rule;
fn main() -> ExitCode { runner::run_cli(vec![rule::rule()]) }
"#,
        r#"use polint::sdk::prelude::*;

#[polint::rule(id = "local/policy-matrix", description = "Policy matrix", severity = "error")]
pub(crate) fn rule(
    ctx: &mut RuleCtx<'_>,
    events: Events<'_>,
    calls: Calls<'_>,
    control: ControlFlow<'_>,
    flow: DataFlow<'_>,
) -> RuleResult {
    for violation in events.matching(EventPattern::call("dangerous")) {
        ctx.report(violation.diagnostic(ctx.rule_id(), "dangerous call event matched"));
    }

    let mut reach = ReachQuery::new(EventPattern::call("dangerous"));
    reach.roots = vec![EventPattern::call("main")];
    reach.max_depth = 4;
    reach.max_paths = 4;

    for violation in calls.forbidden_reachable(reach) {
        ctx.report(violation.diagnostic(ctx.rule_id(), "dangerous call is reachable from main"));
    }

    let mut guard = GuardQuery::new(
        EventPattern::call("writeBalance"),
        GuardPattern::call_any(["authorize"]),
    );
    guard.max_paths = 4;

    for violation in control.missing_guard(guard) {
        ctx.report(violation.diagnostic(ctx.rule_id(), "writeBalance requires authorization first"));
    }

    let mut cleanup = LifecycleQuery::new(
        EventPattern::call("Begin"),
        EventPattern::call("Rollback"),
    );
    cleanup.max_paths = 4;

    for violation in control.missing_cleanup(cleanup) {
        ctx.report(violation.diagnostic(ctx.rule_id(), "transaction begin requires cleanup"));
    }

    let mut flow_query = FlowQuery::new(
        SourcePattern::secret_like(["token"]),
        SinkPattern::logger(),
    );
    flow_query.barriers = BarrierPattern::call_any(["redact"]);
    flow_query.minimum_precision = PolicyPrecision::Heuristic;
    flow_query.max_paths = 4;

    for violation in flow.forbidden(flow_query) {
        ctx.report(violation.diagnostic(ctx.rule_id(), "secret token reaches logs"));
    }

    Ok(())
}
"#,
    );
    write_file(
        &temp.path().join("src/main.go"),
        r#"package main

func main() {
	handler()
}

func handler() {
	dangerous()
	writeBalance()
	tx := Begin()
	_ = tx
}

func dangerous() {}
func writeBalance() {}
func authorize() {}
func Begin() *Tx { return &Tx{} }
func Rollback() {}

type Tx struct{}
"#,
    );
    write_file(
        &temp.path().join("src/app.ts"),
        r#"export function handler(token: string) {
  console.log(token);
}
"#,
    );

    let source = fs::read_to_string(temp.path().join(".polint/rules/src/rule.rs")).unwrap();
    assert!(source.contains("use polint::sdk::prelude::*;"));
    for forbidden in [
        "polint::core",
        "crate::core",
        "Capabilities::new(",
        "impl Rule",
    ] {
        assert!(
            !source.contains(forbidden),
            "external policy rule leaked forbidden API `{forbidden}`:\n{source}"
        );
    }

    let json = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );

    let diagnostics = diagnostics_for_rule(&json, "local/policy-matrix");
    assert_eq!(
        diagnostics.len(),
        5,
        "expected one result per query: {json:#?}"
    );
    for (message, query) in [
        ("dangerous call event matched", "events.matching"),
        (
            "dangerous call is reachable from main",
            "calls.forbidden_reachable",
        ),
        (
            "writeBalance requires authorization first",
            "control_flow.missing_guard",
        ),
        (
            "transaction begin requires cleanup",
            "control_flow.missing_cleanup",
        ),
        ("secret token reaches logs", "data_flow.forbidden"),
    ] {
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic["message"] == message
                    && diagnostic_has_evidence(diagnostic, "policy_query", query)
                    && diagnostic_has_evidence(diagnostic, "policy_query_version", "")
                    && diagnostic_has_evidence(diagnostic, "query_digest", "")
                    && diagnostic_has_evidence(diagnostic, "policy_status", "")
                    && diagnostic_has_evidence(diagnostic, "policy_precision", "")
            }),
            "missing `{message}` / `{query}` diagnostic: {json:#?}"
        );
    }
    assert!(
        diagnostics_for_rule(&json, "polint/capability").is_empty(),
        "supported preview policy views should not emit capability diagnostics: {json:#?}"
    );
}

#[test]
fn check_help_guides_ai_agents_to_compact_output() {
    let help = polint_help(&["check", "--help"]);

    assert!(help.contains("ai-friendly"), "{help}");
    assert!(
        help.contains(".polint/output/"),
        "check help should mention saved AI-friendly output path: {help}"
    );
}

#[test]
fn phase33_internals_do_not_leak_from_real_public_cli_surfaces() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []
"#,
    );
    write_file(
        &temp.path().join("src/app.ts"),
        "export function answer(): number { return 42; }\n",
    );

    let surfaces = vec![
        (
            "polint check --format json",
            output_string(
                polint_cmd()
                    .current_dir(temp.path())
                    .args(["check", "--format", "json", "--fail-on", "none"])
                    .assert()
                    .success(),
            ),
        ),
        ("polint check --help", polint_help(&["check", "--help"])),
        (
            "polint inspect rule --format json",
            output_string(
                polint_cmd()
                    .current_dir(temp.path())
                    .args(["inspect", "rule", "--format", "json"])
                    .assert()
                    .success(),
            ),
        ),
        (
            "polint test --format json",
            output_string(
                polint_cmd()
                    .current_dir(temp.path())
                    .args(["test", "--format", "json"])
                    .assert()
                    .success(),
            ),
        ),
    ];

    for (surface, output) in surfaces {
        assert_no_phase33_internal_markers(surface, &output);
    }
}

#[test]
fn extension_no_leak() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []
"#,
    );
    write_file(
        &temp.path().join("src/app.ts"),
        "export function answer(): number { return 42; }\n",
    );

    let check_json = output_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    let help = polint_help(&["check", "--help"]);
    let mut public_sources = String::new();
    for path in [
        repo_root().join("crates/polint/src/lib.rs"),
        repo_root().join("crates/polint/src/sdk"),
        repo_root().join("crates/polint/src/runner"),
        repo_root().join("README.md"),
        repo_root().join("docs/API-VISIBILITY-PLAN.md"),
    ] {
        public_sources.push_str(&public_surface_text(&path));
    }

    for (surface, output) in [
        ("polint check --format json", check_json.as_str()),
        ("polint check --help", help.as_str()),
        ("public source/docs", public_sources.as_str()),
    ] {
        for marker in [
            "polint-extension-handshake-v1",
            "polint-extension-provider-run-v1",
            "ExtensionProviderRunResponse",
            "extension.real_sink_active",
        ] {
            assert!(
                !output.contains(marker),
                "public surface {surface} leaked extension internal marker `{marker}`:\n{output}"
            );
        }
    }
}

#[test]
fn evidence_public_no_leak() {
    let temp = tempfile::tempdir().unwrap();
    write_evidence_no_leak_rule_repo(temp.path());

    let public_json = output_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    let public_json_value: serde_json::Value = serde_json::from_str(&public_json)
        .unwrap_or_else(|error| panic!("check JSON should parse: {error}\n{public_json}"));
    assert_eq!(
        public_json_value["schema"],
        "https://raw.githubusercontent.com/oaiz-io/polint/main/docs/schemas/polint-report-v1.json"
    );
    let probe = diagnostics_for_rule(&public_json_value, "local/evidence-leak-probe");
    assert_eq!(probe.len(), 1, "{public_json_value:#?}");
    assert!(
        diagnostic_has_evidence(probe[0], "evidence_status", "partial"),
        "no-leak fixture must exercise diagnostic output with structured evidence rows: {probe:#?}"
    );
    let public_report: polint::sdk::prelude::PolintReport = serde_json::from_str(&public_json)
        .unwrap_or_else(|error| panic!("public check report should match SDK schema: {error}"));

    let ai_friendly = output_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "ai-friendly", "--fail-on", "none"])
            .assert()
            .success(),
    );
    let sarif = output_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "sarif", "--fail-on", "none"])
            .assert()
            .success(),
    );
    let help = polint_help(&["check", "--help"]);

    let public_sources = [
        repo_root().join("crates/polint/src/sdk/mod.rs"),
        repo_root().join("crates/polint/src/sdk/facts.rs"),
        repo_root().join("crates/polint/src/runner/mod.rs"),
        repo_root().join("README.md"),
        repo_root().join("docs/facts/evidence.md"),
    ]
    .into_iter()
    .map(|path| fs::read_to_string(path).unwrap())
    .collect::<Vec<_>>()
    .join("\n");

    for (surface, output) in [
        ("check json", public_json.as_str()),
        ("ai-friendly", ai_friendly.as_str()),
        ("sarif", sarif.as_str()),
        ("check help", help.as_str()),
        ("public docs/sdk/runner", public_sources.as_str()),
    ] {
        assert_no_evidence_internal_markers(surface, output);
    }
    assert!(
        public_sources.contains("Evidence is not a public SDK fact view"),
        "evidence docs must state public SDK limits"
    );
    assert_eq!(
        public_report.diagnostics.len(),
        1,
        "schema-validated public report should retain the evidence diagnostic"
    );
}

fn write_evidence_no_leak_rule_repo(root: &Path) {
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#,
        ),
    );
    write_file(
        &root.join(".polint/rules/src/main.rs"),
        r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/evidence-leak-probe",
    description = "Emit evidence-shaped public diagnostic output.",
    severity = "warn"
)]
fn evidence_leak_probe(ctx: &mut RuleCtx<'_>, literals: StringLiterals<'_>) -> RuleResult {
    let rule_id = ctx.rule_id().to_string();
    for literal in literals.iter() {
        if literal.value.contains("LEAK_PROBE") {
            ctx.report(
                Diagnostic::warning(
                    rule_id.clone(),
                    ctx.file_path(literal.file),
                    literal.span.diagnostic_range(),
                    "Evidence leak probe diagnostic.",
                )
                .with_evidence("evidence_status", "partial")
                .with_evidence("evidence_precision", "setup_aware")
                .with_evidence("evidence_path", "source>sink"),
            );
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![evidence_leak_probe()])
}
"#,
    );
    write_file(
        &root.join("src/app.ts"),
        r#"export const probe = "LEAK_PROBE";
"#,
    );
}

fn assert_no_evidence_internal_markers(surface: &str, output: &str) {
    for marker in [
        "analysis::evidence",
        "EvidenceStore",
        "EvidenceNodeFact",
        "EvidenceEdgeFact",
        "ExtensionEvidenceCandidateFact",
        "ExtensionEvidenceMergeFact",
        "polint::sdk::facts::Evidence",
        "pub mod evidence",
        "polint.evidence",
        "/Users/",
    ] {
        assert!(
            !output.contains(marker),
            "public surface {surface} leaked evidence internal marker `{marker}`:\n{output}"
        );
    }
}

fn output_string(assert: Assert) -> String {
    let output = assert.get_output();
    let stdout = String::from_utf8(output.stdout.clone()).expect("stdout should be valid UTF-8");
    let stderr = String::from_utf8(output.stderr.clone()).expect("stderr should be valid UTF-8");
    format!("{stdout}\n{stderr}")
}

fn assert_no_phase33_internal_markers(surface: &str, output: &str) {
    for marker in [
        "demand_query",
        "DemandQuery",
        "scc_schedule",
        "SccSchedule",
        "SccClosure",
        "scc_closure",
        "quarantine",
        "QuarantineStore",
        "QuarantineEntry",
        "quarantine_policy",
        "backdating",
        "backdated",
        "interprocedural_closure",
        "SccClosureResult",
        "SccClosureConfig",
        "DemandQueryEngine",
        "DemandQueryTrace",
        "DemandQueryTraceEntry",
        "DemandQueryResult",
        "is_quarantined",
        "reinstate",
    ] {
        assert!(
            !output.contains(marker),
            "{surface} leaked internal implementation marker `{marker}`:\n{output}"
        );
    }
}

#[test]
fn inspect_rule_manifest_json_is_stable_for_local_rules() {
    let temp = tempfile::tempdir().unwrap();
    write_inspect_rule_repo(temp.path());

    let run_inspect = || {
        stdout_string(
            polint_cmd()
                .current_dir(temp.path())
                .args(["inspect", "rule", "--format", "json"])
                .assert()
                .success(),
        )
    };
    let first = run_inspect();
    let second = run_inspect();

    assert_eq!(
        first, second,
        "inspect JSON should be byte-identical across repeated runs"
    );

    let value: serde_json::Value = serde_json::from_str(&first)
        .unwrap_or_else(|error| panic!("stdout was not inspect JSON: {error}\n{first}"));
    assert_eq!(value["version"], 1);
    assert_eq!(
        value["schema"],
        "https://raw.githubusercontent.com/oaiz-io/polint/main/docs/schemas/polint-rule-inspect-v1.json"
    );
    assert_eq!(value["tool"]["name"], "polint");

    let rules = value["rules"]
        .as_array()
        .unwrap_or_else(|| panic!("inspect rules should be an array: {value:#?}"));
    assert_eq!(rules.len(), 1, "expected one inspected rule: {first}");
    let rule = &rules[0];
    assert_eq!(rule["rule_id"], "local/inspect-probe");
    assert_eq!(rule["host_path"], ".polint/rules/Cargo.toml");
    assert_eq!(rule["description"], "Inspect manifest probe.");

    let fact_views = rule["fact_views"]
        .as_array()
        .unwrap_or_else(|| panic!("fact_views should be an array: {rule:#?}"));
    let fact_view_names = fact_views
        .iter()
        .map(|view| view["view_type"].as_str().unwrap_or_default())
        .collect::<BTreeSet<_>>();
    assert!(fact_view_names.contains("Imports"));
    assert!(fact_view_names.contains("StringLiterals"));
    assert!(fact_views.iter().any(|view| {
        view["canonical_path"] == "polint::sdk::facts::Imports<'_>"
            && view["capability"] == "imports"
            && view["parameter_name"] == "imports"
    }));

    let capabilities = rule["capabilities"]
        .as_array()
        .unwrap_or_else(|| panic!("capabilities should be an array: {rule:#?}"));
    let capability_names = capabilities
        .iter()
        .map(|capability| capability["name"].as_str().unwrap_or_default())
        .collect::<BTreeSet<_>>();
    assert!(capability_names.contains("imports"));
    assert!(capability_names.contains("string_literals"));

    assert_eq!(
        rule["capability_support"][0]["status"], "supported",
        "supported capability rows should be public strings: {rule:#?}"
    );

    let source = fs::read_to_string(temp.path().join(".polint/rules/src/main.rs")).unwrap();
    assert!(source.contains("use polint::sdk::prelude::*;"));
    assert!(source.contains("polint::runner::run_cli"));
    for forbidden in [
        concat!("polint::", "core"),
        "Capabilities::new(",
        "impl Rule for",
    ] {
        assert!(
            !source.contains(forbidden),
            "inspect fixture must use only the public rule-authoring API `{forbidden}`:\n{source}"
        );
    }

    for marker in [
        "AnalysisKernel",
        "LayerCache",
        "ProviderOutputMeta",
        "KernelRunReport",
        "InputSnapshot",
        "source_text",
        "temp_root",
    ] {
        assert!(
            !first.contains(marker),
            "inspect JSON must not leak internal marker `{marker}`:\n{first}"
        );
    }
    let temp_path_marker = temp.path().to_string_lossy().to_string();
    assert!(
        !first.contains(&temp_path_marker),
        "inspect JSON must not leak temp root `{temp_path_marker}`:\n{first}"
    );
}

fn write_inspect_rule_repo(root: &Path) {
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#,
        ),
    );
    write_file(
        &root.join(".polint/rules/src/main.rs"),
        r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/inspect-probe",
    description = "Inspect manifest probe.",
    severity = "warn"
)]
fn inspect_probe(
    _ctx: &mut RuleCtx<'_>,
    imports: Imports<'_>,
    literals: StringLiterals<'_>,
) -> RuleResult {
    let _counts = (imports.iter().count(), literals.iter().count());
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![inspect_probe()])
}
"#,
    );
    write_file(
        &root.join("src/app.ts"),
        r#"import value from "./value";
export const answer = `${value}`;
"#,
    );
}

#[test]
fn polint_test_runs_temp_repo_fixtures() {
    let temp = tempfile::tempdir().unwrap();
    write_polint_test_rule_repo(temp.path(), "design token");

    let pass_stdout = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["test", "--format", "json"])
            .assert()
            .success(),
    );
    let pass_report: serde_json::Value = serde_json::from_str(&pass_stdout)
        .unwrap_or_else(|error| panic!("stdout was not test JSON: {error}\n{pass_stdout}"));
    assert_eq!(pass_report["summary"]["total"], 1);
    assert_eq!(pass_report["summary"]["failed"], 0);
    assert_eq!(pass_report["cases"][0]["rule"], "local/no-raw-colors");
    assert_eq!(pass_report["cases"][0]["case"], "basic");
    assert_eq!(pass_report["cases"][0]["status"], "passed");

    assert_rule_test_json_is_public(&pass_stdout, temp.path());

    write_polint_test_rule_repo(temp.path(), "does not match");
    let fail_stdout = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["test", "--format", "json"])
            .assert()
            .failure(),
    );
    let fail_report: serde_json::Value = serde_json::from_str(&fail_stdout)
        .unwrap_or_else(|error| panic!("stdout was not failing test JSON: {error}\n{fail_stdout}"));
    assert_eq!(fail_report["summary"]["total"], 1);
    assert_eq!(fail_report["summary"]["failed"], 1);
    assert_eq!(fail_report["cases"][0]["status"], "failed");
    assert!(
        fail_report["cases"][0]["failures"][0]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("was not observed"),
        "failing report should explain mismatch: {fail_stdout}"
    );

    assert_rule_test_json_is_public(&fail_stdout, temp.path());

    let source = fs::read_to_string(temp.path().join(".polint/rules/src/main.rs")).unwrap();
    assert!(source.contains("use polint::sdk::prelude::*;"));
    assert!(source.contains("polint::runner::run_cli"));
    for forbidden in [
        concat!("polint::", "core"),
        "Capabilities::new(",
        "impl Rule for",
    ] {
        assert!(
            !source.contains(forbidden),
            "test fixture rule must use only public authoring API `{forbidden}`:\n{source}"
        );
    }
}

fn assert_rule_test_json_is_public(json: &str, root: &Path) {
    for marker in [
        "AnalysisDb",
        "KernelRunReport",
        "ProviderOutputMeta",
        "POLINT_RULES_TARGET_DIR",
        "rules-target",
        ".polint/cache",
        "tempdir",
        "source_text",
    ] {
        assert!(
            !json.contains(marker),
            "polint test JSON must not leak `{marker}`:\n{json}"
        );
    }
    let root_marker = root.to_string_lossy().to_string();
    assert!(
        !json.contains(&root_marker),
        "polint test JSON must not leak fixture repo root `{root_marker}`:\n{json}"
    );
}

fn write_polint_test_rule_repo(root: &Path, message_contains: &str) {
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#,
        ),
    );
    write_file(
        &root.join(".polint/rules/src/main.rs"),
        r##"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/no-raw-colors",
    description = "Reject raw color literals.",
    severity = "warn"
)]
fn no_raw_colors(ctx: &mut RuleCtx<'_>, literals: StringLiterals<'_>) -> RuleResult {
    let rule_id = ctx.rule_id().to_string();
    for literal in literals.iter() {
        if literal.value.contains("#") {
            let file = ctx.file_path(literal.file);
            ctx.report(Diagnostic::warning(
                rule_id.clone(),
                file,
                literal.span.diagnostic_range(),
                "Use a design token instead of raw color literal.",
            ));
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![no_raw_colors()])
}
"##,
    );
    write_file(
        &root.join(".polint/tests/rules/no_raw_colors/basic/polint-test.toml"),
        &format!(
            r#"
rule = "local/no-raw-colors"
paths = ["src/**"]

[[expect.diagnostic]]
rule_id = "local/no-raw-colors"
file = "src/example.ts"
severity = "warn"
message_contains = "{message_contains}"
"#
        ),
    );
    write_file(
        &root.join(".polint/tests/rules/no_raw_colors/basic/src/example.ts"),
        r##"export const color = "#ff00aa";
"##,
    );
}

#[test]
fn new_rule_generates_fixture_that_inspect_and_test_can_run() {
    let temp = tempfile::tempdir().unwrap();

    polint_cmd()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    polint_cmd()
        .current_dir(temp.path())
        .args(["new-rule", "ts", "no-raw-colors"])
        .assert()
        .success();
    point_generated_rule_pack_at_local_polint(temp.path());

    let rule_path = temp.path().join(".polint/rules/src/no_raw_colors.rs");
    let fixture_manifest = temp
        .path()
        .join(".polint/tests/rules/no_raw_colors/violating/polint-test.toml");
    let fixture_source = temp
        .path()
        .join(".polint/tests/rules/no_raw_colors/violating/src/example.ts");
    let clean_manifest = temp
        .path()
        .join(".polint/tests/rules/no_raw_colors/clean/polint-test.toml");
    assert!(rule_path.is_file());
    assert!(fixture_manifest.is_file());
    assert!(fixture_source.is_file());
    assert!(clean_manifest.is_file());

    let generated_rule = fs::read_to_string(&rule_path).unwrap();
    assert!(generated_rule.contains("use polint::sdk::prelude::*;"));
    assert!(generated_rule.contains("#[polint::rule("));
    for forbidden in [
        concat!("polint::", "core"),
        "Capabilities::new(",
        "impl Rule",
        "analysis_kernel",
        "go::",
        "ts::",
    ] {
        assert!(
            !generated_rule.contains(forbidden),
            "generated source must not expose `{forbidden}`:\n{generated_rule}"
        );
    }

    let fixture = fs::read_to_string(&fixture_manifest).unwrap();
    assert!(fixture.contains(r#"rule = "custom/no-raw-colors""#));
    assert!(fixture.contains(r#"paths = ["src/**"]"#));
    assert!(fixture.contains("[expect]"));
    assert!(fixture.contains("[[expect.diagnostic]]"));
    assert!(fixture.contains("message_contains"));

    let inspect_json = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["inspect", "rule", "--format", "json"])
            .assert()
            .success(),
    );
    let inspect: serde_json::Value = serde_json::from_str(&inspect_json)
        .unwrap_or_else(|error| panic!("inspect stdout was not JSON: {error}\n{inspect_json}"));
    assert_eq!(inspect["rules"][0]["rule_id"], "custom/no-raw-colors");

    let test_json = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["test", "--format", "json"])
            .assert()
            .success(),
    );
    let test_report: serde_json::Value = serde_json::from_str(&test_json)
        .unwrap_or_else(|error| panic!("test stdout was not JSON: {error}\n{test_json}"));
    assert_eq!(test_report["summary"]["total"], 2);
    assert_eq!(test_report["summary"]["failed"], 0);

    write_file(
        &rule_path,
        r##"use polint::sdk::prelude::*;

#[polint::rule(
    id = "custom/no-raw-colors",
    description = "Project-specific policy: no-raw-colors.",
    severity = "warn"
)]
pub(crate) fn no_raw_colors(ctx: &mut RuleCtx<'_>, literals: StringLiterals<'_>) -> RuleResult {
    let rule_id = ctx.rule_id().to_string();
    for literal in literals.iter() {
        if literal.value.contains("#") {
            let file = ctx.file_path(literal.file);
            ctx.report(Diagnostic::warning(
                rule_id.clone(),
                file,
                literal.span.diagnostic_range(),
                "Project-specific policy violation.",
            ));
        }
    }
    Ok(())
}
"##,
    );
    write_file(
        &temp.path().join("src/example.ts"),
        r##"export const color = "#ff00aa";
"##,
    );

    let check = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    let diagnostics = diagnostics_for_rule(&check, "custom/no-raw-colors");
    assert_eq!(diagnostics.len(), 1, "{check:#?}");
    assert!(
        diagnostics[0]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("Project-specific policy violation")
    );
}

#[test]
fn inspect_and_test_schema_files_are_valid_json() {
    for schema in [
        "docs/schemas/polint-rule-inspect-v1.json",
        "docs/schemas/polint-test-report-v1.json",
    ] {
        let raw = fs::read_to_string(repo_root().join(schema)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw)
            .unwrap_or_else(|error| panic!("{schema} is not valid JSON: {error}"));
        assert_eq!(value["properties"]["version"]["const"], 1);
        assert!(value["properties"]["schema"].is_object());
    }
}

#[test]
fn eval_harness_stays_internal() {
    let temp = tempfile::tempdir().unwrap();

    polint_cmd()
        .current_dir(temp.path())
        .arg("eval")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand 'eval'"));

    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []
"#,
    );
    write_file(
        &temp.path().join("src/app.ts"),
        r#"export const answer = 42;
"#,
    );

    let run_check = || {
        stdout_string(
            polint_cmd()
                .current_dir(temp.path())
                .args(["check", "--format", "json", "--fail-on", "none"])
                .assert()
                .success(),
        )
    };
    let first = run_check();
    let second = run_check();

    assert_eq!(
        first, second,
        "public check JSON should stay byte-identical across repeated runs"
    );
    let report: polint::sdk::prelude::PolintReport = serde_json::from_str(&first)
        .unwrap_or_else(|error| panic!("stdout was not public check JSON: {error}\n{first}"));
    assert!(
        report.diagnostics.is_empty(),
        "minimal public TS check should not emit diagnostics: {report:#?}"
    );

    for marker in INTERNAL_EVAL_PUBLIC_MARKERS {
        assert!(
            !first.contains(marker),
            "public check JSON must not leak internal eval marker `{marker}`:\n{first}"
        );
    }

    assert_eval_module_is_crate_private();
    assert_eval_internals_are_absent_from_public_surfaces();
}

const INTERNAL_EVAL_PUBLIC_MARKERS: &[&str] = &[
    "polint-eval-internal-1",
    "ExpectedItem",
    "ObservedItem",
    "MetricSummary",
    "output_hash",
    "provider_order.0",
    "extension.synthetic_rejected_fact",
];

fn assert_eval_module_is_crate_private() {
    let lib_rs_path = repo_root().join("crates/polint/src/lib.rs");
    let lib_rs = fs::read_to_string(&lib_rs_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", lib_rs_path.display()));

    assert!(
        lib_rs.contains("#[cfg(all(test, feature = \"lang-go\", feature = \"lang-typescript\"))]"),
        "eval module should stay test-only:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("#[path = \"../../polint-eval/src/harness/mod.rs\"]"),
        "eval harness sources should live in polint-eval:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("pub(crate) mod eval;"),
        "eval module should stay crate-private:\n{lib_rs}"
    );
    assert!(
        !lib_rs.contains("pub mod eval"),
        "eval module must not be promoted as a public crate-root module:\n{lib_rs}"
    );
}

fn assert_eval_internals_are_absent_from_public_surfaces() {
    let root = repo_root();
    let mut public_surface = String::new();
    for path in [
        root.join("crates/polint/src/cli/mod.rs"),
        root.join("crates/polint/src/sdk"),
        root.join("crates/polint/src/runner"),
    ] {
        public_surface.push_str(&source_tree_text(&path));
    }

    for marker in [
        "eval::",
        "pub use crate::eval",
        "pub mod eval",
        "polint-eval-internal-1",
        "ExpectedItem",
        "ObservedItem",
        "MetricSummary",
        "output_hash",
    ] {
        assert!(
            !public_surface.contains(marker),
            "public CLI/SDK/runner surface must not expose eval marker `{marker}`"
        );
    }
}

#[test]
fn input_snapshots_stay_internal() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []
"#,
    );
    write_file(
        &temp.path().join("src/app.ts"),
        "export const amount = 42;\n",
    );

    let run_check = || {
        stdout_string(
            polint_cmd()
                .current_dir(temp.path())
                .args(["check", "--format", "json", "--fail-on", "none"])
                .assert()
                .success(),
        )
    };
    let first = run_check();
    let second = run_check();

    assert_eq!(
        first, second,
        "public check JSON should stay byte-identical across repeated runs"
    );
    let report: polint::sdk::prelude::PolintReport = serde_json::from_str(&first)
        .unwrap_or_else(|error| panic!("stdout was not public check JSON: {error}\n{first}"));
    assert!(
        report.diagnostics.is_empty(),
        "minimal public TS check should not emit diagnostics: {report:#?}"
    );
    assert_input_snapshot_vocabulary_is_internal(&first);
}

fn assert_input_snapshot_vocabulary_is_internal(public_json: &str) {
    for marker in INTERNAL_INPUT_SNAPSHOT_PUBLIC_MARKERS {
        assert!(
            !public_json.contains(marker),
            "public check JSON must not leak internal snapshot/cache marker `{marker}`:\n{public_json}"
        );
    }
    assert_analysis_kernel_module_is_crate_private();
    assert_incremental_vocabulary_absent_from_public_surfaces();
}

const INTERNAL_INPUT_SNAPSHOT_PUBLIC_MARKERS: &[&str] = &[
    "InputSnapshot",
    "LayerKey",
    "QueryKey",
    "SummaryKey",
    "DiagnosticKey",
    "ProviderOutputMeta",
    "CacheStats",
    "KernelRunReport",
    "snapshot.schema_version",
    "provider_output.polint",
    "layer_key.polint",
    "polint-input-snapshot-1",
    "go.tool_invocation",
    "ts_js.tool_invocation",
];

fn assert_analysis_kernel_module_is_crate_private() {
    let lib_rs_path = repo_root().join("crates/polint/src/lib.rs");
    let lib_rs = fs::read_to_string(&lib_rs_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", lib_rs_path.display()));

    assert!(
        lib_rs.contains("pub(crate) mod analysis_kernel;"),
        "analysis_kernel module should stay crate-private:\n{lib_rs}"
    );
    assert!(
        !lib_rs.contains("pub mod analysis_kernel"),
        "analysis_kernel must not be promoted as a public crate-root module:\n{lib_rs}"
    );
}

fn assert_incremental_vocabulary_absent_from_public_surfaces() {
    let root = repo_root();
    let mut public_surface = String::new();
    for path in [
        root.join("crates/polint/src/sdk"),
        root.join("crates/polint/src/runner"),
        root.join("crates/polint/src/cli"),
    ] {
        public_surface.push_str(&source_tree_text(&path));
    }

    for marker in [
        "analysis_kernel::incremental",
        "InputSnapshot",
        "LayerKey",
        "QueryKey",
        "SummaryKey",
        "DiagnosticKey",
        "ProviderOutputMeta",
        "CacheStats",
        "KernelRunReport",
    ] {
        assert!(
            !public_surface.contains(marker),
            "public CLI/SDK/runner surface must not expose incremental marker `{marker}`"
        );
    }
}

#[test]
fn syntax_cache_ignores_unrelated_rule_edits() {
    let temp = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    write_syntax_cache_rule_repo(temp.path(), "v1");

    let cold = run_syntax_cache_check(temp.path(), cache.path());
    let warm = run_syntax_cache_check(temp.path(), cache.path());
    let warm_signatures = syntax_layer_manifest_signatures(cache.path());

    assert!(
        warm_signatures
            .iter()
            .any(|signature| signature.contains("polint.go.syntax")),
        "expected Go syntax layer manifest: {warm_signatures:#?}"
    );
    assert!(
        warm_signatures
            .iter()
            .any(|signature| signature.contains("polint.ts.syntax")),
        "expected TS syntax layer manifest: {warm_signatures:#?}"
    );

    write_syntax_cache_rule_repo(temp.path(), "v2");
    let after_rule_edit = run_syntax_cache_check(temp.path(), cache.path());
    let after_rule_edit_signatures = syntax_layer_manifest_signatures(cache.path());

    assert_eq!(warm, cold);
    assert_eq!(after_rule_edit, cold);
    assert_eq!(after_rule_edit_signatures, warm_signatures);

    for public_json in [&cold, &warm, &after_rule_edit] {
        assert_layer_cache_vocabulary_is_internal(public_json);
        let report: polint::sdk::prelude::PolintReport = serde_json::from_str(public_json)
            .unwrap_or_else(|error| {
                panic!("stdout was not public check JSON: {error}\n{public_json}")
            });
        assert!(
            report.diagnostics.is_empty(),
            "syntax cache fixture should not emit diagnostics: {report:#?}"
        );
    }

    let rule_source = fs::read_to_string(temp.path().join(".polint/rules/src/main.rs")).unwrap();
    assert!(rule_source.contains("use polint::sdk::prelude::*;"));
    assert!(rule_source.contains("polint::runner::run_cli"));
}

fn write_syntax_cache_rule_repo(root: &Path, marker: &str) {
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#,
        ),
    );
    let rule_source = r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/syntax-cache-probe",
    description = "Syntax cache probe __MARKER__.",
    severity = "warn"
)]
fn syntax_cache_probe(_ctx: &mut RuleCtx<'_>, imports: Imports<'_>) -> RuleResult {
    let _marker = "__MARKER__";
    let _import_count = imports.iter().count();
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![syntax_cache_probe()])
}
"#
    .replace("__MARKER__", marker);
    write_file(&root.join(".polint/rules/src/main.rs"), &rule_source);
    write_file(
        &root.join("src/main.go"),
        r#"package main

import "fmt"

func main() {
	fmt.Println("ok")
}
"#,
    );
    write_file(
        &root.join("src/app.ts"),
        r#"export function app() {
  return "ok";
}
"#,
    );
}

fn run_syntax_cache_check(root: &Path, cache_root: &Path) -> String {
    let mut command = polint_cmd();
    command.env("POLINT_CACHE_DIR", cache_root);
    stdout_string(
        command
            .current_dir(root)
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    )
}

fn syntax_layer_manifest_signatures(cache_root: &Path) -> BTreeSet<String> {
    let manifests_dir = cache_root.join("layers/manifests");
    let mut paths = Vec::new();
    collect_cache_json_paths(&manifests_dir, &mut paths);
    paths
        .into_iter()
        .filter_map(|path| syntax_layer_manifest_signature(&path))
        .collect()
}

fn syntax_layer_manifest_signature(path: &Path) -> Option<String> {
    let raw = fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let provider_id = json.pointer("/key/provider_id")?.as_str()?;
    if !matches!(provider_id, "polint.go.syntax" | "polint.ts.syntax") {
        return None;
    }
    let output_kind = json.pointer("/output_digest/kind")?.as_str()?;
    let output_value = json.pointer("/output_digest/value")?.as_str()?;
    let file_name = path.file_name()?.to_string_lossy();
    Some(format!(
        "{provider_id}|{file_name}|{output_kind}:{output_value}"
    ))
}

fn assert_layer_cache_vocabulary_is_internal(public_json: &str) {
    for marker in [
        "LayerCacheManifest",
        "LayerCacheStore",
        "LayerKey",
        "provider_output.polint",
        "verified_reuse",
    ] {
        assert!(
            !public_json.contains(marker),
            "public check JSON must not leak layer cache marker `{marker}`:\n{public_json}"
        );
    }
}

const INTERNAL_LAYER_CACHE_PUBLIC_MARKERS: &[&str] = &[
    "LayerCacheManifest",
    "LayerCacheStore",
    "DependencyIndex",
    "ChangeSet",
    "InvalidationPlan",
    "LayerKey",
    "KernelRunReport",
    "provider_output.polint",
    "verified_reuse",
    "invalid_evicted_reads",
    "layer_cache",
];

#[test]
fn layer_cache_internals_stay_private() {
    let temp = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    write_layer_cache_public_rule_repo(temp.path());

    let run_check = || {
        let mut command = polint_cmd();
        command.env("POLINT_CACHE_DIR", cache.path());
        stdout_string(
            command
                .current_dir(temp.path())
                .args(["check", "--format", "json", "--fail-on", "none"])
                .assert()
                .success(),
        )
    };
    let first = run_check();
    let second = run_check();

    assert_eq!(
        first, second,
        "public check JSON should stay byte-identical across warm layer-cache runs"
    );
    let report: polint::sdk::prelude::PolintReport = serde_json::from_str(&first)
        .unwrap_or_else(|error| panic!("stdout was not public check JSON: {error}\n{first}"));
    assert!(
        report.diagnostics.is_empty(),
        "layer-cache public fixture should not emit diagnostics: {report:#?}"
    );

    assert_layer_cache_public_json_is_private(&first);
    assert_layer_cache_public_json_is_private(&second);
    assert_layer_cache_public_surfaces_are_private();
    assert_layer_cache_cli_help_is_private();

    let rule_source = fs::read_to_string(temp.path().join(".polint/rules/src/main.rs")).unwrap();
    assert!(rule_source.contains("use polint::sdk::prelude::*;"));
    assert!(rule_source.contains("polint::runner::run_cli"));
    for forbidden in [
        concat!("polint::", "core"),
        concat!("polint::", "analysis_kernel"),
        concat!("polint::", "go"),
        concat!("polint::", "ts"),
        "LayerCache",
        "KernelRunReport",
        "Capabilities::new(",
        "impl Rule for",
    ] {
        assert!(
            !rule_source.contains(forbidden),
            "external rule source must not depend on internal API `{forbidden}`:\n{rule_source}"
        );
    }
}

#[test]
fn semantic_index_internals_stay_private() {
    let temp = tempfile::tempdir().unwrap();
    write_semantic_index_public_rule_repo(temp.path());

    let public_json = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    let report: polint::sdk::prelude::PolintReport = serde_json::from_str(&public_json)
        .unwrap_or_else(|error| panic!("stdout was not public check JSON: {error}\n{public_json}"));
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "local/semantic-public-probe"),
        "external semantic public rule should run: {report:#?}"
    );

    let inspect_json = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["inspect", "rule", "--format", "json"])
            .assert()
            .success(),
    );
    let inspect: serde_json::Value = serde_json::from_str(&inspect_json)
        .unwrap_or_else(|error| panic!("stdout was not inspect JSON: {error}\n{inspect_json}"));
    assert_eq!(
        inspect["rules"][0]["rule_id"],
        "local/semantic-public-probe"
    );

    let test_json = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["test", "--format", "json"])
            .assert()
            .success(),
    );
    let test_report: serde_json::Value = serde_json::from_str(&test_json)
        .unwrap_or_else(|error| panic!("stdout was not test JSON: {error}\n{test_json}"));
    assert_eq!(test_report["summary"]["failed"], 0);

    for output in [&public_json, &inspect_json, &test_json] {
        assert_semantic_index_public_output_is_private(output);
    }

    let source = fs::read_to_string(temp.path().join(".polint/rules/src/main.rs")).unwrap();
    assert!(source.contains("use polint::sdk::prelude::*;"));
    assert!(source.contains("Symbols<'_>"));
    assert!(source.contains("References<'_>"));
    assert!(source.contains("polint::runner::run_cli"));
    assert_public_symbol_rule_source(&source);

    let docs =
        fs::read_to_string(repo_root().join("docs/facts/symbols-and-references.md")).unwrap();
    assert!(
        docs.contains("scopes/import closure/resolution-step rows remain internal"),
        "public docs should state semantic internals remain internal"
    );
    assert!(
        docs.contains("generated or external semantic evidence"),
        "public docs should describe bounded generated/external evidence"
    );
}

fn assert_semantic_index_public_output_is_private(output: &str) {
    for marker in [
        "SemanticIndex",
        "ScopeFact",
        "SemanticImportFact",
        "ResolutionFact",
        "GeneratedSymbolFact",
        "StableExportIdentity",
        "alias_reexport_closure",
        "metadata.precision",
        "ProviderOutputMeta",
        "KernelRunReport",
        "polint-go-symbols-semantic-1",
        "tests/eval-fixtures",
    ] {
        assert!(
            !output.contains(marker),
            "public output must not leak semantic internal marker `{marker}`:\n{output}"
        );
    }
}

#[test]
fn semantic_mir_internals_stay_private() {
    let temp = tempfile::tempdir().unwrap();
    write_semantic_mir_public_rule_repo(temp.path());

    let check_json = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    let report: polint::sdk::prelude::PolintReport = serde_json::from_str(&check_json)
        .unwrap_or_else(|error| panic!("stdout was not public check JSON: {error}\n{check_json}"));
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "local/semantic-mir-public-probe"),
        "external semantic MIR public-boundary rule should run: {report:#?}"
    );
    assert_semantic_mir_public_output_is_private(&check_json);

    let inspect_json = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["inspect", "rule", "--format", "json"])
            .assert()
            .success(),
    );
    let inspect: serde_json::Value = serde_json::from_str(&inspect_json)
        .unwrap_or_else(|error| panic!("stdout was not inspect JSON: {error}\n{inspect_json}"));
    assert_eq!(
        inspect["rules"][0]["rule_id"],
        "local/semantic-mir-public-probe"
    );
    let fact_views = inspect["rules"][0]["fact_views"]
        .as_array()
        .unwrap_or_else(|| panic!("fact_views should be an array: {inspect:#?}"));
    for canonical_path in [
        "polint::sdk::facts::ModuleGraphFacts<'_>",
        "polint::sdk::facts::References<'_>",
        "polint::sdk::facts::ResolvedImports<'_>",
        "polint::sdk::facts::Symbols<'_>",
    ] {
        assert!(
            fact_views
                .iter()
                .any(|view| view["canonical_path"] == canonical_path),
            "inspect JSON should include public fact view {canonical_path}: {inspect_json}"
        );
    }

    let test_json = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["test", "--format", "json"])
            .assert()
            .success(),
    );
    let test_report: serde_json::Value = serde_json::from_str(&test_json)
        .unwrap_or_else(|error| panic!("stdout was not test JSON: {error}\n{test_json}"));
    assert_eq!(test_report["summary"]["failed"], 0);

    for output in [&check_json, &inspect_json, &test_json] {
        assert_semantic_mir_public_output_is_private(output);
    }
    assert_semantic_mir_public_surfaces_are_private();
    assert_semantic_mir_cli_help_is_private();

    let source = fs::read_to_string(temp.path().join(".polint/rules/src/main.rs")).unwrap();
    assert!(source.contains("use polint::sdk::prelude::*;"));
    assert!(source.contains("ResolvedImports<'_>"));
    assert!(source.contains("ModuleGraphFacts<'_>"));
    assert!(source.contains("Symbols<'_>"));
    assert!(source.contains("References<'_>"));
    assert!(source.contains("polint::runner::run_cli"));
    assert_semantic_mir_public_rule_source(&source);
}

fn assert_semantic_mir_public_output_is_private(output: &str) {
    for marker in SEMANTIC_MIR_INTERNAL_PUBLIC_MARKERS {
        assert!(
            !output.contains(marker),
            "public output must not leak semantic MIR internal marker `{marker}`:\n{output}"
        );
    }
}

fn assert_semantic_mir_public_surfaces_are_private() {
    let root = repo_root();
    let lib_rs = fs::read_to_string(root.join("crates/polint/src/lib.rs")).unwrap();
    let public_lib_rs = lib_rs
        .split("pub(crate) mod analysis;")
        .next()
        .unwrap_or(&lib_rs);
    let mut public_surface = public_lib_rs.to_string();
    for path in [
        root.join("README.md"),
        root.join("docs"),
        root.join("crates/polint/src/sdk"),
        root.join("crates/polint/src/runner"),
        root.join("crates/polint/src/cli"),
    ] {
        public_surface.push_str(&public_surface_text(&path));
    }

    for marker in SEMANTIC_MIR_INTERNAL_PUBLIC_MARKERS {
        assert!(
            !public_surface.contains(marker),
            "public docs/CLI/SDK/runner/crate-root source must not expose semantic MIR marker `{marker}`"
        );
    }
}

fn public_rule_help_outputs() -> [String; 5] {
    [
        polint_help(&["--help"]),
        polint_help(&["check", "--help"]),
        polint_help(&["inspect", "--help"]),
        polint_help(&["inspect", "rule", "--help"]),
        polint_help(&["test", "--help"]),
    ]
}

fn assert_semantic_mir_cli_help_is_private() {
    let help_outputs = public_rule_help_outputs();

    for help in help_outputs {
        for marker in SEMANTIC_MIR_INTERNAL_PUBLIC_MARKERS {
            assert!(
                !help.contains(marker),
                "public CLI help must not expose semantic MIR marker `{marker}`:\n{help}"
            );
        }
    }
}

const SEMANTIC_MIR_INTERNAL_PUBLIC_MARKERS: &[&str] = &[
    "polint.semantic_mir",
    "semantic-mir-facts",
    "MirBody",
    "MirOperation",
    "PlaceFact",
    "UnsupportedSemanticFact",
    "SemanticStore",
    "analysis::mir",
    "analysis::places",
    "mir.bodies",
    "mir.operations",
    "mir.places",
    "mir.unsupported",
    "Mir<'_>",
    "Places<'_>",
    "polint mir",
];

fn assert_semantic_mir_public_rule_source(source: &str) {
    assert!(
        source.contains("use polint::sdk::prelude::*;"),
        "external rule must import the public SDK prelude only:\n{source}"
    );
    assert!(
        source.contains("ResolvedImports<'_>")
            && source.contains("ModuleGraphFacts<'_>")
            && source.contains("Symbols<'_>")
            && source.contains("References<'_>"),
        "external rule must request supported public fact views through its signature:\n{source}"
    );
    for forbidden in [
        concat!("polint::", "analysis"),
        concat!("polint::", "analysis_kernel"),
        concat!("polint::", "core"),
        concat!("polint::", "go"),
        concat!("polint::", "ts"),
        "Capabilities::new(",
        "impl Rule for",
    ] {
        assert!(
            !source.contains(forbidden),
            "external rule source must not depend on internal API `{forbidden}`:\n{source}"
        );
    }
    assert_semantic_mir_public_output_is_private(source);
}

#[test]
fn cfg_public_no_leak() {
    assert_cfg_public_surfaces_are_private();
    assert_cfg_cli_help_is_private();
}

#[test]
fn type_value_alias_public_no_leak() {
    let temp = tempfile::tempdir().unwrap();
    write_type_value_alias_public_rule_repo(temp.path());

    let check_json = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    let _report: polint::sdk::prelude::PolintReport = serde_json::from_str(&check_json)
        .unwrap_or_else(|error| panic!("stdout was not public check JSON: {error}\n{check_json}"));

    let inspect_json = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["inspect", "rule", "--format", "json"])
            .assert()
            .success(),
    );
    let inspect: serde_json::Value = serde_json::from_str(&inspect_json)
        .unwrap_or_else(|error| panic!("stdout was not inspect JSON: {error}\n{inspect_json}"));
    assert_eq!(
        inspect["rules"][0]["rule_id"],
        "local/type-value-alias-public-probe"
    );

    let test_json = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["test", "--format", "json"])
            .assert()
            .success(),
    );
    let test_report: serde_json::Value = serde_json::from_str(&test_json)
        .unwrap_or_else(|error| panic!("stdout was not test JSON: {error}\n{test_json}"));
    assert_eq!(test_report["summary"]["failed"], 0);

    for output in [&check_json, &inspect_json, &test_json] {
        assert_type_value_alias_public_output_is_private(output);
    }
    assert_type_value_alias_public_surfaces_are_private();
    assert_type_value_alias_cli_help_is_private();
}

#[test]
fn direct_calls_internals_stay_private() {
    let temp = tempfile::tempdir().unwrap();
    write_direct_calls_public_rule_repo(temp.path());

    let check_json = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    let report: polint::sdk::prelude::PolintReport = serde_json::from_str(&check_json)
        .unwrap_or_else(|error| panic!("stdout was not public check JSON: {error}\n{check_json}"));
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "local/direct-calls-public-probe"),
        "external direct-calls public-boundary rule should run: {report:#?}"
    );

    let inspect_json = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["inspect", "rule", "--format", "json"])
            .assert()
            .success(),
    );
    let inspect: serde_json::Value = serde_json::from_str(&inspect_json)
        .unwrap_or_else(|error| panic!("stdout was not inspect JSON: {error}\n{inspect_json}"));
    assert_eq!(
        inspect["rules"][0]["rule_id"],
        "local/direct-calls-public-probe"
    );
    let fact_views = inspect["rules"][0]["fact_views"]
        .as_array()
        .unwrap_or_else(|| panic!("fact_views should be an array: {inspect:#?}"));
    for canonical_path in [
        "polint::sdk::facts::ModuleGraphFacts<'_>",
        "polint::sdk::facts::References<'_>",
        "polint::sdk::facts::ResolvedImports<'_>",
        "polint::sdk::facts::Symbols<'_>",
    ] {
        assert!(
            fact_views
                .iter()
                .any(|view| view["canonical_path"] == canonical_path),
            "inspect JSON should include public fact view {canonical_path}: {inspect_json}"
        );
    }

    let test_json = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["test", "--format", "json"])
            .assert()
            .success(),
    );
    let test_report: serde_json::Value = serde_json::from_str(&test_json)
        .unwrap_or_else(|error| panic!("stdout was not test JSON: {error}\n{test_json}"));
    assert_eq!(test_report["summary"]["failed"], 0);

    for output in [&check_json, &inspect_json, &test_json] {
        assert_direct_calls_public_output_is_private(output);
    }
    assert_direct_calls_public_surfaces_are_private();
    assert_direct_calls_cli_help_is_private();
    assert!(!repo_root().join("docs/facts/call-graph.md").exists());

    let source = fs::read_to_string(temp.path().join(".polint/rules/src/main.rs")).unwrap();
    assert!(source.contains("use polint::sdk::prelude::*;"));
    assert!(source.contains("ResolvedImports<'_>"));
    assert!(source.contains("ModuleGraphFacts<'_>"));
    assert!(source.contains("Symbols<'_>"));
    assert!(source.contains("References<'_>"));
    assert!(source.contains("polint::runner::run_cli"));
    assert_direct_calls_public_rule_source(&source);
}

#[test]
fn abstract_domain_internals_stay_private() {
    let temp = tempfile::tempdir().unwrap();
    write_abstract_domains_public_rule_repo(temp.path());

    let check_json = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    let report: polint::sdk::prelude::PolintReport = serde_json::from_str(&check_json)
        .unwrap_or_else(|error| panic!("stdout was not public check JSON: {error}\n{check_json}"));
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "local/abstract-domains-public-probe"),
        "external abstract-domain public-boundary rule should run: {report:#?}"
    );

    let inspect_json = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["inspect", "rule", "--format", "json"])
            .assert()
            .success(),
    );
    let inspect: serde_json::Value = serde_json::from_str(&inspect_json)
        .unwrap_or_else(|error| panic!("stdout was not inspect JSON: {error}\n{inspect_json}"));
    assert_eq!(
        inspect["rules"][0]["rule_id"],
        "local/abstract-domains-public-probe"
    );
    let fact_views = inspect["rules"][0]["fact_views"]
        .as_array()
        .unwrap_or_else(|| panic!("fact_views should be an array: {inspect:#?}"));
    for canonical_path in [
        "polint::sdk::facts::ComplexityMetrics<'_>",
        "polint::sdk::facts::FileMetrics<'_>",
        "polint::sdk::facts::FunctionMetrics<'_>",
        "polint::sdk::facts::ModuleGraphFacts<'_>",
        "polint::sdk::facts::References<'_>",
        "polint::sdk::facts::ResolvedImports<'_>",
        "polint::sdk::facts::Symbols<'_>",
    ] {
        assert!(
            fact_views
                .iter()
                .any(|view| view["canonical_path"] == canonical_path),
            "inspect JSON should include public fact view {canonical_path}: {inspect_json}"
        );
    }

    let test_json = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["test", "--format", "json"])
            .assert()
            .success(),
    );
    let test_report: serde_json::Value = serde_json::from_str(&test_json)
        .unwrap_or_else(|error| panic!("stdout was not test JSON: {error}\n{test_json}"));
    assert_eq!(test_report["summary"]["failed"], 0);

    for output in [&check_json, &inspect_json, &test_json] {
        assert_abstract_domains_public_output_is_private(output);
    }
    assert_abstract_domains_public_surfaces_are_private();
    assert_abstract_domains_cli_help_is_private();

    let source = fs::read_to_string(temp.path().join(".polint/rules/src/main.rs")).unwrap();
    assert!(source.contains("use polint::sdk::prelude::*;"));
    assert!(source.contains("ResolvedImports<'_>"));
    assert!(source.contains("ModuleGraphFacts<'_>"));
    assert!(source.contains("Symbols<'_>"));
    assert!(source.contains("References<'_>"));
    assert!(source.contains("FileMetrics<'_>"));
    assert!(source.contains("FunctionMetrics<'_>"));
    assert!(source.contains("ComplexityMetrics<'_>"));
    assert!(source.contains("polint::runner::run_cli"));
    assert_abstract_domains_public_rule_source(&source);
}

fn assert_type_value_alias_public_output_is_private(output: &str) {
    for marker in TYPE_VALUE_ALIAS_INTERNAL_PUBLIC_MARKERS {
        assert!(
            !output.contains(marker),
            "public output must not leak type/value/alias internal marker `{marker}`:\n{output}"
        );
    }
}

fn assert_direct_calls_public_output_is_private(output: &str) {
    for marker in DIRECT_CALLS_INTERNAL_PUBLIC_MARKERS {
        assert!(
            !output.contains(marker),
            "public output must not leak direct-call internal marker `{marker}`:\n{output}"
        );
    }
}

fn assert_abstract_domains_public_output_is_private(output: &str) {
    for marker in ABSTRACT_DOMAINS_INTERNAL_PUBLIC_MARKERS {
        assert!(
            !output.contains(marker),
            "public output must not leak abstract-domain internal marker `{marker}`:\n{output}"
        );
    }
}

fn assert_type_value_alias_public_surfaces_are_private() {
    let root = repo_root();
    let lib_rs = fs::read_to_string(root.join("crates/polint/src/lib.rs")).unwrap();
    let public_lib_rs = lib_rs
        .split("pub(crate) mod analysis;")
        .next()
        .unwrap_or(&lib_rs);
    let mut public_surface = public_lib_rs.to_string();
    for path in [
        root.join("README.md"),
        root.join("docs/API-VISIBILITY-PLAN.md"),
        root.join("docs/facts"),
        root.join("crates/polint/src/sdk"),
        root.join("crates/polint/src/runner"),
        root.join("crates/polint/src/cli"),
    ] {
        public_surface.push_str(&public_surface_text(&path));
    }

    for marker in TYPE_VALUE_ALIAS_INTERNAL_PUBLIC_MARKERS {
        assert!(
            !public_surface.contains(marker),
            "public README/docs/facts/CLI/SDK/runner/crate-root source must not expose type/value/alias marker `{marker}`"
        );
    }
}

fn assert_abstract_domains_public_surfaces_are_private() {
    let root = repo_root();
    let lib_rs = fs::read_to_string(root.join("crates/polint/src/lib.rs")).unwrap();
    let public_lib_rs = lib_rs
        .split("pub(crate) mod analysis;")
        .next()
        .unwrap_or(&lib_rs);
    let mut public_surface = public_lib_rs.to_string();
    for path in [
        root.join("README.md"),
        root.join("docs/facts"),
        root.join("crates/polint/src/sdk"),
        root.join("crates/polint/src/runner"),
        root.join("crates/polint/src/cli"),
    ] {
        public_surface.push_str(&public_surface_text(&path));
    }

    for marker in ABSTRACT_DOMAINS_INTERNAL_PUBLIC_MARKERS {
        assert!(
            !public_surface.contains(marker),
            "public README/docs/facts/CLI/SDK/runner/crate-root source must not expose abstract-domain marker `{marker}`"
        );
    }
}

fn assert_type_value_alias_cli_help_is_private() {
    let help_outputs = public_rule_help_outputs();

    for help in help_outputs {
        for marker in TYPE_VALUE_ALIAS_INTERNAL_PUBLIC_MARKERS {
            assert!(
                !help.contains(marker),
                "public CLI help must not expose type/value/alias marker `{marker}`:\n{help}"
            );
        }
    }
}

fn assert_abstract_domains_cli_help_is_private() {
    let help_outputs = public_rule_help_outputs();

    for help in help_outputs {
        for marker in ABSTRACT_DOMAINS_INTERNAL_PUBLIC_MARKERS {
            assert!(
                !help.contains(marker),
                "public CLI help must not expose abstract-domain marker `{marker}`:\n{help}"
            );
        }
    }
}

const TYPE_VALUE_ALIAS_INTERNAL_PUBLIC_MARKERS: &[&str] = &[
    "polint.type_value_alias",
    "type-value-alias-facts",
    "TypeFact",
    "NarrowedTypeFact",
    "ValueFact",
    "AllocationTokenFact",
    "AccessPathFact",
    "PointsToConstraintFact",
    "AliasAnswerFact",
    "PointsToSetFact",
    "Types<'_>",
    "Values<'_>",
    "Aliases<'_>",
    "points-to",
    "type_value_alias",
];

fn write_type_value_alias_public_rule_repo(root: &Path) {
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#,
        ),
    );
    write_file(
        &root.join(".polint/rules/src/main.rs"),
        TYPE_VALUE_ALIAS_PUBLIC_RULE_SOURCE,
    );
    write_type_value_alias_public_fixture_files(root);

    let case_root = root.join(".polint/tests/rules/type_value_alias_public/basic");
    write_file(
        &case_root.join("polint-test.toml"),
        r#"
rule = "local/type-value-alias-public-probe"
paths = ["src/**"]

[[expect.diagnostic]]
rule_id = "local/type-value-alias-public-probe"
file = "<workspace>"
severity = "warn"
message_contains = "public facts remain compatible while private analysis stays private"
"#,
    );
    write_type_value_alias_public_fixture_files(&case_root);
}

const TYPE_VALUE_ALIAS_PUBLIC_RULE_SOURCE: &str = r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/type-value-alias-public-probe",
    description = "Reads supported public facts while private analysis facts stay private.",
    severity = "warn"
)]
fn type_value_alias_public_probe(
    ctx: &mut RuleCtx<'_>,
    imports: Imports<'_>,
    file_metrics: FileMetrics<'_>,
) -> RuleResult {
    let import_count = imports.iter().count();
    let file_metric_count = file_metrics.iter().count();
    let public_count = import_count + file_metric_count;
    if public_count == 0 {
        return Ok(());
    }

    ctx.report(
        Diagnostic::warning(
            ctx.rule_id(),
            "<workspace>",
            DiagnosticRange::point(1, 1),
            "public facts remain compatible while private analysis stays private",
        )
        .with_evidence("imports", import_count.to_string())
        .with_evidence("file_metrics", file_metric_count.to_string()),
    );
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![type_value_alias_public_probe()])
}
"#;

fn write_type_value_alias_public_fixture_files(root: &Path) {
    write_file(
        &root.join("src/app.ts"),
        r#"import { value } from "./value";

export function app(): number {
  const box = { value };
  return box.value;
}
"#,
    );
    write_file(&root.join("src/value.ts"), "export const value = 1;\n");
}

const ABSTRACT_DOMAINS_INTERNAL_PUBLIC_MARKERS: &[&str] = &[
    "polint.abstract_domains",
    "abstract-domain-facts",
    "DomainObservation",
    "DomainEvent",
    "Nilness<'_>",
    "Truthiness<'_>",
    "Constants<'_>",
    "StringValues<'_>",
    "Initializedness<'_>",
    "AbstractDomains<'_>",
];

fn assert_direct_calls_public_surfaces_are_private() {
    let root = repo_root();
    let lib_rs = fs::read_to_string(root.join("crates/polint/src/lib.rs")).unwrap();
    let public_lib_rs = lib_rs
        .split("pub(crate) mod analysis;")
        .next()
        .unwrap_or(&lib_rs);
    let mut public_surface = public_lib_rs.to_string();
    for path in [
        root.join("README.md"),
        root.join("docs/facts"),
        root.join("crates/polint/src/sdk"),
        root.join("crates/polint/src/runner"),
        root.join("crates/polint/src/cli"),
    ] {
        public_surface.push_str(&public_surface_text(&path));
    }

    for marker in DIRECT_CALLS_INTERNAL_PUBLIC_MARKERS {
        assert!(
            !public_surface.contains(marker),
            "public README/docs/facts/CLI/SDK/runner/crate-root source must not expose direct-call marker `{marker}`"
        );
    }
}

fn assert_direct_calls_cli_help_is_private() {
    let help_outputs = public_rule_help_outputs();

    for help in help_outputs {
        for marker in DIRECT_CALLS_INTERNAL_PUBLIC_MARKERS {
            assert!(
                !help.contains(marker),
                "public CLI help must not expose direct-call marker `{marker}`:\n{help}"
            );
        }
    }
}

const DIRECT_CALLS_INTERNAL_PUBLIC_MARKERS: &[&str] = &[
    "polint.calls",
    "calls-facts-1",
    "CallSiteFact",
    "CallTargetFact",
    "UnresolvedCallFact",
    "analysis::calls",
    "direct-call-facts",
    "CallEdges<'_>",
];

fn assert_cfg_public_surfaces_are_private() {
    let root = repo_root();
    let lib_rs = fs::read_to_string(root.join("crates/polint/src/lib.rs")).unwrap();
    let public_lib_rs = lib_rs
        .split("pub(crate) mod analysis;")
        .next()
        .unwrap_or(&lib_rs);
    let mut public_surface = public_lib_rs.to_string();
    for path in [
        root.join("README.md"),
        root.join("docs/facts"),
        root.join("crates/polint/src/sdk"),
        root.join("crates/polint/src/runner"),
        root.join("crates/polint/src/cli"),
    ] {
        public_surface.push_str(&public_surface_text(&path));
    }

    for marker in CFG_INTERNAL_PUBLIC_MARKERS {
        assert!(
            !public_surface.contains(marker),
            "public README/docs/facts/CLI/SDK/runner/crate-root source must not expose CFG marker `{marker}`"
        );
    }
}

fn assert_cfg_cli_help_is_private() {
    let help_outputs = public_rule_help_outputs();

    for help in help_outputs {
        for marker in CFG_INTERNAL_PUBLIC_MARKERS {
            assert!(
                !help.contains(marker),
                "public CLI help must not expose CFG marker `{marker}`:\n{help}"
            );
        }
    }
}

const CFG_INTERNAL_PUBLIC_MARKERS: &[&str] = &[
    "polint.cfg",
    "cfg-facts-1",
    "CfgFunction",
    "CfgNode",
    "BasicBlock",
    "CfgEdge",
    "CfgReachability",
    "CfgDominator",
    "CfgPostDominator",
    "CfgControlDependence",
    "UnsupportedControlFlow",
    "analysis::cfg",
];

fn write_abstract_domains_public_rule_repo(root: &Path) {
    assert_abstract_domains_public_rule_source(ABSTRACT_DOMAINS_PUBLIC_RULE_SOURCE);
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint.toml"),
        r#"
[workspace]
include = ["web/src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#,
        ),
    );
    write_file(
        &root.join(".polint/rules/src/main.rs"),
        ABSTRACT_DOMAINS_PUBLIC_RULE_SOURCE,
    );
    write_abstract_domains_public_fixture_files(root);

    let case_root = root.join(".polint/tests/rules/abstract_domains_public/basic");
    write_file(
        &case_root.join("polint-test.toml"),
        r#"
rule = "local/abstract-domains-public-probe"
paths = ["web/src/**"]

[[expect.diagnostic]]
rule_id = "local/abstract-domains-public-probe"
file = "<workspace>"
severity = "warn"
message_contains = "public facts remain compatible while abstract domains stay private"
"#,
    );
    write_abstract_domains_public_fixture_files(&case_root);
}

const ABSTRACT_DOMAINS_PUBLIC_RULE_SOURCE: &str = r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/abstract-domains-public-probe",
    description = "Reads supported public facts while abstract domain facts stay private.",
    severity = "warn"
)]
fn abstract_domains_public_probe(
    ctx: &mut RuleCtx<'_>,
    resolved: ResolvedImports<'_>,
    graph: ModuleGraphFacts<'_>,
    symbols: Symbols<'_>,
    references: References<'_>,
    file_metrics: FileMetrics<'_>,
    function_metrics: FunctionMetrics<'_>,
    complexity_metrics: ComplexityMetrics<'_>,
) -> RuleResult {
    let resolved_count = resolved.iter().count();
    let edge_count = graph.edges().len();
    let symbol_count = symbols.iter().count();
    let reference_count = references.iter().count();
    let file_metric_count = file_metrics.iter().count();
    let function_metric_count = function_metrics.iter().count();
    let complexity_metric_count = complexity_metrics.iter().count();
    let public_fact_count = resolved_count
        + edge_count
        + symbol_count
        + reference_count
        + file_metric_count
        + function_metric_count
        + complexity_metric_count;
    if public_fact_count == 0 {
        return Ok(());
    }

    ctx.report(
        Diagnostic::warning(
            ctx.rule_id(),
            "<workspace>",
            DiagnosticRange::point(1, 1),
            "public facts remain compatible while abstract domains stay private",
        )
        .with_evidence("resolved_imports", resolved_count.to_string())
        .with_evidence("module_edges", edge_count.to_string())
        .with_evidence("symbols", symbol_count.to_string())
        .with_evidence("references", reference_count.to_string())
        .with_evidence("file_metrics", file_metric_count.to_string())
        .with_evidence("function_metrics", function_metric_count.to_string())
        .with_evidence("complexity_metrics", complexity_metric_count.to_string()),
    );
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![abstract_domains_public_probe()])
}
"#;

fn assert_abstract_domains_public_rule_source(source: &str) {
    assert!(
        source.contains("use polint::sdk::prelude::*;"),
        "external rule must import the public SDK prelude only:\n{source}"
    );
    assert!(
        source.contains("ResolvedImports<'_>")
            && source.contains("ModuleGraphFacts<'_>")
            && source.contains("Symbols<'_>")
            && source.contains("References<'_>")
            && source.contains("FileMetrics<'_>")
            && source.contains("FunctionMetrics<'_>")
            && source.contains("ComplexityMetrics<'_>"),
        "external rule must request supported public fact views through its signature:\n{source}"
    );
    for forbidden in [
        concat!("polint::", "analysis"),
        concat!("polint::", "analysis_kernel"),
        concat!("polint::", "core"),
        concat!("polint::", "graph"),
        concat!("polint::", "go"),
        concat!("polint::", "ts"),
        "Capabilities::new(",
        "impl Rule for",
    ] {
        assert!(
            !source.contains(forbidden),
            "external rule source must not depend on internal API `{forbidden}`:\n{source}"
        );
    }
    assert_abstract_domains_public_output_is_private(source);
}

fn write_abstract_domains_public_fixture_files(root: &Path) {
    write_file(
        &root.join("web/package.json"),
        r#"{"name":"abstract-domains-public-fixture","private":true,"type":"module"}"#,
    );
    write_file(
        &root.join("web/src/lib.ts"),
        r#"export function helper(input: string) {
  return input.trim();
}
"#,
    );
    write_file(
        &root.join("web/src/app.ts"),
        r#"import { helper } from "./lib";

export function render(name: string) {
  return helper(name);
}
"#,
    );
}

fn write_direct_calls_public_rule_repo(root: &Path) {
    assert_direct_calls_public_rule_source(DIRECT_CALLS_PUBLIC_RULE_SOURCE);
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint.toml"),
        r#"
[workspace]
include = ["src/**", "web/src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#,
        ),
    );
    write_file(
        &root.join(".polint/rules/src/main.rs"),
        DIRECT_CALLS_PUBLIC_RULE_SOURCE,
    );
    write_direct_calls_public_fixture_files(root);

    let case_root = root.join(".polint/tests/rules/direct_calls_public/basic");
    write_file(
        &case_root.join("polint-test.toml"),
        r#"
rule = "local/direct-calls-public-probe"
paths = ["src/**", "web/src/**"]

[[expect.diagnostic]]
rule_id = "local/direct-calls-public-probe"
file = "<workspace>"
severity = "warn"
message_contains = "public facts remain compatible while direct calls stay private"
"#,
    );
    write_direct_calls_public_fixture_files(&case_root);
}

const DIRECT_CALLS_PUBLIC_RULE_SOURCE: &str = r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/direct-calls-public-probe",
    description = "Reads supported public facts while direct call facts stay private.",
    severity = "warn"
)]
fn direct_calls_public_probe(
    ctx: &mut RuleCtx<'_>,
    resolved: ResolvedImports<'_>,
    graph: ModuleGraphFacts<'_>,
    symbols: Symbols<'_>,
    references: References<'_>,
) -> RuleResult {
    let resolved_count = resolved.iter().count();
    let edge_count = graph.edges().len();
    let symbol_count = symbols.iter().count();
    let reference_count = references.iter().count();
    if resolved_count == 0 && edge_count == 0 && symbol_count == 0 && reference_count == 0 {
        return Ok(());
    }

    ctx.report(
        Diagnostic::warning(
            ctx.rule_id(),
            "<workspace>",
            DiagnosticRange::point(1, 1),
            "public facts remain compatible while direct calls stay private",
        )
        .with_evidence("resolved_imports", resolved_count.to_string())
        .with_evidence("module_edges", edge_count.to_string())
        .with_evidence("symbols", symbol_count.to_string())
        .with_evidence("references", reference_count.to_string()),
    );
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![direct_calls_public_probe()])
}
"#;

fn assert_direct_calls_public_rule_source(source: &str) {
    assert!(
        source.contains("use polint::sdk::prelude::*;"),
        "external rule must import the public SDK prelude only:\n{source}"
    );
    assert!(
        source.contains("ResolvedImports<'_>")
            && source.contains("ModuleGraphFacts<'_>")
            && source.contains("Symbols<'_>")
            && source.contains("References<'_>"),
        "external rule must request supported public fact views through its signature:\n{source}"
    );
    for forbidden in [
        concat!("polint::", "analysis"),
        concat!("polint::", "analysis_kernel"),
        concat!("polint::", "core"),
        concat!("polint::", "graph"),
        concat!("polint::", "go"),
        concat!("polint::", "ts"),
        "Capabilities::new(",
        "impl Rule for",
    ] {
        assert!(
            !source.contains(forbidden),
            "external rule source must not depend on internal API `{forbidden}`:\n{source}"
        );
    }
    assert_direct_calls_public_output_is_private(source);
}

fn write_direct_calls_public_fixture_files(root: &Path) {
    write_file(
        &root.join("go.mod"),
        r#"module example.com/directcallspublic

go 1.22
"#,
    );
    write_file(
        &root.join("src/service/service.go"),
        r#"package service

import "fmt"

func FormatUser(name string) string {
	if name == "" {
		return fmt.Sprint("anonymous")
	}
	return name
}
"#,
    );
    write_file(
        &root.join("web/package.json"),
        r#"{"name":"direct-calls-public-fixture","private":true,"type":"module"}"#,
    );
    write_file(
        &root.join("web/src/lib.ts"),
        r#"export function helper(input: string) {
  return input.trim();
}
"#,
    );
    write_file(
        &root.join("web/src/app.ts"),
        r#"import { helper } from "./lib";

export function render(name: string) {
  return helper(name);
}
"#,
    );
}

fn write_semantic_mir_public_rule_repo(root: &Path) {
    assert_semantic_mir_public_rule_source(SEMANTIC_MIR_PUBLIC_RULE_SOURCE);
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint.toml"),
        r#"
[workspace]
include = ["src/**", "web/src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#,
        ),
    );
    write_file(
        &root.join(".polint/rules/src/main.rs"),
        SEMANTIC_MIR_PUBLIC_RULE_SOURCE,
    );
    write_semantic_mir_fixture_files(root);

    let case_root = root.join(".polint/tests/rules/semantic_mir_public/basic");
    write_file(
        &case_root.join("polint-test.toml"),
        r#"
rule = "local/semantic-mir-public-probe"
paths = ["src/**", "web/src/**"]

[[expect.diagnostic]]
rule_id = "local/semantic-mir-public-probe"
file = "<workspace>"
severity = "warn"
message_contains = "public facts remain compatible"
"#,
    );
    write_semantic_mir_fixture_files(&case_root);
}

const SEMANTIC_MIR_PUBLIC_RULE_SOURCE: &str = r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/semantic-mir-public-probe",
    description = "Reads supported public facts while semantic MIR stays private.",
    severity = "warn"
)]
fn semantic_mir_public_probe(
    ctx: &mut RuleCtx<'_>,
    resolved: ResolvedImports<'_>,
    graph: ModuleGraphFacts<'_>,
    symbols: Symbols<'_>,
    references: References<'_>,
) -> RuleResult {
    let resolved_count = resolved.iter().count();
    let edge_count = graph.edges().len();
    let symbol_count = symbols.iter().count();
    let reference_count = references.iter().count();
    if resolved_count == 0 && edge_count == 0 && symbol_count == 0 && reference_count == 0 {
        return Ok(());
    }

    ctx.report(
        Diagnostic::warning(
            ctx.rule_id(),
            "<workspace>",
            DiagnosticRange::point(1, 1),
            "public facts remain compatible",
        )
        .with_evidence("resolved_imports", resolved_count.to_string())
        .with_evidence("module_edges", edge_count.to_string())
        .with_evidence("symbols", symbol_count.to_string())
        .with_evidence("references", reference_count.to_string()),
    );
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![semantic_mir_public_probe()])
}
"#;

fn write_semantic_mir_fixture_files(root: &Path) {
    write_file(
        &root.join("go.mod"),
        r#"module example.com/semanticmirpublic

go 1.22
"#,
    );
    write_file(
        &root.join("src/service/service.go"),
        r#"package service

import "fmt"

func FormatUser(name string) string {
    if name == "" {
        return fmt.Sprint("anonymous")
    }
    return name
}
"#,
    );
    write_file(
        &root.join("web/package.json"),
        r#"{"name":"semantic-mir-public-fixture","private":true,"type":"module"}"#,
    );
    write_file(
        &root.join("web/src/lib.ts"),
        r#"export function helper(input: string) {
  return input.trim();
}
"#,
    );
    write_file(
        &root.join("web/src/app.ts"),
        r#"import { helper } from "./lib";

export function render(name: string) {
  return helper(name);
}
"#,
    );
    write_file(
        &root.join("web/src/widget.js"),
        r#"export function widget(items) {
  return items;
}
"#,
    );
}

#[test]
fn module_topology_internals_stay_private() {
    let temp = tempfile::tempdir().unwrap();
    write_module_topology_public_rule_repo(temp.path());

    let check_json = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    let report: polint::sdk::prelude::PolintReport = serde_json::from_str(&check_json)
        .unwrap_or_else(|error| panic!("stdout was not public check JSON: {error}\n{check_json}"));
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "local/module-relationships-public"),
        "external module relationship rule should run through public SDK views: {report:#?}"
    );

    let inspect_json = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["inspect", "rule", "--format", "json"])
            .assert()
            .success(),
    );
    let inspect: serde_json::Value = serde_json::from_str(&inspect_json)
        .unwrap_or_else(|error| panic!("stdout was not inspect JSON: {error}\n{inspect_json}"));
    assert_eq!(
        inspect["rules"][0]["rule_id"],
        "local/module-relationships-public"
    );
    let fact_views = inspect["rules"][0]["fact_views"]
        .as_array()
        .unwrap_or_else(|| panic!("fact_views should be an array: {inspect:#?}"));
    assert!(fact_views.iter().any(|view| {
        view["canonical_path"] == "polint::sdk::facts::ResolvedImports<'_>"
            && view["capability"] == "resolved_imports"
    }));
    assert!(fact_views.iter().any(|view| {
        view["canonical_path"] == "polint::sdk::facts::ModuleGraphFacts<'_>"
            && view["capability"] == "module_graph"
    }));

    let test_json = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["test", "--format", "json"])
            .assert()
            .success(),
    );
    let test_report: serde_json::Value = serde_json::from_str(&test_json)
        .unwrap_or_else(|error| panic!("stdout was not test JSON: {error}\n{test_json}"));
    assert_eq!(test_report["summary"]["failed"], 0);

    for output in [&check_json, &inspect_json, &test_json] {
        assert_module_topology_public_output_is_private(output);
    }
    assert_module_topology_public_surfaces_are_private();
    assert_module_topology_cli_help_is_private();

    let source = fs::read_to_string(temp.path().join(".polint/rules/src/main.rs")).unwrap();
    assert!(source.contains("use polint::sdk::prelude::*;"));
    assert!(source.contains("ResolvedImports<'_>"));
    assert!(source.contains("ModuleGraphFacts<'_>"));
    assert!(source.contains("polint::runner::run_cli"));
    assert_public_relationship_rule_source(&source);

    let docs = fs::read_to_string(repo_root().join("docs/facts/imports.md")).unwrap();
    assert!(
        docs.contains(
            "ResolvedImports<'_>` and `ModuleGraphFacts<'_>` remain the supported public relationship surfaces"
        ),
        "public import docs should clarify that richer topology internals are not SDK views"
    );
}

fn assert_module_topology_public_output_is_private(output: &str) {
    for marker in MODULE_TOPOLOGY_INTERNAL_PUBLIC_MARKERS {
        assert!(
            !output.contains(marker),
            "public output must not leak module-topology internal marker `{marker}`:\n{output}"
        );
    }
}

fn assert_module_topology_public_surfaces_are_private() {
    let root = repo_root();
    let mut public_surface = String::new();
    for path in [
        root.join("crates/polint/src/lib.rs"),
        root.join("crates/polint/src/sdk"),
        root.join("crates/polint/src/runner"),
        root.join("crates/polint/src/cli"),
    ] {
        public_surface.push_str(&source_tree_text(&path));
    }

    for marker in MODULE_TOPOLOGY_INTERNAL_PUBLIC_MARKERS {
        assert!(
            !public_surface.contains(marker),
            "public crate-root/CLI/SDK/runner source must not expose module-topology marker `{marker}`"
        );
    }
}

fn assert_module_topology_cli_help_is_private() {
    let help_outputs = public_rule_help_outputs();

    for help in help_outputs {
        for marker in [
            "topology",
            "Packages",
            "Dependencies",
            "SourceSets",
            "RepoTopology",
        ] {
            assert!(
                !help.contains(marker),
                "public CLI help must not expose module-topology marker `{marker}`:\n{help}"
            );
        }
    }
}

const MODULE_TOPOLOGY_INTERNAL_PUBLIC_MARKERS: &[&str] = &[
    "workspace_roots",
    "topology_packages",
    "source_sets",
    "dependency_requirements",
    "resolved_dependency_edges",
    "import_to_package_edges",
    "repo_topology_overlays",
    "polint.module_topology",
    "module-topology-facts",
    "Packages<'_>",
    "Dependencies<'_>",
    "SourceSets<'_>",
    "RepoTopology<'_>",
];

fn assert_public_relationship_rule_source(source: &str) {
    assert!(
        source.contains("use polint::sdk::prelude::*;"),
        "external rule must import the public SDK prelude only:\n{source}"
    );
    assert!(
        source.contains("ResolvedImports<'_>") && source.contains("ModuleGraphFacts<'_>"),
        "external rule must request public relationship facts through its signature:\n{source}"
    );
    for forbidden in [
        concat!("polint::", "core"),
        concat!("polint::", "module_graph"),
        concat!("polint::", "analysis_kernel"),
        concat!("polint::", "go"),
        concat!("polint::", "ts"),
        "Capabilities::new(",
        "impl Rule for",
        "workspace_roots",
        "topology_packages",
        "source_sets",
        "dependency_requirements",
        "resolved_dependency_edges",
        "import_to_package_edges",
        "repo_topology_overlays",
        "polint.module_topology",
        "module-topology-facts",
    ] {
        assert!(
            !source.contains(forbidden),
            "external rule source must not depend on internal API `{forbidden}`:\n{source}"
        );
    }
}

fn write_module_topology_public_rule_repo(root: &Path) {
    assert_public_relationship_rule_source(MODULE_TOPOLOGY_PUBLIC_RULE_SOURCE);
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#,
        ),
    );
    write_file(
        &root.join(".polint/rules/src/main.rs"),
        MODULE_TOPOLOGY_PUBLIC_RULE_SOURCE,
    );
    write_module_topology_fixture_files(root);

    let case_root = root.join(".polint/tests/rules/module_relationships/basic");
    write_file(
        &case_root.join("polint-test.toml"),
        r#"
rule = "local/module-relationships-public"
paths = ["src/**"]

[[expect.diagnostic]]
rule_id = "local/module-relationships-public"
file = "<workspace>"
severity = "warn"
message_contains = "public relationship facts remain available"
"#,
    );
    write_module_topology_fixture_files(&case_root);
}

const MODULE_TOPOLOGY_PUBLIC_RULE_SOURCE: &str = r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/module-relationships-public",
    description = "Reads public relationship facts without topology internals.",
    severity = "warn"
)]
fn module_relationships_public(
    ctx: &mut RuleCtx<'_>,
    resolved: ResolvedImports<'_>,
    graph: ModuleGraphFacts<'_>,
) -> RuleResult {
    let resolved_count = resolved.iter().count();
    let unresolved_count = resolved
        .iter()
        .filter(|import| {
            !matches!(
                import.status,
                ResolutionStatus::Resolved | ResolutionStatus::External
            )
        })
        .count();
    let edge_count = graph.edges().len();
    if resolved_count == 0 && edge_count == 0 {
        return Ok(());
    }

    ctx.report(
        Diagnostic::warning(
            ctx.rule_id(),
            "<workspace>",
            DiagnosticRange::point(1, 1),
            "public relationship facts remain available",
        )
        .with_evidence("resolved_imports", resolved_count.to_string())
        .with_evidence("unresolved_imports", unresolved_count.to_string())
        .with_evidence("module_edges", edge_count.to_string()),
    );
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![module_relationships_public()])
}
"#;

fn write_module_topology_fixture_files(root: &Path) {
    write_file(
        &root.join("package.json"),
        r#"{"name":"module-topology-public-fixture","private":true,"type":"module"}"#,
    );
    write_file(
        &root.join("tsconfig.json"),
        r#"{
  "compilerOptions": {
    "baseUrl": ".",
    "paths": {
      "@domain/*": ["src/domain/*"]
    }
  }
}
"#,
    );
    write_file(
        &root.join("src/domain/model.ts"),
        r#"export const model = "domain";"#,
    );
    write_file(
        &root.join("src/ui/view.ts"),
        r#"import { model } from "../domain/model";
import { missing } from "./missing";

export const view = `${model}:${missing}`;
"#,
    );
}

fn write_semantic_index_public_rule_repo(root: &Path) {
    write_external_symbol_rule_repo(
        root,
        r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/semantic-public-probe",
    description = "Reads public symbols and references after semantic deepening.",
    severity = "warn"
)]
fn semantic_public_probe(
    ctx: &mut RuleCtx<'_>,
    symbols: Symbols<'_>,
    references: References<'_>,
) -> RuleResult {
    let symbol_count = symbols.iter().count();
    let reference_count = references.iter().count();
    if symbol_count == 0 && reference_count == 0 {
        return Ok(());
    }

    ctx.report(
        Diagnostic::warning(
            ctx.rule_id(),
            "<workspace>",
            DiagnosticRange::point(1, 1),
            "public symbol/reference facts remain available",
        )
        .with_evidence("symbols", symbol_count.to_string())
        .with_evidence("references", reference_count.to_string())
        .with_evidence("unresolved", references.unresolved().count().to_string()),
    );
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![semantic_public_probe()])
}
"#,
    );
    write_file(
        &root.join("src/app.ts"),
        r#"import { handler } from "./lib";

export function run() {
  return handler();
}
"#,
    );
    write_file(
        &root.join("src/lib.ts"),
        r#"export function handler() {
  return "ok";
}
"#,
    );
    write_file(
        &root.join(".polint/tests/rules/semantic_public/basic/polint-test.toml"),
        r#"
rule = "local/semantic-public-probe"
paths = ["src/**"]

[[expect.diagnostic]]
rule_id = "local/semantic-public-probe"
file = "<workspace>"
severity = "warn"
message_contains = "public symbol/reference facts remain available"
"#,
    );
    write_file(
        &root.join(".polint/tests/rules/semantic_public/basic/src/app.ts"),
        r#"import { handler } from "./lib";

export function run() {
  return handler();
}
"#,
    );
    write_file(
        &root.join(".polint/tests/rules/semantic_public/basic/src/lib.ts"),
        r#"export function handler() {
  return "ok";
}
"#,
    );
}

fn write_layer_cache_public_rule_repo(root: &Path) {
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#,
        ),
    );
    write_file(
        &root.join(".polint/rules/src/main.rs"),
        r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/layer-cache-public-probe",
    description = "Reads cached fact views without exposing cache internals.",
    severity = "warn"
)]
fn layer_cache_public_probe(
    _ctx: &mut RuleCtx<'_>,
    imports: Imports<'_>,
    module_graph: ModuleGraphFacts<'_>,
    symbols: Symbols<'_>,
    references: References<'_>,
    file_metrics: FileMetrics<'_>,
    function_metrics: FunctionMetrics<'_>,
    complexity_metrics: ComplexityMetrics<'_>,
) -> RuleResult {
    let _observed_counts = (
        imports.iter().count(),
        module_graph.nodes().len(),
        module_graph.edges().len(),
        symbols.iter().count(),
        references.iter().count(),
        file_metrics.iter().count(),
        function_metrics.iter().count(),
        complexity_metrics.iter().count(),
    );
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![layer_cache_public_probe()])
}
"#,
    );
    write_file(
        &root.join("src/token.ts"),
        r#"export const token = "ok";
"#,
    );
    write_file(
        &root.join("src/app.ts"),
        r#"import { token } from "./token";

export function answer(input: number) {
  const localValue = token;
  return `${localValue}:${input + 1}`;
}

export const value = answer(41);
"#,
    );
}

fn assert_layer_cache_public_json_is_private(public_json: &str) {
    for marker in INTERNAL_LAYER_CACHE_PUBLIC_MARKERS {
        assert!(
            !public_json.contains(marker),
            "public check JSON must not leak layer-cache marker `{marker}`:\n{public_json}"
        );
    }
}

fn assert_layer_cache_public_surfaces_are_private() {
    let root = repo_root();
    let mut public_surface = String::new();
    for path in [
        root.join("crates/polint/src/lib.rs"),
        root.join("crates/polint/src/sdk"),
        root.join("crates/polint/src/runner"),
        root.join("crates/polint/src/cli"),
    ] {
        public_surface.push_str(&source_tree_text(&path));
    }

    for marker in INTERNAL_LAYER_CACHE_PUBLIC_MARKERS {
        assert!(
            !public_surface.contains(marker),
            "public crate-root/CLI/SDK/runner source must not expose layer-cache marker `{marker}`"
        );
    }
}

fn assert_layer_cache_cli_help_is_private() {
    let help_outputs = [
        polint_help(&["--help"]),
        polint_help(&["check", "--help"]),
        polint_help(&["cache", "--help"]),
        polint_help(&["cache", "status", "--help"]),
    ];

    for help in help_outputs {
        let normalized = help.to_lowercase();
        for marker in [
            "layer cache",
            "cache stats",
            "provider output",
            "provider outputs",
            "dependency index",
            "invalidation",
            "verified reuse",
            "invalid evicted",
        ] {
            assert!(
                !normalized.contains(marker),
                "public CLI help must not expose layer-cache internals `{marker}`:\n{help}"
            );
        }
    }
}

fn source_tree_text(path: &Path) -> String {
    if path.is_file() {
        return fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("read source file {}: {error}", path.display()));
    }

    let mut files = Vec::new();
    collect_rust_files(path, &mut files);
    files.sort();
    files
        .into_iter()
        .map(|file| {
            fs::read_to_string(&file)
                .unwrap_or_else(|error| panic!("read source file {}: {error}", file.display()))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn public_surface_text(path: &Path) -> String {
    if path.is_file() {
        return fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("read public file {}: {error}", path.display()));
    }

    let mut files = Vec::new();
    collect_public_surface_files(path, &mut files);
    files.sort();
    files
        .into_iter()
        .map(|file| {
            fs::read_to_string(&file)
                .unwrap_or_else(|error| panic!("read public file {}: {error}", file.display()))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn collect_public_surface_files(path: &Path, files: &mut Vec<PathBuf>) {
    for entry in
        fs::read_dir(path).unwrap_or_else(|error| panic!("read dir {}: {error}", path.display()))
    {
        let path = entry
            .unwrap_or_else(|error| panic!("read dir entry under {}: {error}", path.display()))
            .path();
        if path.is_dir() {
            if path
                .file_name()
                .is_some_and(|name| name == "architecture-review")
            {
                continue;
            }
            collect_public_surface_files(&path, files);
        } else if path.extension().is_some_and(|extension| {
            matches!(
                extension.to_string_lossy().as_ref(),
                "json" | "md" | "rs" | "toml" | "txt"
            )
        }) {
            files.push(path);
        }
    }
}

fn collect_rust_files(path: &Path, files: &mut Vec<PathBuf>) {
    for entry in
        fs::read_dir(path).unwrap_or_else(|error| panic!("read dir {}: {error}", path.display()))
    {
        let path = entry
            .unwrap_or_else(|error| panic!("read dir entry under {}: {error}", path.display()))
            .path();
        if path.is_dir() {
            collect_rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn write_plan_capability_rule_repo(root: &Path) {
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#,
        ),
    );
    write_file(
        &root.join(".polint/rules/src/main.rs"),
        r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/needs-imports",
    description = "Needs import facts.",
    severity = "warn"
)]
fn needs_imports(_ctx: &mut RuleCtx<'_>, imports: Imports<'_>) -> RuleResult {
    let _import_count = imports.iter().count();
    Ok(())
}

#[polint::rule(
    id = "local/needs-cfg",
    description = "Needs CFG facts.",
    severity = "warn"
)]
fn needs_cfg(_ctx: &mut RuleCtx<'_>, _cfg: Cfg<'_>) -> RuleResult {
    _ctx.warn(
        &Span::point(FileId::from_raw(0), 1, 1),
        "this should not run while cfg is unsupported",
    );
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![needs_imports(), needs_cfg()])
}
"#,
    );
    write_file(
        &root.join("src/component.ts"),
        r#"import { token } from "./token";

export const value = token;
"#,
    );
    write_file(&root.join("src/token.ts"), r#"export const token = "ok";"#);
}

fn write_call_graph_capability_rule_repo(root: &Path) {
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#,
        ),
    );
    write_file(
        &root.join(".polint/rules/src/main.rs"),
        r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/needs-call-graph",
    description = "Needs call graph facts.",
    severity = "warn"
)]
fn needs_call_graph(ctx: &mut RuleCtx<'_>, _call_graph: CallGraph<'_>) -> RuleResult {
    ctx.report(Diagnostic::warning(
        ctx.rule_id(),
        "<workspace>",
        DiagnosticRange::point(1, 1),
        "this should not run while call_graph is unsupported",
    ));
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![needs_call_graph()])
}
"#,
    );
    write_file(
        &root.join("src/component.ts"),
        r#"export function component() {
  return "ok";
}
"#,
    );
}

fn write_symbol_capability_rule_repo(root: &Path) {
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#,
        ),
    );
    write_file(
        &root.join(".polint/rules/src/main.rs"),
        r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/needs-references",
    description = "Needs reference facts.",
    severity = "warn"
)]
fn needs_references(ctx: &mut RuleCtx<'_>, references: References<'_>) -> RuleResult {
    let reference_count = references.iter().count();
    if reference_count > 0 {
        ctx.report(
            Diagnostic::warning(
                ctx.rule_id(),
                "<workspace>",
                DiagnosticRange::point(1, 1),
                "symbol reference rule ran with provider facts",
            )
            .with_evidence("reference_count", reference_count.to_string()),
        );
    }
    Ok(())
}

#[polint::rule(
    id = "local/supported-imports",
    description = "Reports after supported import facts.",
    severity = "warn"
)]
fn supported_imports(ctx: &mut RuleCtx<'_>, imports: Imports<'_>) -> RuleResult {
    let _import_count = imports.iter().count();
    ctx.report(Diagnostic::warning(
        ctx.rule_id(),
        "<workspace>",
        DiagnosticRange::point(1, 1),
        "supported rule ran after provider diagnostics",
    ));
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![needs_references(), supported_imports()])
}
"#,
    );
    write_file(
        &root.join("src/component.ts"),
        r#"import { token } from "./token";

export const value = token;
"#,
    );
    write_file(&root.join("src/token.ts"), r#"export const token = "ok";"#);
}

fn write_symbol_reference_cache_rule_repo(root: &Path) {
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#,
        ),
    );
    write_file(
        &root.join(".polint/rules/src/main.rs"),
        r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/stable-symbol-reference",
    description = "Reports stable symbol and reference IDs.",
    severity = "warn"
)]
fn stable_symbol_reference(
    ctx: &mut RuleCtx<'_>,
    symbols: Symbols<'_>,
    references: References<'_>,
) -> RuleResult {
    let Some(symbol) = symbols.by_name("answer").next() else {
        return Ok(());
    };
    let Some(reference) = references.to(symbol.id).next() else {
        return Ok(());
    };
    let Some(file) = reference.file else {
        return Ok(());
    };

    ctx.report(
        Diagnostic::warning(
            ctx.rule_id(),
            ctx.file_path(file),
            DiagnosticRange::point(1, 1),
            "stable symbol/reference evidence",
        )
        .with_evidence("symbol_name", symbol.name.clone())
        .with_evidence("symbol_kind", format!("{:?}", symbol.kind))
        .with_evidence("symbol_precision", format!("{:?}", symbol.precision))
        .with_evidence("symbol_id", symbol.id.0.to_string())
        .with_evidence("reference_id", reference.id.0.to_string())
        .with_evidence(
            "reference_target",
            reference
                .target
                .map(|target| target.0.to_string())
                .unwrap_or_else(|| "none".to_string()),
        )
        .with_evidence("reference_status", format!("{:?}", reference.status))
        .with_evidence("reference_precision", format!("{:?}", reference.precision)),
    );
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![stable_symbol_reference()])
}
"#,
    );
    write_file(
        &root.join("src/component.ts"),
        r#"export function answer(input: number) {
  return input + 1;
}

export const value = answer(41);
"#,
    );
}

fn write_go_symbol_setup_missing_rule_repo(root: &Path) {
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#,
        ),
    );
    write_file(
        &root.join(".polint/rules/src/main.rs"),
        r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/go-symbols",
    description = "Reads Go symbols and references.",
    severity = "warn"
)]
fn go_symbols(
    ctx: &mut RuleCtx<'_>,
    symbols: Symbols<'_>,
    references: References<'_>,
) -> RuleResult {
    let _fact_count = symbols.iter().count() + references.iter().count();
    ctx.report(Diagnostic::warning(
        ctx.rule_id(),
        "<workspace>",
        DiagnosticRange::point(1, 1),
        "this rule should be blocked by Go setup-missing support",
    ));
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![go_symbols()])
}
"#,
    );
    write_file(
        &root.join("src/main.go"),
        r#"package main

func Build() string {
	return "ok"
}

func main() {
	_ = Build()
}
"#,
    );
}

fn assert_public_symbol_rule_source(source: &str) {
    assert!(
        source.contains("use polint::sdk::prelude::*;"),
        "external rule must import the public SDK prelude only:\n{source}"
    );
    assert!(
        source.contains("Symbols<'_>") || source.contains("References<'_>"),
        "external rule must request typed symbol/reference facts through its signature:\n{source}"
    );
    for forbidden in [
        concat!("polint::", "core"),
        concat!("polint::", "go"),
        concat!("polint::", "ts"),
        concat!("oxc", "_"),
        concat!("Go", "Symbol"),
        concat!("Go", "Sidecar"),
        "Capabilities::new(",
        "impl Rule for",
    ] {
        assert!(
            !source.contains(forbidden),
            "external rule source must not depend on internal API `{forbidden}`:\n{source}"
        );
    }
}

fn write_external_symbol_rule_repo(root: &Path, rule_source: &str) {
    assert_public_symbol_rule_source(rule_source);
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#,
        ),
    );
    write_file(&root.join(".polint/rules/src/main.rs"), rule_source);
}

fn write_external_ts_symbol_reference_rule_repo(root: &Path) {
    write_external_symbol_rule_repo(
        root,
        r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

fn diagnostic_file(ctx: &RuleCtx<'_>, file: Option<FileId>) -> String {
    file.map(|file| ctx.file_path(file))
        .unwrap_or_else(|| "<workspace>".to_string())
}

fn reference_target(reference: &ReferenceFact) -> String {
    reference
        .target
        .map(|target| target.0.to_string())
        .unwrap_or_else(|| "none".to_string())
}

fn report_symbol(
    ctx: &mut RuleCtx<'_>,
    symbols: Symbols<'_>,
    case: &str,
    symbol: &SymbolFact,
    extra_label: &str,
    extra_value: String,
) {
    ctx.report(
        Diagnostic::warning(
            ctx.rule_id(),
            diagnostic_file(ctx, symbol.file),
            DiagnosticRange::point(1, 1),
            format!("public symbol SDK evidence: {case}"),
        )
        .with_evidence("case", case)
        .with_evidence("symbol_name", symbol.name.clone())
        .with_evidence("symbol_kind", format!("{:?}", symbol.kind))
        .with_evidence("symbol_precision", format!("{:?}", symbol.precision))
        .with_evidence("symbol_id", symbol.id.0.to_string())
        .with_evidence("symbol_stable_key", symbols.stable_key(symbol).to_string())
        .with_evidence(extra_label, extra_value),
    );
}

fn report_reference(
    ctx: &mut RuleCtx<'_>,
    references: References<'_>,
    case: &str,
    symbol: Option<&SymbolFact>,
    reference: &ReferenceFact,
) {
    let file = reference.file.or_else(|| symbol.and_then(|symbol| symbol.file));
    let symbol_name = symbol
        .map(|symbol| symbol.name.clone())
        .unwrap_or_else(|| "none".to_string());
    let symbol_kind = symbol
        .map(|symbol| format!("{:?}", symbol.kind))
        .unwrap_or_else(|| "none".to_string());
    let symbol_precision = symbol
        .map(|symbol| format!("{:?}", symbol.precision))
        .unwrap_or_else(|| "none".to_string());
    let symbol_id = symbol
        .map(|symbol| symbol.id.0.to_string())
        .unwrap_or_else(|| "none".to_string());

    ctx.report(
        Diagnostic::warning(
            ctx.rule_id(),
            diagnostic_file(ctx, file),
            DiagnosticRange::point(1, 1),
            format!("public reference SDK evidence: {case}"),
        )
        .with_evidence("case", case)
        .with_evidence("symbol_name", symbol_name)
        .with_evidence("symbol_kind", symbol_kind)
        .with_evidence("symbol_precision", symbol_precision)
        .with_evidence("symbol_id", symbol_id)
        .with_evidence("reference_name", reference.name.clone())
        .with_evidence("reference_kind", format!("{:?}", reference.kind))
        .with_evidence("reference_precision", format!("{:?}", reference.precision))
        .with_evidence("reference_status", format!("{:?}", reference.status))
        .with_evidence("reference_id", reference.id.0.to_string())
        .with_evidence("reference_target", reference_target(reference))
        .with_evidence(
            "reference_stable_key",
            references.stable_key(reference).to_string(),
        ),
    );
}

#[polint::rule(
    id = "local/ts-symbol-reference-public-sdk",
    description = "Reads TypeScript symbol and reference facts through public SDK views.",
    severity = "warn"
)]
fn ts_symbol_reference_public_sdk(
    ctx: &mut RuleCtx<'_>,
    symbols: Symbols<'_>,
    references: References<'_>,
) -> RuleResult {
    if let Some(symbol) = symbols
        .by_name("answer")
        .find(|symbol| symbol.kind == SymbolKind::Function)
    {
        report_symbol(ctx, symbols, "ts-symbol:function", symbol, "definition_count", symbols.definitions(symbol.id).count().to_string());
        if let Some(reference) = references
            .to(symbol.id)
            .find(|reference| reference.kind == ReferenceKind::Call)
        {
            report_reference(ctx, references, "ts-reference:function-call", Some(symbol), reference);
        }
    }

    if let Some(symbol) = symbols
        .by_name("localValue")
        .find(|symbol| symbol.kind == SymbolKind::Variable)
    {
        report_symbol(ctx, symbols, "ts-symbol:local-variable", symbol, "definition_count", symbols.definitions(symbol.id).count().to_string());
        if let Some(reference) = references
            .to(symbol.id)
            .find(|reference| reference.kind == ReferenceKind::Read)
        {
            report_reference(ctx, references, "ts-reference:local-variable", Some(symbol), reference);
        }
    }

    let merge_symbols = symbols.by_name("MergeMe").collect::<Vec<_>>();
    if let Some(symbol) = merge_symbols.first().copied() {
        let declaration_count = merge_symbols
            .iter()
            .map(|symbol| symbols.definitions(symbol.id).count())
            .sum::<usize>();
        if declaration_count >= 2 || merge_symbols.len() >= 2 {
            report_symbol(
                ctx,
                symbols,
                "ts-symbol:declaration-merge",
                symbol,
                "declaration_count",
                declaration_count.max(merge_symbols.len()).to_string(),
            );
        }
    }

    if let Some(reference) = references.iter().find(|reference| {
        reference.name == "importedToken"
            && reference.kind == ReferenceKind::Import
            && reference.precision == SymbolPrecision::ModuleLinked
    }) {
        let symbol = reference.target.and_then(|target| symbols.get(target));
        report_reference(ctx, references, "ts-reference:module-import", symbol, reference);
    }

    if let Some(reference) = references
        .unresolved()
        .find(|reference| reference.name == "missingGlobal")
    {
        report_reference(ctx, references, "ts-reference:unresolved-global", None, reference);
    }

    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![ts_symbol_reference_public_sdk()])
}
"#,
    );
    write_file(
        &root.join("src/tokens.ts"),
        r#"export const token = "ok";
"#,
    );
    write_file(
        &root.join("src/component.ts"),
        r#"import { token as importedToken } from "./tokens";

export interface MergeMe {
  first: string;
}

export interface MergeMe {
  second: string;
}

export function answer(input: number) {
  let localValue = importedToken;
  return missingGlobal + localValue.length;
}

export const value = answer(41);
"#,
    );
}

fn write_external_go_symbol_reference_rule_repo(root: &Path) {
    write_external_symbol_rule_repo(
        root,
        r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

fn diagnostic_file(ctx: &RuleCtx<'_>, file: Option<FileId>) -> String {
    file.map(|file| ctx.file_path(file))
        .unwrap_or_else(|| "<workspace>".to_string())
}

fn reference_target(reference: &ReferenceFact) -> String {
    reference
        .target
        .map(|target| target.0.to_string())
        .unwrap_or_else(|| "none".to_string())
}

fn report_reference(
    ctx: &mut RuleCtx<'_>,
    symbols: Symbols<'_>,
    references: References<'_>,
    case: &str,
    symbol: &SymbolFact,
    reference: &ReferenceFact,
) {
    ctx.report(
        Diagnostic::warning(
            ctx.rule_id(),
            diagnostic_file(ctx, reference.file.or(symbol.file)),
            DiagnosticRange::point(1, 1),
            format!("public Go reference SDK evidence: {case}"),
        )
        .with_evidence("case", case)
        .with_evidence("symbol_name", symbol.name.clone())
        .with_evidence("symbol_kind", format!("{:?}", symbol.kind))
        .with_evidence("symbol_precision", format!("{:?}", symbol.precision))
        .with_evidence("symbol_id", symbol.id.0.to_string())
        .with_evidence("symbol_stable_key", symbols.stable_key(symbol).to_string())
        .with_evidence("reference_name", reference.name.clone())
        .with_evidence("reference_kind", format!("{:?}", reference.kind))
        .with_evidence("reference_precision", format!("{:?}", reference.precision))
        .with_evidence("reference_status", format!("{:?}", reference.status))
        .with_evidence("reference_id", reference.id.0.to_string())
        .with_evidence("reference_target", reference_target(reference))
        .with_evidence(
            "reference_stable_key",
            references.stable_key(reference).to_string(),
        ),
    );
}

#[polint::rule(
    id = "local/go-symbol-reference-public-sdk",
    description = "Reads Go symbol and reference facts through public SDK views.",
    severity = "warn"
)]
fn go_symbol_reference_public_sdk(
    ctx: &mut RuleCtx<'_>,
    symbols: Symbols<'_>,
    references: References<'_>,
) -> RuleResult {
    if let Some(symbol) = symbols
        .by_name("Build")
        .find(|symbol| symbol.kind == SymbolKind::Function)
    {
        if let Some(reference) = references
            .to(symbol.id)
            .find(|reference| reference.kind == ReferenceKind::Call)
        {
            report_reference(ctx, symbols, references, "go-reference:function-call", symbol, reference);
        }
    }

    if let Some(symbol) = symbols
        .by_name("Label")
        .find(|symbol| symbol.kind == SymbolKind::Method)
    {
        if let Some(reference) = references
            .to(symbol.id)
            .find(|reference| reference.kind == ReferenceKind::Call)
        {
            report_reference(ctx, symbols, references, "go-reference:method-call", symbol, reference);
        }
    }

    if let Some(symbol) = symbols
        .by_name("Name")
        .find(|symbol| symbol.kind == SymbolKind::Field)
    {
        if let Some(reference) = references
            .to(symbol.id)
            .find(|reference| reference.kind == ReferenceKind::MemberAccess)
        {
            report_reference(ctx, symbols, references, "go-reference:field-selector", symbol, reference);
        }
    }

    if let Some(symbol) = symbols
        .by_name("local")
        .find(|symbol| symbol.kind == SymbolKind::Variable)
    {
        if let Some(reference) = references
            .to(symbol.id)
            .find(|reference| reference.kind == ReferenceKind::Read)
        {
            report_reference(ctx, symbols, references, "go-reference:local-variable", symbol, reference);
        }
    }

    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![go_symbol_reference_public_sdk()])
}
"#,
    );
    write_file(
        &root.join("go.mod"),
        r#"module example.com/polint-symbols

go 1.24
"#,
    );
    write_file(
        &root.join("src/main.go"),
        r#"package app

type Widget struct {
	Name string
}

func Build(input string) string {
	local := input
	return local
}

func (w Widget) Label() string {
	return w.Name
}

func Use() string {
	w := Widget{Name: "ok"}
	local := Build(w.Label())
	return local + w.Name
}
"#,
    );
}

fn write_cache_capability_rule_repo(root: &Path) {
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#,
        ),
    );
    write_file(
        &root.join(".polint/rules/src/main.rs"),
        r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/cache-capability",
    description = "Reads capability-gated facts.",
    severity = "warn"
)]
fn cache_capability(_ctx: &mut RuleCtx<'_>, literals: StringLiterals<'_>) -> RuleResult {
    let _literal_count = literals.iter().count();
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![cache_capability()])
}
"#,
    );
    write_file(
        &root.join("src/component.tsx"),
        r##"export function Component() {
  return <div data-color="#fff">ok</div>;
}
"##,
    );
}

fn write_relationship_capability_rule_repo(root: &Path) {
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#,
        ),
    );
    write_file(
        &root.join(".polint/rules/src/main.rs"),
        r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/relationships-available",
    description = "Reads relationship facts.",
    severity = "warn"
)]
fn relationships_available(
    ctx: &mut RuleCtx<'_>,
    resolved: ResolvedImports<'_>,
    graph: ModuleGraphFacts<'_>,
) -> RuleResult {
    if resolved.iter().count() == 0 && graph.nodes().is_empty() && graph.edges().is_empty() {
        ctx.report(Diagnostic::warning(
            ctx.rule_id(),
            "<workspace>",
            DiagnosticRange::point(1, 1),
            "relationship facts are available",
        ));
    }
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![relationships_available()])
}
"#,
    );
    fs::create_dir_all(root.join("src")).unwrap();
}

fn write_go_relationship_setup_missing_rule_repo(root: &Path) {
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#,
        ),
    );
    write_file(
        &root.join(".polint/rules/src/main.rs"),
        r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/go-relationships",
    description = "Reads Go relationship facts.",
    severity = "warn"
)]
fn go_relationships(ctx: &mut RuleCtx<'_>, resolved: ResolvedImports<'_>) -> RuleResult {
    let _ = resolved.iter().count();
    ctx.report(Diagnostic::warning(
        ctx.rule_id(),
        "<workspace>",
        DiagnosticRange::point(1, 1),
        "this rule should be blocked by setup-missing support",
    ));
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![go_relationships()])
}
"#,
    );
    write_file(
        &root.join("src/main.go"),
        r#"package main

import "example.com/project/pkg"

func main() {}
"#,
    );
}

fn write_module_relationship_rule_repo(root: &Path) {
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#,
        ),
    );
    write_file(
        &root.join(".polint/rules/src/main.rs"),
        r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/unresolved-import",
    description = "Report imports that did not resolve.",
    severity = "warn"
)]
fn unresolved_import(ctx: &mut RuleCtx<'_>, resolved: ResolvedImports<'_>) -> RuleResult {
    let rule_id = ctx.rule_id().to_string();
    for import in resolved.iter() {
        if matches!(
            import.status,
            ResolutionStatus::Resolved | ResolutionStatus::External
        ) {
            continue;
        }

        let reason = import
            .reason
            .map(|reason| format!("{reason:?}"))
            .unwrap_or_else(|| "None".to_string());
        ctx.report(
            Diagnostic::warning(
                rule_id.clone(),
                ctx.file_path(import.from_file),
                DiagnosticRange::point(1, 1),
                "Import did not resolve.",
            )
            .with_evidence("status", format!("{:?}", import.status))
            .with_evidence("reason", reason),
        );
    }
    Ok(())
}

#[polint::rule(
    id = "local/no-ui-to-domain",
    description = "Disallow UI files importing domain files.",
    severity = "error"
)]
fn no_ui_to_domain(ctx: &mut RuleCtx<'_>, graph: ModuleGraphFacts<'_>) -> RuleResult {
    let rule_id = ctx.rule_id().to_string();
    for edge in graph.edges() {
        let Some(source) = graph.nodes().iter().find(|node| node.id == edge.from) else {
            continue;
        };
        let Some(target) = graph.nodes().iter().find(|node| node.id == edge.to) else {
            continue;
        };
        if !source.label.starts_with("src/ui/") || !target.label.starts_with("src/domain/") {
            continue;
        }

        let file = source
            .file
            .map(|file| ctx.file_path(file))
            .unwrap_or_else(|| source.label.clone());
        ctx.report(
            Diagnostic::error(
                rule_id.clone(),
                file,
                DiagnosticRange::point(1, 1),
                "UI code must not import the domain layer directly.",
            )
            .with_evidence("source", source.label.clone())
            .with_evidence("target", target.label.clone())
            .with_evidence("status", format!("{:?}", graph.dependency_status(edge))),
        );
    }
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![unresolved_import(), no_ui_to_domain()])
}
"#,
    );
    write_file(
        &root.join("package.json"),
        r#"{"name":"relationship-fixture","private":true,"type":"module"}"#,
    );
    write_file(
        &root.join("tsconfig.json"),
        r#"{
  "compilerOptions": {
    "baseUrl": ".",
    "paths": {
      "@domain/*": ["src/domain/*"]
    }
  }
}
"#,
    );
    write_file(
        &root.join("src/domain/model.ts"),
        r#"export const model = "domain";"#,
    );
    write_file(
        &root.join("src/ui/view.ts"),
        r#"import { model } from "../domain/model";
import { missing } from "./missing";

export const view = `${model}:${missing}`;
"#,
    );
    write_file(
        &root.join("src/ui/alias.ts"),
        r#"import { model } from "@domain/model";

export const aliasView = model;
"#,
    );
}

fn write_no_todo_rule_repo(root: &Path, require_reason: bool) {
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint.toml"),
        &format!(
            r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]

[ignores]
require_reason = {require_reason}

[[rules.config]]
id = "local/no-todo-literals"
severity = "warn"
files = ["src/**/*.ts"]
"#
        ),
    );
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#,
        ),
    );
    write_file(
        &root.join(".polint/rules/src/main.rs"),
        r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/no-todo-literals",
    description = "Reject TODO string literals.",
    severity = "warn"
)]
fn no_todo_literals(ctx: &mut RuleCtx<'_>, literals: StringLiterals<'_>) -> RuleResult {
    let rule_id = ctx.rule_id().to_string();
    for literal in literals.iter() {
        let file = ctx.file_path(literal.file);
        if file_in_scope(ctx.options(), &file) && literal.value == "TODO" {
            ctx.report(Diagnostic::warning(
                rule_id.clone(),
                file,
                literal.span.diagnostic_range(),
                "TODO string literal is not allowed.",
            ));
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![no_todo_literals()])
}
"#,
    );
}

fn write_metric_signal_rule_repo(root: &Path) {
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]

[[rules.config]]
id = "local/code-quality-signals"
severity = "warn"
files = ["src/**"]
max = 1
"#,
    );
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#,
        ),
    );
    write_file(
        &root.join(".polint/rules/src/main.rs"),
        r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/code-quality-signals",
    description = "Compose reusable file/function/complexity metric signals.",
    severity = "warn"
)]
fn code_quality_signals(
    ctx: &mut RuleCtx<'_>,
    files: FileMetrics<'_>,
    functions: FunctionMetrics<'_>,
    complexity: ComplexityMetrics<'_>,
) -> RuleResult {
    let rule_id = ctx.rule_id().to_string();
    let max_complexity = ctx.options().max.unwrap_or(1);

    for file_metric in files.iter() {
        let file = ctx.file_path(file_metric.file);
        if file_in_scope(ctx.options(), &file) && file_metric.line_count > 4 {
            ctx.report(
                Diagnostic::warning(
                    rule_id.clone(),
                    file,
                    DiagnosticRange::point(1, 1),
                    "File is larger than the local quality signal threshold.",
                )
                .with_evidence("file_lines", file_metric.line_count.to_string())
                .with_evidence("functions", file_metric.function_count.to_string()),
            );
        }
    }

    for function_metric in functions.iter() {
        let file = ctx.file_path(function_metric.file);
        if file_in_scope(ctx.options(), &file) && function_metric.line_count > 4 {
            ctx.report(
                Diagnostic::warning(
                    rule_id.clone(),
                    file,
                    function_metric.span.diagnostic_range(),
                    "Function is larger than the local quality signal threshold.",
                )
                .with_evidence("function", function_metric.name.clone())
                .with_evidence("function_lines", function_metric.line_count.to_string()),
            );
        }
    }

    for metric in complexity.over(max_complexity) {
        let file = ctx.file_path(metric.file);
        if file_in_scope(ctx.options(), &file) {
            ctx.report(
                Diagnostic::warning(
                    rule_id.clone(),
                    file,
                    metric.span.diagnostic_range(),
                    "Function is over the local complexity signal threshold.",
                )
                .with_evidence("function", metric.name.clone())
                .with_evidence("complexity", metric.cyclomatic_complexity.to_string()),
            );
        }
    }

    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![code_quality_signals()])
}
"#,
    );
    write_file(
        &root.join("src/router.go"),
        r#"package app

func route(path string) string {
	if path == "/admin" {
		return "admin"
	}
	if path == "/health" {
		return "health"
	}
	return "public"
}
"#,
    );
    write_file(
        &root.join("src/panel.ts"),
        r#"export function label(value: string) {
  if (value === "admin") {
    return "Admin";
  }
  if (value === "health") {
    return "Health";
  }
  return value ? value : "Public";
}
"#,
    );
}

fn cache_json_file_names(root: &Path) -> BTreeSet<String> {
    let cache_dir = root.join(".polint/cache");
    let mut names = BTreeSet::new();
    collect_cache_json_file_names(&cache_dir, &cache_dir, &mut names);
    names
}

fn cache_json_paths(root: &Path) -> Vec<PathBuf> {
    let cache_dir = root.join(".polint/cache");
    let mut paths = Vec::new();
    collect_cache_json_paths(&cache_dir, &mut paths);
    paths.sort();
    paths
}

fn collect_cache_json_paths(current: &Path, paths: &mut Vec<PathBuf>) {
    if !current.exists() {
        return;
    }
    for entry in fs::read_dir(current).unwrap_or_else(|error| panic!("read cache dir: {error}")) {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            collect_cache_json_paths(&path, paths);
        } else if path.extension().is_some_and(|ext| ext == "json") {
            paths.push(path);
        }
    }
}

fn collect_cache_json_file_names(root: &Path, current: &Path, names: &mut BTreeSet<String>) {
    if !current.exists() {
        return;
    }
    for entry in fs::read_dir(current).unwrap_or_else(|error| panic!("read cache dir: {error}")) {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            collect_cache_json_file_names(root, &path, names);
        } else if path.extension().is_some_and(|ext| ext == "json") {
            let relative = path
                .strip_prefix(root)
                .unwrap_or_else(|_| panic!("cache path outside root: {}", path.display()));
            names.insert(relative.to_string_lossy().replace('\\', "/"));
        }
    }
}

#[test]
fn init_creates_config() {
    let temp = tempfile::tempdir().unwrap();
    polint_cmd()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialized polint config"));

    assert!(temp.path().join(".polint.toml").exists());
    assert!(temp.path().join(".polint/rules").exists());
    assert!(temp.path().join(".polint/rules/src").exists());
    assert!(temp.path().join(".polint/cache").exists());
    assert!(temp.path().join(".polint/output").exists());
    let nested_gitignore = fs::read_to_string(temp.path().join(".polint/.gitignore")).unwrap();
    assert!(nested_gitignore.contains("cache/"), "{nested_gitignore:?}");
    assert!(nested_gitignore.contains("output/"), "{nested_gitignore:?}");
    let toolchain = fs::read_to_string(temp.path().join("rust-toolchain.toml")).unwrap();
    assert!(toolchain.contains("channel = \"1.95\""), "{toolchain:?}");
}

#[test]
fn init_does_not_overwrite_existing_rust_toolchain() {
    let temp = tempfile::tempdir().unwrap();
    let tc = temp.path().join("rust-toolchain.toml");
    fs::write(
        &tc,
        "# sentinel-toolchain\n[toolchain]\nchannel = \"1.94.0\"\n",
    )
    .unwrap();

    polint_cmd()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();

    assert_eq!(
        fs::read_to_string(&tc).unwrap(),
        "# sentinel-toolchain\n[toolchain]\nchannel = \"1.94.0\"\n"
    );
}

#[test]
fn init_does_not_overwrite_existing_config() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join(".polint.toml");
    fs::write(&config, "# sentinel\n[workspace]\ninclude = [\"src/**\"]\n").unwrap();

    polint_cmd()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();

    assert!(temp.path().join(".polint/rules").exists());
    assert!(temp.path().join(".polint/rules/src").exists());
    assert!(temp.path().join(".polint/cache").exists());
    assert!(temp.path().join(".polint/output").exists());
    assert!(temp.path().join(".polint/.gitignore").exists());
    assert_eq!(
        fs::read_to_string(config).unwrap(),
        "# sentinel\n[workspace]\ninclude = [\"src/**\"]\n"
    );
    let toolchain = fs::read_to_string(temp.path().join("rust-toolchain.toml")).unwrap();
    assert!(toolchain.contains("channel = \"1.95\""), "{toolchain:?}");
}

#[test]
fn init_appends_cache_and_output_to_existing_polint_gitignore() {
    let temp = tempfile::tempdir().unwrap();
    let polint_dir = temp.path().join(".polint");
    fs::create_dir_all(&polint_dir).unwrap();
    fs::write(polint_dir.join(".gitignore"), "# keep\nother/\n").unwrap();

    polint_cmd()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();

    let gi = fs::read_to_string(polint_dir.join(".gitignore")).unwrap();
    assert!(gi.contains("# keep"));
    assert!(gi.contains("other/"));
    assert!(gi.contains("cache/"));
    assert!(gi.contains("output/"));
}

#[test]
fn new_rule_creates_skeleton() {
    let temp = tempfile::tempdir().unwrap();
    polint_cmd()
        .current_dir(temp.path())
        .args(["new-rule", "go", "branch-error-paths"])
        .assert()
        .success();

    let manifest_path = temp.path().join(".polint/rules/Cargo.toml");
    assert!(
        manifest_path.exists(),
        "expected one pack manifest at .polint/rules/Cargo.toml"
    );
    let manifest = fs::read_to_string(manifest_path).unwrap();
    assert!(manifest.contains("default-features = false"));
    assert_eq!(manifest.contains(r#""lang-go""#), cfg!(feature = "lang-go"));
    assert_eq!(
        manifest.contains(r#""lang-typescript""#),
        cfg!(feature = "lang-typescript")
    );
    assert!(
        temp.path()
            .join(".polint/rules/src/branch_error_paths.rs")
            .exists()
    );
    assert!(temp.path().join(".polint/rules/src/main.rs").exists());
}

#[test]
fn new_rule_rejects_unsafe_rule_names_without_writing_outside_rules_dir() {
    for rule_name in ["../..", "nested/rule", "", "branch_error_paths"] {
        let temp = tempfile::tempdir().unwrap();

        polint_cmd()
            .current_dir(temp.path())
            .args(["new-rule", "go", rule_name])
            .assert()
            .failure()
            .stderr(predicate::str::contains("rule name must be"));

        assert!(
            !temp.path().join("Cargo.toml").exists(),
            "{rule_name:?} must not write Cargo.toml outside .polint/rules"
        );
        assert!(
            !temp.path().join("src/main.rs").exists(),
            "{rule_name:?} must not write src/main.rs outside .polint/rules"
        );
        assert!(
            !temp.path().join(".polint/rules/Cargo.toml").exists(),
            "{rule_name:?} must not write into .polint/rules directly"
        );
        assert!(
            !temp.path().join(".polint/rules/src/nested").exists(),
            "{rule_name:?} must not create stray module paths"
        );
        assert!(
            !temp
                .path()
                .join(".polint/rules/src/branch_error_paths.rs")
                .exists(),
            "{rule_name:?} must not create underscored module files"
        );
    }
}

#[test]
fn new_rule_rejects_existing_rule_without_overwriting_files() {
    let temp = tempfile::tempdir().unwrap();
    let rules = temp.path().join(".polint/rules");
    fs::create_dir_all(rules.join("src")).unwrap();
    let sentinel = "// sentinel module\n";
    fs::write(rules.join("src/demo.rs"), sentinel).unwrap();

    polint_cmd()
        .current_dir(temp.path())
        .args(["new-rule", "go", "demo"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("rule already exists"));

    assert_eq!(
        fs::read_to_string(rules.join("src/demo.rs")).unwrap(),
        sentinel
    );
}

#[cfg(unix)]
#[test]
fn new_rule_rejects_symlinked_rules_parent_without_writing_outside_repo() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join(".polint/rules")).unwrap();
    std::os::unix::fs::symlink(outside.path(), temp.path().join(".polint/rules/src")).unwrap();

    polint_cmd()
        .current_dir(temp.path())
        .args(["new-rule", "go", "escaped-rule"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("path escapes repository root"));

    assert!(fs::read_dir(outside.path()).unwrap().next().is_none());
    assert!(!temp.path().join(".polint/rules/Cargo.toml").exists());
    assert!(!temp.path().join(".polint/tests").exists());
}

#[cfg(unix)]
#[test]
fn new_rule_rejects_symlinked_fixture_parent_before_mutating_rule_pack() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let rules = temp.path().join(".polint/rules");
    fs::create_dir_all(rules.join("src")).unwrap();
    fs::create_dir_all(temp.path().join(".polint/tests")).unwrap();
    std::os::unix::fs::symlink(outside.path(), temp.path().join(".polint/tests/rules")).unwrap();
    let cargo_sentinel = "# cargo sentinel\n";
    let main_sentinel = "fn main() { polint::runner::run_cli(vec![]) }\n";
    fs::write(rules.join("Cargo.toml"), cargo_sentinel).unwrap();
    fs::write(rules.join("src/main.rs"), main_sentinel).unwrap();

    polint_cmd()
        .current_dir(temp.path())
        .args(["new-rule", "ts", "escaped-fixture"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("path escapes repository root"));

    assert!(fs::read_dir(outside.path()).unwrap().next().is_none());
    assert_eq!(
        fs::read_to_string(rules.join("Cargo.toml")).unwrap(),
        cargo_sentinel
    );
    assert_eq!(
        fs::read_to_string(rules.join("src/main.rs")).unwrap(),
        main_sentinel
    );
    assert!(!rules.join("src/escaped_fixture.rs").exists());
}

#[test]
fn new_rule_preflights_broken_fixture_parent_without_leaving_partial_pack() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join(".polint/tests")).unwrap();
    fs::write(temp.path().join(".polint/tests/rules"), "not a directory").unwrap();

    polint_cmd()
        .current_dir(temp.path())
        .args(["new-rule", "go", "transactional-rule"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a directory"));

    assert!(!temp.path().join(".polint/rules").exists());
    assert_eq!(
        fs::read_to_string(temp.path().join(".polint/tests/rules")).unwrap(),
        "not a directory"
    );
}

#[test]
fn new_rule_rejects_fixture_destination_collision_without_mutating_pack() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = temp.path().join(".polint/tests/rules/demo");
    fs::create_dir_all(&fixture).unwrap();
    fs::write(fixture.join("sentinel"), "keep").unwrap();

    polint_cmd()
        .current_dir(temp.path())
        .args(["new-rule", "ts", "demo"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("rule fixture already exists"));

    assert!(!temp.path().join(".polint/rules").exists());
    assert_eq!(
        fs::read_to_string(fixture.join("sentinel")).unwrap(),
        "keep"
    );
}

#[test]
fn add_skill_installs_claude_skill_non_interactively() {
    let temp = tempfile::tempdir().unwrap();
    polint_cmd()
        .current_dir(temp.path())
        .args(["add-skill", "--agent", "claude"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Installed Claude Code skill"));

    let skill = temp.path().join(".claude/skills/polint/SKILL.md");
    let contents = fs::read_to_string(skill).unwrap();
    assert!(contents.contains("name: polint"));
    assert!(contents.contains("polint check --format ai-friendly --fail-on none"));
    assert!(contents.contains(".polint/output/latest.json"));
    assert!(contents.contains("Do not `cat` the whole file"));
    assert!(contents.contains("polint ignores --shortstat"));
    assert!(contents.contains("docs/schemas/polint-report-v1.json"));
    assert!(contents.contains("polint ships no built-in"));
    assert!(contents.contains("use polint::sdk::prelude::*;"));
    assert!(contents.contains("FileMetrics<'_>"));
    assert!(contents.contains("ResolvedImports<'_>"));
    assert!(contents.contains("ModuleGraphFacts<'_>"));
    assert!(contents.contains("Symbols<'_>"));
    assert!(contents.contains("References<'_>"));
    assert!(contents.contains("symbols.by_name"));
    assert!(contents.contains("references.unresolved()"));
    assert!(contents.contains("Do not implement `Rule` manually"));
}

#[test]
fn add_skill_installs_codex_skill_to_agents_skills_by_default() {
    let temp = tempfile::tempdir().unwrap();
    polint_cmd()
        .current_dir(temp.path())
        .args(["add-skill", "--agent", "codex"])
        .assert()
        .success()
        .stdout(predicate::str::contains(".agents/skills/polint/SKILL.md"));

    assert!(temp.path().join(".agents/skills/polint/SKILL.md").exists());
    assert!(!temp.path().join(".codex/skills/polint/SKILL.md").exists());
}

#[test]
fn add_skill_uses_existing_codex_skills_folder_when_present() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join(".codex/skills")).unwrap();

    polint_cmd()
        .current_dir(temp.path())
        .args(["add-skill", "--agent", "codex"])
        .assert()
        .success()
        .stdout(predicate::str::contains(".codex/skills/polint/SKILL.md"));

    assert!(temp.path().join(".codex/skills/polint/SKILL.md").exists());
    assert!(!temp.path().join(".agents/skills/polint/SKILL.md").exists());
}

#[test]
fn add_skill_all_installs_claude_and_codex() {
    let temp = tempfile::tempdir().unwrap();
    polint_cmd()
        .current_dir(temp.path())
        .args(["add-skill", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Installed Claude Code skill"))
        .stdout(predicate::str::contains("Installed Codex skill"));

    assert!(temp.path().join(".claude/skills/polint/SKILL.md").exists());
    assert!(temp.path().join(".agents/skills/polint/SKILL.md").exists());
}

#[test]
fn add_skill_prompts_when_agent_is_not_provided() {
    let temp = tempfile::tempdir().unwrap();
    let mut command = polint_cmd();
    command
        .current_dir(temp.path())
        .arg("add-skill")
        .write_stdin("1\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("Install the polint skill"))
        .stdout(predicate::str::contains("Installed Claude Code skill"));

    assert!(temp.path().join(".claude/skills/polint/SKILL.md").exists());
    assert!(!temp.path().join(".agents/skills/polint/SKILL.md").exists());
}

#[test]
fn add_skill_prompts_before_overwriting_existing_skill() {
    let temp = tempfile::tempdir().unwrap();
    polint_cmd()
        .current_dir(temp.path())
        .args(["add-skill", "--agent", "claude"])
        .assert()
        .success();

    let skill = temp.path().join(".claude/skills/polint/SKILL.md");
    fs::write(&skill, "sentinel").unwrap();

    polint_cmd()
        .current_dir(temp.path())
        .args(["add-skill", "--agent", "claude"])
        .write_stdin("n\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("Overwrite existing polint skill?"))
        .stdout(predicate::str::contains("Kept existing Claude Code skill"));
    assert_eq!(fs::read_to_string(&skill).unwrap(), "sentinel");

    polint_cmd()
        .current_dir(temp.path())
        .args(["add-skill", "--agent", "claude"])
        .write_stdin("y\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("Overwrite existing polint skill?"))
        .stdout(predicate::str::contains("Installed Claude Code skill"));
    assert!(fs::read_to_string(&skill).unwrap().contains("name: polint"));

    fs::write(&skill, "sentinel").unwrap();
    polint_cmd()
        .current_dir(temp.path())
        .args(["add-skill", "--agent", "claude", "--force"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Overwrite existing polint skill?").not());
    assert!(fs::read_to_string(skill).unwrap().contains("name: polint"));
}

#[test]
fn add_skill_all_continues_when_existing_skill_is_kept() {
    let temp = tempfile::tempdir().unwrap();
    let claude_skill = temp.path().join(".claude/skills/polint/SKILL.md");
    write_file(&claude_skill, "sentinel");

    polint_cmd()
        .current_dir(temp.path())
        .args(["add-skill", "--all"])
        .write_stdin("n\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("Overwrite existing polint skill?"))
        .stdout(predicate::str::contains("Kept existing Claude Code skill"))
        .stdout(predicate::str::contains("Installed Codex skill"));

    assert_eq!(fs::read_to_string(claude_skill).unwrap(), "sentinel");
    assert!(temp.path().join(".agents/skills/polint/SKILL.md").exists());
}

#[cfg(unix)]
#[test]
fn add_skill_rejects_symlinked_agent_folder() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), temp.path().join(".claude")).unwrap();

    polint_cmd()
        .current_dir(temp.path())
        .args(["add-skill", "--agent", "claude"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "refusing to write through symlink",
        ));

    assert!(!outside.path().join("skills/polint/SKILL.md").exists());
}

#[test]
fn new_rule_go_creates_sdk_oriented_skeleton() {
    let temp = tempfile::tempdir().unwrap();
    polint_cmd()
        .current_dir(temp.path())
        .args(["new-rule", "go", "branch-error-paths"])
        .assert()
        .success();

    let rules = temp.path().join(".polint/rules");
    assert!(rules.join("Cargo.toml").exists());
    let manifest = fs::read_to_string(rules.join("Cargo.toml")).unwrap();
    let module = fs::read_to_string(rules.join("src/branch_error_paths.rs")).unwrap();
    let main = fs::read_to_string(rules.join("src/main.rs")).unwrap();
    assert!(manifest.contains("polint ="));
    assert!(main.contains("polint::runner::run_cli"));
    assert!(main.contains("branch_error_paths::branch_error_paths()"));
    assert!(module.contains("#[polint::rule("));
    assert!(module.contains("id = \"custom/branch-error-paths\""));
    assert!(module.contains("use polint::sdk::prelude::*;"));
    assert!(!module.contains("SourceFiles<'_>"));
    assert!(module.contains("tests: GoTests<'_>"));
    assert!(module.contains("branches: BranchObligations<'_>"));
    assert!(module.contains("tests.related_for_file(branch.file)"));
    assert!(module.contains("branches.iter()"));
    assert!(module.contains("Project-specific policy (heuristic)"));
    assert!(!module.contains("fn capabilities"));
    assert!(!module.contains("impl Rule"));
    let internal_core_path = ["crate", "core"].join("::");
    assert!(!module.contains(&internal_core_path));
}

#[test]
fn new_rule_go_generates_fixture_that_test_can_run() {
    let temp = tempfile::tempdir().unwrap();
    polint_cmd()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    polint_cmd()
        .current_dir(temp.path())
        .args(["new-rule", "go", "branch-error-paths"])
        .assert()
        .success();
    point_generated_rule_pack_at_local_polint(temp.path());

    let value = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["test", "--no-cache", "--format", "json"])
            .assert()
            .success(),
    );
    assert_eq!(value["summary"]["total"], 2);
    assert_eq!(value["summary"]["failed"], 0, "{value:#?}");
}

#[test]
fn new_rule_ts_creates_sdk_oriented_skeleton() {
    let temp = tempfile::tempdir().unwrap();
    polint_cmd()
        .current_dir(temp.path())
        .args(["new-rule", "ts", "no-raw-brand-colors"])
        .assert()
        .success();

    let rules = temp.path().join(".polint/rules");
    let module = fs::read_to_string(rules.join("src/no_raw_brand_colors.rs")).unwrap();
    let main = fs::read_to_string(rules.join("src/main.rs")).unwrap();
    assert!(main.contains("polint::runner::run_cli"));
    assert!(main.contains("no_raw_brand_colors::no_raw_brand_colors()"));
    assert!(module.contains("#[polint::rule("));
    assert!(module.contains("id = \"custom/no-raw-brand-colors\""));
    assert!(module.contains("use polint::sdk::prelude::*;"));
    assert!(!module.contains("SourceFiles<'_>"));
    assert!(module.contains("literals: StringLiterals<'_>"));
    assert!(module.contains("jsx: JsxAttributes<'_>"));
    assert!(module.contains("for literal in literals.iter()"));
    assert!(module.contains("jsx.iter().count()"));
    assert!(!module.contains("fn capabilities"));
    assert!(!module.contains("impl Rule"));
    let internal_core_path = ["crate", "core"].join("::");
    assert!(!module.contains(&internal_core_path));
}

#[test]
fn new_rule_generic_uses_sdk_query_helpers() {
    let temp = tempfile::tempdir().unwrap();
    polint_cmd()
        .current_dir(temp.path())
        .args(["new-rule", "generic", "domain-names"])
        .assert()
        .success();

    let rules = temp.path().join(".polint/rules");
    let module = fs::read_to_string(rules.join("src/domain_names.rs")).unwrap();
    let main = fs::read_to_string(rules.join("src/main.rs")).unwrap();
    assert!(main.contains("polint::runner::run_cli"));
    assert!(main.contains("domain_names::domain_names()"));
    assert!(module.contains("#[polint::rule("));
    assert!(module.contains("id = \"custom/domain-names\""));
    assert!(module.contains("use polint::sdk::prelude::*;"));
    assert!(!module.contains("SourceFiles<'_>"));
    assert!(module.contains("functions: Functions<'_>"));
    assert!(module.contains("function.name == \"replaceMe\""));
    assert!(!module.contains("fn capabilities"));
    assert!(!module.contains("impl Rule"));
    let internal_core_path = ["crate", "core"].join("::");
    assert!(!module.contains(&internal_core_path));

    point_generated_rule_pack_at_local_polint(temp.path());
    let value = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["test", "--no-cache", "--format", "json"])
            .assert()
            .success(),
    );
    assert_eq!(value["summary"]["total"], 2);
    assert_eq!(value["summary"]["failed"], 0, "{value:#?}");
}

#[test]
fn new_rule_rejects_unknown_language() {
    let temp = tempfile::tempdir().unwrap();

    polint_cmd()
        .current_dir(temp.path())
        .args(["new-rule", "python", "bad-python"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "unsupported rule language `python`",
        ));
}

#[test]
fn new_rule_js_uses_js_fixture_files_for_non_template_rules() {
    let temp = tempfile::tempdir().unwrap();
    polint_cmd()
        .current_dir(temp.path())
        .args(["new-rule", "js", "no-debug-name"])
        .assert()
        .success();

    assert!(
        temp.path()
            .join(".polint/tests/rules/no_debug_name/clean/src/example.js")
            .is_file()
    );
    assert!(
        !temp
            .path()
            .join(".polint/tests/rules/no_debug_name/clean/src/example.ts")
            .exists()
    );
}

#[test]
fn new_rule_policy_templates_reject_unbacked_language_combinations() {
    let temp = tempfile::tempdir().unwrap();

    polint_cmd()
        .current_dir(temp.path())
        .args([
            "new-rule",
            "go",
            "bad-flow",
            "--template",
            "request-to-shell",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "template `request-to-shell` is not available for Go yet",
        ));

    polint_cmd()
        .current_dir(temp.path())
        .args(["new-rule", "js", "bad-js", "--template", "secret-to-log"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "policy templates are currently available for `ts` and selected `go` templates",
        ));
}

#[test]
fn new_rule_policy_template_modules_use_public_sdk_only() {
    let cases = [
        (
            "ts",
            "request-to-shell",
            "DataFlow<'_>",
            "FlowQuery::new",
            "SourcePattern::http_request()",
            "SinkPattern::call(\"exec\")",
        ),
        (
            "ts",
            "secret-to-log",
            "DataFlow<'_>",
            "FlowQuery::new",
            "SourcePattern::secret_like",
            "SinkPattern::logger()",
        ),
        (
            "ts",
            "pii-to-analytics",
            "DataFlow<'_>",
            "FlowQuery::new",
            "email",
            "analyticsTrack",
        ),
        (
            "go",
            "sensitive-write-guard",
            "ControlFlow<'_>",
            "GuardQuery::new",
            "writeBalance",
            "validate_payment",
        ),
        (
            "go",
            "transaction-cleanup",
            "ControlFlow<'_>",
            "LifecycleQuery::new",
            "Begin",
            "Rollback",
        ),
        (
            "go",
            "raw-reachable-api",
            "Calls<'_>",
            "ReachQuery::new",
            "dangerousAdmin",
            "EventPattern::call(\"main\")",
        ),
        (
            "ts",
            "ssrf",
            "DataFlow<'_>",
            "FlowQuery::new",
            "SourcePattern::http_request()",
            "fetchUrl",
        ),
        (
            "ts",
            "dangerous-html",
            "DataFlow<'_>",
            "FlowQuery::new",
            "SourcePattern::http_request()",
            "setInnerHTML",
        ),
        (
            "ts",
            "unsafe-deserialization",
            "DataFlow<'_>",
            "FlowQuery::new",
            "SourcePattern::http_request()",
            "unsafeDeserialize",
        ),
        (
            "ts",
            "user-file-path",
            "DataFlow<'_>",
            "FlowQuery::new",
            "SourcePattern::http_request()",
            "readFile",
        ),
    ];

    for (language, template, view, query, first, second) in cases {
        let temp = tempfile::tempdir().unwrap();
        let rule_name = format!("template-{template}");
        polint_cmd()
            .current_dir(temp.path())
            .args(["new-rule", language, &rule_name, "--template", template])
            .assert()
            .success();

        let module_name = rule_name.replace('-', "_");
        let module = fs::read_to_string(
            temp.path()
                .join(".polint/rules/src")
                .join(format!("{module_name}.rs")),
        )
        .unwrap();
        assert!(module.contains("use polint::sdk::prelude::*;"), "{module}");
        assert!(module.contains("#[polint::rule("), "{module}");
        assert!(module.contains("severity = \"error\""), "{module}");
        assert!(module.contains(view), "{template}: {module}");
        assert!(module.contains(query), "{template}: {module}");
        assert!(module.contains(first), "{template}: {module}");
        assert!(module.contains(second), "{template}: {module}");
        if view == "DataFlow<'_>" {
            assert!(
                module.contains("query.max_depth = 24;"),
                "{template}: {module}"
            );
            assert!(
                module.contains("query.max_paths = 128;"),
                "{template}: {module}"
            );
        }
        if template == "raw-reachable-api" {
            assert!(module.contains("ctx.completeness().status_for(\"calls\")"));
            assert!(module.contains("analysis_completeness"));
        }
        assert!(module.contains("violation.diagnostic("), "{module}");
        assert!(!module.contains("Capabilities::new("), "{module}");
        assert!(!module.contains("impl Rule"), "{module}");
        assert!(!module.contains("polint::core"), "{module}");
        assert!(!module.contains("crate::core"), "{module}");
    }
}

#[test]
fn raw_reachable_template_labels_budget_incomplete_run() {
    let temp = tempfile::tempdir().unwrap();
    polint_cmd()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    polint_cmd()
        .current_dir(temp.path())
        .args([
            "new-rule",
            "go",
            "budget-aware-reachability",
            "--template",
            "raw-reachable-api",
        ])
        .assert()
        .success();
    point_generated_rule_pack_at_local_polint(temp.path());
    write_file(
        &temp.path().join(".polint.toml"),
        r#"[workspace]
include = ["**/*.go"]
exclude = []

[rules]
paths = [".polint/rules"]

[languages.go]
module_roots = ["."]
package_patterns = ["./..."]

[solver.go]
max_candidates_per_callsite = 1
"#,
    );
    write_file(
        &temp.path().join("go.mod"),
        "module example.com/completeness\n\ngo 1.22\n",
    );
    let budget_fixture = fs::read_to_string(
        repo_root().join("tests/eval-fixtures/go-rta/iteration-cap/repo/main.go"),
    )
    .unwrap();
    write_file(&temp.path().join("main.go"), &budget_fixture);

    let report = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args([
                "check",
                "--no-cache",
                "--format",
                "json",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );
    let diagnostic = diagnostics_for_rule(&report, "custom/budget-aware-reachability")
        .into_iter()
        .find(|diagnostic| {
            diagnostic["message"]
                .as_str()
                .is_some_and(|message| message.contains("Calls analysis is incomplete"))
        })
        .unwrap_or_else(|| panic!("missing template completeness diagnostic: {report:#?}"));
    assert!(diagnostic_has_evidence(
        diagnostic,
        "analysis_completeness",
        "budget_exceeded"
    ));
}

#[test]
fn new_rule_policy_templates_generate_fixture_tests() {
    let temp = tempfile::tempdir().unwrap();
    polint_cmd()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    write_file(
        &temp.path().join("go.mod"),
        "module example.com/policytemplates\n\ngo 1.22\n",
    );

    for (language, template) in [
        ("ts", "request-to-shell"),
        ("ts", "secret-to-log"),
        ("ts", "pii-to-analytics"),
        ("go", "sensitive-write-guard"),
        ("go", "transaction-cleanup"),
        ("go", "raw-reachable-api"),
        ("ts", "ssrf"),
        ("ts", "dangerous-html"),
        ("ts", "unsafe-deserialization"),
        ("ts", "user-file-path"),
    ] {
        let rule_name = format!("template-{template}");
        polint_cmd()
            .current_dir(temp.path())
            .args(["new-rule", language, &rule_name, "--template", template])
            .assert()
            .success();
    }
    point_generated_rule_pack_at_local_polint(temp.path());

    let value = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["test", "--no-cache", "--format", "json"])
            .assert()
            .success(),
    );
    assert_eq!(value["summary"]["total"], 20);
    assert_eq!(value["summary"]["failed"], 0, "{value:#?}");
}

#[test]
fn new_rule_policy_templates_are_deterministic() {
    let temp = tempfile::tempdir().unwrap();
    polint_cmd()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    write_file(
        &temp.path().join("go.mod"),
        "module example.com/policytemplatedeterminism\n\ngo 1.22\n",
    );

    for (language, template) in [
        ("ts", "secret-to-log"),
        ("go", "sensitive-write-guard"),
        ("go", "raw-reachable-api"),
    ] {
        let rule_name = format!("deterministic-{template}");
        polint_cmd()
            .current_dir(temp.path())
            .args(["new-rule", language, &rule_name, "--template", template])
            .assert()
            .success();
    }
    point_generated_rule_pack_at_local_polint(temp.path());

    let first = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["test", "--no-cache", "--format", "json"])
            .assert()
            .success(),
    );
    let second = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["test", "--no-cache", "--format", "json"])
            .assert()
            .success(),
    );

    assert_eq!(
        first["summary"], second["summary"],
        "{first:#?}\n{second:#?}"
    );
    assert_eq!(first["cases"], second["cases"], "{first:#?}\n{second:#?}");
    assert_eq!(first["summary"]["total"], 6);
    assert_eq!(first["summary"]["failed"], 0, "{first:#?}");
}

#[test]
fn rule_macro_rejects_unknown_fact_view_parameter() {
    let temp = tempfile::tempdir().unwrap();
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");

    write_file(
        &temp.path().join("Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-macro-fail-smoke"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}
"#,
        ),
    );
    write_file(
        &temp.path().join("src/main.rs"),
        r#"use polint::sdk::prelude::*;
use std::process::ExitCode;

struct UserDefinedFacts<'a>(&'a ());

#[polint::rule(
    id = "local/invalid-facts",
    description = "Invalid facts.",
    severity = "warn"
)]
fn invalid_facts(
    ctx: &mut RuleCtx<'_>,
    facts: UserDefinedFacts<'_>,
) -> RuleResult {
    let _ = (ctx, facts);
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![invalid_facts()])
}
"#,
    );

    cargo_cmd()
        .current_dir(temp.path())
        .args(["check", "--quiet"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "unsupported polint fact view parameter",
        ));
}

#[test]
fn rule_macro_rejects_shadowed_fact_view_names() {
    let temp = tempfile::tempdir().unwrap();
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");

    write_file(
        &temp.path().join("Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-shadowed-facts-fail-smoke"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}
"#,
        ),
    );
    write_file(
        &temp.path().join("src/main.rs"),
        r#"use std::process::ExitCode;

use polint::sdk::prelude::{RuleCtx, RuleResult};

struct Imports<'a>(&'a ());

#[polint::rule(
    id = "local/shadowed-imports",
    description = "Shadowed imports.",
    severity = "warn"
)]
fn shadowed_imports(ctx: &mut RuleCtx<'_>, _imports: Imports<'_>) -> RuleResult {
    let _ = ctx;
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![shadowed_imports()])
}
"#,
    );

    cargo_cmd()
        .current_dir(temp.path())
        .args(["check", "--quiet"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("mismatched types"));
}

#[test]
fn rule_macro_rejects_non_canonical_qualified_fact_view_paths() {
    let temp = tempfile::tempdir().unwrap();
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");

    write_file(
        &temp.path().join("Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-qualified-facts-fail-smoke"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}
"#,
        ),
    );
    write_file(
        &temp.path().join("src/main.rs"),
        r#"use std::process::ExitCode;

use polint::sdk::prelude::{RuleCtx, RuleResult};

mod local {
    pub struct Imports<'a>(&'a ());
}

#[polint::rule(
    id = "local/qualified-imports",
    description = "Qualified imports.",
    severity = "warn"
)]
fn qualified_imports(ctx: &mut RuleCtx<'_>, _imports: local::Imports<'_>) -> RuleResult {
    let _ = ctx;
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![qualified_imports()])
}
"#,
    );

    cargo_cmd()
        .current_dir(temp.path())
        .args(["check", "--quiet"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "fact-view parameters must use canonical polint SDK fact views",
        ));
}

#[test]
fn rule_macro_rejects_non_analyzable_rule_signatures() {
    let temp = tempfile::tempdir().unwrap();
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");

    write_file(
        &temp.path().join("Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-signature-fail-smoke"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}
"#,
        ),
    );
    write_file(
        &temp.path().join("src/main.rs"),
        r#"use polint::sdk::prelude::*;
use std::process::ExitCode;

#[polint::rule(
    id = "local/async-rule",
    description = "Async rule.",
    severity = "warn"
)]
async fn async_rule(ctx: &mut RuleCtx<'_>) -> RuleResult {
    let _ = ctx;
    Ok(())
}

#[polint::rule(
    id = "local/value-return",
    description = "Value return.",
    severity = "warn"
)]
fn value_return(ctx: &mut RuleCtx<'_>) -> RuleResult<usize> {
    let _ = ctx;
    Ok(1)
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![async_rule(), value_return()])
}
"#,
    );

    cargo_cmd()
        .current_dir(temp.path())
        .args(["check", "--quiet"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("plain non-generic sync functions")
                .and(predicate::str::contains("RuleResult<()>")),
        );
}

#[test]
fn external_rule_cannot_manually_implement_rule_trait() {
    let temp = tempfile::tempdir().unwrap();
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");

    write_file(
        &temp.path().join("Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-manual-rule-fail-smoke"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}
"#,
        ),
    );
    write_file(
        &temp.path().join("src/main.rs"),
        r#"use polint::sdk::prelude::*;

struct ManualRule;

impl Rule for ManualRule {}

fn main() {}
"#,
    );

    cargo_cmd()
        .current_dir(temp.path())
        .args(["check", "--quiet"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "expected trait, found struct `Rule`",
        ));
}

#[test]
fn external_prelude_does_not_export_manual_capability_types() {
    let temp = tempfile::tempdir().unwrap();
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");

    write_file(
        &temp.path().join("Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-capabilities-prelude-fail-smoke"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}
"#,
        ),
    );
    write_file(
        &temp.path().join("src/main.rs"),
        r#"use polint::sdk::prelude::*;

fn main() {
    let _ = Capabilities::new();
}
"#,
    );

    cargo_cmd()
        .current_dir(temp.path())
        .args(["check", "--quiet"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "use of undeclared type `Capabilities`",
        ));
}

#[test]
fn external_generated_rule_uses_sdk_facts_settings_and_reports_diagnostic() {
    let temp = tempfile::tempdir().unwrap();
    polint_cmd()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    polint_cmd()
        .current_dir(temp.path())
        .args(["new-rule", "ts", "no-todo-literals"])
        .assert()
        .success();

    point_generated_rule_pack_at_local_polint(temp.path());
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]

[profiles.sdk]
rules = ["custom/no-todo-literals"]

[[rules.config]]
id = "custom/no-todo-literals"
severity = "warn"
files = ["src/**/*.ts"]
token = "TODO"
message = "Configured forbidden literal found."
"#,
    );
    write_file(
        &temp.path().join(".polint/rules/src/no_todo_literals.rs"),
        r#"use polint::sdk::prelude::*;

#[polint::rule(
    id = "custom/no-todo-literals",
    description = "Reject configured placeholder literals.",
    severity = "warn"
)]
pub(crate) fn no_todo_literals(
    ctx: &mut RuleCtx<'_>,
    literals: StringLiterals<'_>,
) -> RuleResult {
    let token = ctx
        .options()
        .settings
        .get("token")
        .and_then(|value| value.as_str())
        .unwrap_or("TODO")
        .to_string();
    let message = ctx
        .options()
        .settings
        .get("message")
        .and_then(|value| value.as_str())
        .unwrap_or("Forbidden literal found.")
        .to_string();
    let rule_id = ctx.rule_id().to_string();
    let mut diagnostics = Vec::new();

    for literal in literals.iter() {
        let file = ctx.file_path(literal.file);
        if file_in_scope(ctx.options(), &file) && literal.value == token {
            diagnostics.push(
                Diagnostic::warning(
                    rule_id.clone(),
                    file,
                    literal.span.diagnostic_range(),
                    message.clone(),
                )
                .with_evidence("token", token.clone()),
            );
        }
    }

    for diagnostic in diagnostics {
        ctx.report(diagnostic);
    }
    Ok(())
}
"#,
    );
    write_file(
        &temp.path().join("src/component.ts"),
        r#"export const status = "TODO";"#,
    );

    let json = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args([
                "check",
                "--profile",
                "sdk",
                "--format",
                "json",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );

    let diagnostics = diagnostics_for_rule(&json, "custom/no-todo-literals");
    assert_eq!(diagnostics.len(), 1, "{json:#?}");
    assert_eq!(diagnostics[0]["file"], "src/component.ts");
    assert_eq!(
        diagnostics[0]["message"],
        "Configured forbidden literal found."
    );
    assert!(diagnostic_has_evidence(diagnostics[0], "token", "TODO"));
}

#[test]
fn external_rule_json_preserves_indexed_unicode_crlf_and_eof_span() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_phase41_rule_pack(
        temp.path(),
        r#"use std::process::ExitCode;
use polint::runner;
mod rule;
fn main() -> ExitCode { runner::run_cli(vec![rule::rule()]) }
"#,
        r#"use polint::sdk::prelude::*;

#[polint::rule(id = "local/indexed-span", description = "Indexed span", severity = "warn")]
pub(crate) fn rule(ctx: &mut RuleCtx<'_>, literals: StringLiterals<'_>) -> RuleResult {
    for literal in literals.iter().filter(|literal| literal.value == "πneedle") {
        ctx.warn(&literal.span, "indexed span matched");
    }
    Ok(())
}
"#,
    );
    write_file(
        &temp.path().join("src/component.ts"),
        "const prior = \"é\";\r\nconst target = \"πneedle\"",
    );

    let json = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    let diagnostics = diagnostics_for_rule(&json, "local/indexed-span");
    let range = &diagnostics
        .first()
        .unwrap_or_else(|| panic!("missing indexed-span diagnostic: {json:#?}"))["range"];

    assert_eq!(
        (
            diagnostics.len(),
            range["start_line"].as_u64(),
            range["start_col"].as_u64(),
            range["end_line"].as_u64(),
            range["end_col"].as_u64(),
        ),
        (1, Some(2), Some(16), Some(2), Some(25))
    );
}

#[test]
fn check_ai_friendly_saves_full_json_and_prints_compact_summary() {
    let temp = tempfile::tempdir().unwrap();
    polint_cmd()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    polint_cmd()
        .current_dir(temp.path())
        .args(["new-rule", "ts", "no-todo-literals"])
        .assert()
        .success();

    point_generated_rule_pack_at_local_polint(temp.path());
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]

[[rules.config]]
id = "custom/no-todo-literals"
severity = "warn"
files = ["src/**/*.ts"]
"#,
    );
    write_file(
        &temp.path().join(".polint/rules/src/no_todo_literals.rs"),
        r#"use polint::sdk::prelude::*;

#[polint::rule(
    id = "custom/no-todo-literals",
    description = "Reject TODO literals.",
    severity = "warn"
)]
pub(crate) fn no_todo_literals(
    ctx: &mut RuleCtx<'_>,
    literals: StringLiterals<'_>,
) -> RuleResult {
    let rule_id = ctx.rule_id().to_string();
    let mut diagnostics = Vec::new();
    for literal in literals.iter() {
        if literal.value == "TODO" {
            diagnostics.push(Diagnostic::warning(
                rule_id.clone(),
                ctx.file_path(literal.file),
                literal.span.diagnostic_range(),
                "Configured forbidden literal found.",
            ));
        }
    }
    for diagnostic in diagnostics {
        ctx.report(diagnostic);
    }
    Ok(())
}
"#,
    );
    write_file(
        &temp.path().join("src/component.ts"),
        r#"export const status = "TODO";"#,
    );

    let stdout = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "ai-friendly", "--fail-on", "none"])
            .assert()
            .success(),
    );

    assert!(
        stdout.contains("polint: 1 diagnostic across 1 rule"),
        "{stdout}"
    );
    assert!(stdout.contains("custom/no-todo-literals: 1"), "{stdout}");
    assert!(
        stdout.contains("warn[custom/no-todo-literals]: Configured forbidden literal found."),
        "{stdout}"
    );
    assert!(stdout.contains("--> src/component.ts:"), "{stdout}");
    assert!(stdout.contains("fingerprint:"), "{stdout}");
    assert!(
        stdout.contains("Full JSON: .polint/output/latest.json"),
        "{stdout}"
    );
    assert!(
        stdout.contains("jq '.summary.by_rule' .polint/output/latest.json"),
        "{stdout}"
    );
    assert!(
        stdout.contains(
            "jq '.summary.rules[] | select(.diagnostics_emitted == 0)' .polint/output/latest.json"
        ),
        "{stdout}"
    );
    assert!(
        stdout.contains(
            "jq '[.diagnostics[] | select(.rule_id==\"custom/no-todo-literals\")][0:20]' .polint/output/latest.json"
        ),
        "{stdout}"
    );
    assert!(
        stdout.contains(
            "jq '.diagnostics[] | select(.file==\"src/component.ts\") | {rule_id, range, message}' .polint/output/latest.json | head -c 12000"
        ),
        "{stdout}"
    );
    assert!(
        !stdout.contains("\"diagnostics\""),
        "stdout should stay compact: {stdout}"
    );

    let latest = temp.path().join(".polint/output/latest.json");
    let report: serde_json::Value = serde_json::from_str(&fs::read_to_string(&latest).unwrap())
        .unwrap_or_else(|error| panic!("AI-friendly report should be JSON: {error}"));
    assert_eq!(
        report["schema"],
        "https://raw.githubusercontent.com/oaiz-io/polint/main/docs/schemas/polint-ai-friendly-v1.json"
    );
    assert_eq!(report["summary"]["total_diagnostics"], 1);
    assert_eq!(report["summary"]["rules_triggered"], 1);
    assert_eq!(
        report["summary"]["by_rule"][0]["rule_id"],
        "custom/no-todo-literals"
    );
    let rules = report["summary"]["rules"]
        .as_array()
        .expect("summary.rules must be present");
    let silent_or_hit = rules
        .iter()
        .find(|row| row["rule_id"] == "custom/no-todo-literals");
    let row = silent_or_hit.expect("registered rule must appear in summary.rules");
    assert_eq!(row["planned"], true);
    assert_eq!(row["capabilities_ok"], true);
    assert!(row["files_in_scope"].as_u64().unwrap() > 0);
    assert_eq!(row["diagnostics_emitted"], 1);
    assert_eq!(report["examples"].as_array().unwrap().len(), 1);
    assert_eq!(report["examples"][0]["rule_id"], "custom/no-todo-literals");
    assert_eq!(report["examples"][0]["labels"], serde_json::json!([]));
    assert_eq!(report["examples"][0]["help"], serde_json::Value::Null);
    assert_eq!(report["examples"][0]["evidence"], serde_json::json!([]));
    assert_eq!(report["examples"][0]["suggestions"], serde_json::json!([]));
    assert_eq!(report["examples"][0]["fix"], serde_json::Value::Null);
    assert_eq!(diagnostics(&report).len(), 1);
    assert_eq!(diagnostics(&report)[0]["file"], "src/component.ts");

    let nested_gitignore = fs::read_to_string(temp.path().join(".polint/.gitignore")).unwrap();
    assert!(nested_gitignore.contains("output/"), "{nested_gitignore}");
    let output_entries = fs::read_dir(temp.path().join(".polint/output"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<BTreeSet<_>>();
    assert!(output_entries.contains("latest.json"), "{output_entries:?}");
    assert!(
        output_entries
            .iter()
            .any(|name| name.starts_with("check-") && name.ends_with(".json")),
        "{output_entries:?}"
    );
}

#[test]
fn external_rule_consumes_derived_metric_signals_through_public_sdk() {
    let temp = tempfile::tempdir().unwrap();
    write_metric_signal_rule_repo(temp.path());

    let json = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );

    let diagnostics = diagnostics_for_rule(&json, "local/code-quality-signals");
    assert!(
        diagnostics.len() >= 6,
        "expected file, function, and complexity diagnostics for Go and TS: {json:#?}"
    );
    for file in ["src/router.go", "src/panel.ts"] {
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic["file"] == file),
            "expected metric signal diagnostics in {file}: {json:#?}"
        );
    }
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic_has_evidence(diagnostic, "file_lines", "")),
        "expected file metric evidence: {json:#?}"
    );
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic_has_evidence(
            diagnostic,
            "function_lines",
            ""
        )),
        "expected function metric evidence: {json:#?}"
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic_has_evidence(diagnostic, "complexity", "")),
        "expected complexity metric evidence: {json:#?}"
    );
}

#[test]
fn check_suppresses_next_line_ignore_and_ignores_reports_stats() {
    let temp = tempfile::tempdir().unwrap();
    write_no_todo_rule_repo(temp.path(), false);
    write_file(
        &temp.path().join("src/component.ts"),
        r#"// polint-ignore-next-line local/no-todo-literals -- legacy fixture
export const skipped = "TODO";
export const reported = "TODO";
"#,
    );

    let json = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    let diagnostics = diagnostics_for_rule(&json, "local/no-todo-literals");
    assert_eq!(diagnostics.len(), 1, "{json:#?}");
    assert_eq!(diagnostics[0]["range"]["start_line"], 3);

    let report = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["ignores", "--format", "json", "--filter", "local/*"])
            .assert()
            .success(),
    );
    assert_eq!(report["summary"]["directives"], 1, "{report:#?}");
    assert_eq!(report["summary"]["active"], 1);
    assert_eq!(report["summary"]["suppressed_diagnostics"], 1);
    assert_eq!(
        report["directives"][0]["suppressed"][0]["rule_id"],
        "local/no-todo-literals"
    );

    let shortstat = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["ignores", "--shortstat"])
            .assert()
            .success(),
    );
    assert!(shortstat.contains("1 directives, 1 active"), "{shortstat}");
}

#[test]
fn check_reports_missing_reason_when_config_requires_it_but_still_suppresses() {
    let temp = tempfile::tempdir().unwrap();
    write_no_todo_rule_repo(temp.path(), true);
    write_file(
        &temp.path().join("src/component.ts"),
        r#"// polint-ignore-next-line local/no-todo-literals
export const skipped = "TODO";
"#,
    );

    let json = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    assert!(
        diagnostics_for_rule(&json, "local/no-todo-literals").is_empty(),
        "{json:#?}"
    );
    let missing = diagnostics_for_rule(&json, "polint/ignore-missing-reason");
    assert_eq!(missing.len(), 1, "{json:#?}");
    assert_eq!(missing[0]["file"], "src/component.ts");

    let report = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["ignores", "--format", "json"])
            .assert()
            .success(),
    );
    assert_eq!(report["summary"]["missing_reasons"], 1);
    assert_eq!(report["summary"]["suppressed_diagnostics"], 1);
    assert_eq!(report["directives"][0]["status"], "missing_reason");
}

#[test]
fn check_reports_unused_and_malformed_ignore_directives() {
    let temp = tempfile::tempdir().unwrap();
    write_no_todo_rule_repo(temp.path(), false);
    write_file(
        &temp.path().join("src/component.ts"),
        r#"// polint-ignore-next-line local/no-todo-literals -- no finding here
export const ok = "DONE";
// polint-ignore-next-line
export const also_ok = "DONE";
"#,
    );

    let json = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    assert_eq!(
        diagnostics_for_rule(&json, "polint/unused-ignore").len(),
        1,
        "{json:#?}"
    );
    assert_eq!(
        diagnostics_for_rule(&json, "polint/malformed-ignore").len(),
        1,
        "{json:#?}"
    );
}

#[test]
fn baseline_create_writes_compact_yaml_entries() {
    let temp = tempfile::tempdir().unwrap();
    write_no_todo_rule_repo(temp.path(), false);
    write_file(
        &temp.path().join("src/component.ts"),
        r#"export const marker = "TODO";
"#,
    );

    polint_cmd()
        .current_dir(temp.path())
        .args(["baseline", "create"])
        .assert()
        .success();

    let baseline = fs::read_to_string(temp.path().join(".polint/baseline.yaml")).unwrap();
    assert!(
        baseline.starts_with("version: 1\n\nbaseline:\n"),
        "{baseline}"
    );
    assert!(
        baseline.contains("  - 'local/no-todo-literals "),
        "{baseline}"
    );
    assert!(baseline.contains(" src/component.ts'\n"), "{baseline}");
    assert!(baseline.ends_with("ignore: []\n"), "{baseline}");
}

#[test]
fn baseline_create_preserves_repeated_default_diagnostics_in_same_file() {
    let temp = tempfile::tempdir().unwrap();
    write_no_todo_rule_repo(temp.path(), false);
    write_file(
        &temp.path().join("src/component.ts"),
        r#"export const first = "TODO";
export const second = "TODO";
"#,
    );

    polint_cmd()
        .current_dir(temp.path())
        .args(["baseline", "create"])
        .assert()
        .success();

    let baseline = fs::read_to_string(temp.path().join(".polint/baseline.yaml")).unwrap();
    assert_eq!(baseline.matches("local/no-todo-literals").count(), 2);

    let existing_json = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args([
                "check",
                "--baseline",
                "--new-only",
                "--format",
                "json",
                "--fail-on",
                "warn",
            ])
            .assert()
            .success(),
    );
    assert_eq!(diagnostics(&existing_json).len(), 0, "{existing_json:#?}");
}

#[test]
fn check_baseline_new_only_passes_existing_and_fails_new_diagnostics() {
    let temp = tempfile::tempdir().unwrap();
    write_no_todo_rule_repo(temp.path(), false);
    write_file(
        &temp.path().join("src/existing.ts"),
        r#"export const existing = "TODO";
"#,
    );

    polint_cmd()
        .current_dir(temp.path())
        .args(["baseline", "create"])
        .assert()
        .success();

    let existing_json = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args([
                "check",
                "--baseline",
                "--new-only",
                "--format",
                "json",
                "--fail-on",
                "warn",
            ])
            .assert()
            .success(),
    );
    assert_eq!(diagnostics(&existing_json).len(), 0, "{existing_json:#?}");

    write_file(
        &temp.path().join("src/new.ts"),
        r#"export const new_marker = "TODO";
"#,
    );
    let new_json = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args([
                "check",
                "--baseline",
                "--new-only",
                "--format",
                "json",
                "--fail-on",
                "warn",
            ])
            .assert()
            .failure(),
    );
    let diagnostics = diagnostics_for_rule(&new_json, "local/no-todo-literals");
    assert_eq!(diagnostics.len(), 1, "{new_json:#?}");
    assert_eq!(diagnostics[0]["file"], "src/new.ts");
}

#[test]
fn check_baseline_central_ignore_suppresses_matching_diagnostic() {
    let temp = tempfile::tempdir().unwrap();
    write_no_todo_rule_repo(temp.path(), false);
    write_file(
        &temp.path().join("src/component.ts"),
        r#"export const marker = "TODO";
"#,
    );
    let json = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    let diagnostic = diagnostics_for_rule(&json, "local/no-todo-literals")[0];
    let fingerprint = diagnostic["stable_fingerprint"].as_str().unwrap();
    write_file(
        &temp.path().join(".polint/baseline.yaml"),
        &format!(
            r#"version: 1

baseline: []
ignore:
  - "local/no-todo-literals {fingerprint} src/component.ts"
"#
        ),
    );

    let ignored_json = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args([
                "check",
                "--baseline",
                "--format",
                "json",
                "--fail-on",
                "warn",
            ])
            .assert()
            .success(),
    );

    assert_eq!(diagnostics(&ignored_json).len(), 0, "{ignored_json:#?}");
}

#[test]
fn baseline_update_removes_fixed_entries() {
    let temp = tempfile::tempdir().unwrap();
    write_no_todo_rule_repo(temp.path(), false);
    write_file(
        &temp.path().join("src/keep.ts"),
        r#"export const keep = "TODO";
"#,
    );
    write_file(
        &temp.path().join("src/fix.ts"),
        r#"export const fix = "TODO";
"#,
    );
    polint_cmd()
        .current_dir(temp.path())
        .args(["baseline", "create"])
        .assert()
        .success();

    write_file(
        &temp.path().join("src/fix.ts"),
        r#"export const fix = "DONE";
"#,
    );
    let stdout = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["baseline", "update"])
            .assert()
            .success(),
    );

    let baseline = fs::read_to_string(temp.path().join(".polint/baseline.yaml")).unwrap();
    assert!(baseline.contains("src/keep.ts"), "{baseline}");
    assert!(!baseline.contains("src/fix.ts"), "{baseline}");
    assert!(stdout.contains("fixed: 1"), "{stdout}");
}

#[test]
fn baseline_update_refreshes_unambiguous_moved_default_diagnostic_paths() {
    let temp = tempfile::tempdir().unwrap();
    write_no_todo_rule_repo(temp.path(), false);
    write_file(
        &temp.path().join("src/old.ts"),
        r#"export const old_marker = "TODO";
"#,
    );
    polint_cmd()
        .current_dir(temp.path())
        .args(["baseline", "create"])
        .assert()
        .success();

    fs::remove_file(temp.path().join("src/old.ts")).unwrap();
    write_file(
        &temp.path().join("src/new.ts"),
        r#"export const new_marker = "TODO";
"#,
    );
    polint_cmd()
        .current_dir(temp.path())
        .args(["check", "--baseline", "--new-only", "--fail-on", "warn"])
        .assert()
        .success();

    let stdout = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["baseline", "update"])
            .assert()
            .success(),
    );

    let baseline = fs::read_to_string(temp.path().join(".polint/baseline.yaml")).unwrap();
    assert!(baseline.contains("src/new.ts"), "{baseline}");
    assert!(!baseline.contains("src/old.ts"), "{baseline}");
    assert!(stdout.contains("existing: 1"), "{stdout}");
    assert!(stdout.contains("stale_path: 1"), "{stdout}");
}

#[test]
fn check_baseline_rejects_malformed_yaml_entry() {
    let temp = tempfile::tempdir().unwrap();
    write_no_todo_rule_repo(temp.path(), false);
    write_file(
        &temp.path().join("src/component.ts"),
        r#"export const marker = "TODO";
"#,
    );
    write_file(
        &temp.path().join(".polint/baseline.yaml"),
        r#"version: 1
baseline:
  - "local/no-todo-literals"
"#,
    );

    polint_cmd()
        .current_dir(temp.path())
        .args(["check", "--baseline"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid baseline entry"));
}

#[test]
fn baseline_commands_reject_custom_baseline_paths() {
    let temp = tempfile::tempdir().unwrap();
    write_no_todo_rule_repo(temp.path(), false);

    polint_cmd()
        .current_dir(temp.path())
        .args(["baseline", "create", "--output", ".polint-baseline.yaml"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unexpected argument"));

    polint_cmd()
        .current_dir(temp.path())
        .args(["baseline", "update", "--baseline", ".polint-baseline.yaml"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unexpected argument"));

    polint_cmd()
        .current_dir(temp.path())
        .args(["check", "--baseline=.polint-baseline.yaml"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unexpected value"));
}

#[test]
fn check_new_only_requires_baseline() {
    let temp = tempfile::tempdir().unwrap();

    polint_cmd()
        .current_dir(temp.path())
        .args(["check", "--new-only"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--new-only requires --baseline"));
}

#[test]
fn check_with_no_bundled_rules_reports_no_policy_diagnostics() {
    let temp = tempfile::tempdir().unwrap();
    polint_cmd()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    fs::write(
        temp.path().join("component.tsx"),
        "export function Button() { return <button style={{ color: \"#ff00aa\" }}>Pay</button>; }",
    )
    .unwrap();

    let json = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    assert_eq!(diagnostics(&json).len(), 0);
}

#[test]
fn check_reports_go_parser_diagnostic_for_invalid_source() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[profiles.phase4]
rules = []
"#,
    );
    write_file(
        &temp.path().join("broken.go"),
        "package broken\nfunc Broken( {\n",
    );

    let json = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args([
                "check",
                "--profile",
                "phase4",
                "--format",
                "json",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );

    let diagnostic = diagnostics(&json)
        .iter()
        .find(|diagnostic| {
            diagnostic["rule_id"] == "parser/go" && diagnostic["file"] == "broken.go"
        })
        .expect("invalid Go source should emit parser/go for broken.go");
    assert!(
        diagnostic["message"]
            .as_str()
            .is_some_and(|message| message.contains("syntax error")),
        "parser/go diagnostic should mention syntax error: {diagnostic:#?}"
    );
}

#[test]
fn check_clean_go_fixture_has_no_parser_diagnostics() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[profiles.phase4]
rules = []
"#,
    );
    write_file(
        &temp.path().join("payment.go"),
        include_str!("../../../tests/fixtures/go/clean/payment.go"),
    );
    write_file(
        &temp.path().join("payment_test.go"),
        include_str!("../../../tests/fixtures/go/clean/payment_test.go"),
    );

    let json = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args([
                "check",
                "--profile",
                "phase4",
                "--format",
                "json",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );

    assert!(
        !diagnostics(&json)
            .iter()
            .any(|diagnostic| diagnostic["rule_id"] == "parser/go"),
        "clean Go fixtures should not emit parser/go diagnostics: {json:#?}"
    );
}

#[test]
fn check_go_named_profile_uses_branch_and_test_facts() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[profiles.phase4]
rules = ["local/go-branch-obligations"]
"#,
    );
    write_file(
        &temp.path().join("payment.go"),
        include_str!("../../../tests/fixtures/go/failing/payment.go"),
    );

    let json = stdout_json(
        go_branch_obligations_rule_cmd()
            .current_dir(temp.path())
            .args([
                "check",
                "--profile",
                "phase4",
                "--format",
                "json",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );

    let diagnostic = diagnostics(&json)
        .iter()
        .find(|diagnostic| diagnostic["rule_id"] == "local/go-branch-obligations")
        .expect("failing Go fixture should emit branch-obligation diagnostic");
    assert!(
        diagnostic_has_evidence(diagnostic, "edge", ""),
        "branch diagnostic should include edge evidence: {diagnostic:#?}"
    );
    assert!(
        diagnostic["help"]
            .as_str()
            .is_some_and(|help| help.contains("heuristic")),
        "branch diagnostic help should disclose heuristic behavior: {diagnostic:#?}"
    );
}

#[test]
fn check_go_import_boundary_uses_import_facts() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[profiles.phase4]
rules = ["local/go-import-boundaries"]

[[rules.config]]
id = "local/go-import-boundaries"
files = ["**/*.go"]

[rules.config.forbidden_imports]
"**/*.go" = ["net/http"]
"#,
    );
    write_file(
        &temp.path().join("main.go"),
        "package main\nimport \"net/http\"\nfunc main() { _ = http.MethodGet }\n",
    );

    let json = stdout_json(
        go_import_boundaries_rule_cmd()
            .current_dir(temp.path())
            .args([
                "check",
                "--profile",
                "phase4",
                "--format",
                "json",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );

    let diagnostic = diagnostics(&json)
        .iter()
        .find(|diagnostic| diagnostic["rule_id"] == "local/go-import-boundaries")
        .expect("forbidden Go import should emit import-boundary diagnostic");
    assert!(
        diagnostic_has_evidence(diagnostic, "import", "net/http"),
        "import-boundary diagnostic should include net/http evidence: {diagnostic:#?}"
    );
}

#[test]
fn check_reports_ts_parser_diagnostic_for_invalid_source() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[profiles.phase5]
rules = []
"#,
    );
    write_file(&temp.path().join("broken.ts"), "export function Broken( {");

    let json = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args([
                "check",
                "--profile",
                "phase5",
                "--format",
                "json",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );

    let diagnostic = diagnostics(&json)
        .iter()
        .find(|diagnostic| {
            diagnostic["rule_id"] == "parser/ts" && diagnostic["file"] == "broken.ts"
        })
        .expect("invalid TS source should emit parser/ts for broken.ts");
    assert!(
        diagnostic["message"]
            .as_str()
            .is_some_and(|message| { message.contains("TS/JS parser reported a syntax error") }),
        "parser/ts diagnostic should mention syntax error: {diagnostic:#?}"
    );
}

#[test]
fn check_clean_ts_fixture_has_no_parser_diagnostics() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[profiles.phase5]
rules = []
"#,
    );
    write_file(
        &temp.path().join("component.tsx"),
        include_str!("../../../tests/fixtures/ts/clean/component.tsx"),
    );

    let json = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args([
                "check",
                "--profile",
                "phase5",
                "--format",
                "json",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );

    assert!(
        !diagnostics(&json)
            .iter()
            .any(|diagnostic| diagnostic["rule_id"] == "parser/ts"),
        "clean TS fixture should not emit parser/ts diagnostics: {json:#?}"
    );
}

#[test]
fn check_ts_named_profile_uses_phase5_facts() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r##"
[profiles.phase5]
rules = ["local/ts-cyclomatic-complexity"]

[[rules.config]]
id = "local/ts-cyclomatic-complexity"
max = 3
files = ["**/*.tsx", "**/*.ts", "**/*.jsx", "**/*.js"]
"##,
    );
    write_file(
        &temp.path().join("component.tsx"),
        include_str!("../../../tests/fixtures/ts/failing/component.tsx"),
    );

    let json = stdout_json(
        ts_complexity_rule_cmd()
            .current_dir(temp.path())
            .args([
                "check",
                "--profile",
                "phase5",
                "--format",
                "json",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );

    let complexity = diagnostics(&json)
        .iter()
        .find(|diagnostic| diagnostic["rule_id"] == "local/ts-cyclomatic-complexity")
        .expect("failing TS fixture should emit TS complexity diagnostic");
    assert!(
        diagnostic_has_evidence(complexity, "complexity", ""),
        "complexity diagnostic should include complexity evidence: {complexity:#?}"
    );
}

#[test]
fn check_ts_design_token_example_reports_raw_colors() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[profiles.phase5]
rules = ["local/no-raw-colors"]

[[rules.config]]
id = "local/no-raw-colors"
files = ["**/*.tsx", "**/*.ts", "**/*.jsx", "**/*.js"]
"#,
    );
    write_file(
        &temp.path().join("Button.tsx"),
        include_str!("../../../examples/ts-design-tokens/Button.tsx"),
    );

    let json = stdout_json(
        raw_color_rule_cmd()
            .current_dir(temp.path())
            .args([
                "check",
                "--profile",
                "phase5",
                "--format",
                "json",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );

    let diagnostic = diagnostics(&json)
        .iter()
        .find(|diagnostic| diagnostic["rule_id"] == "local/no-raw-colors")
        .expect("TS design-token example should emit raw-color diagnostic");
    assert_eq!(diagnostic["file"], "Button.tsx");
}

#[test]
fn check_mixed_fixture_handles_go_and_ts_sources() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("mixed/main.go"),
        include_str!("../../../tests/fixtures/mixed/main.go"),
    );
    write_file(
        &temp.path().join("mixed/view.ts"),
        include_str!("../../../tests/fixtures/mixed/view.ts"),
    );
    write_file(
        &temp.path().join(".polint.toml"),
        r#"
[workspace]
include = ["mixed/**"]
exclude = []

[profiles.mixed]
rules = []
"#,
    );

    let json = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args([
                "check",
                "--profile",
                "mixed",
                "--format",
                "json",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );
    assert!(
        json.get("version").and_then(|v| v.as_u64()) == Some(1) && diagnostics(&json).is_empty(),
        "check output should be polint JSON report: {json:#?}"
    );
}

#[test]
fn checked_in_examples_are_runnable_cli_fixtures() {
    struct ExampleCase {
        name: &'static str,
        rule_modules: &'static [&'static str],
        expected_rules: &'static [&'static str],
        expected_files: &'static [&'static str],
    }

    let cases: &[ExampleCase] = &[
        ExampleCase {
            name: "basic",
            rule_modules: &["no_raw_colors.rs"],
            expected_rules: &["local/no-raw-colors"],
            expected_files: &["Button.tsx"],
        },
        ExampleCase {
            name: "code-quality-metrics",
            rule_modules: &[
                "large_files.rs",
                "large_functions.rs",
                "code_quality_score.rs",
            ],
            expected_rules: &[
                "local/large-file",
                "local/large-function",
                "local/code-quality-score",
            ],
            expected_files: &["service.go", "Panel.tsx"],
        },
        ExampleCase {
            name: "comment-ignores",
            rule_modules: &["no_denied_literals.rs"],
            expected_rules: &["local/no-denied-literals"],
            expected_files: &["app.ts"],
        },
        ExampleCase {
            name: "config-denied-literal",
            rule_modules: &["no_denied_literals.rs"],
            expected_rules: &["local/no-denied-literals"],
            expected_files: &["query.ts"],
        },
        ExampleCase {
            name: "custom-rule-go",
            rule_modules: &["require_error_branch_tests.rs"],
            expected_rules: &["local/require-error-branch-tests"],
            expected_files: &["authorize.go"],
        },
        ExampleCase {
            name: "custom-rule-ts",
            rule_modules: &["no_product_hex_colors.rs"],
            expected_rules: &["local/no-product-hex-colors"],
            expected_files: &["Button.tsx"],
        },
        ExampleCase {
            name: "go-branch-obligations",
            rule_modules: &["go_branch_obligations.rs"],
            expected_rules: &["local/go-branch-obligations"],
            expected_files: &["authorize.go"],
        },
        ExampleCase {
            name: "go-complexity",
            rule_modules: &["go_complexity.rs"],
            expected_rules: &["local/go-cyclomatic-complexity"],
            expected_files: &["router.go"],
        },
        ExampleCase {
            name: "go-import-boundaries",
            rule_modules: &["go_import_boundaries.rs"],
            expected_rules: &["local/go-import-boundaries"],
            expected_files: &["handler.go"],
        },
        ExampleCase {
            name: "go-sensitive-writes",
            rule_modules: &["no_sensitive_balance_writes.rs"],
            expected_rules: &["local/no-sensitive-balance-writes"],
            expected_files: &["ledger.go"],
        },
        ExampleCase {
            name: "go-test-quality",
            rule_modules: &["go_test_quality.rs"],
            expected_rules: &["local/go-test-quality"],
            expected_files: &["payment_test.go"],
        },
        ExampleCase {
            name: "ts-complexity",
            rule_modules: &["ts_complexity.rs"],
            expected_rules: &["local/ts-cyclomatic-complexity"],
            expected_files: &["label.ts"],
        },
        ExampleCase {
            name: "ts-design-tokens",
            rule_modules: &["no_raw_colors.rs"],
            expected_rules: &["local/no-raw-colors"],
            expected_files: &["Button.tsx"],
        },
        ExampleCase {
            name: "ts-no-raw-api-calls",
            rule_modules: &["no_raw_api_calls.rs"],
            expected_rules: &["local/no-raw-api-calls"],
            expected_files: &["src/user-page.ts"],
        },
    ];

    for case in cases {
        let example = case.name;
        let example_dir = repo_root().join("examples").join(example);
        assert!(
            example_dir.join("README.md").exists(),
            "{example} should document how to run the example"
        );
        assert!(
            example_dir.join(".polint.toml").exists(),
            "{example} should be runnable as a mini repository"
        );
        assert!(
            case.rule_modules.iter().all(|rule_module| example_dir
                .join(".polint/rules/src")
                .join(rule_module)
                .exists()),
            "{example} should contain its local rule implementations ({:?})",
            case.rule_modules
        );

        let json = stdout_json(
            polint_cmd()
                .current_dir(&example_dir)
                .args([
                    "check",
                    "--format",
                    "json",
                    "--no-cache",
                    "--fail-on",
                    "none",
                ])
                .assert()
                .success(),
        );

        let actual_rules = diagnostics(&json)
            .iter()
            .filter_map(|diagnostic| diagnostic["rule_id"].as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let expected_rules = case
            .expected_rules
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            actual_rules, expected_rules,
            "{example} should emit only its expected local rules: {json:#?}"
        );
        for file in case.expected_files {
            assert!(
                diagnostics(&json)
                    .iter()
                    .any(|diagnostic| diagnostic["file"] == *file),
                "{example} should report a diagnostic in {file}: {json:#?}"
            );
        }
    }
}

#[test]
fn ts_no_raw_api_calls_example_reports_resolved_and_unresolved_calls() {
    let example_dir = repo_root().join("examples/ts-no-raw-api-calls");
    let json = stdout_json(
        polint_cmd()
            .current_dir(&example_dir)
            .args([
                "check",
                "--format",
                "json",
                "--no-cache",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );

    let diagnostics = diagnostics_for_rule(&json, "local/no-raw-api-calls");
    assert_eq!(diagnostics.len(), 2, "{json:#?}");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic["file"] == "src/user-page.ts"
                && diagnostic_has_evidence(diagnostic, "api", "rawRequest")
                && diagnostic_has_evidence(diagnostic, "status", "Resolved")
                && diagnostic_has_evidence(diagnostic, "precision", "ExactLocal")
        }),
        "expected resolved rawRequest call diagnostic: {json:#?}"
    );
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic["file"] == "src/user-page.ts"
                && diagnostic_has_evidence(diagnostic, "api", "fetch")
                && diagnostic_has_evidence(diagnostic, "status", "Unresolved")
                && diagnostic_has_evidence(diagnostic, "symbol_id", "unresolved")
        }),
        "expected unresolved fetch call diagnostic: {json:#?}"
    );
}

#[test]
fn go_sensitive_writes_example_reports_write_and_readwrite_references() {
    let example_dir = repo_root().join("examples/go-sensitive-writes");
    let json = stdout_json(
        polint_cmd()
            .current_dir(&example_dir)
            .args([
                "check",
                "--format",
                "json",
                "--no-cache",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );

    let diagnostics = diagnostics_for_rule(&json, "local/no-sensitive-balance-writes");
    assert_eq!(diagnostics.len(), 2, "{json:#?}");
    assert!(
        diagnostics.iter().all(|diagnostic| {
            diagnostic["file"] == "ledger.go"
                && diagnostic_has_evidence(diagnostic, "field", "Balance")
                && diagnostic_has_evidence(diagnostic, "status", "Resolved")
                && diagnostic_has_evidence(diagnostic, "precision", "ExactSemantic")
        }),
        "expected only exact semantic Balance write diagnostics in ledger.go: {json:#?}"
    );
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic_has_evidence(
            diagnostic,
            "reference_kind",
            "Write"
        )),
        "expected a direct write diagnostic: {json:#?}"
    );
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic_has_evidence(
            diagnostic,
            "reference_kind",
            "ReadWrite"
        )),
        "expected a read-write diagnostic: {json:#?}"
    );
}

#[test]
fn comment_ignores_example_reports_one_visible_and_one_ignored_diagnostic() {
    let example_dir = repo_root().join("examples/comment-ignores");

    let check_json = stdout_json(
        polint_cmd()
            .current_dir(&example_dir)
            .args([
                "check",
                "--format",
                "json",
                "--no-cache",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );
    let visible = diagnostics_for_rule(&check_json, "local/no-denied-literals");
    assert_eq!(visible.len(), 1, "{check_json:#?}");
    assert_eq!(visible[0]["range"]["start_line"], 6);
    assert!(
        diagnostics_for_rule(&check_json, "polint/ignore-missing-reason").is_empty(),
        "{check_json:#?}"
    );

    let ignores_json = stdout_json(
        polint_cmd()
            .current_dir(&example_dir)
            .args([
                "ignores",
                "--format",
                "json",
                "--filter",
                "local/no-denied-literals",
                "--no-cache",
            ])
            .assert()
            .success(),
    );
    assert_eq!(
        ignores_json["summary"]["directives"], 1,
        "{ignores_json:#?}"
    );
    assert_eq!(ignores_json["summary"]["active"], 1);
    assert_eq!(ignores_json["summary"]["suppressed_diagnostics"], 1);
    assert_eq!(ignores_json["directives"][0]["line"], 2);
    assert_eq!(
        ignores_json["directives"][0]["suppressed"][0]["rule_id"],
        "local/no-denied-literals"
    );

    let stat = stdout_string(
        polint_cmd()
            .current_dir(&example_dir)
            .args(["ignores", "--stat", "--no-cache"])
            .assert()
            .success(),
    );
    assert!(stat.contains("1 directives, 1 active"), "{stat}");
    assert!(stat.contains("By rule"), "{stat}");

    let check_shortstat = stdout_string(
        polint_cmd()
            .current_dir(&example_dir)
            .args(["check", "--shortstat", "--no-cache", "--fail-on", "none"])
            .assert()
            .success(),
    );
    assert!(
        check_shortstat
            .contains("Scanned 1 file; 1 diagnostics; 1 suppressed by 1 ignore directives"),
        "{check_shortstat}"
    );

    let check_stat = stdout_string(
        polint_cmd()
            .current_dir(&example_dir)
            .args(["check", "--stat", "--no-cache", "--fail-on", "none"])
            .assert()
            .success(),
    );
    assert!(check_stat.contains("By language"), "{check_stat}");
    assert!(check_stat.contains("  ts: 1"), "{check_stat}");
    assert!(check_stat.contains("Ignores"), "{check_stat}");

    let check_json_with_stats = stdout_json(
        polint_cmd()
            .current_dir(&example_dir)
            .args([
                "check",
                "--format",
                "json",
                "--stat",
                "--shortstat",
                "--no-cache",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );
    assert_eq!(
        diagnostics_for_rule(&check_json_with_stats, "local/no-denied-literals").len(),
        1,
        "{check_json_with_stats:#?}"
    );
}

#[test]
fn check_with_local_rule_host_respects_positional_paths() {
    let example_dir = repo_root().join("examples/config-denied-literal");

    let json = stdout_json(
        polint_cmd()
            .current_dir(&example_dir)
            .args([
                "check",
                "query.ts",
                "--format",
                "json",
                "--no-cache",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );
    assert_eq!(
        diagnostic_files(&json, "local/no-denied-literals"),
        vec!["query.ts".to_string()]
    );

    let empty = stdout_json(
        polint_cmd()
            .current_dir(&example_dir)
            .args([
                "check",
                "README.md",
                "--format",
                "json",
                "--no-cache",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );
    assert!(diagnostics(&empty).is_empty());
}

#[test]
fn check_with_local_rule_host_can_emit_github_annotations() {
    let example_dir = repo_root().join("examples/config-denied-literal");

    let stdout = stdout_string(
        polint_cmd()
            .current_dir(&example_dir)
            .args([
                "check",
                "--format",
                "github",
                "--no-cache",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );

    assert!(
        stdout.contains("::error file=query.ts,line="),
        "expected GitHub annotation output: {stdout}"
    );
    assert!(
        stdout.contains("title=local/no-denied-literals::Configured denied literal"),
        "expected rule id and message in annotation: {stdout}"
    );
}

#[test]
fn checked_in_multiple_rules_example_uses_one_rule_pack_crate() {
    let example_dir = repo_root().join("examples/multiple-rules");
    assert!(
        example_dir.join("README.md").exists(),
        "multiple-rules should document the rule-pack structure"
    );
    assert!(
        example_dir.join(".polint.toml").exists(),
        "multiple-rules should be runnable as a mini repository"
    );
    assert!(
        example_dir.join(".polint/rules/Cargo.toml").exists(),
        "multiple-rules should use one Cargo manifest for its rule pack"
    );
    assert!(
        example_dir
            .join(".polint/rules/src/no_raw_colors.rs")
            .exists(),
        "multiple-rules should keep no-raw-colors in its own module"
    );
    assert!(
        example_dir
            .join(".polint/rules/src/go_import_boundaries.rs")
            .exists(),
        "multiple-rules should keep go-import-boundaries in its own module"
    );
    assert!(
        !example_dir
            .join(".polint/rules/no-raw-colors/Cargo.toml")
            .exists(),
        "multiple-rules should not create one Cargo package per rule"
    );

    let json = stdout_json(
        polint_cmd()
            .current_dir(&example_dir)
            .args([
                "check",
                "--format",
                "json",
                "--no-cache",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );

    let actual_rules = diagnostics(&json)
        .iter()
        .filter_map(|diagnostic| diagnostic["rule_id"].as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        actual_rules,
        std::collections::BTreeSet::from(["local/go-import-boundaries", "local/no-raw-colors"]),
        "multiple-rules should emit diagnostics from both registered local rules: {json:#?}"
    );
    for file in ["Button.tsx", "handler.go"] {
        assert!(
            diagnostics(&json)
                .iter()
                .any(|diagnostic| diagnostic["file"] == file),
            "multiple-rules should report a diagnostic in {file}: {json:#?}"
        );
    }
}

#[test]
fn check_with_unknown_profile_is_an_error() {
    let example_dir = repo_root().join("examples/config-denied-literal");

    polint_cmd()
        .current_dir(&example_dir)
        .args(["check", "--profile", "missing", "--fail-on", "none"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "profile `missing` is not defined in .polint.toml",
        ));
}

#[test]
fn check_rejects_unknown_config_flag() {
    polint_cmd()
        .args(["check", "--config"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unexpected argument '--config'"));
}

#[test]
fn check_ts_no_raw_colors_dedupes_real_jsx_attribute_literal() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join(".polint.toml"),
        r##"
[profiles.phase6]
rules = ["local/no-raw-colors"]

[[rules.config]]
id = "local/no-raw-colors"
files = ["**/*.tsx"]
"##,
    );
    write_file(
        &temp.path().join("Button.tsx"),
        r##"
export function Button() {
  return <button data-color="#00ff00">Pay</button>;
}
"##,
    );

    let json = stdout_json(
        raw_color_rule_cmd()
            .current_dir(temp.path())
            .args([
                "check",
                "--profile",
                "phase6",
                "--format",
                "json",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );

    let raw_color_diagnostics = diagnostics_for_rule(&json, "local/no-raw-colors");
    assert_eq!(
        raw_color_diagnostics.len(),
        1,
        "JSX attribute literal should be reported once: {raw_color_diagnostics:#?}"
    );
    assert!(diagnostic_has_evidence(
        raw_color_diagnostics[0],
        "literal",
        "#00ff00"
    ));
}

#[test]
fn check_json_without_config_is_parseable() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("component.tsx"),
        "export function Button() { return <button style={{ color: \"#ff00aa\" }}>Pay</button>; }",
    )
    .unwrap();

    let assert = polint_cmd()
        .current_dir(temp.path())
        .args(["check", "--format", "json", "--fail-on", "none"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Config not found").not());
    let json = stdout_json(assert);

    assert_eq!(diagnostics(&json).len(), 0);
}

#[test]
fn check_human_without_config_suggests_init() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("component.tsx"),
        "export const label = \"ok\";",
    )
    .unwrap();

    polint_cmd()
        .current_dir(temp.path())
        .args(["check", "--fail-on", "none"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Config not found. Run `polint init` to create .polint.toml.",
        ));
}

#[test]
fn profile_and_severity_override_affect_json_and_exit_code() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join(".polint.toml"),
        r#"
[profiles.phase2]
rules = ["local/no-raw-colors"]

[[rules.config]]
id = "local/no-raw-colors"
severity = "info"
files = ["**/*.tsx"]
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("component.tsx"),
        "export const color = \"#ff00aa\";",
    )
    .unwrap();

    let json = stdout_json(
        raw_color_rule_cmd()
            .current_dir(temp.path())
            .args([
                "check",
                "--profile",
                "phase2",
                "--format",
                "json",
                "--fail-on",
                "warn",
            ])
            .assert()
            .success(),
    );

    let diagnostic = diagnostics(&json)
        .iter()
        .find(|diagnostic| diagnostic["rule_id"] == "local/no-raw-colors")
        .unwrap();
    assert_eq!(diagnostic["severity"], "info");
}

mod capability_planning {
    use super::*;

    #[test]
    fn cfg_capability_remains_unsupported() {
        let temp = tempfile::tempdir().unwrap();
        write_plan_capability_rule_repo(temp.path());

        let json = stdout_json(
            polint_cmd()
                .current_dir(temp.path())
                .args(["check", "--format", "json", "--fail-on", "none"])
                .assert()
                .success(),
        );

        let diagnostic = diagnostics(&json)
            .iter()
            .find(|diagnostic| diagnostic["rule_id"] == "polint/capability")
            .unwrap_or_else(|| panic!("expected CFG capability diagnostic: {json:#?}"));
        assert!(
            diagnostic["message"]
                .as_str()
                .is_some_and(|message| message.contains("unsupported capability `cfg`")),
            "{diagnostic:#?}"
        );
        assert!(diagnostic_has_evidence(
            diagnostic,
            "rule",
            "local/needs-cfg"
        ));
    }

    #[test]
    fn call_graph_capability_remains_unsupported() {
        let temp = tempfile::tempdir().unwrap();
        write_call_graph_capability_rule_repo(temp.path());

        let json = stdout_json(
            polint_cmd()
                .current_dir(temp.path())
                .args(["check", "--format", "json", "--fail-on", "none"])
                .assert()
                .success(),
        );

        assert!(
            diagnostics_for_rule(&json, "local/needs-call-graph").is_empty(),
            "rule requesting unsupported call_graph must not execute with fabricated facts: {json:#?}"
        );
        let diagnostic = diagnostics(&json)
            .iter()
            .find(|diagnostic| diagnostic["rule_id"] == "polint/capability")
            .unwrap_or_else(|| panic!("expected call_graph capability diagnostic: {json:#?}"));
        assert!(
            diagnostic["message"]
                .as_str()
                .is_some_and(|message| message.contains("unsupported capability `call_graph`")),
            "{diagnostic:#?}"
        );
        assert!(
            diagnostic["help"]
                .as_str()
                .is_some_and(|help| help.contains("docs/facts/capability-plans.md")),
            "{diagnostic:#?}"
        );
        assert!(diagnostic_has_evidence(
            diagnostic,
            "rule",
            "local/needs-call-graph"
        ));
        assert!(diagnostic_has_evidence(
            diagnostic,
            "capability",
            "call_graph"
        ));
        assert!(diagnostic_has_evidence(diagnostic, "status", "unsupported"));
    }

    #[test]
    fn reserved_cfg_and_call_graph_remain_unsupported() {
        let cfg_temp = tempfile::tempdir().unwrap();
        write_plan_capability_rule_repo(cfg_temp.path());
        let cfg_json = stdout_json(
            polint_cmd()
                .current_dir(cfg_temp.path())
                .args(["check", "--format", "json", "--fail-on", "none"])
                .assert()
                .success(),
        );
        assert!(
            diagnostics_for_rule(&cfg_json, "polint/capability")
                .iter()
                .any(
                    |diagnostic| diagnostic_has_evidence(diagnostic, "capability", "cfg")
                        && diagnostic_has_evidence(diagnostic, "status", "unsupported")
                ),
            "Cfg<'_> should stay a reserved raw capability: {cfg_json:#?}"
        );

        let call_temp = tempfile::tempdir().unwrap();
        write_call_graph_capability_rule_repo(call_temp.path());
        let call_json = stdout_json(
            polint_cmd()
                .current_dir(call_temp.path())
                .args(["check", "--format", "json", "--fail-on", "none"])
                .assert()
                .success(),
        );
        assert!(
            diagnostics_for_rule(&call_json, "polint/capability")
                .iter()
                .any(
                    |diagnostic| diagnostic_has_evidence(diagnostic, "capability", "call_graph")
                        && diagnostic_has_evidence(diagnostic, "status", "unsupported")
                ),
            "CallGraph<'_> should stay a reserved raw capability: {call_json:#?}"
        );
    }

    #[test]
    fn check_reports_unsupported_reserved_capability() {
        let temp = tempfile::tempdir().unwrap();
        write_plan_capability_rule_repo(temp.path());

        let json = stdout_json(
            polint_cmd()
                .current_dir(temp.path())
                .args(["check", "--format", "json", "--fail-on", "none"])
                .assert()
                .success(),
        );

        let diagnostic = diagnostics(&json)
            .iter()
            .find(|diagnostic| diagnostic["rule_id"] == "polint/capability")
            .unwrap_or_else(|| panic!("expected capability diagnostic: {json:#?}"));
        assert!(
            diagnostic["message"]
                .as_str()
                .is_some_and(|message| message.contains("unsupported capability `cfg`")),
            "{diagnostic:#?}"
        );
        assert!(
            diagnostic["help"]
                .as_str()
                .is_some_and(|help| help.contains("docs/facts/capability-plans.md")),
            "{diagnostic:#?}"
        );
        assert!(diagnostic_has_evidence(
            diagnostic,
            "rule",
            "local/needs-cfg"
        ));
    }

    #[test]
    fn check_only_rule_keeps_selected_capability_diagnostic() {
        let temp = tempfile::tempdir().unwrap();
        write_plan_capability_rule_repo(temp.path());

        let json = stdout_json(
            polint_cmd()
                .current_dir(temp.path())
                .args([
                    "check",
                    "--format",
                    "json",
                    "--only-rule",
                    "local/needs-cfg",
                ])
                .assert()
                .failure(),
        );

        let diagnostics = diagnostics(&json);
        assert_eq!(diagnostics.len(), 1, "{json:#?}");
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic["rule_id"], "polint/capability");
        assert!(
            diagnostic["message"]
                .as_str()
                .is_some_and(|message| message.contains("unsupported capability `cfg`")),
            "{diagnostic:#?}"
        );
        assert!(diagnostic_has_evidence(
            diagnostic,
            "rule",
            "local/needs-cfg"
        ));
    }

    #[test]
    fn relationship_capability_rule_runs_on_empty_repo() {
        let temp = tempfile::tempdir().unwrap();
        write_relationship_capability_rule_repo(temp.path());

        let json = stdout_json(
            polint_cmd()
                .current_dir(temp.path())
                .args(["check", "--format", "json", "--fail-on", "none"])
                .assert()
                .success(),
        );

        assert!(
            diagnostics(&json)
                .iter()
                .all(|diagnostic| diagnostic["rule_id"] != "polint/capability"),
            "{json:#?}"
        );
        assert!(
            diagnostics(&json)
                .iter()
                .any(|diagnostic| diagnostic["rule_id"] == "local/relationships-available"),
            "{json:#?}"
        );
    }

    #[test]
    fn provider_setup_missing_support_blocks_requesting_rule() {
        let temp = tempfile::tempdir().unwrap();
        write_go_relationship_setup_missing_rule_repo(temp.path());

        let json = stdout_json(
            polint_cmd()
                .current_dir(temp.path())
                .args(["check", "--format", "json", "--fail-on", "none"])
                .assert()
                .success(),
        );

        assert!(
            diagnostics(&json)
                .iter()
                .all(|diagnostic| diagnostic["rule_id"] != "local/go-relationships"),
            "{json:#?}"
        );
        let diagnostic = diagnostics(&json)
            .iter()
            .find(|diagnostic| diagnostic["rule_id"] == "polint/capability")
            .unwrap_or_else(|| panic!("expected capability diagnostic: {json:#?}"));
        assert!(
            diagnostic["message"]
                .as_str()
                .is_some_and(|message| message.contains("required setup is missing")),
            "{diagnostic:#?}"
        );
        assert!(diagnostic_has_evidence(
            diagnostic,
            "status",
            "setup_missing"
        ));
    }

    #[test]
    fn symbol_provider_support_allows_requesting_rule_before_supported_rules() {
        let temp = tempfile::tempdir().unwrap();
        write_symbol_capability_rule_repo(temp.path());

        let json = stdout_json(
            polint_cmd()
                .current_dir(temp.path())
                .args(["check", "--format", "json", "--fail-on", "none"])
                .assert()
                .success(),
        );

        assert!(
            diagnostics_for_rule(&json, "local/needs-references")
                .iter()
                .any(|diagnostic| diagnostic_has_evidence(diagnostic, "reference_count", "2")),
            "supported symbol providers should allow the requesting rule to run: {json:#?}"
        );
        assert!(
            diagnostics(&json)
                .iter()
                .all(|diagnostic| diagnostic["rule_id"] != "polint/capability"),
            "supported TS symbol/reference providers should not emit capability diagnostics: {json:#?}"
        );
    }

    #[test]
    fn kernel_delegation_preserves_existing_rule_facts() {
        let runner_source =
            fs::read_to_string(repo_root().join("crates/polint/src/runner/mod.rs")).unwrap();
        let cli_source =
            fs::read_to_string(repo_root().join("crates/polint/src/cli/mod.rs")).unwrap();
        assert!(
            runner_source.contains("AnalysisKernel::run") && runner_source.contains("KernelInput"),
            "runner analysis path must delegate through AnalysisKernel"
        );
        assert!(
            cli_source.contains("AnalysisKernel::run") && cli_source.contains("KernelInput"),
            "parent CLI analysis path must delegate through AnalysisKernel"
        );

        let temp = tempfile::tempdir().unwrap();
        write_external_symbol_rule_repo(
            temp.path(),
            r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/kernel-fact-probe",
    description = "Reads facts after kernel delegation.",
    severity = "warn"
)]
fn kernel_fact_probe(
    ctx: &mut RuleCtx<'_>,
    file_metrics: FileMetrics<'_>,
    function_metrics: FunctionMetrics<'_>,
    complexity_metrics: ComplexityMetrics<'_>,
    symbols: Symbols<'_>,
    references: References<'_>,
) -> RuleResult {
    let Some(symbol) = symbols.by_name("answer").next() else {
        return Ok(());
    };
    let _reference_count = references.iter().count();
    let file = symbol
        .file
        .map(|file| ctx.file_path(file))
        .unwrap_or_else(|| "<workspace>".to_string());

    ctx.report(
        Diagnostic::warning(
            ctx.rule_id(),
            file,
            DiagnosticRange::point(1, 1),
            "kernel delegation preserved rule-visible facts",
        )
        .with_evidence("file_metrics", file_metrics.iter().count().to_string())
        .with_evidence("function_metrics", function_metrics.iter().count().to_string())
        .with_evidence("complexity_metrics", complexity_metrics.iter().count().to_string())
        .with_evidence("symbol", symbol.name.clone()),
    );
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![kernel_fact_probe()])
}
"#,
        );
        write_file(
            &temp.path().join("src/app.ts"),
            "export function answer() { return 42; }\n",
        );

        let json = stdout_json(
            polint_cmd()
                .current_dir(temp.path())
                .args(["check", "--format", "json", "--fail-on", "none"])
                .assert()
                .success(),
        );

        let diagnostics = diagnostics_for_rule(&json, "local/kernel-fact-probe");
        assert_eq!(diagnostics.len(), 1, "{json:#?}");
        let diagnostic = diagnostics[0];
        assert!(diagnostic_has_evidence(diagnostic, "file_metrics", "1"));
        assert!(diagnostic_has_evidence(diagnostic, "function_metrics", "1"));
        assert!(diagnostic_has_evidence(
            diagnostic,
            "complexity_metrics",
            "1"
        ));
        assert!(diagnostic_has_evidence(diagnostic, "symbol", "answer"));
    }

    #[test]
    fn kernel_metadata_preserves_public_check_behavior() {
        let temp = tempfile::tempdir().unwrap();
        write_external_symbol_rule_repo(
            temp.path(),
            r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/kernel-metadata-public-probe",
    description = "Reads public facts after kernel metadata checks.",
    severity = "warn"
)]
fn kernel_metadata_public_probe(
    ctx: &mut RuleCtx<'_>,
    symbols: Symbols<'_>,
    references: References<'_>,
) -> RuleResult {
    let Some(symbol) = symbols
        .by_name("answer")
        .find(|symbol| symbol.kind == SymbolKind::Function)
    else {
        return Ok(());
    };
    let reference_count = references.to(symbol.id).count();
    let file = symbol
        .file
        .map(|file| ctx.file_path(file))
        .unwrap_or_else(|| "<workspace>".to_string());

    ctx.report(
        Diagnostic::warning(
            ctx.rule_id(),
            file,
            DiagnosticRange::point(1, 1),
            "kernel metadata preserved public symbol facts",
        )
        .with_evidence("symbol_name", symbol.name.clone())
        .with_evidence("symbol_stable_key", symbols.stable_key(symbol).to_string())
        .with_evidence("reference_count", reference_count.to_string()),
    );
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![kernel_metadata_public_probe()])
}
"#,
        );
        write_file(
            &temp.path().join("src/app.ts"),
            r#"export function answer() {
  return 42;
}

export const value = answer();
"#,
        );

        let run = || {
            stdout_string(
                polint_cmd()
                    .current_dir(temp.path())
                    .args(["check", "--format", "json", "--fail-on", "none"])
                    .assert()
                    .success(),
            )
        };
        let first = run();
        let second = run();
        assert_eq!(first, second);

        let report: polint::sdk::prelude::PolintReport = serde_json::from_str(&first)
            .unwrap_or_else(|error| {
                panic!("stdout was not a public PolintReport: {error}\n{first}")
            });
        assert!(
            report
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.rule_id != "polint/internal"),
            "clean repo should not expose internal diagnostics: {report:#?}"
        );
        assert!(
            report
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.rule_id != "polint/capability"),
            "public symbol/reference flow should not emit capability diagnostics: {report:#?}"
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.rule_id == "local/kernel-metadata-public-probe"),
            "external public SDK rule should run: {report:#?}"
        );

        for metadata_key in ["producer_id", "layer_id", "confidence", "validation"] {
            assert!(
                !first.contains(&format!("\"{metadata_key}\"")),
                "public JSON must not include metadata-only key `{metadata_key}`:\n{first}"
            );
        }
    }

    fn symbol_reference_id_evidence(
        report: &polint::sdk::prelude::PolintReport,
    ) -> Vec<(String, String, String, String, String)> {
        report
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.rule_id == "local/stable-symbol-reference")
            .map(|diagnostic| {
                (
                    evidence_value(diagnostic, "symbol_id"),
                    evidence_value(diagnostic, "reference_id"),
                    evidence_value(diagnostic, "reference_target"),
                    evidence_value(diagnostic, "reference_status"),
                    evidence_value(diagnostic, "reference_precision"),
                )
            })
            .collect()
    }

    fn evidence_value(diagnostic: &polint::sdk::prelude::Diagnostic, label: &str) -> String {
        diagnostic
            .evidence
            .iter()
            .find(|evidence| evidence.label == label)
            .unwrap_or_else(|| panic!("missing evidence `{label}` in {diagnostic:#?}"))
            .value
            .clone()
    }

    fn run_symbol_reference_cache_check(root: &Path) -> polint::sdk::prelude::PolintReport {
        let raw = stdout_string(
            polint_cmd()
                .current_dir(root)
                .args(["check", "--format", "json", "--fail-on", "none"])
                .assert()
                .success(),
        );
        serde_json::from_str(&raw)
            .unwrap_or_else(|error| panic!("stdout was not a PolintReport: {error}\n{raw}"))
    }

    fn run_external_symbol_reference_check(root: &Path) -> polint::sdk::prelude::PolintReport {
        let raw = stdout_string(
            polint_cmd()
                .current_dir(root)
                .args(["check", "--format", "json", "--fail-on", "none"])
                .assert()
                .success(),
        );
        serde_json::from_str(&raw)
            .unwrap_or_else(|error| panic!("stdout was not a PolintReport: {error}\n{raw}"))
    }

    fn assert_written_external_symbol_rule_is_public(root: &Path) {
        let source = fs::read_to_string(root.join(".polint/rules/src/main.rs")).unwrap();
        assert_public_symbol_rule_source(&source);
    }

    fn report_diagnostics_for_rule<'a>(
        report: &'a polint::sdk::prelude::PolintReport,
        rule_id: &str,
    ) -> Vec<&'a polint::sdk::prelude::Diagnostic> {
        report
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.rule_id == rule_id)
            .collect()
    }

    fn assert_no_capability_diagnostics(report: &polint::sdk::prelude::PolintReport) {
        assert!(
            report_diagnostics_for_rule(report, "polint/capability").is_empty(),
            "public symbol/reference facts should not emit capability diagnostics: {report:#?}"
        );
    }

    fn diagnostic_cases(diagnostics: &[&polint::sdk::prelude::Diagnostic]) -> Vec<String> {
        diagnostics
            .iter()
            .map(|diagnostic| evidence_value(diagnostic, "case"))
            .collect()
    }

    fn diagnostic_for_case<'a>(
        diagnostics: &'a [&polint::sdk::prelude::Diagnostic],
        case: &str,
    ) -> &'a polint::sdk::prelude::Diagnostic {
        diagnostics
            .iter()
            .copied()
            .find(|diagnostic| evidence_value(diagnostic, "case") == case)
            .unwrap_or_else(|| panic!("missing diagnostic case `{case}` in {diagnostics:#?}"))
    }

    fn evidence_snapshot(
        report: &polint::sdk::prelude::PolintReport,
        rule_id: &str,
    ) -> Vec<Vec<(String, String)>> {
        report_diagnostics_for_rule(report, rule_id)
            .iter()
            .map(|diagnostic| {
                diagnostic
                    .evidence
                    .iter()
                    .map(|evidence| (evidence.label.clone(), evidence.value.clone()))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn symbol_reference_cache_and_setup_keeps_stable_ids_across_cached_runs() {
        let temp = tempfile::tempdir().unwrap();
        write_symbol_reference_cache_rule_repo(temp.path());

        let first = run_symbol_reference_cache_check(temp.path());
        assert!(
            !cache_json_file_names(temp.path()).is_empty(),
            "first run should write cache entries"
        );
        let second = run_symbol_reference_cache_check(temp.path());

        let first_ids = symbol_reference_id_evidence(&first);
        let second_ids = symbol_reference_id_evidence(&second);
        assert!(
            !first_ids.is_empty(),
            "expected rule-reported symbol/reference ID evidence: {first:#?}"
        );
        assert_eq!(first_ids, second_ids);
        assert!(first_ids.iter().all(
            |(_symbol_id, _reference_id, reference_target, status, precision)| {
                reference_target != "none" && status == "Resolved" && precision == "ExactLocal"
            }
        ));
    }

    #[test]
    fn symbol_reference_cache_and_setup_reports_go_setup_missing() {
        let temp = tempfile::tempdir().unwrap();
        write_go_symbol_setup_missing_rule_repo(temp.path());

        let json = stdout_json(
            polint_cmd()
                .current_dir(temp.path())
                .args(["check", "--format", "json", "--fail-on", "none"])
                .assert()
                .success(),
        );

        assert!(
            diagnostics_for_rule(&json, "local/go-symbols").is_empty(),
            "setup-missing Go symbol support must block the requesting rule: {json:#?}"
        );
        let capability_diagnostics = diagnostics_for_rule(&json, "polint/capability");
        assert!(
            capability_diagnostics.iter().any(|diagnostic| {
                diagnostic_has_evidence(diagnostic, "capability", "symbols")
                    && diagnostic_has_evidence(diagnostic, "language", "Go")
                    && diagnostic_has_evidence(diagnostic, "status", "setup_missing")
            }),
            "expected setup-missing symbols diagnostic: {json:#?}"
        );
        assert!(
            capability_diagnostics.iter().any(|diagnostic| {
                diagnostic_has_evidence(diagnostic, "capability", "references")
                    && diagnostic_has_evidence(diagnostic, "language", "Go")
                    && diagnostic_has_evidence(diagnostic, "status", "setup_missing")
            }),
            "expected setup-missing references diagnostic: {json:#?}"
        );
    }

    #[test]
    fn external_rule_consumes_ts_symbols_and_references_through_public_sdk() {
        let temp = tempfile::tempdir().unwrap();
        write_external_ts_symbol_reference_rule_repo(temp.path());
        assert_written_external_symbol_rule_is_public(temp.path());

        let report = run_external_symbol_reference_check(temp.path());
        assert_no_capability_diagnostics(&report);

        let diagnostics =
            report_diagnostics_for_rule(&report, "local/ts-symbol-reference-public-sdk");
        let mut cases = diagnostic_cases(&diagnostics);
        cases.sort();
        assert_eq!(
            cases,
            [
                "ts-reference:function-call",
                "ts-reference:local-variable",
                "ts-reference:module-import",
                "ts-reference:unresolved-global",
                "ts-symbol:declaration-merge",
                "ts-symbol:function",
                "ts-symbol:local-variable",
            ],
            "{report:#?}"
        );

        let function = diagnostic_for_case(&diagnostics, "ts-symbol:function");
        assert_eq!(evidence_value(function, "symbol_name"), "answer");
        assert_eq!(evidence_value(function, "symbol_kind"), "Function");
        assert_eq!(evidence_value(function, "symbol_precision"), "ExactLocal");
        assert!(!evidence_value(function, "symbol_stable_key").is_empty());

        let merge = diagnostic_for_case(&diagnostics, "ts-symbol:declaration-merge");
        assert_eq!(evidence_value(merge, "symbol_name"), "MergeMe");
        assert_eq!(evidence_value(merge, "symbol_kind"), "Interface");
        assert_eq!(evidence_value(merge, "declaration_count"), "2");

        let call = diagnostic_for_case(&diagnostics, "ts-reference:function-call");
        assert_eq!(evidence_value(call, "reference_name"), "answer");
        assert_eq!(evidence_value(call, "reference_kind"), "Call");
        assert_eq!(evidence_value(call, "reference_status"), "Resolved");
        assert_eq!(evidence_value(call, "reference_precision"), "ExactLocal");
        assert_ne!(evidence_value(call, "reference_target"), "none");

        let import = diagnostic_for_case(&diagnostics, "ts-reference:module-import");
        assert_eq!(evidence_value(import, "reference_name"), "importedToken");
        assert_eq!(evidence_value(import, "reference_kind"), "Import");
        assert_eq!(evidence_value(import, "reference_status"), "Resolved");
        assert_eq!(
            evidence_value(import, "reference_precision"),
            "ModuleLinked"
        );
        assert_ne!(evidence_value(import, "reference_target"), "none");

        let unresolved = diagnostic_for_case(&diagnostics, "ts-reference:unresolved-global");
        assert_eq!(
            evidence_value(unresolved, "reference_name"),
            "missingGlobal"
        );
        assert_eq!(evidence_value(unresolved, "reference_status"), "Unresolved");
        assert_eq!(
            evidence_value(unresolved, "reference_precision"),
            "Unresolved"
        );
        assert_eq!(evidence_value(unresolved, "reference_target"), "none");
    }

    #[test]
    fn external_rule_consumes_go_symbols_and_references_through_public_sdk() {
        let temp = tempfile::tempdir().unwrap();
        write_external_go_symbol_reference_rule_repo(temp.path());
        assert_written_external_symbol_rule_is_public(temp.path());

        let report = run_external_symbol_reference_check(temp.path());
        assert_no_capability_diagnostics(&report);

        assert_go_symbol_reference_cases(&report);
    }

    #[test]
    fn external_rule_consumes_go_symbols_from_monorepo_submodule_without_repo_go_mod() {
        let temp = tempfile::tempdir().unwrap();
        write_external_go_symbol_reference_rule_repo(temp.path());
        write_file(
            &temp.path().join(".polint.toml"),
            r#"
[workspace]
include = ["services/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
        );
        fs::create_dir_all(temp.path().join("services/app/src")).unwrap();
        fs::rename(
            temp.path().join("go.mod"),
            temp.path().join("services/app/go.mod"),
        )
        .unwrap();
        fs::rename(
            temp.path().join("src/main.go"),
            temp.path().join("services/app/src/main.go"),
        )
        .unwrap();
        fs::remove_dir(temp.path().join("src")).unwrap();
        assert_written_external_symbol_rule_is_public(temp.path());

        let report = run_external_symbol_reference_check(temp.path());
        assert_no_capability_diagnostics(&report);

        assert_go_symbol_reference_cases(&report);
    }

    #[test]
    fn external_rule_consumes_go_symbols_from_multiple_monorepo_modules_with_one_polint_setup() {
        let temp = tempfile::tempdir().unwrap();
        write_external_go_symbol_reference_rule_repo(temp.path());
        write_file(
            &temp.path().join(".polint.toml"),
            r#"
[workspace]
include = ["services/**", "libs/**"]
exclude = []

[languages.go]
module_roots = ["services/app", "libs/shared"]
package_patterns = ["./..."]

[rules]
paths = [".polint/rules"]
"#,
        );
        write_file(
            &temp.path().join("go.work"),
            r#"go 1.24

use ./tools/only
"#,
        );
        write_file(
            &temp.path().join("tools/only/go.mod"),
            r#"module example.com/tools

go 1.24
"#,
        );
        write_file(&temp.path().join("tools/only/tool.go"), "package tool\n");
        write_file(
            &temp.path().join("libs/shared/go.mod"),
            r#"module example.com/shared

go 1.24
"#,
        );
        write_file(
            &temp.path().join("libs/shared/shared.go"),
            "package shared\n\nfunc Shared() string { return \"ok\" }\n",
        );
        fs::create_dir_all(temp.path().join("services/app/src")).unwrap();
        fs::rename(
            temp.path().join("go.mod"),
            temp.path().join("services/app/go.mod"),
        )
        .unwrap();
        fs::rename(
            temp.path().join("src/main.go"),
            temp.path().join("services/app/src/main.go"),
        )
        .unwrap();
        fs::remove_dir(temp.path().join("src")).unwrap();
        assert_written_external_symbol_rule_is_public(temp.path());

        let report = run_external_symbol_reference_check(temp.path());
        assert_no_capability_diagnostics(&report);

        assert_go_symbol_reference_cases(&report);
    }

    fn assert_go_symbol_reference_cases(report: &polint::sdk::prelude::PolintReport) {
        let diagnostics =
            report_diagnostics_for_rule(report, "local/go-symbol-reference-public-sdk");
        let mut cases = diagnostic_cases(&diagnostics);
        cases.sort();
        assert_eq!(
            cases,
            [
                "go-reference:field-selector",
                "go-reference:function-call",
                "go-reference:local-variable",
                "go-reference:method-call",
            ],
            "{report:#?}"
        );

        let function = diagnostic_for_case(&diagnostics, "go-reference:function-call");
        assert_eq!(evidence_value(function, "symbol_name"), "Build");
        assert_eq!(evidence_value(function, "symbol_kind"), "Function");
        assert_eq!(
            evidence_value(function, "symbol_precision"),
            "ExactSemantic"
        );
        assert_eq!(evidence_value(function, "reference_kind"), "Call");
        assert_eq!(evidence_value(function, "reference_status"), "Resolved");
        assert_eq!(
            evidence_value(function, "reference_precision"),
            "ExactSemantic"
        );
        assert_ne!(evidence_value(function, "reference_target"), "none");

        let method = diagnostic_for_case(&diagnostics, "go-reference:method-call");
        assert_eq!(evidence_value(method, "symbol_name"), "Label");
        assert_eq!(evidence_value(method, "symbol_kind"), "Method");
        assert_eq!(evidence_value(method, "reference_kind"), "Call");
        assert_eq!(evidence_value(method, "reference_status"), "Resolved");

        let field = diagnostic_for_case(&diagnostics, "go-reference:field-selector");
        assert_eq!(evidence_value(field, "symbol_name"), "Name");
        assert_eq!(evidence_value(field, "symbol_kind"), "Field");
        assert_eq!(evidence_value(field, "reference_kind"), "MemberAccess");
        assert_eq!(evidence_value(field, "reference_status"), "Resolved");

        let local = diagnostic_for_case(&diagnostics, "go-reference:local-variable");
        assert_eq!(evidence_value(local, "symbol_name"), "local");
        assert_eq!(evidence_value(local, "symbol_kind"), "Variable");
        assert_eq!(evidence_value(local, "reference_kind"), "Read");
        assert_eq!(evidence_value(local, "reference_status"), "Resolved");
    }

    #[test]
    fn symbol_reference_macro_mapping_and_determinism() {
        let temp = tempfile::tempdir().unwrap();
        write_external_ts_symbol_reference_rule_repo(temp.path());
        assert_written_external_symbol_rule_is_public(temp.path());

        let first = run_external_symbol_reference_check(temp.path());
        let second = run_external_symbol_reference_check(temp.path());
        let third = run_external_symbol_reference_check(temp.path());

        assert_no_capability_diagnostics(&first);
        assert_eq!(
            evidence_snapshot(&first, "local/ts-symbol-reference-public-sdk"),
            evidence_snapshot(&second, "local/ts-symbol-reference-public-sdk")
        );
        assert_eq!(
            evidence_snapshot(&second, "local/ts-symbol-reference-public-sdk"),
            evidence_snapshot(&third, "local/ts-symbol-reference-public-sdk")
        );
        assert!(
            !report_diagnostics_for_rule(&first, "local/ts-symbol-reference-public-sdk").is_empty(),
            "typed macro-derived symbol/reference rule should execute: {first:#?}"
        );
    }

    #[test]
    fn external_rule_consumes_module_relationship_facts_through_public_sdk() {
        let temp = tempfile::tempdir().unwrap();
        write_module_relationship_rule_repo(temp.path());

        let json = stdout_json(
            polint_cmd()
                .current_dir(temp.path())
                .args(["check", "--format", "json", "--fail-on", "none"])
                .assert()
                .success(),
        );

        let unresolved = diagnostics_for_rule(&json, "local/unresolved-import");
        assert!(
            unresolved
                .iter()
                .any(|diagnostic| diagnostic_has_evidence(diagnostic, "reason", "NotFound")),
            "expected unresolved import reason evidence: {json:#?}"
        );
        assert!(
            unresolved.iter().any(|diagnostic| diagnostic_has_evidence(
                diagnostic,
                "status",
                "Unresolved"
            )),
            "expected unresolved import status evidence: {json:#?}"
        );

        let graph = diagnostics_for_rule(&json, "local/no-ui-to-domain");
        assert!(
            graph.iter().any(|diagnostic| diagnostic_has_evidence(
                diagnostic,
                "target",
                "src/domain/model.ts"
            )),
            "expected graph diagnostic target evidence: {json:#?}"
        );
        assert!(
            diagnostics(&json)
                .iter()
                .all(|diagnostic| diagnostic["rule_id"] != "polint/capability"),
            "relationship fact views should be available through the public SDK: {json:#?}"
        );
    }

    #[test]
    fn module_relationship_setup_missing_and_determinism() {
        let temp = tempfile::tempdir().unwrap();
        write_go_relationship_setup_missing_rule_repo(temp.path());

        let json = stdout_json(
            polint_cmd()
                .current_dir(temp.path())
                .args(["check", "--format", "json", "--fail-on", "none"])
                .assert()
                .success(),
        );

        assert!(
            diagnostics_for_rule(&json, "local/go-relationships").is_empty(),
            "setup-missing capabilities must block the requesting local rule: {json:#?}"
        );
        assert!(
            diagnostics_for_rule(&json, "polint/capability")
                .iter()
                .any(|diagnostic| diagnostic_has_evidence(diagnostic, "status", "setup_missing")),
            "expected setup-missing capability evidence: {json:#?}"
        );

        let deterministic = tempfile::tempdir().unwrap();
        write_module_relationship_rule_repo(deterministic.path());
        let run = || {
            stdout_json(
                polint_cmd()
                    .current_dir(deterministic.path())
                    .args(["check", "--format", "json", "--fail-on", "none"])
                    .assert()
                    .success(),
            )
        };

        let first = run();
        let second = run();
        let third = run();
        let first_diagnostics = diagnostics(&first);
        let second_diagnostics = diagnostics(&second);
        let third_diagnostics = diagnostics(&third);

        assert_eq!(first_diagnostics, second_diagnostics);
        assert_eq!(second_diagnostics, third_diagnostics);
    }

    #[test]
    fn capability_change_changes_cache_entries() {
        let temp = tempfile::tempdir().unwrap();
        write_cache_capability_rule_repo(temp.path());

        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success();
        assert!(
            repo_root().join("target/polint-cli-test-rules").exists(),
            "repo-local rule host should use POLINT_RULES_TARGET_DIR when provided"
        );
        let first = cache_json_file_names(temp.path());
        assert!(!first.is_empty(), "first check should write cache files");

        let rule_path = temp.path().join(".polint/rules/src/main.rs");
        let rule_source = fs::read_to_string(&rule_path).unwrap();
        fs::write(
            &rule_path,
            rule_source
                .replace(
                    "literals: StringLiterals<'_>",
                    "module_graph: ModuleGraphFacts<'_>",
                )
                .replace("literals.iter().count()", "module_graph.nodes().len()"),
        )
        .unwrap();

        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success();
        let second = cache_json_file_names(temp.path());

        assert!(
            second.difference(&first).next().is_some(),
            "changing only requested capabilities should create new cache entries\nfirst={first:#?}\nsecond={second:#?}"
        );
    }
}

#[test]
fn check_max_diagnostics_truncates_sorted_json_in_example_repo() {
    let root = repo_root().join("examples/ts-design-tokens");
    let json = stdout_json(
        polint_cmd()
            .current_dir(&root)
            .args([
                "check",
                "--format",
                "json",
                "--fail-on",
                "none",
                "--max-diagnostics",
                "1",
            ])
            .assert()
            .success(),
    );
    assert_eq!(diagnostics(&json).len(), 1);
}

#[test]
fn check_max_diagnostics_does_not_hide_failing_diagnostics() {
    let root = repo_root().join("examples/ts-design-tokens");
    polint_cmd()
        .current_dir(&root)
        .args([
            "check",
            "--format",
            "json",
            "--fail-on",
            "error",
            "--max-diagnostics",
            "0",
        ])
        .assert()
        .failure();
}

#[test]
fn check_only_rule_filters_out_diagnostics_when_pattern_matches_nothing() {
    let root = repo_root().join("examples/ts-design-tokens");
    let json = stdout_json(
        polint_cmd()
            .current_dir(&root)
            .args([
                "check",
                "--format",
                "json",
                "--fail-on",
                "none",
                "--only-rule",
                "absent/nope",
            ])
            .assert()
            .success(),
    );
    assert!(diagnostics(&json).is_empty());
}

#[test]
fn init_new_rule_and_check_json_smoke() {
    let temp = tempfile::tempdir().unwrap();
    polint_cmd()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    polint_cmd()
        .current_dir(temp.path())
        .args(["new-rule", "generic", "agent-smoke"])
        .assert()
        .success();
    point_generated_rule_pack_at_local_polint(temp.path());
    polint_cmd()
        .current_dir(temp.path())
        .args(["check", "--format", "json", "--fail-on", "none"])
        .assert()
        .success();
}

#[test]
fn cookbook_golden_empty_report_fragment_is_valid_polint_json() {
    let raw = include_str!("fixtures/cookbook-empty-report-fragment.json");
    let report: polint::sdk::prelude::PolintReport = serde_json::from_str(raw).unwrap();
    assert_eq!(report.version, 1);
    assert!(report.diagnostics.is_empty());
}

#[test]
fn rules_json_stdout_should_be_parseable_json() {
    let temp = tempfile::tempdir().unwrap();
    write_phase8_raw_color_fixture(temp.path(), "error");

    let json = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args([
                "check",
                "--profile",
                "phase8",
                "--format",
                "json",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );

    assert_eq!(diagnostics(&json).len(), 0);
}

#[test]
fn check_fail_on_warn_error_none_exit_codes_are_stable() {
    let temp = tempfile::tempdir().unwrap();
    write_phase8_raw_color_fixture(temp.path(), "warn");

    raw_color_rule_cmd()
        .current_dir(temp.path())
        .args([
            "check",
            "--profile",
            "phase8",
            "--format",
            "json",
            "--fail-on",
            "warn",
        ])
        .assert()
        .failure()
        .code(1);

    raw_color_rule_cmd()
        .current_dir(temp.path())
        .args([
            "check",
            "--profile",
            "phase8",
            "--format",
            "json",
            "--fail-on",
            "error",
        ])
        .assert()
        .success();

    raw_color_rule_cmd()
        .current_dir(temp.path())
        .args([
            "check",
            "--profile",
            "phase8",
            "--format",
            "json",
            "--fail-on",
            "none",
        ])
        .assert()
        .success();
}

#[test]
fn fatal_config_parse_error_exits_two() {
    let temp = tempfile::tempdir().unwrap();
    write_file(&temp.path().join(".polint.toml"), "[profiles.phase8\n");

    polint_cmd()
        .current_dir(temp.path())
        .arg("check")
        .assert()
        .failure()
        .code(2);
}

#[test]
fn sarif_no_cache_and_rule_paths_are_supported() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join(".polint.toml"),
        r#"
[rules]
paths = [".polint/rules", "tools/policy-rules"]

[profiles.phase2]
rules = ["local/no-raw-colors"]
"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("component.tsx"),
        "export const color = \"#ff00aa\";",
    )
    .unwrap();

    let sarif = stdout_json(
        raw_color_rule_cmd()
            .current_dir(temp.path())
            .args([
                "check",
                "--profile",
                "phase2",
                "--format",
                "sarif",
                "--no-cache",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );

    assert_eq!(sarif.pointer("/runs/0/tool/driver/name").unwrap(), "polint");
    assert!(sarif.pointer("/runs/0/tool/driver/rules").is_some());
    assert_eq!(
        sarif.pointer("/runs/0/results/0/ruleId").unwrap(),
        "local/no-raw-colors"
    );
}

#[test]
fn sarif_output_includes_ci_fields_and_honors_fail_threshold() {
    let temp = tempfile::tempdir().unwrap();
    write_phase8_raw_color_fixture(temp.path(), "warn");

    let sarif = stdout_json(
        raw_color_rule_cmd()
            .current_dir(temp.path())
            .args([
                "check",
                "--profile",
                "phase8",
                "--format",
                "sarif",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );

    assert_eq!(sarif.pointer("/version").unwrap(), "2.1.0");
    assert_eq!(sarif.pointer("/runs/0/tool/driver/name").unwrap(), "polint");
    assert!(sarif.pointer("/runs/0/tool/driver/rules").is_some());
    assert_eq!(
        sarif.pointer("/runs/0/results/0/ruleId").unwrap(),
        "local/no-raw-colors"
    );
    assert!(
        sarif
            .pointer("/runs/0/results/0/message/text")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|message| message.contains("Raw color"))
    );
    assert!(
        sarif
            .pointer("/runs/0/results/0/fingerprints/polint~1v1")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|fingerprint| !fingerprint.is_empty())
    );
    assert_eq!(
        sarif
            .pointer("/runs/0/results/0/locations/0/physicalLocation/artifactLocation/uri")
            .unwrap(),
        "component.tsx"
    );

    raw_color_rule_cmd()
        .current_dir(temp.path())
        .args([
            "check",
            "--profile",
            "phase8",
            "--format",
            "sarif",
            "--fail-on",
            "warn",
        ])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn check_no_cache_does_not_create_cache_directory() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join(".polint.toml"),
        r##"
[profiles.phase7]
rules = ["local/no-raw-colors"]
"##,
    )
    .unwrap();
    fs::write(
        temp.path().join("component.tsx"),
        "export const color = \"#ff00aa\";",
    )
    .unwrap();

    polint_cmd()
        .current_dir(temp.path())
        .args([
            "check",
            "--profile",
            "phase7",
            "--format",
            "json",
            "--no-cache",
            "--fail-on",
            "none",
        ])
        .assert()
        .success();

    assert!(!temp.path().join(".polint/cache").exists());
}

#[test]
fn check_no_cache_bypasses_cache_writes() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join(".polint.toml"),
        r##"
[profiles.phase7]
rules = ["local/no-raw-colors"]
"##,
    )
    .unwrap();
    fs::write(
        temp.path().join("component.tsx"),
        "export const color = \"#ff00aa\";",
    )
    .unwrap();

    polint_cmd()
        .current_dir(temp.path())
        .args([
            "check",
            "--profile",
            "phase7",
            "--format",
            "json",
            "--no-cache",
            "--fail-on",
            "none",
        ])
        .assert()
        .success();

    assert_eq!(cache_json_count(temp.path()), 0);
}

#[test]
fn cache_status_reports_structured_cache_layout() {
    let temp = tempfile::tempdir().unwrap();
    write_file(&temp.path().join(".polint/cache/analysis/a.json"), "{}");
    write_file(
        &temp.path().join(".polint/cache/rules-target/debug/rules"),
        "bin",
    );

    let mut command = polint_cmd();
    command.env_remove("POLINT_RULES_TARGET_DIR");
    let json = stdout_json(
        command
            .current_dir(temp.path())
            .args(["cache", "status", "--format", "json"])
            .assert()
            .success(),
    );

    assert_eq!(
        json["schema"],
        "https://raw.githubusercontent.com/oaiz-io/polint/main/docs/schemas/polint-cache-status-v1.json"
    );
    assert!(
        path_str_ends_with(json["root"].as_str().unwrap(), ".polint/cache"),
        "unexpected cache root: {json:#?}"
    );
    let categories = json["categories"].as_array().unwrap();
    // Every directory polint writes under the cache root is reported, with the
    // role that decides how CI may cache it. A directory missing from this list
    // is a directory `polint cache clean` cannot reach and `action.yml` cannot
    // know about.
    let reported = categories
        .iter()
        .map(|category| {
            (
                category["name"].as_str().unwrap().to_string(),
                category["role"].as_str().unwrap().to_string(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        reported,
        vec![
            ("analysis".to_string(), "source-validated".to_string()),
            ("layers".to_string(), "source-validated".to_string()),
            ("derived".to_string(), "source-validated".to_string()),
            ("semantic-store".to_string(), "source-validated".to_string()),
            ("rules-target".to_string(), "compiler-output".to_string()),
            (
                "extensions-target".to_string(),
                "compiler-output".to_string()
            ),
            ("review".to_string(), "scratch".to_string()),
        ],
        "unexpected cache categories: {json:#?}"
    );
    assert!(categories.iter().any(|category| {
        category["name"] == "analysis"
            && category["files"] == 1
            && category["path"]
                .as_str()
                .is_some_and(|path| path_str_ends_with(path, ".polint/cache/analysis"))
    }));
    assert!(
        categories
            .iter()
            .any(|category| { category["name"] == "rules-target" && category["files"] == 1 })
    );
    assert!(
        categories.iter().any(|category| {
            category["name"] == "layers" && category["exists"] == false && category["files"] == 0
        }),
        "cache status should report the managed layers category without requiring it to exist: {json:#?}"
    );
}

#[test]
fn cache_clean_analysis_removes_only_analysis_directory() {
    let temp = tempfile::tempdir().unwrap();
    write_file(&temp.path().join(".polint/cache/analysis/a.json"), "{}");
    write_file(&temp.path().join(".polint/cache/unmanaged.json"), "{}");
    write_file(
        &temp.path().join(".polint/cache/rules-target/debug/rules"),
        "bin",
    );

    polint_cmd()
        .current_dir(temp.path())
        .args(["cache", "clean", "--category", "analysis"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed"));

    assert!(!temp.path().join(".polint/cache/analysis").exists());
    assert!(temp.path().join(".polint/cache/unmanaged.json").exists());
    assert!(temp.path().join(".polint/cache/rules-target").exists());
}

#[test]
fn cache_prune_size_supports_dry_run_and_actual_removal() {
    let temp = tempfile::tempdir().unwrap();
    write_file(&temp.path().join(".polint/cache/derived/a.bin"), "abcdef");

    polint_cmd()
        .current_dir(temp.path())
        .args([
            "cache",
            "prune",
            "--category",
            "derived",
            "--max-size-mb",
            "0",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Would remove"));
    assert!(temp.path().join(".polint/cache/derived/a.bin").exists());

    polint_cmd()
        .current_dir(temp.path())
        .args([
            "cache",
            "prune",
            "--category",
            "derived",
            "--max-size-mb",
            "0",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed"));
    assert!(!temp.path().join(".polint/cache/derived/a.bin").exists());
}

fn write_phase7_cache_fixture(root: &Path) {
    fs::write(
        root.join(".polint.toml"),
        r##"
[profiles.phase7]
rules = ["local/no-raw-colors", "local/go-cyclomatic-complexity"]

[[rules.config]]
id = "local/no-raw-colors"
files = ["**/*.tsx"]
"##,
    )
    .unwrap();
    fs::write(
        root.join("component.tsx"),
        r##"
import { token } from "./tokens";

export function Button() {
  return <div data-color="#ff00aa">{token}</div>;
}
"##,
    )
    .unwrap();
    fs::write(
        root.join("payment.go"),
        r#"
package payment

func Authorize(user string) bool {
    return user != ""
}
"#,
    )
    .unwrap();
}

#[test]
fn check_cache_writes_fact_metadata() {
    let temp = tempfile::tempdir().unwrap();
    write_phase7_cache_fixture(temp.path());

    let _json = stdout_json(
        polint_cmd()
            .current_dir(temp.path())
            .args([
                "check",
                "--profile",
                "phase7",
                "--format",
                "json",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );

    assert!(cache_json_count(temp.path()) >= 2);
    let cache_entry =
        fs::read_to_string(cache_json_paths(temp.path()).into_iter().next().unwrap()).unwrap();
    assert!(!cache_entry.contains("export function Button"));
    assert!(!cache_entry.contains("package payment"));
}

#[test]
fn check_cached_output_is_deterministic_across_repeated_runs() {
    let temp = tempfile::tempdir().unwrap();
    write_phase7_cache_fixture(temp.path());
    let run = || {
        stdout_string(
            polint_cmd()
                .current_dir(temp.path())
                .args([
                    "check",
                    "--profile",
                    "phase7",
                    "--format",
                    "json",
                    "--fail-on",
                    "none",
                ])
                .assert()
                .success(),
        )
    };

    let first = run();
    let second = run();
    let third = run();

    assert_eq!(second, first);
    assert_eq!(third, first);
    assert!(cache_json_count(temp.path()) >= 2);
}

#[test]
fn check_parallel_cached_output_is_deterministic_across_repeated_runs() {
    let temp = tempfile::tempdir().unwrap();
    write_phase7_cache_fixture(temp.path());
    let run = || {
        stdout_string(
            polint_cmd()
                .current_dir(temp.path())
                .args([
                    "check",
                    "--profile",
                    "phase7",
                    "--format",
                    "json",
                    "--fail-on",
                    "none",
                ])
                .assert()
                .success(),
        )
    };

    let first = run();
    let second = run();
    let third = run();

    assert_eq!(second, first);
    assert_eq!(third, first);
    assert!(cache_json_count(temp.path()) >= 2);
}

#[test]
fn check_no_cache_bypasses_cache_reads_and_writes() {
    let temp = tempfile::tempdir().unwrap();
    write_phase7_cache_fixture(temp.path());
    let run = |no_cache: bool| {
        let mut args = vec![
            "check",
            "--profile",
            "phase7",
            "--format",
            "json",
            "--fail-on",
            "none",
        ];
        if no_cache {
            args.push("--no-cache");
        }
        stdout_string(
            polint_cmd()
                .current_dir(temp.path())
                .args(args)
                .assert()
                .success(),
        )
    };

    let cold = run(false);
    let cache_files_after_cold = cache_json_count(temp.path());
    let warm = run(false);
    let no_cache = run(true);
    let cache_files_after_no_cache = cache_json_count(temp.path());

    assert!(cache_files_after_cold >= 2);
    assert_eq!(warm, cold);
    assert_eq!(no_cache, cold);
    assert_eq!(cache_files_after_no_cache, cache_files_after_cold);
}

#[test]
fn discovery_detects_all_supported_extensions() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = []

[profiles.phase2]
rules = ["local/no-raw-colors"]

[[rules.config]]
id = "local/no-raw-colors"
severity = "error"
files = ["**/*.ts", "**/*.tsx", "**/*.js", "**/*.jsx"]
"#,
    )
    .unwrap();
    for path in ["src/a.ts", "src/b.tsx", "src/c.js", "src/d.jsx"] {
        write_file(&temp.path().join(path), "export const color = \"#ff00aa\";");
    }
    write_file(
        &temp.path().join("src/main.go"),
        "package main\nfunc main() {}\n",
    );

    let json = stdout_json(
        raw_color_rule_cmd()
            .current_dir(temp.path())
            .args([
                "check",
                "--profile",
                "phase2",
                "--format",
                "json",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );

    let files = diagnostic_files(&json, "local/no-raw-colors");
    assert!(files.contains(&"src/a.ts".to_string()));
    assert!(files.contains(&"src/b.tsx".to_string()));
    assert!(files.contains(&"src/c.js".to_string()));
    assert!(files.contains(&"src/d.jsx".to_string()));
    assert!(!files.contains(&"src/main.go".to_string()));
}

#[test]
fn discovery_respects_gitignore_include_exclude_and_supported_extensions() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join(".gitignore"), "ignored.tsx\n").unwrap();
    fs::write(
        temp.path().join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = ["src/excluded.tsx"]

[profiles.phase2]
rules = ["local/no-raw-colors"]
"#,
    )
    .unwrap();
    write_file(
        &temp.path().join("src/included.tsx"),
        "export const color = \"#ff00aa\";",
    );
    write_file(
        &temp.path().join("src/excluded.tsx"),
        "export const color = \"#ff00aa\";",
    );
    write_file(
        &temp.path().join("ignored.tsx"),
        "export const color = \"#ff00aa\";",
    );
    write_file(
        &temp.path().join("src/ignored.tsx"),
        "export const color = \"#ff00aa\";",
    );
    write_file(&temp.path().join("src/notes.txt"), "#ff00aa\n");

    let json = stdout_json(
        raw_color_rule_cmd()
            .current_dir(temp.path())
            .args([
                "check",
                "--profile",
                "phase2",
                "--format",
                "json",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );

    let files = diagnostic_files(&json, "local/no-raw-colors");
    assert!(files.contains(&"src/included.tsx".to_string()));
    assert!(!files.contains(&"src/excluded.tsx".to_string()));
    assert!(!files.contains(&"ignored.tsx".to_string()));
    assert!(!files.contains(&"src/ignored.tsx".to_string()));
    assert!(!files.contains(&"src/notes.txt".to_string()));
}

#[test]
fn discovery_respects_default_excludes() {
    let temp = tempfile::tempdir().unwrap();
    for path in [
        "src/included.tsx",
        "vendor/vendor.tsx",
        "node_modules/pkg/index.tsx",
        "target/generated.tsx",
        "src/generated.pb.go",
    ] {
        write_file(&temp.path().join(path), "export const color = \"#ff00aa\";");
    }

    let json = stdout_json(
        raw_color_rule_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );

    let files = diagnostic_files(&json, "local/no-raw-colors");
    assert!(files.contains(&"src/included.tsx".to_string()));
    assert!(!files.contains(&"vendor/vendor.tsx".to_string()));
    assert!(!files.contains(&"node_modules/pkg/index.tsx".to_string()));
    assert!(!files.contains(&"target/generated.tsx".to_string()));
    assert!(!files.contains(&"src/generated.pb.go".to_string()));
}

#[test]
fn check_json_output_is_deterministic_across_repeated_runs() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join(".gitignore"), "src/ignored.ts\n").unwrap();
    fs::write(
        temp.path().join(".polint.toml"),
        r#"
[workspace]
include = ["src/**"]
exclude = ["src/excluded.ts"]

[profiles.phase3]
rules = ["local/no-raw-colors"]
"#,
    )
    .unwrap();
    write_file(
        &temp.path().join("src/z.tsx"),
        "export function Button() { return <div style={{ color: \"#ffffff\" }} />; }",
    );
    write_file(
        &temp.path().join("src/excluded.ts"),
        "export const excluded = \"#333333\";",
    );
    write_file(
        &temp.path().join("src/ignored.ts"),
        "export const ignored = \"#444444\";",
    );
    write_file(
        &temp.path().join("src/a.ts"),
        "export const accent = \"#111111\";",
    );

    let first = phase3_check_json(temp.path());
    let second = phase3_check_json(temp.path());
    let third = phase3_check_json(temp.path());

    assert_eq!(second, first);
    assert_eq!(third, first);
    assert_eq!(
        diagnostic_files(&first, "local/no-raw-colors"),
        ["src/a.ts", "src/z.tsx"]
    );
}

#[test]
fn check_clean_repo_succeeds() {
    let temp = tempfile::tempdir().unwrap();
    polint_cmd()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    fs::write(
        temp.path().join("component.tsx"),
        "export const label = \"ok\";",
    )
    .unwrap();

    polint_cmd()
        .current_dir(temp.path())
        .arg("check")
        .assert()
        .success()
        .stdout(predicate::str::contains("No diagnostics"));
}

fn phase3_check_json(root: &Path) -> serde_json::Value {
    stdout_json(
        raw_color_rule_cmd()
            .current_dir(root)
            .args([
                "check",
                "--profile",
                "phase3",
                "--format",
                "json",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    )
}

// Direct summary internals stay private.

#[test]
fn direct_summaries_internals_stay_private() {
    let temp = tempfile::tempdir().unwrap();
    write_direct_summaries_public_rule_repo(temp.path());

    let check_json = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    let report: polint::sdk::prelude::PolintReport = serde_json::from_str(&check_json)
        .unwrap_or_else(|error| panic!("stdout was not public check JSON: {error}\n{check_json}"));
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "local/direct-summaries-public-probe"),
        "external direct-summaries public-boundary rule should run: {report:#?}"
    );

    let inspect_json = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["inspect", "rule", "--format", "json"])
            .assert()
            .success(),
    );
    let inspect: serde_json::Value = serde_json::from_str(&inspect_json)
        .unwrap_or_else(|error| panic!("stdout was not inspect JSON: {error}\n{inspect_json}"));
    assert_eq!(
        inspect["rules"][0]["rule_id"],
        "local/direct-summaries-public-probe"
    );
    let fact_views = inspect["rules"][0]["fact_views"]
        .as_array()
        .unwrap_or_else(|| panic!("fact_views should be an array: {inspect:#?}"));
    for canonical_path in [
        "polint::sdk::facts::ComplexityMetrics<'_>",
        "polint::sdk::facts::FileMetrics<'_>",
        "polint::sdk::facts::FunctionMetrics<'_>",
        "polint::sdk::facts::ModuleGraphFacts<'_>",
        "polint::sdk::facts::References<'_>",
        "polint::sdk::facts::ResolvedImports<'_>",
        "polint::sdk::facts::Symbols<'_>",
    ] {
        assert!(
            fact_views
                .iter()
                .any(|view| view["canonical_path"] == canonical_path),
            "inspect JSON should include public fact view {canonical_path}: {inspect_json}"
        );
    }

    let test_json = stdout_string(
        polint_cmd()
            .current_dir(temp.path())
            .args(["test", "--format", "json"])
            .assert()
            .success(),
    );
    let test_report: serde_json::Value = serde_json::from_str(&test_json)
        .unwrap_or_else(|error| panic!("stdout was not test JSON: {error}\n{test_json}"));
    assert_eq!(test_report["summary"]["failed"], 0);

    for output in [&check_json, &inspect_json, &test_json] {
        assert_direct_summaries_public_output_is_private(output);
    }
    assert_direct_summaries_public_surfaces_are_private();
    assert_direct_summaries_cli_help_is_private();
    assert!(!repo_root().join("docs/facts/summaries.md").exists());
    assert!(!repo_root().join("docs/facts/effects.md").exists());
    assert!(!repo_root().join("docs/facts/direct-summaries.md").exists());

    let source = fs::read_to_string(temp.path().join(".polint/rules/src/main.rs")).unwrap();
    assert!(source.contains("use polint::sdk::prelude::*;"));
    assert!(source.contains("ResolvedImports<'_>"));
    assert!(source.contains("ModuleGraphFacts<'_>"));
    assert!(source.contains("Symbols<'_>"));
    assert!(source.contains("References<'_>"));
    assert!(source.contains("FileMetrics<'_>"));
    assert!(source.contains("FunctionMetrics<'_>"));
    assert!(source.contains("ComplexityMetrics<'_>"));
    assert!(source.contains("polint::runner::run_cli"));
    assert_direct_summaries_public_rule_source(&source);
}

fn assert_direct_summaries_public_output_is_private(output: &str) {
    for marker in DIRECT_SUMMARIES_INTERNAL_PUBLIC_MARKERS {
        assert!(
            !output.contains(marker),
            "public output must not leak direct-summary internal marker `{marker}`:\n{output}"
        );
    }
}

fn assert_direct_summaries_public_surfaces_are_private() {
    let root = repo_root();
    let lib_rs = fs::read_to_string(root.join("crates/polint/src/lib.rs")).unwrap();
    let public_lib_rs = lib_rs
        .split("pub(crate) mod analysis;")
        .next()
        .unwrap_or(&lib_rs);
    let mut public_surface = public_lib_rs.to_string();
    for path in [
        root.join("README.md"),
        root.join("docs/facts"),
        root.join("crates/polint/src/sdk"),
        root.join("crates/polint/src/runner"),
        root.join("crates/polint/src/cli"),
    ] {
        public_surface.push_str(&public_surface_text(&path));
    }

    for marker in DIRECT_SUMMARIES_INTERNAL_PUBLIC_MARKERS {
        assert!(
            !public_surface.contains(marker),
            "public README/docs/facts/CLI/SDK/runner/crate-root source must not expose direct-summary marker `{marker}`"
        );
    }
}

fn assert_direct_summaries_cli_help_is_private() {
    let help_outputs = public_rule_help_outputs();

    for help in help_outputs {
        for marker in DIRECT_SUMMARIES_INTERNAL_PUBLIC_MARKERS {
            assert!(
                !help.contains(marker),
                "public CLI help must not expose direct-summary marker `{marker}`:\n{help}"
            );
        }
    }
}

const DIRECT_SUMMARIES_INTERNAL_PUBLIC_MARKERS: &[&str] = &[
    "polint.direct_summaries",
    "direct-summaries-facts",
    "summary_control",
    "summary_call",
    "summary_memory",
    "summary_tito",
    "SummaryDomain",
    "SummaryStore",
    "SummaryBuilder",
    "SummaryOutput",
    "SummaryKey",
    "ControlEffects",
    "CallEffects",
    "MemoryEffects",
    "DataFlowTito",
    "control_effects",
    "memory_effects",
    "data_flow_tito",
    "analysis::summaries",
    "Effects<'_>",
    "TaintFlows<'_>",
];

fn write_direct_summaries_public_rule_repo(root: &Path) {
    assert_direct_summaries_public_rule_source(DIRECT_SUMMARIES_PUBLIC_RULE_SOURCE);
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint.toml"),
        r#"
[workspace]
include = ["src/**", "web/src/**"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#,
        ),
    );
    write_file(
        &root.join(".polint/rules/src/main.rs"),
        DIRECT_SUMMARIES_PUBLIC_RULE_SOURCE,
    );
    write_direct_summaries_public_fixture_files(root);

    let case_root = root.join(".polint/tests/rules/direct_summaries_public/basic");
    write_file(
        &case_root.join("polint-test.toml"),
        r#"
rule = "local/direct-summaries-public-probe"
paths = ["src/**", "web/src/**"]

[[expect.diagnostic]]
rule_id = "local/direct-summaries-public-probe"
file = "<workspace>"
severity = "warn"
message_contains = "public facts remain compatible while direct summaries stay private"
"#,
    );
    write_direct_summaries_public_fixture_files(&case_root);
}

const DIRECT_SUMMARIES_PUBLIC_RULE_SOURCE: &str = r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "local/direct-summaries-public-probe",
    description = "Reads supported public facts while direct summary facts stay private.",
    severity = "warn"
)]
fn direct_summaries_public_probe(
    ctx: &mut RuleCtx<'_>,
    resolved: ResolvedImports<'_>,
    graph: ModuleGraphFacts<'_>,
    symbols: Symbols<'_>,
    references: References<'_>,
    file_metrics: FileMetrics<'_>,
    function_metrics: FunctionMetrics<'_>,
    complexity_metrics: ComplexityMetrics<'_>,
) -> RuleResult {
    let resolved_count = resolved.iter().count();
    let edge_count = graph.edges().len();
    let symbol_count = symbols.iter().count();
    let reference_count = references.iter().count();
    let file_metric_count = file_metrics.iter().count();
    let function_metric_count = function_metrics.iter().count();
    let complexity_metric_count = complexity_metrics.iter().count();
    let public_fact_count = resolved_count
        + edge_count
        + symbol_count
        + reference_count
        + file_metric_count
        + function_metric_count
        + complexity_metric_count;
    if public_fact_count == 0 {
        return Ok(());
    }

    ctx.report(
        Diagnostic::warning(
            ctx.rule_id(),
            "<workspace>",
            DiagnosticRange::point(1, 1),
            "public facts remain compatible while direct summaries stay private",
        )
        .with_evidence("resolved_imports", resolved_count.to_string())
        .with_evidence("module_edges", edge_count.to_string())
        .with_evidence("symbols", symbol_count.to_string())
        .with_evidence("references", reference_count.to_string())
        .with_evidence("file_metrics", file_metric_count.to_string())
        .with_evidence("function_metrics", function_metric_count.to_string())
        .with_evidence("complexity_metrics", complexity_metric_count.to_string()),
    );
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![direct_summaries_public_probe()])
}
"#;

fn assert_direct_summaries_public_rule_source(source: &str) {
    assert!(
        source.contains("use polint::sdk::prelude::*;"),
        "external rule must import the public SDK prelude only:\n{source}"
    );
    assert!(
        source.contains("ResolvedImports<'_>")
            && source.contains("ModuleGraphFacts<'_>")
            && source.contains("Symbols<'_>")
            && source.contains("References<'_>")
            && source.contains("FileMetrics<'_>")
            && source.contains("FunctionMetrics<'_>")
            && source.contains("ComplexityMetrics<'_>"),
        "external rule must request supported public fact views through its signature:\n{source}"
    );
    for forbidden in [
        concat!("polint::", "analysis"),
        concat!("polint::", "analysis_kernel"),
        concat!("polint::", "core"),
        concat!("polint::", "graph"),
        concat!("polint::", "go"),
        concat!("polint::", "ts"),
        "Capabilities::new(",
        "impl Rule for",
    ] {
        assert!(
            !source.contains(forbidden),
            "external rule source must not depend on internal API `{forbidden}`:\n{source}"
        );
    }
    assert_direct_summaries_public_output_is_private(source);
}

fn write_direct_summaries_public_fixture_files(root: &Path) {
    write_file(
        &root.join("go.mod"),
        r#"module example.com/directsummariespublic

go 1.22
"#,
    );
    write_file(
        &root.join("src/service/service.go"),
        r#"package service

import "fmt"

func FormatUser(name string) string {
	if name == "" {
		return fmt.Sprint("anonymous")
	}
	return name
}
"#,
    );
    write_file(
        &root.join("web/package.json"),
        r#"{"name":"direct-summaries-public-fixture","private":true,"type":"module"}"#,
    );
    write_file(
        &root.join("web/src/lib.ts"),
        r#"export function helper(input: string) {
  return input.trim();
}
"#,
    );
    write_file(
        &root.join("web/src/app.ts"),
        r#"import { helper } from "./lib";

export function render(name: string) {
  return helper(name);
}
"#,
    );
}

// ---------------------------------------------------------------------------
// `polint review` (review-kind rules, diff-gated).
// ---------------------------------------------------------------------------

fn review_git_available() -> bool {
    std::process::Command::new("git")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

fn review_run_git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn review_git_init_identity(dir: &Path) {
    review_run_git(dir, &["init", "--quiet"]);
    review_run_git(dir, &["config", "user.email", "t@example.com"]);
    review_run_git(dir, &["config", "user.name", "Test"]);
    review_run_git(dir, &["config", "commit.gpgsign", "false"]);
}

fn review_git_commit_all(dir: &Path, message: &str) -> String {
    review_run_git(dir, &["add", "-A"]);
    review_run_git(dir, &["commit", "--quiet", "-m", message]);
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("git rev-parse");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Write a repo-local review rule pack with one `kind = "review"` rule that
/// fires for any changed path under `db/migrations/**`.
fn write_review_rule_repo(root: &Path) {
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint.toml"),
        r#"
[workspace]
include = ["**/*.go", "db/**/*.sql"]
exclude = []

[rules]
paths = [".polint/rules"]
"#,
    );
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#,
        ),
    );
    write_file(
        &root.join(".polint/rules/src/main.rs"),
        r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "review/migrations",
    description = "Migrations changed — a DB owner must review.",
    severity = "warn",
    kind = "review"
)]
fn migrations(ctx: &mut RuleCtx<'_>, changes: ChangedFiles<'_>) -> RuleResult {
    let rule_id = ctx.rule_id().to_string();
    for changed in changes.iter() {
        if changed.matches_glob("db/migrations/**") {
            let line = changed.lines().first().map(|&(lo, _)| lo).unwrap_or(1);
            ctx.report(Diagnostic::warning(
                rule_id.clone(),
                changed.path().to_string(),
                DiagnosticRange::point(line, 1),
                "Migration changed: a DB owner must review.",
            ));
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![migrations()])
}
"#,
    );
}

fn write_fixed_line_review_rule_repo(root: &Path) {
    fs::create_dir_all(root.join(".polint/rules/src")).unwrap();
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint.toml"),
        "[workspace]\ninclude = [\"**/*.go\", \"db/migrations/**/*.sql\"]\n",
    );
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#,
        ),
    );
    write_file(
        &root.join(".polint/rules/src/main.rs"),
        r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "review/migrations",
    description = "Migrations changed — a DB owner must review.",
    severity = "warn",
    kind = "review"
)]
fn migrations(ctx: &mut RuleCtx<'_>, changes: ChangedFiles<'_>) -> RuleResult {
    let rule_id = ctx.rule_id().to_string();
    for changed in changes.iter() {
        if changed.matches_glob("db/migrations/**") {
            ctx.report(Diagnostic::warning(
                rule_id.clone(),
                changed.path().to_string(),
                DiagnosticRange::point(1, 1),
                "Migration changed: a DB owner must review.",
            ));
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![migrations()])
}
"#,
    );
}

fn write_ts_review_rule_repo(root: &Path) {
    fs::create_dir_all(root.join(".polint/rules/src")).unwrap();
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint.toml"),
        "[workspace]\ninclude = [\"src/**/*.ts\"]\n",
    );
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#,
        ),
    );
    write_file(
        &root.join(".polint/rules/src/main.rs"),
        r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "review/changed-ts",
    description = "Changed TypeScript file requires review.",
    severity = "warn",
    kind = "review"
)]
fn changed_ts(ctx: &mut RuleCtx<'_>, changes: ChangedFiles<'_>) -> RuleResult {
    let rule_id = ctx.rule_id().to_string();
    for changed in changes.iter() {
        if changed.matches_glob("src/**/*.ts") {
            let line = changed.lines().first().map(|&(lo, _)| lo).unwrap_or(1);
            ctx.report(Diagnostic::warning(
                rule_id.clone(),
                changed.path().to_string(),
                DiagnosticRange::point(line, 1),
                "Changed TypeScript file requires review.",
            ));
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![changed_ts()])
}
"#,
    );
}

fn write_gorm_review_rule_repo(root: &Path) {
    fs::create_dir_all(root.join(".polint/rules/src")).unwrap();
    let polint_path = repo_root()
        .join("crates/polint")
        .to_string_lossy()
        .replace('\\', "/");
    write_file(
        &root.join(".polint.toml"),
        "[workspace]\ninclude = [\"internal/**/models/**/*.go\"]\n",
    );
    write_file(
        &root.join(".polint/rules/Cargo.toml"),
        &format!(
            r#"[package]
name = "polint-local-rules"
version = "0.1.0"
edition = "2024"
publish = false

[dependencies]
polint = {{ path = "{polint_path}" }}

[workspace]
"#,
        ),
    );
    write_file(
        &root.join(".polint/rules/src/main.rs"),
        r#"use std::process::ExitCode;

use polint::sdk::prelude::*;

#[polint::rule(
    id = "review/gorm-model-read-indexes",
    description = "GORM model changes require read-index validation.",
    severity = "error",
    kind = "review"
)]
fn gorm_model_read_indexes(ctx: &mut RuleCtx<'_>, changes: ChangedFiles<'_>) -> RuleResult {
    let rule_id = ctx.rule_id().to_string();
    for changed in changes.iter() {
        let is_gorm_model =
            changed.path().ends_with(".go") && changed.matches_glob("internal/**/models/**");
        if changed.is_deleted() || !is_gorm_model {
            continue;
        }
        let line = changed.lines().first().map(|&(lo, _)| lo).unwrap_or(1);
        ctx.report(
            Diagnostic::error(
                rule_id.clone(),
                changed.path().to_string(),
                DiagnosticRange::point(line, 1),
                "GORM model changed: validate the correct read indexes for this model.",
            )
            .with_help(
                "Check the read paths for this model and add or update composite indexes in \
                 GORM tags or migrations. If no index is needed, explain why in the PR.",
            ),
        );
    }
    Ok(())
}

fn main() -> ExitCode {
    polint::runner::run_cli(vec![gorm_model_read_indexes()])
}
"#,
    );
}

#[test]
fn review_fires_on_changed_path_and_is_diff_gated() {
    if !review_git_available() {
        eprintln!("skipping `polint review` test; `git` not on PATH");
        return;
    }
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_review_rule_repo(root);
    review_git_init_identity(root);

    // Base commit: a Go source and a migration.
    write_file(&root.join("app.go"), "package app\n\nfunc main() {}\n");
    write_file(
        &root.join("db/migrations/0001_init.sql"),
        "CREATE TABLE t (id INT);\n",
    );
    let base = review_git_commit_all(root, "base");

    // Change the migration (watched) — the review rule should fire.
    write_file(
        &root.join("db/migrations/0001_init.sql"),
        "CREATE TABLE t (id INT, name TEXT);\n",
    );
    review_git_commit_all(root, "change migration");

    let report = stdout_json(
        polint_cmd()
            .current_dir(root)
            .args(["review", &base, "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    let files = diagnostic_files(&report, "review/migrations");
    assert_eq!(
        files,
        vec!["db/migrations/0001_init.sql".to_string()],
        "review should fire for the changed migration: {report:#?}"
    );

    // `polint check` must NOT run the review rule (kind filter).
    let check_report = stdout_json(
        polint_cmd()
            .current_dir(root)
            .args(["check", "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    assert!(
        diagnostics_for_rule(&check_report, "review/migrations").is_empty(),
        "polint check must not emit review-kind diagnostics: {check_report:#?}"
    );
}

#[test]
fn review_line_gate_keeps_watched_path_when_change_is_below_line_one() {
    if !review_git_available() {
        eprintln!("skipping `polint review` test; `git` not on PATH");
        return;
    }
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_review_rule_repo(root);
    review_git_init_identity(root);

    write_file(&root.join("app.go"), "package app\n\nfunc main() {}\n");
    write_file(
        &root.join("db/migrations/0001_init.sql"),
        "CREATE TABLE t (\n    id INT\n);\n",
    );
    let base = review_git_commit_all(root, "base");

    write_file(
        &root.join("db/migrations/0001_init.sql"),
        "CREATE TABLE t (\n    id BIGINT\n);\n",
    );
    review_git_commit_all(root, "change migration below first line");

    let report = stdout_json(
        polint_cmd()
            .current_dir(root)
            .args(["review", &base, "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    let diagnostics = diagnostics_for_rule(&report, "review/migrations");
    assert_eq!(
        diagnostics.len(),
        1,
        "line-aware review gate should keep a changed-line anchored path watcher: {report:#?}"
    );
    assert_eq!(
        diagnostics[0]["range"]["start_line"], 2,
        "path watcher should anchor on the first changed line: {report:#?}"
    );
}

#[test]
fn review_diff_gate_drops_off_diff_findings_unless_opted_out() {
    if !review_git_available() {
        eprintln!("skipping `polint review` test; `git` not on PATH");
        return;
    }
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_review_rule_repo(root);
    review_git_init_identity(root);

    write_file(&root.join("app.go"), "package app\n\nfunc main() {}\n");
    write_file(
        &root.join("db/migrations/0001_init.sql"),
        "CREATE TABLE t (id INT);\n",
    );
    let base = review_git_commit_all(root, "base");

    // Change ONLY app.go — the migration is unchanged, so the rule's finding on
    // db/migrations/** is off-diff and must be gated out by default.
    write_file(
        &root.join("app.go"),
        "package app\n\nfunc main() { _ = 1; }\n",
    );
    review_git_commit_all(root, "change unrelated file");

    // Default diff gate: the off-diff migration finding is dropped.
    let gated = stdout_json(
        polint_cmd()
            .current_dir(root)
            .args(["review", &base, "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    assert!(
        diagnostics_for_rule(&gated, "review/migrations").is_empty(),
        "default diff gate should drop the off-diff finding: {gated:#?}"
    );

    // Now change the migration too and confirm `--no-diff-gate` surfaces it.
    write_file(
        &root.join("db/migrations/0001_init.sql"),
        "CREATE TABLE t (id INT, extra TEXT);\n",
    );
    review_git_commit_all(root, "change migration too");
    let ungated = stdout_json(
        polint_cmd()
            .current_dir(root)
            .args([
                "review",
                &base,
                "--no-diff-gate",
                "--format",
                "json",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );
    assert_eq!(
        diagnostic_files(&ungated, "review/migrations"),
        vec!["db/migrations/0001_init.sql".to_string()],
        "with --no-diff-gate the review finding is surfaced: {ungated:#?}"
    );
}

#[test]
fn review_whole_file_keeps_fixed_line_finding_on_changed_file() {
    if !review_git_available() {
        eprintln!("skipping `polint review` test; `git` not on PATH");
        return;
    }
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_fixed_line_review_rule_repo(root);
    review_git_init_identity(root);

    write_file(&root.join("app.go"), "package app\n\nfunc main() {}\n");
    write_file(
        &root.join("db/migrations/0001_init.sql"),
        "CREATE TABLE t (\n    id INT\n);\n",
    );
    let base = review_git_commit_all(root, "base");

    write_file(
        &root.join("db/migrations/0001_init.sql"),
        "CREATE TABLE t (\n    id BIGINT\n);\n",
    );
    review_git_commit_all(root, "change migration below first line");

    let line_gated = stdout_json(
        polint_cmd()
            .current_dir(root)
            .args(["review", &base, "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    assert!(
        diagnostics_for_rule(&line_gated, "review/migrations").is_empty(),
        "default line-aware gate should drop a fixed line-1 finding: {line_gated:#?}"
    );

    let whole_file = stdout_json(
        polint_cmd()
            .current_dir(root)
            .args([
                "review",
                &base,
                "--whole-file",
                "--format",
                "json",
                "--fail-on",
                "none",
            ])
            .assert()
            .success(),
    );
    assert_eq!(
        diagnostic_files(&whole_file, "review/migrations"),
        vec!["db/migrations/0001_init.sql".to_string()],
        "--whole-file should keep findings on changed files regardless of line range: {whole_file:#?}"
    );
}

#[test]
fn review_honors_comment_ignores_by_default() {
    if !review_git_available() {
        eprintln!("skipping `polint review` test; `git` not on PATH");
        return;
    }
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_ts_review_rule_repo(root);
    review_git_init_identity(root);

    write_file(
        &root.join("src/component.ts"),
        r#"// polint-ignore-next-line review/changed-ts -- reviewed separately
export const value = "old";
"#,
    );
    let base = review_git_commit_all(root, "base");

    write_file(
        &root.join("src/component.ts"),
        r#"// polint-ignore-next-line review/changed-ts -- reviewed separately
export const value = "new";
"#,
    );
    review_git_commit_all(root, "change ignored line");

    let report = stdout_json(
        polint_cmd()
            .current_dir(root)
            .args(["review", &base, "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    assert!(
        diagnostics_for_rule(&report, "review/changed-ts").is_empty(),
        "review should honor comment ignores by default: {report:#?}"
    );
}

#[test]
fn review_gorm_model_example_requires_read_index_validation() {
    if !review_git_available() {
        eprintln!("skipping `polint review` test; `git` not on PATH");
        return;
    }
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_gorm_review_rule_repo(root);
    review_git_init_identity(root);

    write_file(
        &root.join("internal/billing/models/invoice.go"),
        r#"package models

import "time"

type Invoice struct {
	ID        string
	AccountID string
	Status    string
	CreatedAt time.Time
}
"#,
    );
    write_file(
        &root.join("internal/billing/service.go"),
        "package billing\n\nfunc Service() {}\n",
    );
    let base = review_git_commit_all(root, "base");

    write_file(
        &root.join("internal/billing/models/invoice.go"),
        r#"package models

import "time"

type Invoice struct {
	ID        string
	AccountID string `gorm:"index:idx_invoices_account_status_created_at,priority:1"`
	Status    string `gorm:"index:idx_invoices_account_status_created_at,priority:2"`
	CreatedAt time.Time `gorm:"index:idx_invoices_account_status_created_at,priority:3"`
}
"#,
    );
    review_git_commit_all(root, "add invoice read index");

    let report = stdout_json(
        polint_cmd()
            .current_dir(root)
            .args(["review", &base, "--format", "json", "--fail-on", "none"])
            .assert()
            .success(),
    );
    let diagnostics = diagnostics_for_rule(&report, "review/gorm-model-read-indexes");
    assert_eq!(
        diagnostics.len(),
        1,
        "changed GORM model should require read-index validation: {report:#?}"
    );
    assert_eq!(
        diagnostics[0]["file"], "internal/billing/models/invoice.go",
        "diagnostic should point at the changed GORM model: {report:#?}"
    );
    assert_eq!(
        diagnostics[0]["range"]["start_line"], 7,
        "diagnostic should anchor on the first changed model line: {report:#?}"
    );
}

#[test]
fn review_requires_local_rule_hosts() {
    if !review_git_available() {
        eprintln!("skipping `polint review` test; `git` not on PATH");
        return;
    }
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write_file(
        &root.join(".polint.toml"),
        "[workspace]\ninclude = [\"**/*.txt\"]\n",
    );
    review_git_init_identity(root);
    write_file(&root.join("a.txt"), "x\n");
    review_git_commit_all(root, "base");

    polint_cmd()
        .current_dir(root)
        .args(["review", "HEAD"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("requires repo-local rules"))
        .stderr(predicate::str::contains(
            "polint new-rule generic <name> --review",
        ));
}

#[cfg(not(feature = "lang-go"))]
#[test]
fn disabled_go_feature_reports_capability_diagnostic() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        &dir.path().join("main.go"),
        "package main\nfunc main() {}\n",
    );

    let report = stdout_json(
        polint_cmd()
            .current_dir(dir.path())
            .args([
                "check",
                "--format",
                "json",
                "--fail-on",
                "none",
                "--no-cache",
            ])
            .assert()
            .success(),
    );
    let diagnostic = diagnostics_for_rule(&report, "polint/capability")
        .into_iter()
        .find(|diagnostic| diagnostic_has_evidence(diagnostic, "feature", "lang-go"))
        .expect("disabled Go frontend should emit a capability diagnostic");
    assert_eq!(diagnostic["file"], "main.go");
    assert!(diagnostic_has_evidence(
        diagnostic,
        "reason",
        "language-feature-disabled"
    ));
    assert!(diagnostic_has_evidence(diagnostic, "status", "unsupported"));
}

#[cfg(not(feature = "lang-typescript"))]
#[test]
fn disabled_typescript_feature_reports_capability_diagnostic() {
    let dir = tempfile::tempdir().unwrap();
    write_file(&dir.path().join("main.ts"), "export const value = 1;\n");

    let report = stdout_json(
        polint_cmd()
            .current_dir(dir.path())
            .args([
                "check",
                "--format",
                "json",
                "--fail-on",
                "none",
                "--no-cache",
            ])
            .assert()
            .success(),
    );
    let diagnostic = diagnostics_for_rule(&report, "polint/capability")
        .into_iter()
        .find(|diagnostic| diagnostic_has_evidence(diagnostic, "feature", "lang-typescript"))
        .expect("disabled TypeScript frontend should emit a capability diagnostic");
    assert_eq!(diagnostic["file"], "main.ts");
    assert!(diagnostic_has_evidence(
        diagnostic,
        "reason",
        "language-feature-disabled"
    ));
    assert!(diagnostic_has_evidence(diagnostic, "status", "unsupported"));
}

#[cfg(feature = "lang-go")]
#[test]
fn enabled_go_feature_does_not_report_feature_disabled() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        &dir.path().join("main.go"),
        "package main\nfunc main() {}\n",
    );
    let report = stdout_json(
        polint_cmd()
            .current_dir(dir.path())
            .args([
                "check",
                "--format",
                "json",
                "--fail-on",
                "none",
                "--no-cache",
            ])
            .assert()
            .success(),
    );
    assert!(
        diagnostics_for_rule(&report, "polint/capability")
            .iter()
            .all(|diagnostic| { !diagnostic_has_evidence(diagnostic, "feature", "lang-go") })
    );
}

#[cfg(feature = "lang-typescript")]
#[test]
fn enabled_typescript_feature_does_not_report_feature_disabled() {
    let dir = tempfile::tempdir().unwrap();
    write_file(&dir.path().join("main.ts"), "export const value = 1;\n");
    let report = stdout_json(
        polint_cmd()
            .current_dir(dir.path())
            .args([
                "check",
                "--format",
                "json",
                "--fail-on",
                "none",
                "--no-cache",
            ])
            .assert()
            .success(),
    );
    assert!(
        diagnostics_for_rule(&report, "polint/capability")
            .iter()
            .all(|diagnostic| {
                !diagnostic_has_evidence(diagnostic, "feature", "lang-typescript")
            })
    );
}
