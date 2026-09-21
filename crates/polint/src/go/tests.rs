use super::*;
use crate::analysis_api::{BranchObligation, FactDatabase};
use crate::go::local_db::LocalFactDb;
use crate::internal_core::DiagnosticRange as TextRange;
use crate::internal_core::Language;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static CACHE_ROOT_COUNTER: AtomicU64 = AtomicU64::new(0);

fn db_with_go_file(relative_path: &str, source: &str) -> LocalFactDb {
    let mut db = LocalFactDb::new();
    db.add_file(
        PathBuf::from(relative_path),
        relative_path.to_string(),
        source.to_string(),
    );
    db
}

fn unique_cache_root() -> PathBuf {
    let sequence = CACHE_ROOT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "polint-go-cache-{}-{nanos}-{sequence}",
        std::process::id()
    ))
}

fn collect_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_files_into(root, &mut files);
    files.sort();
    files
}

fn collect_files_into(root: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries {
        let entry = entry.expect("read cache entry");
        let path = entry.path();
        if path.is_dir() {
            collect_files_into(&path, files);
        } else {
            files.push(path);
        }
    }
}

fn first_layer_file(cache_root: &Path, category: &str) -> PathBuf {
    collect_files(&cache_root.join("layers").join(category))
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("expected layer cache {category} file"))
}

fn layer_cache_text(cache_root: &Path) -> String {
    collect_files(&cache_root.join("layers"))
        .into_iter()
        .map(|path| fs::read_to_string(path).expect("read layer cache file"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn corrupt_first_manifest_output_digest(cache_root: &Path) {
    let manifest_path = first_layer_file(cache_root, "manifests");
    let mut manifest_json: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).expect("read manifest"))
            .expect("manifest json");
    manifest_json["output_digest"]["value"] = serde_json::json!("deadbeef");
    fs::write(
        manifest_path,
        serde_json::to_vec(&manifest_json).expect("manifest serializes"),
    )
    .expect("write manifest");
}

#[test]
fn cache_writes_and_restores_go_facts() {
    let cache_root = unique_cache_root();
    let cache = crate::go::test_cache::FsAnalysisCache::new(&cache_root, true);
    let source = r#"
package payment

import "fmt"

func Authorize(user string) error {
if user == "" {
    return fmt.Errorf("blocked")
}
return nil
}
"#;
    let mut first = db_with_go_file("payment.go", source);

    let first_diagnostics = analyze_with_cache(&mut first, &cache, "config", "rule");

    assert!(first_diagnostics.is_empty());
    assert!(cache_root.exists());
    let first_functions = first
        .functions()
        .iter()
        .map(|fact| fact.name.as_str())
        .collect::<Vec<_>>();

    let mut second = db_with_go_file("payment.go", source);
    let second_diagnostics = analyze_with_cache(&mut second, &cache, "config", "rule");
    let second_functions = second
        .functions()
        .iter()
        .map(|fact| fact.name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(second_diagnostics, first_diagnostics);
    assert_eq!(second_functions, first_functions);
    assert_eq!(second.imports()[0].path, "fmt");
    assert!(
        second
            .string_literals()
            .iter()
            .any(|literal| literal.value == "blocked")
    );

    fs::remove_dir_all(cache_root).ok();
}

#[test]
fn go_syntax_layer_cache_cold_warm() {
    let cache_root = unique_cache_root();
    let cache = crate::go::test_cache::FsAnalysisCache::new(cache_root.join("analysis"), true);
    let source = r#"
package payment

func Authorize() {}
"#;
    let mut first = db_with_go_file("payment.go", source);

    let first_result =
        analyze_with_plan_options_and_cache_stats(&mut first, &cache, "config", "rule", "", false);

    assert!(first_result.diagnostics.is_empty());
    assert_eq!(first_result.cache_stats.misses, 1);
    assert_eq!(first_result.cache_stats.recomputes, 1);
    assert_eq!(first_result.cache_stats.writes, 1);
    assert_eq!(first_result.cache_stats.verified_reuse, 0);
    assert_eq!(first_result.cache_stats.quarantines, 0);
    assert!(cache_root.join("layers").exists());

    let mut second = db_with_go_file("payment.go", source);
    let second_result =
        analyze_with_plan_options_and_cache_stats(&mut second, &cache, "config", "rule", "", false);

    assert!(second_result.diagnostics.is_empty());
    assert_eq!(second_result.cache_stats.hits, 1);
    assert_eq!(second_result.cache_stats.verified_reuse, 1);
    assert_eq!(second_result.cache_stats.recomputes, 0);
    assert_eq!(second_result.cache_stats.writes, 0);

    fs::remove_dir_all(cache_root).ok();
}

#[test]
fn go_syntax_layer_cache_corrupt() {
    let cache_root = unique_cache_root();
    let cache = crate::go::test_cache::FsAnalysisCache::new(cache_root.join("analysis"), true);
    let source = "package main\nfunc main() {}\n";
    let mut first = db_with_go_file("main.go", source);

    let first_result =
        analyze_with_plan_options_and_cache_stats(&mut first, &cache, "config", "rule", "", false);
    assert!(first_result.diagnostics.is_empty());

    let manifest = first_layer_file(&cache_root, "manifests");
    fs::write(manifest, "{not-json").expect("corrupt manifest");

    let mut second = db_with_go_file("main.go", source);
    let second_result = analyze_with_plan_options_and_cache_stats(
        &mut second,
        &cache,
        "config",
        "changed-rule",
        "",
        false,
    );

    assert!(second_result.diagnostics.is_empty());
    assert_eq!(second_result.cache_stats.invalid_evicted_reads, 1);
    assert_eq!(second_result.cache_stats.recomputes, 1);
    assert_eq!(second_result.cache_stats.writes, 1);

    fs::remove_dir_all(cache_root).ok();
}

#[test]
fn go_syntax_layer_cache_output_digest_mismatch_recomputes() {
    let cache_root = unique_cache_root();
    let cache = crate::go::test_cache::FsAnalysisCache::new(cache_root.join("analysis"), true);
    let source = "package main\nfunc main() {}\n";
    let mut first = db_with_go_file("main.go", source);

    let first_result =
        analyze_with_plan_options_and_cache_stats(&mut first, &cache, "config", "rule", "", false);
    assert!(first_result.diagnostics.is_empty());
    corrupt_first_manifest_output_digest(&cache_root);

    let mut second = db_with_go_file("main.go", source);
    let second_result =
        analyze_with_plan_options_and_cache_stats(&mut second, &cache, "config", "rule", "", false);

    assert!(second_result.diagnostics.is_empty());
    assert_eq!(second_result.cache_stats.invalid_evicted_reads, 1);
    assert_eq!(second_result.cache_stats.recomputes, 1);
    assert_eq!(second_result.cache_stats.writes, 1);

    fs::remove_dir_all(cache_root).ok();
}

#[test]
fn go_syntax_layer_cache_disabled_bypass() {
    let cache_root = unique_cache_root();
    let cache = crate::go::test_cache::FsAnalysisCache::new(cache_root.join("analysis"), false);
    let mut db = LocalFactDb::new();
    db.add_file(
        PathBuf::from("first.go"),
        "first.go".to_string(),
        "package first\nfunc One() {}\n".to_string(),
    );
    db.add_file(
        PathBuf::from("second.go"),
        "second.go".to_string(),
        "package second\nfunc Two() {}\n".to_string(),
    );

    let result =
        analyze_with_plan_options_and_cache_stats(&mut db, &cache, "config", "rule", "", false);

    assert!(result.diagnostics.is_empty());
    assert_eq!(result.cache_stats.bypasses_disabled, 1);
    assert_eq!(result.cache_stats.recomputes, 1);
    assert_eq!(result.cache_stats.writes, 0);
    assert!(result.output_digest.is_some());
    assert!(!cache_root.join("layers").exists());
}

#[test]
fn go_syntax_output_identity_matches_disabled_cold_and_warm_runs() {
    let source = "package main\nfunc main() {}\n";
    let disabled_cache = crate::go::test_cache::FsAnalysisCache::new("", false);
    let mut disabled_db = db_with_go_file("main.go", source);
    let disabled = analyze_with_plan_options_and_cache_stats(
        &mut disabled_db,
        &disabled_cache,
        "config",
        "rule",
        "",
        false,
    );

    let cache_root = unique_cache_root();
    let cache = crate::go::test_cache::FsAnalysisCache::new(cache_root.join("analysis"), true);
    let mut cold_db = db_with_go_file("main.go", source);
    let cold = analyze_with_plan_options_and_cache_stats(
        &mut cold_db,
        &cache,
        "config",
        "rule",
        "",
        false,
    );
    let mut warm_db = db_with_go_file("main.go", source);
    let warm = analyze_with_plan_options_and_cache_stats(
        &mut warm_db,
        &cache,
        "config",
        "rule",
        "",
        false,
    );

    assert!(disabled.output_digest.is_some());
    assert_eq!(disabled.output_digest, cold.output_digest);
    assert_eq!(disabled.output_digest, warm.output_digest);

    fs::remove_dir_all(cache_root).ok();
}

#[test]
fn go_syntax_layer_write_failure_withholds_native_output_identity() {
    let temp = tempfile::tempdir().expect("tempdir");
    let blocked_root = temp.path().join("blocked-cache-root");
    fs::write(&blocked_root, "not a directory").expect("write blocker");
    let cache = crate::go::test_cache::FsAnalysisCache::new(blocked_root.join("analysis"), true);
    let mut db = db_with_go_file("main.go", "package main\nfunc main() {}\n");

    let result =
        analyze_with_plan_options_and_cache_stats(&mut db, &cache, "config", "rule", "", false);

    assert!(result.output_digest.is_none());
    assert!(result.diagnostics.iter().any(|diagnostic| {
        diagnostic.rule_id == "internal/cache" && diagnostic.message.contains("cache write failed")
    }));
}

#[test]
fn go_syntax_layer_cache_payload_excludes_source_and_temp_paths() {
    let cache_root = unique_cache_root();
    let cache = crate::go::test_cache::FsAnalysisCache::new(cache_root.join("analysis"), true);
    let source = "package main\nfunc main() {}\n";
    let mut db = db_with_go_file("main.go", source);

    let result =
        analyze_with_plan_options_and_cache_stats(&mut db, &cache, "config", "rule", "", false);

    assert!(result.diagnostics.is_empty());
    assert!(
        !collect_files(&cache_root.join("layers")).is_empty(),
        "expected syntax layer cache files"
    );
    let cache_text = layer_cache_text(&cache_root);
    assert!(!cache_text.contains("func main"));
    assert!(!cache_text.contains(cache_root.to_string_lossy().as_ref()));

    fs::remove_dir_all(cache_root).ok();
}

#[test]
fn go_parallel_analysis_matches_sequential() {
    let source = r#"
package payment

import "fmt"

func Authorize(user string) error {
if user == "" {
    return fmt.Errorf("blocked")
}
return nil
}
"#;
    let cache = crate::go::test_cache::FsAnalysisCache::new("", false);
    let mut sequential = db_with_go_file("payment.go", source);
    let mut parallel = db_with_go_file("payment.go", source);

    let sequential_diagnostics =
        analyze_with_options(&mut sequential, &cache, "config", "rule", false);
    let parallel_diagnostics = analyze_with_options(&mut parallel, &cache, "config", "rule", true);

    assert_eq!(parallel_diagnostics, sequential_diagnostics);
    assert_eq!(parallel.packages().len(), sequential.packages().len());
    assert_eq!(
        parallel
            .functions()
            .iter()
            .map(|fact| (&fact.name, fact.cyclomatic_complexity))
            .collect::<Vec<_>>(),
        sequential
            .functions()
            .iter()
            .map(|fact| (&fact.name, fact.cyclomatic_complexity))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        parallel
            .imports()
            .iter()
            .map(|fact| fact.path.as_str())
            .collect::<Vec<_>>(),
        sequential
            .imports()
            .iter()
            .map(|fact| fact.path.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        parallel
            .string_literals()
            .iter()
            .map(|fact| fact.value.as_str())
            .collect::<Vec<_>>(),
        sequential
            .string_literals()
            .iter()
            .map(|fact| fact.value.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn reports_tree_sitter_parse_errors_with_stable_range() {
    let mut db = db_with_go_file("payment.go", "package payment\n\nfunc Broken( {\n");

    let diagnostics = analyze(&mut db);

    assert_eq!(diagnostics.len(), 1);
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.rule_id, "parser/go");
    assert_eq!(diagnostic.file, "payment.go");
    assert_eq!(diagnostic.message, "Go parser reported a syntax error.");
    assert_ne!(diagnostic.range, TextRange::point(1, 1));
    assert!(diagnostic.range.start_line >= 3, "{:?}", diagnostic.range);
    assert!(
        (diagnostic.range.end_line, diagnostic.range.end_col)
            >= (diagnostic.range.start_line, diagnostic.range.start_col),
        "{:?}",
        diagnostic.range
    );
}

#[test]
fn continues_best_effort_package_extraction_after_parse_error() {
    let mut db = db_with_go_file("payment.go", "package payment\n\nfunc Broken( {\n");

    let diagnostics = analyze(&mut db);

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].rule_id, "parser/go");
    assert_eq!(db.packages().len(), 1);
    assert_eq!(db.packages()[0].name, "payment");
    assert_eq!(db.packages()[0].language, Language::Go);
}

#[test]
fn extracts_go_package_name_from_tree_sitter() {
    let mut db = db_with_go_file("payment.go", "package payment\n\nfunc Authorize() {}\n");

    let diagnostics = analyze(&mut db);

    assert!(diagnostics.is_empty());
    assert_eq!(db.packages().len(), 1);
    let package = &db.packages()[0];
    assert_eq!(package.name, "payment");
    assert_eq!(package.language, Language::Go);
    assert_eq!(package.span.diagnostic_range().start_line, 1);
    assert_eq!(package.span.diagnostic_range().start_col, 9);
}

#[test]
fn parser_foundation_covers_diagnostics_and_package_facts() {
    reports_tree_sitter_parse_errors_with_stable_range();
    continues_best_effort_package_extraction_after_parse_error();
    extracts_go_package_name_from_tree_sitter();
}

#[test]
fn extracts_go_imports_from_tree_sitter() {
    let mut db = db_with_go_file(
        "payment.go",
        r#"package payment

import "fmt"
import (
	alias "github.com/acme/aliased"
	. "github.com/acme/dot"
	_ "github.com/acme/sideeffect"
	"context"
)
"#,
    );

    let diagnostics = analyze(&mut db);

    assert!(diagnostics.is_empty());
    let imports = db
        .imports()
        .iter()
        .map(|import| (import.package.as_deref(), import.path.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        imports,
        vec![
            (None, "fmt"),
            (Some("alias"), "github.com/acme/aliased"),
            (Some("."), "github.com/acme/dot"),
            (Some("_"), "github.com/acme/sideeffect"),
            (None, "context"),
        ]
    );
    assert!(
        db.imports()
            .iter()
            .all(|import| import.language == Language::Go)
    );
    assert_eq!(db.imports()[0].span.diagnostic_range().start_line, 3);
    assert_eq!(db.imports()[1].span.diagnostic_range().start_line, 5);
}

#[test]
fn extracts_go_string_literals_for_sdk_rules() {
    let mut db = db_with_go_file(
        "payment.go",
        r#"package payment

const status = "blocked"

func Validate() {
	message := "invalid empty payment"
	token := `legacy-token`
	_, _, _ = status, message, token
}
"#,
    );

    let diagnostics = analyze(&mut db);

    assert!(diagnostics.is_empty());
    let literals = db
        .string_literals()
        .iter()
        .map(|literal| (literal.value.as_str(), literal.language))
        .collect::<Vec<_>>();
    assert_eq!(
        literals,
        vec![
            ("blocked", Language::Go),
            ("invalid empty payment", Language::Go),
            ("legacy-token", Language::Go),
        ]
    );
}

#[test]
fn does_not_duplicate_go_import_paths_as_string_literals() {
    let mut db = db_with_go_file(
        "payment.go",
        r#"package payment

import "net/http"

func Validate() {
	_ = "blocked"
}
"#,
    );

    let diagnostics = analyze(&mut db);

    assert!(diagnostics.is_empty());
    let literal_values = db
        .string_literals()
        .iter()
        .map(|literal| literal.value.as_str())
        .collect::<Vec<_>>();
    assert_eq!(literal_values, vec!["blocked"]);
    assert!(
        !literal_values.contains(&"net/http"),
        "import paths should remain ImportFact-only"
    );
}

#[test]
fn extracts_go_functions_methods_calls_and_complexity_from_tree_sitter() {
    let mut db = db_with_go_file(
        "payment.go",
        r#"package payment

import "log"

type Service struct{}

func Authorize(ok bool, svc *Service) error {
	if ok && svc.Enabled() {
		svc.Charge()
		svc.Charge()
		log.Printf("authorized")
	}
	for _, item := range []int{1, 2} {
		process(item)
	}
	switch ok {
	case true:
		audit()
	default:
		fallback()
	}
	return nil
}

func (svc *Service) Charge() {}

func (svc *Service) Enabled() bool {
	return true
}
"#,
    );

    let diagnostics = analyze(&mut db);

    assert!(diagnostics.is_empty());
    let functions = db.functions();
    assert_eq!(
        functions
            .iter()
            .map(|function| function.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Authorize", "Service.Charge", "Service.Enabled"]
    );

    let authorize = &functions[0];
    assert_eq!(authorize.language, Language::Go);
    assert!(authorize.is_exported);
    assert!(!authorize.is_test);
    assert_eq!(
        authorize.calls,
        vec![
            "audit".to_string(),
            "fallback".to_string(),
            "log.Printf".to_string(),
            "process".to_string(),
            "svc.Charge".to_string(),
            "svc.Enabled".to_string(),
        ]
    );
    assert_eq!(authorize.cyclomatic_complexity, 6);
    assert_eq!(authorize.span.diagnostic_range().start_line, 7);
    assert_eq!(authorize.span.diagnostic_range().end_line, 23);

    let method = &functions[1];
    assert_eq!(method.name, "Service.Charge");
    assert!(method.is_exported);
    assert_eq!(method.cyclomatic_complexity, 1);
}

#[test]
fn go_import_facts_feed_import_graph() {
    // ImportGraph lives in the polint facade; this asserts the Go import facts it consumes.
    let mut db = db_with_go_file(
        "payment.go",
        r#"package payment

import "github.com/acme/authz"

func Authorize() {}
"#,
    );

    let diagnostics = analyze(&mut db);

    assert!(diagnostics.is_empty());
    assert_eq!(db.imports().len(), 1);
    assert_eq!(db.imports()[0].path, "github.com/acme/authz");
    assert_eq!(db.path_for(db.imports()[0].file), "payment.go");
}

#[test]
fn extracts_go_test_functions_subtests_and_table_evidence() {
    let mut db = db_with_go_file(
        "payment_test.go",
        r#"package payment

import "testing"

func TestAuthorize(t *testing.T) {
	cases := []struct {
		name string
		allowed bool
		wantErr bool
	}{
		{name: "allowed", allowed: true, wantErr: false},
		{name: "denied", allowed: false, wantErr: true},
		{name: "invalid token", allowed: false, wantErr: true},
	}

	for _, tt := range cases {
		t.Run(tt.name, func(t *testing.T) {
			err := Authorize(tt.allowed)
			if tt.wantErr && err == nil {
				t.Fatalf("expected error for %s", tt.name)
			}
			if !tt.wantErr && err != nil {
				t.Errorf("unexpected denied error: %v", err)
			}
		})
	}
}
"#,
    );

    let diagnostics = analyze(&mut db);

    assert!(diagnostics.is_empty());
    assert_eq!(db.functions().len(), 1);
    assert!(db.functions()[0].is_test);
    assert_eq!(db.tests().len(), 1);
    let test = &db.tests()[0];
    assert_eq!(test.name, "TestAuthorize");
    assert_eq!(test.function, Some(db.functions()[0].id));
    assert_eq!(test.subtest_count, 1);
    assert_eq!(test.table_rows, 3);
    assert_eq!(test.assertion_count, 2);
    assert_eq!(
        test.evidence_terms,
        vec![
            "allowed".to_string(),
            "denied".to_string(),
            "err".to_string(),
            "error".to_string(),
            "invalid".to_string(),
            "nil".to_string(),
        ]
    );
}

#[test]
fn does_not_mark_non_test_go_functions_as_tests() {
    let mut helper_db = db_with_go_file(
        "payment_test.go",
        r#"package payment

func TestHelper() {}
"#,
    );
    let mut non_test_file_db = db_with_go_file(
        "payment.go",
        r#"package payment

import "testing"

func TestAuthorize(t *testing.T) {}
"#,
    );
    let mut extra_param_db = db_with_go_file(
        "payment_test.go",
        r#"package payment

import "testing"

func TestAuthorize(t *testing.T, extra int) {}
"#,
    );
    let mut near_match_db = db_with_go_file(
        "payment_test.go",
        r#"package payment

import "testing"

func TestAuthorize(t *testing.TB) {}
"#,
    );
    let mut unnamed_param_db = db_with_go_file(
        "payment_test.go",
        r#"package payment

import "testing"

func TestAuthorize(*testing.T) {}
"#,
    );

    let helper_diagnostics = analyze(&mut helper_db);
    let non_test_file_diagnostics = analyze(&mut non_test_file_db);
    let extra_param_diagnostics = analyze(&mut extra_param_db);
    let near_match_diagnostics = analyze(&mut near_match_db);
    let unnamed_param_diagnostics = analyze(&mut unnamed_param_db);

    assert!(helper_diagnostics.is_empty());
    assert!(non_test_file_diagnostics.is_empty());
    assert!(extra_param_diagnostics.is_empty());
    assert!(near_match_diagnostics.is_empty());
    assert!(unnamed_param_diagnostics.is_empty());
    assert_eq!(helper_db.tests().len(), 0);
    assert!(!helper_db.functions()[0].is_test);
    assert_eq!(non_test_file_db.tests().len(), 0);
    assert!(!non_test_file_db.functions()[0].is_test);
    assert_eq!(extra_param_db.tests().len(), 0);
    assert!(!extra_param_db.functions()[0].is_test);
    assert_eq!(near_match_db.tests().len(), 0);
    assert!(!near_match_db.functions()[0].is_test);
    assert_eq!(unnamed_param_db.tests().len(), 1);
    assert!(unnamed_param_db.functions()[0].is_test);
}

#[test]
fn counts_multiline_go_table_rows_without_nested_literals() {
    let mut db = db_with_go_file(
        "payment_test.go",
        r#"package payment

import "testing"

type charge struct {
	ID string
}

func TestAuthorize(t *testing.T) {
	cases := []struct {
		name string
		charges []charge
		wantErr bool
	}{
		{
			name: "allowed",
			charges: []charge{
				{ID: "one"},
				{ID: "two"},
			},
			wantErr: false,
		},
		{
			name: "denied",
			charges: nil,
			wantErr: true,
		},
	}

	for _, tt := range cases {
		t.Run(tt.name, func(t *testing.T) {
			if tt.wantErr {
				t.Fatal("denied")
			}
		})
	}
}
"#,
    );

    let diagnostics = analyze(&mut db);

    assert!(diagnostics.is_empty());
    assert_eq!(db.tests().len(), 1);
    assert_eq!(db.tests()[0].table_rows, 2);
}

#[test]
fn go_assertion_evidence_counts_common_failure_calls() {
    let mut db = db_with_go_file(
        "payment_test.go",
        r#"package payment

import (
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestAssertions(t *testing.T) {
	err := Authorize(false)
	if err == nil {
		t.Fatal("expected error")
	}
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	got := "denied"
	want := "allowed"
	if got != want {
		t.Errorf("got %s want %s", got, want)
	}
	require.NoError(t, err)
	assert.Equal(t, want, got)
}
"#,
    );

    let diagnostics = analyze(&mut db);

    assert!(diagnostics.is_empty());
    assert_eq!(db.tests().len(), 1);
    assert_eq!(db.tests()[0].assertion_count, 8);
    assert_eq!(
        db.tests()[0].evidence_terms,
        vec![
            "allowed".to_string(),
            "denied".to_string(),
            "err".to_string(),
            "error".to_string(),
            "nil".to_string(),
        ]
    );
}

#[test]
fn extracts_go_branch_obligations_from_control_flow() {
    let mut db = db_with_go_file(
        "payment.go",
        r#"package payment

func Authorize(ok bool, kind string, value any, items []int, ch <-chan int) error {
	if ok {
		approve()
	} else {
		deny()
	}

	switch kind {
	case "card", "bank":
		approve()
	default:
		deny()
	}

	switch typed := value.(type) {
	case string:
		_ = typed
	default:
		deny()
	}

	for i := 0; i < len(items); i++ {
		_ = i
	}
	for _, item := range items {
		_ = item
	}

	select {
	case <-ch:
		approve()
	default:
		deny()
	}

	return nil
}
"#,
    );

    let diagnostics = analyze(&mut db);

    assert!(diagnostics.is_empty());
    assert_eq!(db.functions().len(), 1);
    let function = db.functions()[0].id;
    assert!(
        db.branches()
            .iter()
            .all(|branch| branch.function == Some(function))
    );

    let branches = db
        .branches()
        .iter()
        .map(|branch| {
            (
                branch.edge_label.as_str(),
                branch.condition_text.as_str(),
                branch.decision_span.start_line,
            )
        })
        .collect::<Vec<_>>();

    assert!(
        branches.contains(&("true", "ok", 4)),
        "missing if true branch: {branches:?}"
    );
    assert!(
        branches.contains(&("false", "ok", 4)),
        "missing if false branch: {branches:?}"
    );
    assert!(
        branches.contains(&("switch", "kind", 10)),
        "missing expression switch branch: {branches:?}"
    );
    assert!(
        branches.contains(&("case", r#"case "card", "bank":"#, 11)),
        "missing expression case branch: {branches:?}"
    );
    assert!(
        branches.contains(&("default", "default", 13)),
        "missing expression default branch: {branches:?}"
    );
    assert!(
        branches.contains(&("switch", "typed := value.(type)", 17)),
        "missing type switch branch: {branches:?}"
    );
    assert!(
        branches.contains(&("case", "case string:", 18)),
        "missing type case branch: {branches:?}"
    );
    assert!(
        branches.contains(&("loop", "for i := 0; i < len(items); i++", 24)),
        "missing ordinary for branch: {branches:?}"
    );
    assert!(
        branches.contains(&("range", "_, item := range items", 27)),
        "missing range branch: {branches:?}"
    );
    assert!(
        branches.contains(&("select", "select", 31)),
        "missing select branch: {branches:?}"
    );
    assert!(
        branches.contains(&("case", "case <-ch:", 32)),
        "missing select communication case branch: {branches:?}"
    );
}

#[test]
fn branch_spans_come_from_tree_sitter_nodes() {
    let mut db = db_with_go_file(
        "payment.go",
        r#"package payment

func Authorize(ok bool, kind string) {
	if ok {
		approve()
	}
	switch kind {
	case "card":
		approve()
	default:
		deny()
	}
}
"#,
    );

    let diagnostics = analyze(&mut db);

    assert!(diagnostics.is_empty());
    let branches = db.branches();
    let if_true = branches
        .iter()
        .find(|branch| branch.edge_label == "true" && branch.condition_text == "ok")
        .expect("if true branch exists");
    assert_eq!(if_true.decision_span.start_line, 4);
    assert_eq!(if_true.decision_span.start_col, 5);
    assert_eq!(if_true.decision_span.end_col, 7);
    assert_ne!(if_true.decision_span.start_col, 1);

    let switch = branches
        .iter()
        .find(|branch| branch.edge_label == "switch" && branch.condition_text == "kind")
        .expect("switch branch exists");
    assert_eq!(switch.decision_span.start_line, 7);
    assert_eq!(switch.decision_span.start_col, 9);
    assert_eq!(switch.decision_span.end_col, 13);

    let case = branches
        .iter()
        .find(|branch| branch.edge_label == "case")
        .expect("case branch exists");
    assert_eq!(case.decision_span.start_line, 8);
    assert_eq!(case.decision_span.start_col, 2);
    assert_eq!(case.decision_span.end_col, 14);
}

#[test]
fn marks_basic_go_error_paths_heuristically() {
    let mut db = db_with_go_file(
        "payment.go",
        r#"package payment

import "errors"

var ErrDenied error

func Authorize(err error, target error, ok bool, shouldReject bool) error {
	if err != nil {
		return err
	}
	if err == nil {
		return ErrDenied
	}
	if errors.Is(err, target) {
		return err
	}
	if ok {
		return nil
	}
	if shouldReject {
		return ErrDenied
	}
	return nil
}
"#,
    );

    let diagnostics = analyze(&mut db);

    assert!(diagnostics.is_empty());
    assert!(branch_for(&db, "err != nil", "true").is_error_path);
    assert!(branch_for(&db, "err == nil", "true").is_error_path);
    assert!(branch_for(&db, "errors.Is(err, target)", "true").is_error_path);
    assert!(branch_for(&db, "shouldReject", "true").is_error_path);
    assert!(!branch_for(&db, "ok", "true").is_error_path);
}

#[test]
fn classifies_if_error_paths_by_edge_and_body() {
    let mut db = db_with_go_file(
        "payment.go",
        r#"package payment

var ErrDenied error

func Authorize(err error, ok bool) error {
	if err != nil {
		return err
	}
	if ok {
		return nil
	} else {
		return ErrDenied
	}
	if err == nil {
		return nil
	}
	return err
}
"#,
    );

    let diagnostics = analyze(&mut db);

    assert!(diagnostics.is_empty());
    assert!(branch_for(&db, "err != nil", "true").is_error_path);
    assert!(!branch_for(&db, "err != nil", "false").is_error_path);
    assert!(!branch_for(&db, "ok", "true").is_error_path);
    assert!(branch_for(&db, "ok", "false").is_error_path);
    assert!(!branch_for(&db, "err == nil", "true").is_error_path);
    assert!(branch_for(&db, "err == nil", "false").is_error_path);
}

#[test]
fn marks_case_and_loop_error_returns_without_marking_whole_switch() {
    let mut db = db_with_go_file(
        "payment.go",
        r#"package payment

var ErrInvalid error

func Authorize(kind string, items []int) error {
	switch kind {
	case "invalid":
		return ErrInvalid
	default:
		return nil
	}
	for i := 0; i < len(items); i++ {
		return ErrInvalid
	}
	return nil
}
"#,
    );

    let diagnostics = analyze(&mut db);

    assert!(diagnostics.is_empty());
    assert!(!branch_for(&db, "kind", "switch").is_error_path);
    assert!(branch_for(&db, r#"case "invalid":"#, "case").is_error_path);
    assert!(!branch_for(&db, "default", "default").is_error_path);
    assert!(branch_for(&db, "for i := 0; i < len(items); i++", "loop").is_error_path);
}

#[test]
fn ordinary_for_ignores_nested_range_clause() {
    let mut db = db_with_go_file(
        "payment.go",
        r#"package payment

func Process(items []int) {
	for i := 0; i < 1; i++ {
		for _, item := range items {
			_ = item
		}
	}
}
"#,
    );

    let diagnostics = analyze(&mut db);

    assert!(diagnostics.is_empty());
    let loop_branches = db
        .branches()
        .iter()
        .filter(|branch| branch.edge_label == "loop")
        .collect::<Vec<_>>();
    let range_branches = db
        .branches()
        .iter()
        .filter(|branch| branch.edge_label == "range")
        .collect::<Vec<_>>();
    assert_eq!(loop_branches.len(), 1, "{loop_branches:?}");
    assert_eq!(range_branches.len(), 1, "{range_branches:?}");
    assert_eq!(loop_branches[0].condition_text, "for i := 0; i < 1; i++");
    assert_eq!(range_branches[0].condition_text, "_, item := range items");
    assert!(range_branches[0].decision_span.start_line > loop_branches[0].decision_span.start_line);
}

#[test]
fn case_headers_keep_colons_inside_literals_and_short_declarations() {
    let mut db = db_with_go_file(
        "payment.go",
        r#"package payment

func Process(kind string, ch <-chan string) {
	switch kind {
	case "bad:token":
		deny()
	case map[string]int{"a:b": 1}["a:b"]:
		deny()
	default:
		allow()
	}

	select {
	case msg := <-ch:
		_ = msg
	default:
		return
	}
}
"#,
    );

    let diagnostics = analyze(&mut db);

    assert!(diagnostics.is_empty());
    let case_headers = db
        .branches()
        .iter()
        .filter(|branch| branch.edge_label == "case")
        .map(|branch| branch.condition_text.as_str())
        .collect::<Vec<_>>();
    assert!(
        case_headers.contains(&r#"case "bad:token":"#),
        "{case_headers:?}"
    );
    assert!(
        case_headers.contains(&r#"case map[string]int{"a:b": 1}["a:b"]:"#),
        "{case_headers:?}"
    );
    assert!(
        case_headers.contains(&"case msg := <-ch:"),
        "{case_headers:?}"
    );
}

#[test]
fn branch_fingerprints_are_stable_for_same_source() {
    let source = r#"package payment

func Authorize(err error, ok bool) error {
	if err != nil {
		return err
	}
	if ok {
		return nil
	}
	return nil
}
"#;
    let first = analyzed_go_file("payment.go", source);
    let second = analyzed_go_file("payment.go", source);

    let first_fingerprints = branch_fingerprints(&first);
    let second_fingerprints = branch_fingerprints(&second);

    assert_eq!(first_fingerprints, second_fingerprints);
    assert!(
        first_fingerprints
            .iter()
            .all(|fingerprint| !fingerprint.is_empty())
    );
}

#[test]
fn branch_fingerprints_do_not_use_branch_ids() {
    let source = r#"package payment

func Authorize(err error) error {
	if err != nil {
		return err
	}
	return nil
}
"#;
    let baseline = analyzed_go_file("payment.go", source);
    let baseline_branch = branch_for(&baseline, "err != nil", "true");

    let mut shifted = LocalFactDb::new();
    shifted.add_file(
        PathBuf::from("other.go"),
        "other.go".to_string(),
        r#"package payment

func Other(ok bool) {
	if ok {
		return
	}
}
"#
        .to_string(),
    );
    shifted.add_file(
        PathBuf::from("payment.go"),
        "payment.go".to_string(),
        source.to_string(),
    );

    let diagnostics = analyze(&mut shifted);

    assert!(diagnostics.is_empty());
    let shifted_branch = shifted
        .branches()
        .iter()
        .find(|branch| {
            shifted.path_for(branch.file) == "payment.go"
                && branch.condition_text == "err != nil"
                && branch.edge_label == "true"
        })
        .expect("shifted branch exists");

    assert_ne!(baseline_branch.id, shifted_branch.id);
    assert_eq!(
        baseline_branch.stable_fingerprint,
        shifted_branch.stable_fingerprint
    );
}

fn analyzed_go_file(relative_path: &str, source: &str) -> LocalFactDb {
    let mut db = db_with_go_file(relative_path, source);
    let diagnostics = analyze(&mut db);
    assert!(diagnostics.is_empty());
    db
}

fn branch_for<'db>(
    db: &'db LocalFactDb,
    condition_text: &str,
    edge_label: &str,
) -> &'db BranchObligation {
    db.branches()
        .iter()
        .find(|branch| branch.condition_text == condition_text && branch.edge_label == edge_label)
        .expect("branch exists")
}

fn branch_fingerprints(db: &LocalFactDb) -> Vec<String> {
    db.branches()
        .iter()
        .map(|branch| branch.stable_fingerprint.clone())
        .collect()
}

#[test]
fn extracts_go_type_declaration_facts() {
    let db = analyzed_go_file(
        "payment.go",
        r#"package payment

import "time"

type Plain struct {
	ID   int
	When time.Time `json:"when"`
}

type Handler interface {
	Do() error
}

type Alias = Plain
type Named time.Time

type (
	Grouped struct {
		A int
	}
	Grouped2 struct {
		B int
	}
)

func NewPlain() (*Plain, error) {
	type local struct {
		inner int
	}
	var rows []struct {
		Name string
	}
	_ = rows
	return nil, nil
}
"#,
    );

    let named: Vec<&crate::analysis_api::GoTypeDeclFact> = db
        .go_types()
        .iter()
        .filter(|fact| fact.name.is_some())
        .collect();
    let by_name = |name: &str| {
        named
            .iter()
            .copied()
            .find(|fact| fact.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("missing named fact {name}"))
    };

    let plain = by_name("Plain");
    assert!(matches!(
        plain.kind,
        crate::analysis_api::GoTypeDeclKind::Struct
    ));
    assert!(plain.is_top_level && !plain.is_grouped && !plain.is_alias);
    assert!(!plain.has_type_parameters);
    assert_eq!(plain.direct_name, None);
    let (body_start, body_end) = plain.body_range.expect("struct body range");
    let source = db.file(plain.file).unwrap().source.as_ref();
    assert!(source[body_start as usize..body_end as usize].contains("ID   int"));
    assert_eq!(
        &source[plain.declaration_start_byte as usize..plain.declaration_start_byte as usize + 4],
        "type"
    );
    assert_eq!(plain.span.start_line, 5);
    assert_eq!(plain.span.start_col, 6);

    let handler = by_name("Handler");
    assert!(matches!(
        handler.kind,
        crate::analysis_api::GoTypeDeclKind::Interface
    ));
    let (interface_start, interface_end) = handler.body_range.expect("interface body range");
    assert!(source[interface_start as usize..interface_end as usize].contains("Do() error"));

    let alias = by_name("Alias");
    assert!(alias.is_alias);
    assert!(matches!(
        alias.kind,
        crate::analysis_api::GoTypeDeclKind::Named
    ));
    assert_eq!(alias.direct_name.as_deref(), Some("Plain"));
    assert_eq!(alias.body_range, None);

    assert_eq!(by_name("Named").direct_name.as_deref(), Some("Time"));

    let grouped = by_name("Grouped");
    assert!(grouped.is_grouped && grouped.is_top_level);
    let grouped2 = by_name("Grouped2");
    assert!(grouped2.is_grouped && grouped2.is_top_level);
    assert!(grouped2.body_range.is_some());

    let local = by_name("local");
    assert!(!local.is_top_level);
    assert!(matches!(
        local.kind,
        crate::analysis_api::GoTypeDeclKind::Struct
    ));

    let anonymous: Vec<&crate::analysis_api::GoTypeDeclFact> = db
        .go_types()
        .iter()
        .filter(|fact| fact.name.is_none())
        .collect();
    // Grouped specs, the function-local `local` spec, and the composite-literal
    // element struct are all reached through the anonymous path (their lines do not
    // start with `type NAME struct {`), matching the historical pack regex behavior.
    assert_eq!(anonymous.len(), 4, "anonymous rows: {anonymous:?}");
    for fact in &anonymous {
        assert!(!fact.is_top_level);
        assert_eq!(
            &source[fact.declaration_start_byte as usize..fact.declaration_start_byte as usize + 6],
            "struct"
        );
    }
    let var_row = anonymous
        .iter()
        .copied()
        .rev()
        .find(|fact| {
            let (start, end) = fact.body_range.expect("anonymous body");
            source[start as usize..end as usize].contains("Name string")
        })
        .expect("var struct row");
    assert!(var_row.span.start_byte > local.span.start_byte);
}

#[test]
fn go_type_facts_round_trip_through_file_cache_facts() {
    let db = analyzed_go_file(
        "payment.go",
        r#"package payment

type Plain struct {
	ID int
}
"#,
    );
    let file = db.files()[0].id;
    let facts = db.facts_for_file(file);
    assert_eq!(facts.go_types.len(), 1);
    assert_eq!(facts.go_types[0].name.as_deref(), Some("Plain"));

    let bytes = serde_json::to_vec(&facts).expect("serialize facts");
    let parsed: crate::analysis_api::CachedFileFacts =
        serde_json::from_slice(&bytes).expect("deserialize facts");
    assert_eq!(parsed.go_types.len(), 1);
    assert_eq!(parsed.go_types[0].body_range, facts.go_types[0].body_range);
}

const GO_1_26_NEW_EXPR_SOURCES: &[(&str, &str)] = &[
    (
        "new_int.go",
        "package models\n\nfunc Example() { _ = new(int) }\n",
    ),
    (
        "new_literal.go",
        "package models\n\nfunc Example() { _ = new(1) }\n",
    ),
    (
        "new_qualified_type.go",
        "package models\n\nfunc Example() { _ = new(types.Cost) }\n",
    ),
    (
        "new_call.go",
        "package models\n\nfunc Example() { _ = new(types.NewCost(0.35)) }\n",
    ),
];

#[test]
fn go_1_26_new_expr_forms_should_not_emit_parser_diagnostics() {
    for (path, source) in GO_1_26_NEW_EXPR_SOURCES {
        let mut db = db_with_go_file(path, source);
        let diagnostics = analyze(&mut db);
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.rule_id != "parser/go"),
            "{path} should not emit parser/go: {diagnostics:?}"
        );
        assert_eq!(db.packages()[0].name, "models");
        assert_eq!(db.functions()[0].name, "Example");
    }
}

#[test]
fn go_1_26_new_expr_should_extract_same_syntax_facts_as_new_type() {
    let old_style = r#"package payment

import "fmt"

func Authorize() {
	x := new(int)
	_ = fmt.Sprintf("blocked")
	_ = x
}
"#;
    let new_style = r#"package payment

import "fmt"

func Authorize() {
	x := new(1)
	_ = fmt.Sprintf("blocked")
	_ = x
}
"#;
    let mut old_db = db_with_go_file("payment.go", old_style);
    let mut new_db = db_with_go_file("payment.go", new_style);
    assert!(analyze(&mut old_db).is_empty());
    assert!(analyze(&mut new_db).is_empty());

    let names = |db: &LocalFactDb| {
        db.functions()
            .iter()
            .map(|fact| fact.name.clone())
            .collect::<Vec<_>>()
    };
    let imports = |db: &LocalFactDb| {
        db.imports()
            .iter()
            .map(|fact| fact.path.clone())
            .collect::<Vec<_>>()
    };
    let strings = |db: &LocalFactDb| {
        db.string_literals()
            .iter()
            .map(|fact| fact.value.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&new_db), names(&old_db));
    assert_eq!(imports(&new_db), imports(&old_db));
    assert_eq!(strings(&new_db), strings(&old_db));
}

#[test]
fn oaiz_units_new_expr_struct_literal_should_parse_and_extract_functions() {
    let source = r#"package models

import "example.com/oaiz/types"

type Units struct {
	InputCostPerMillionTokens *types.Cost
}

func Example() Units {
	return Units{
		InputCostPerMillionTokens: new(types.NewCost(2.50)),
	}
}

func Budget() *string {
	return new("budget exceeded")
}

func Ones() *int {
	return new(1)
}
"#;
    let mut db = db_with_go_file("units.go", source);
    let diagnostics = analyze(&mut db);
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.rule_id != "parser/go"),
        "oaiz Units new(expr) pattern should not emit parser/go: {diagnostics:?}"
    );
    let function_names: Vec<_> = db
        .functions()
        .iter()
        .map(|fact| fact.name.as_str())
        .collect();
    assert_eq!(function_names, ["Example", "Budget", "Ones"]);
    assert_eq!(db.imports()[0].path, "example.com/oaiz/types");
    assert!(
        db.string_literals()
            .iter()
            .any(|literal| literal.value == "budget exceeded")
    );
}
