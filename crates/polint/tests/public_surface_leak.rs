// Compile-time gate that locks the supported public surface.
//
// This test compiles the probe crate at tests/fixtures/public-surface-leak-probe/
// and asserts that the public polint::sdk::prelude::* allow-list changes only
// through documented promotion records.
//
// The ALLOWED_PRELUDE constant below is the source of truth. Extending the list
// is a deliberate API change that requires a documented promotion record under
// docs/API-VISIBILITY-PLAN.md before it can be merged. The probe deliberately
// adds preview policy-query vocabulary while keeping raw CFG/call graph/solver
// internals private.
//
// The gate runs on Linux + macOS in fast CI on every PR (D-18). Both platforms
// must pass independently — no averaging, no skipping either platform.
//
// Strategy: APPROACH B (direct cargo invocation, no new dependency). The gate
// shells out to `cargo build` on the excluded probe crate and asserts a clean
// compile, then snapshot-compares the prelude re-export block against
// ALLOWED_PRELUDE. trybuild was deliberately NOT added — the no-new-deps
// discipline of Plans 01/02 (T-42-SC) carries here, and a direct cargo
// invocation gives rustc-level granularity without a UI-test dependency.
//
// If you are reading this comment because the test failed:
//   1. Did you accidentally add `pub` to a v1.3 type? Make it `pub(crate)`.
//   2. Did you add a sanctioned new public type via release-close review?
//      Extend ALLOWED_PRELUDE in the same PR and reference the review record,
//      and add a witness for it in the probe crate's allowlist_witness module.
//   3. Did the probe crate's dependency on polint break? Restore the probe's
//      `#![no_implicit_prelude]` and single `use ::polint::sdk::prelude::*;` import.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The deliberate public `polint::sdk::prelude` re-export allow-list.
///
/// Source of truth: `crates/polint/src/sdk/mod.rs` `pub mod prelude { ... }`.
/// `allowlist_matches_prelude_source` asserts the parsed prelude block equals
/// this set EXACTLY. Any addition (sanctioned or not) trips the gate; sanctioned
/// additions extend this list in the same PR with a documented promotion record.
const ALLOWED_PRELUDE: &[&str] = &[
    // crate::core types
    "BranchId",
    "BranchObligation",
    "CapabilitySupport",
    "CapabilitySupportStatus",
    "CapabilitySupportView",
    "CapabilityCompleteness",
    "CapabilityCompletenessStatus",
    "ChangeStatus",
    "ComplexityMetricFact",
    "CompletenessView",
    "CoverageFact",
    "DefinitionFact",
    "DefinitionId",
    "DefinitionKind",
    "FileId",
    "FileMetricFact",
    "FunctionFact",
    "FunctionId",
    "FunctionMetricFact",
    // H7 promotion (perf/algo-10x wave 5): typed Go structural facts behind the
    // GoTypeDecls view so consumer rules stop regex-parsing Go source.
    "GoTypeDeclFact",
    "GoTypeDeclKind",
    "ImportFact",
    "ImportId",
    "JsxAttributeFact",
    "Language",
    "ModuleEdge",
    "ModuleEdgeId",
    "ModuleEdgeKind",
    "ModuleNode",
    "ModuleNodeId",
    "ModuleNodeKind",
    "NodeId",
    "PackageFact",
    "PackageId",
    "ReferenceFact",
    "ReferenceId",
    "ReferenceKind",
    "ResolutionPrecision",
    "ResolutionStatus",
    "ResolvedImportFact",
    "ResolvedImportId",
    "Rule",
    "RuleConfigValue",
    "RuleCtx",
    "RuleId",
    "RuleOptions",
    "SourceFile",
    "Span",
    "StringLiteralFact",
    "SymbolFact",
    "SymbolId",
    "SymbolKind",
    "SymbolNamespace",
    "SymbolPrecision",
    "SymbolResolutionStatus",
    "TestFact",
    "TextRange",
    "TsClassFact",
    "TsComponentFact",
    "UnresolvedReason",
    // crate::diagnostics
    "ColorChoice",
    "Diagnostic",
    "Evidence",
    "Fix",
    "JsonReportMeta",
    "Label",
    "OutputFormat",
    "POLINT_REPORT_JSON_SCHEMA_V1_URL",
    "PolintReport",
    "PolintToolInfo",
    "RenderOpts",
    "Severity",
    "StructuredEvidenceV1",
    "Suggestion",
    "DiagnosticRange", // re-export alias: `TextRange as DiagnosticRange`
    "diagnostics_from_json_report",
    // crate::rule_error
    "RuleError",
    "RuleResult",
    // crate::sdk free fn
    "collect_go_tests",
    // crate::sdk::facts
    "BranchObligations",
    "CallGraph",
    "ChangedFiles",
    "Calls",
    "Cfg",
    "ComplexityMetrics",
    "ControlFlow",
    "CoverageFacts",
    "DataFlow",
    "Events",
    "FileMetrics",
    "FunctionMetrics",
    "Functions",
    "GoTests",
    "GoTypeDecls",
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
    // crate::sdk::policy preview vocabulary
    "BarrierPattern",
    "EventPattern",
    "FlowQuery",
    "GuardPattern",
    "GuardQuery",
    "LifecycleQuery",
    "PolicyConfidence",
    "PolicyPrecision",
    "PolicyStatus",
    "PolicyViolation",
    "ReachQuery",
    "SinkPattern",
    "SourcePattern",
    // crate::sdk::scope
    "file_in_scope",
    "file_matches_globs",
    "glob_matches",
];

/// Repository root, derived from this crate's manifest dir (`crates/polint`).
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/polint has a workspace root two levels up")
        .to_path_buf()
}

fn probe_manifest_path() -> PathBuf {
    repo_root().join("tests/fixtures/public-surface-leak-probe/Cargo.toml")
}

fn sdk_mod_source() -> String {
    let path = repo_root().join("crates/polint/src/sdk/mod.rs");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
}

fn probe_lib_source() -> String {
    let path = repo_root().join("tests/fixtures/public-surface-leak-probe/src/lib.rs");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
}

const STORE_PRIVATE_MARKERS: &[&str] = &[
    "polint::analysis_kernel::store",
    "polint::store",
    "SemanticStore",
    "StoreConfig",
    "StoreStatus",
    "rusqlite",
    "_polint_schema_migrations",
    "SQLITE_OPEN_READ_WRITE",
    "SQLITE_OPEN_READ_ONLY",
    "TransactionBehavior::Immediate",
    "PRAGMA user_version",
    "raw_row_id",
    "sqlite_row_id",
    "ProviderOutcomeStatus",
    "ProviderOutputIdentity",
    "ProviderFailureSignal",
    "ProviderFailureStage",
    "ProviderFailureReason",
    "ValidationReport",
    "ValidationIssue",
    "MetricsProviderProjection",
    "CanonicalMetricsInputs",
    "GoSyntaxProviderProjection",
    "CanonicalGoSyntaxInputs",
    "CanonicalGoSyntaxOutput",
    "GoSyntaxParserContract",
    "PublicationInputs",
    "MetricsMatch",
    "GoSyntaxMatch",
    "metrics_provider_mirror",
    "metrics_provider_members",
    "metrics_provider_blockers",
    "metrics_provider_sources",
    "metrics_provider_functions",
    "go_syntax_provider_mirror",
    "go_syntax_provider_members",
    "go_syntax_provider_blockers",
    "go_syntax_provider_sources",
    "go_syntax_provider_parser",
    "polint-go-syntax-provider-mirror",
    "go-syntax-layer-v1",
    "go-parser-contract-v1",
    "go-facts-v2",
    "go-source:",
    "go-parser:",
];

fn store_private_marker_hits(source: &str) -> Vec<&'static str> {
    STORE_PRIVATE_MARKERS
        .iter()
        .copied()
        .filter(|marker| source.contains(marker))
        .collect()
}

fn collect_public_tree(path: &Path, texts: &mut Vec<(PathBuf, String)>) {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return;
    };
    if metadata.file_type().is_symlink() {
        return;
    }
    if metadata.is_file() {
        let supported = matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("rs" | "md")
        );
        if supported {
            let source = std::fs::read_to_string(path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
            texts.push((path.to_path_buf(), source));
        }
        return;
    }
    let mut entries = std::fs::read_dir(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|error| panic!("failed to enumerate {}: {error}", path.display()));
    entries.sort_by_key(std::fs::DirEntry::path);
    for entry in entries {
        collect_public_tree(&entry.path(), texts);
    }
}

fn assert_no_store_private_markers(label: &str, source: &str) {
    let hits = store_private_marker_hits(source);
    assert!(
        hits.is_empty(),
        "{label} leaks private semantic-store vocabulary: {hits:?}"
    );
}

/// Parses the `pub mod prelude { ... }` block of `sdk/mod.rs` and returns the
/// set of identifier names re-exported through it.
///
/// BLOCKER #6: this helper is exercised directly by
/// `parser_self_test_detects_synthetic_leak` so a buggy parser cannot silently
/// hollow out the gate. It returns the FINAL nameable identifier for each
/// re-export, honoring `X as Y` aliases (yielding `Y`).
pub fn parse_prelude_reexports(source: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();

    let Some(block_start) = source.find("pub mod prelude") else {
        return names;
    };
    let after_kw = &source[block_start..];
    let Some(open_rel) = after_kw.find('{') else {
        return names;
    };

    // Walk from the opening brace tracking depth so we stop at the prelude
    // module's matching close brace and never read sibling modules.
    let bytes = after_kw.as_bytes();
    let mut depth = 0usize;
    let mut end = open_rel;
    for (i, &b) in bytes.iter().enumerate().skip(open_rel) {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    end = i;
                    break;
                }
            }
            _ => {}
        }
    }
    let block = &after_kw[open_rel + 1..end];

    // Collect every `pub use ...;` statement, then extract the leaf identifiers.
    for raw_stmt in block.split(';') {
        let stmt = raw_stmt.trim();
        let Some(rest) = stmt.strip_prefix("pub use ") else {
            continue;
        };
        if let Some(brace_open) = rest.find('{') {
            // Grouped: `pub use crate::path::{A, B, C as D}`
            let Some(brace_close) = rest.rfind('}') else {
                continue;
            };
            let inner = &rest[brace_open + 1..brace_close];
            for item in inner.split(',') {
                if let Some(name) = leaf_identifier(item) {
                    names.insert(name);
                }
            }
        } else {
            // Single: `pub use crate::path::name` (optionally `as alias`)
            if let Some(name) = leaf_identifier(rest) {
                names.insert(name);
            }
        }
    }

    names
}

/// Extracts the final nameable identifier from a single re-export item,
/// honoring `X as Y` aliases (returns `Y`) and `path::Leaf` paths (returns
/// `Leaf`). Returns `None` for empty / glob items.
fn leaf_identifier(item: &str) -> Option<String> {
    let item = item.trim();
    if item.is_empty() || item == "*" {
        return None;
    }
    // Honor `... as alias` first.
    if let Some((_, alias)) = item.rsplit_once(" as ") {
        let alias = alias.trim();
        return (!alias.is_empty()).then(|| alias.to_string());
    }
    // Otherwise take the final path segment.
    let leaf = item.rsplit("::").next().unwrap_or(item).trim();
    (!leaf.is_empty() && leaf != "*").then(|| leaf.to_string())
}

#[test]
fn probe_crate_compiles_against_prelude_only() {
    let manifest = probe_manifest_path();
    assert!(
        manifest.exists(),
        "probe manifest missing at {} — restore the checked-in leak-gate fixture",
        manifest.display()
    );

    // NOTE: intentionally NOT `--locked`. This gate only proves the probe still
    // compiles against `polint::sdk::prelude::*`; pinning the probe's transitive
    // dep versions is not its job. The probe's Cargo.lock pins `polint` by version,
    // but release bumps update the workspace lock without touching the excluded
    // probe lock, so `--locked` would abort with "cannot update the lock file" on
    // every release. Letting cargo refresh the stale version entry in-memory keeps
    // the gate robust. (The OUTER `cargo test --locked` in CI still validates the
    // workspace lock.)
    let output = Command::new(env!("CARGO"))
        .args([
            "build",
            "--manifest-path",
            manifest.to_str().expect("utf-8 manifest path"),
            "--message-format=short",
        ])
        .output()
        .expect("failed to invoke cargo to build the leak-gate probe crate");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "leak-gate probe crate FAILED to compile against `polint::sdk::prelude::*`.\n\
         A v1.0–v1.2 allow-listed identifier was likely dropped from the prelude, \
         or the probe was tampered with.\n\
         See crates/polint/tests/public_surface_leak.rs top comment.\n\
         --- cargo stdout ---\n{stdout}\n--- cargo stderr ---\n{stderr}"
    );

    assert!(
        !stderr.contains("error[E0"),
        "leak-gate probe emitted rustc errors:\n{stderr}"
    );
}

#[test]
fn allowlist_matches_prelude_source() {
    let source = sdk_mod_source();
    let actual = parse_prelude_reexports(&source);
    let expected: BTreeSet<String> = ALLOWED_PRELUDE.iter().map(|s| s.to_string()).collect();

    let unsanctioned: Vec<&String> = actual.difference(&expected).collect();
    let missing: Vec<&String> = expected.difference(&actual).collect();

    assert!(
        unsanctioned.is_empty() && missing.is_empty(),
        "polint::sdk::prelude exports diverged from the documented public allow-list — \
         see crates/polint/tests/public_surface_leak.rs top comment for the discipline policy.\n\
         UNSANCTIONED additions (in prelude, NOT in ALLOWED_PRELUDE): {unsanctioned:?}\n\
         MISSING (in ALLOWED_PRELUDE, NOT in prelude): {missing:?}"
    );

    // Defensive cross-check: the prelude block must never re-export private
    // analysis namespaces directly.
    let block_start = source
        .find("pub mod prelude")
        .expect("prelude block exists");
    let block = &source[block_start..];
    let block_end = block
        .find("\n}")
        .map(|i| i + block_start)
        .unwrap_or(source.len());
    let prelude_block = &source[block_start..block_end];
    assert!(
        !prelude_block.contains("pub use crate::analysis::")
            && !prelude_block.contains("pub use crate::analysis_kernel::"),
        "polint::sdk::prelude re-exports a private analysis namespace — this is a public-surface leak.\n{prelude_block}"
    );
}

#[test]
fn ensure_no_private_namespace_in_probe() {
    let src = probe_lib_source();

    // (a) #![no_implicit_prelude] must be present (T-42-04-04 mitigation).
    assert!(
        src.lines().any(|l| l.trim() == "#![no_implicit_prelude]"),
        "probe lib.rs is missing `#![no_implicit_prelude]` — removing it lets std's \
         prelude mask accidental polint-prelude additions (T-42-04-04). \
         Restore it to preserve the leak-gate isolation contract."
    );

    // (b) EXACTLY one `use polint::` line, and it is the prelude glob
    // (the leading `::` is required under no_implicit_prelude; T-42-04-05).
    let polint_use_lines: Vec<&str> = src
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("use ") && l.contains("polint::"))
        .collect();
    assert_eq!(
        polint_use_lines.len(),
        1,
        "probe lib.rs must contain EXACTLY one `use polint::` import; found: {polint_use_lines:?} \
         (T-42-04-05 — the single glob is the catch-all)"
    );
    assert!(
        polint_use_lines[0] == "use ::polint::sdk::prelude::*;"
            || polint_use_lines[0] == "use polint::sdk::prelude::*;",
        "probe lib.rs's single polint import must be the prelude glob, found: {}",
        polint_use_lines[0]
    );

    // (c) ZERO private-namespace path substrings anywhere in executable code.
    // Strip comment lines first so the cautionary doc-comment listing the
    // forbidden namespaces does not trip the check.
    let code_only: String = src
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !t.starts_with("//")
        })
        .collect::<Vec<_>>()
        .join("\n");
    for forbidden in [
        "polint::analysis",
        "polint::analysis_kernel",
        "polint::core",
        "polint::cache",
        "polint::config",
        "polint::cli",
        "polint::go",
        "polint::ts",
        "polint::graph",
        "polint::eval",
        "polint::rule_manifest",
        "polint::store",
        "SemanticStore",
        "StoreConfig",
        "StoreStatus",
        "rusqlite",
        "_polint_schema_migrations",
    ] {
        assert!(
            !code_only.contains(forbidden),
            "probe lib.rs references private namespace `{forbidden}` in code — the whole point \
             is that private types are NOT nameable here (D-23). Remove it."
        );
    }
}

#[test]
fn semantic_store_markers_do_not_leak_into_supported_public_surfaces() {
    let root = repo_root();
    let mut texts = Vec::new();
    for relative in [
        "crates/polint/src/sdk",
        "crates/polint/src/runner/mod.rs",
        "crates/polint/src/cli/mod.rs",
        "crates/polint/src/lib.rs",
        "README.md",
        "docs/API-VISIBILITY-PLAN.md",
        "docs/facts",
        "examples",
    ] {
        collect_public_tree(&root.join(relative), &mut texts);
    }
    assert!(!texts.is_empty(), "public surface scan collected no files");
    for (path, source) in &texts {
        assert_no_store_private_markers(&path.display().to_string(), source);
    }

    let repo = tempfile::tempdir().expect("temporary public-output repo");
    std::fs::write(
        repo.path().join("main.go"),
        "package main\n\nfunc main() {}\n",
    )
    .expect("write public-output source");
    let check = Command::new(env!("CARGO_BIN_EXE_polint"))
        .current_dir(repo.path())
        .args(["check", "--format", "json", "--fail-on", "none"])
        .output()
        .expect("run public check JSON");
    assert!(
        check.status.success(),
        "public check failed: {}",
        String::from_utf8_lossy(&check.stderr)
    );
    let check_json = String::from_utf8(check.stdout).expect("check JSON is UTF-8");
    serde_json::from_str::<serde_json::Value>(&check_json).expect("check output is JSON");
    assert_no_store_private_markers("polint check --format json", &check_json);

    let skill = Command::new(env!("CARGO_BIN_EXE_polint"))
        .current_dir(repo.path())
        .args(["add-skill", "--agent", "claude"])
        .output()
        .expect("generate supported skill text");
    assert!(
        skill.status.success(),
        "skill generation failed: {}",
        String::from_utf8_lossy(&skill.stderr)
    );
    let skill_path = repo.path().join(".claude/skills/polint/SKILL.md");
    let skill_text = std::fs::read_to_string(&skill_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", skill_path.display()));
    assert_no_store_private_markers("generated polint skill", &skill_text);
}

#[test]
fn semantic_store_marker_scanner_negative_controls_cover_every_family() {
    let controls = [
        ("private module", "use polint::analysis_kernel::store;"),
        ("private type", "let _: SemanticStore;"),
        ("sqlite crate", "use rusqlite::Connection;"),
        ("private table", "SELECT * FROM _polint_schema_migrations"),
        ("raw database flag", "SQLITE_OPEN_READ_WRITE"),
        ("migration statement", "PRAGMA user_version"),
        ("raw identifier", "sqlite_row_id"),
        ("provider outcome", "ProviderOutcomeStatus::Succeeded"),
        ("Go syntax mirror", "let _: GoSyntaxMatch;"),
        ("validation ownership", "ValidationIssue { provider_ids }"),
    ];

    for (family, source) in controls {
        assert!(
            !store_private_marker_hits(source).is_empty(),
            "negative control for {family} was not detected"
        );
    }
    assert!(
        store_private_marker_hits("ordinary row/store/connection policy text").is_empty(),
        "scanner must not ban generic public prose"
    );
}

#[test]
fn parser_self_test_detects_synthetic_leak() {
    // Negative control: a synthetic prelude block that smuggles in an
    // unsanctioned private re-export. The parser MUST surface `IdentityRecord`.
    let synthetic_leak = r#"
        pub mod prelude {
            pub use crate::core::{Rule, RuleCtx, RuleId};
            pub use crate::diagnostics::{Diagnostic, Severity};
            pub use crate::analysis::identity::IdentityRecord;
        }
    "#;
    let parsed = parse_prelude_reexports(synthetic_leak);
    assert!(
        parsed.contains("IdentityRecord"),
        "parse_prelude_reexports FAILED to detect the synthetic `IdentityRecord` leak — \
         the gate's parser is broken and would silently accept unsanctioned additions \
         (BLOCKER #6). Parsed set was: {parsed:?}"
    );

    // Positive control: a synthetic block with ONLY allow-listed names must
    // parse to exactly that set, with no false positives and no dropped names.
    let synthetic_clean = r#"
        pub mod prelude {
            pub use crate::core::{Rule, RuleCtx};
            pub use crate::diagnostics::{Diagnostic};
            pub use crate::diagnostics::{TextRange as DiagnosticRange};
        }
    "#;
    let parsed_clean = parse_prelude_reexports(synthetic_clean);
    let expected_clean: BTreeSet<String> = ["Rule", "RuleCtx", "Diagnostic", "DiagnosticRange"]
        .into_iter()
        .map(str::to_string)
        .collect();
    assert_eq!(
        parsed_clean, expected_clean,
        "parse_prelude_reexports produced a false positive or dropped an allow-listed name \
         (BLOCKER #6). Expected {expected_clean:?}, got {parsed_clean:?}"
    );
    // Every name returned by the clean control IS in the real allow-list.
    let allowed: BTreeSet<String> = ALLOWED_PRELUDE.iter().map(|s| s.to_string()).collect();
    for name in &parsed_clean {
        assert!(
            allowed.contains(name),
            "parser self-test positive control yielded `{name}`, which is not in ALLOWED_PRELUDE"
        );
    }
}

#[test]
fn allowlist_has_no_duplicates_and_expected_count() {
    let unique: BTreeSet<&&str> = ALLOWED_PRELUDE.iter().collect();
    assert_eq!(
        unique.len(),
        ALLOWED_PRELUDE.len(),
        "ALLOWED_PRELUDE contains duplicate entries"
    );
    // Locked count derived from sdk/mod.rs; last bumped for the rule-visible
    // completeness accessor and its typed result vocabulary.
    assert_eq!(
        ALLOWED_PRELUDE.len(),
        122,
        "ALLOWED_PRELUDE count changed — update this assertion ONLY alongside a sanctioned \
         API promotion record"
    );
}

/// Internal source roots that intentionally own identifiers re-exported by the facade prelude.
///
/// Walking each internal owner tree keeps this gate independent of the current module layout
/// inside `polint` while excluding examples, benchmarks, and test fixtures from the definition
/// scan.
const PRELUDE_OWNER_SOURCE_ROOTS: &[&str] = &[
    "crates/polint/src/internal_core",
    "crates/polint/src/ir",
    "crates/polint/src/analysis_api",
    "crates/polint/src/frontend_api",
    "crates/polint/src/go",
    "crates/polint/src/ts",
    "crates/polint/src/analysis_neutral",
    "crates/polint/src",
];

fn prelude_definition_sources() -> Vec<(PathBuf, String)> {
    let root = repo_root();
    let mut sources = Vec::new();
    for relative_root in PRELUDE_OWNER_SOURCE_ROOTS {
        let source_root = root.join(relative_root);
        assert!(
            source_root.is_dir(),
            "prelude owner source root missing: {}",
            source_root.display()
        );
        collect_public_tree(&source_root, &mut sources);
    }
    sources.retain(|(path, _)| path.extension().and_then(|ext| ext.to_str()) == Some("rs"));
    sources
}

/// Prelude-exported structs and enums must stay `#[non_exhaustive]` so adding a
/// field or variant is not a silent semver break for rule packs.
///
/// Type aliases, free functions, and consts in `ALLOWED_PRELUDE` are skipped.
/// `DiagnosticRange` is the prelude alias for diagnostics `TextRange`.
#[test]
fn prelude_structs_and_language_are_non_exhaustive() {
    let sources = prelude_definition_sources();

    let skip: BTreeSet<&str> = [
        "RuleConfigValue",
        "RuleResult",
        "POLINT_REPORT_JSON_SCHEMA_V1_URL",
        "collect_go_tests",
        "diagnostics_from_json_report",
        "file_in_scope",
        "file_matches_globs",
        "glob_matches",
        // Alias of diagnostics::TextRange — covered under TextRange below.
        "DiagnosticRange",
        // Position/identity value types that rule packs legitimately construct when they
        // compute their own ranges. `#[non_exhaustive]` would forbid the struct literal
        // while buying nothing: every constructor for these takes all fields positionally,
        // so adding a field is a breaking change either way. Verified against a real
        // consumer — these three were the only compile breaks in a 25-rule repo-local pack.
        // Do NOT add a type here to silence this gate; the exemption is for value types
        // with total constructors, not for anything inconvenient.
        "Span",
        "TextRange",
        "RuleId",
        // The `id_newtype!` identity handles, for the same reason: rule packs construct them
        // directly in their own fixtures (`Span::point(FileId(1), 1, 1)` appears verbatim in a
        // real consumer pack), and `from_raw` already takes the single field, so the attribute
        // forbids tuple construction without buying room to add one.
        "BranchId",
        "DefinitionId",
        "FileId",
        "FunctionId",
        "ImportId",
        "ModuleEdgeId",
        "ModuleNodeId",
        "NodeId",
        "PackageId",
        "ReferenceId",
        "ResolvedImportId",
        "SymbolId",
    ]
    .into_iter()
    .collect();

    let mut defs: BTreeMap<String, Vec<(PathBuf, usize, bool)>> = BTreeMap::new();
    for (path, text) in &sources {
        let lines: Vec<&str> = text.lines().collect();
        for (idx, line) in lines.iter().enumerate() {
            let trimmed = line.trim_start();
            let Some(rest) = trimmed
                .strip_prefix("pub struct ")
                .or_else(|| trimmed.strip_prefix("pub enum "))
            else {
                continue;
            };
            let name = rest
                .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .next()
                .unwrap_or("");
            if name.is_empty() {
                continue;
            }
            let has_ne = preceding_item_attrs_include_non_exhaustive(&lines, idx);
            defs.entry(name.to_string())
                .or_default()
                .push((path.clone(), idx + 1, has_ne));
        }
        collect_macro_generated_definitions(path, text, &mut defs);
    }

    let mut missing = Vec::new();
    for &name in ALLOWED_PRELUDE {
        if skip.contains(name) {
            continue;
        }
        let Some(sites) = defs.get(name) else {
            missing.push(format!("{name}: definition not found in SDK source scan"));
            continue;
        };
        // TextRange appears in both core and diagnostics; every definition must
        // carry the attribute. Other names should be unique.
        for (path, line, has_ne) in sites {
            if !has_ne {
                missing.push(format!(
                    "{name} at {}:{} lacks #[non_exhaustive]",
                    path.display(),
                    line
                ));
            }
        }
    }

    assert!(
        missing.is_empty(),
        "prelude-exported structs/enums must be #[non_exhaustive] (Language included).\n\
         Missing:\n  {}",
        missing.join("\n  ")
    );

    let language = defs
        .get("Language")
        .expect("Language must remain a prelude-exported enum");
    assert!(
        language.iter().any(|(_, _, has_ne)| *has_ne),
        "Language must be #[non_exhaustive]"
    );
}

/// Accounts for public structs generated by the core ID newtype macro.
///
/// The macro body contains the item attribute, while each invocation supplies the concrete
/// public type name. Recording invocations keeps the source scan honest without requiring a
/// compiler-expanded AST or a second public-type allow-list.
fn collect_macro_generated_definitions(
    path: &Path,
    text: &str,
    defs: &mut BTreeMap<String, Vec<(PathBuf, usize, bool)>>,
) {
    let lines: Vec<&str> = text.lines().collect();
    let macro_has_non_exhaustive = lines.iter().enumerate().any(|(idx, line)| {
        line.trim() == "#[non_exhaustive]"
            && lines
                .get(idx + 1)
                .is_some_and(|next| next.contains("pub struct $name"))
    });
    if !macro_has_non_exhaustive {
        return;
    }

    for (idx, line) in lines.iter().enumerate() {
        let Some(rest) = line.trim_start().strip_prefix("id_newtype!(") else {
            continue;
        };
        let name = rest
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .next()
            .unwrap_or("");
        if name.is_empty() {
            continue;
        }
        defs.entry(name.to_string())
            .or_default()
            .push((path.to_path_buf(), idx + 1, true));
    }
}

fn preceding_item_attrs_include_non_exhaustive(lines: &[&str], item_idx: usize) -> bool {
    let mut j = item_idx;
    while j > 0 {
        j -= 1;
        let s = lines[j].trim();
        if s.is_empty() {
            continue;
        }
        if s.starts_with("///") || s.starts_with("//!") {
            continue;
        }
        if s.starts_with("#[") {
            if s.contains("non_exhaustive") {
                return true;
            }
            continue;
        }
        break;
    }
    false
}
