#[cfg(test)]
mod symbol_graph_go_setup {
    fn go_settings(root: &Path) -> BTreeMap<String, toml::Value> {
        let Ok(raw) = std::fs::read_to_string(root.join(".polint.toml")) else {
            return BTreeMap::new();
        };
        toml::from_str::<toml::Table>(&raw)
            .ok()
            .and_then(|table| table.get("languages")?.get("go")?.as_table().cloned())
            .unwrap_or_default()
            .into_iter()
            .collect()
    }

    fn options(root: &Path, request: SymbolGraphRequest) -> GoSymbolOptions {
        GoSymbolOptions {
            root: root.to_path_buf(),
            settings: go_settings(root),
            request,
            reference_files: None,
        }
    }

    use super::*;
    use crate::go::lifecycle::{self, GoAnalysisConfig};
    use crate::go::local_db::LocalFactDb;
    use crate::analysis_neutral::symbol_graph::{model::SymbolGraphBuilder, SymbolGraphRequest};
    use crate::analysis_api::{SymbolPrecision, SymbolResolutionStatus};
    use crate::analysis_neutral::symbol_graph::SymbolCapabilityStatus;
    use crate::internal_core::StableKeyInterner;
    use std::collections::BTreeMap;
    use std::path::Path;

    fn add_file(db: &mut LocalFactDb, root: &Path, relative_path: &str, source: &str) {
        let path = root.join(relative_path);
        std::fs::create_dir_all(path.parent().expect("fixture has parent")).expect("mkdirs");
        std::fs::write(&path, source).expect("write fixture");
        db.add_file(path, relative_path.to_string(), source.to_string());
    }

    fn add_go_file(db: &mut LocalFactDb, root: &Path, relative_path: &str, source: &str) {
        add_file(db, root, relative_path, source);
    }




    #[test]
    fn embedded_go_sidecar_sources_match_workspace_sources() {
        let workspace_sidecar =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/go-sidecar/polint-go-symbols");
        for (relative_path, embedded) in EMBEDDED_GO_SIDECAR_FILES {
            let workspace = std::fs::read_to_string(workspace_sidecar.join(relative_path))
                .unwrap_or_else(|error| panic!("read workspace sidecar {relative_path}: {error}"));
            assert_eq!(
                workspace, *embedded,
                "embedded sidecar drifted at {relative_path}"
            );
        }
    }

    #[test]
    fn embedded_go_sidecar_keeps_go_1_25_minimum() {
        let go_mod = EMBEDDED_GO_SIDECAR_FILES
            .iter()
            .find_map(|(relative_path, contents)| (*relative_path == "go.mod").then_some(*contents))
            .expect("embedded go.mod exists");

        assert!(
            go_mod.lines().any(|line| line == "go 1.25.0"),
            "embedded sidecar should keep Go 1.25 as its minimum supported toolchain: {go_mod:?}"
        );
        assert!(
            go_mod
                .lines()
                .map(str::trim)
                .any(|line| line == "golang.org/x/tools v0.49.0"),
            "embedded sidecar should stay on the newest Go 1.25-compatible x/tools line: {go_mod:?}"
        );
    }

    #[test]
    fn go_symbol_config_parses_string_and_array_settings() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join(".polint.toml"),
            r#"
[languages.go]
module_roots = ["cmd/service", "libs/platform"]
package_patterns = ["./cmd/...", "./pkg/..."]
build_tags = "enterprise,polint"
include_tests = false
"#,
        )
        .expect("write config");

        let files = Vec::new();
        let config = GoAnalysisConfig::from_settings_files(
            temp.path(),
            &go_settings(temp.path()),
            &files,
        )
        .unwrap();

        assert_eq!(
            config,
            GoAnalysisConfig {
                module_roots: vec!["cmd/service".to_string(), "libs/platform".to_string(),],
                package_patterns: vec!["./cmd/...".to_string(), "./pkg/...".to_string()],
                build_tags: vec!["enterprise".to_string(), "polint".to_string()],
                include_tests: false,
                offline: false,
                semantic_timeout_ms: None,
                emit_rta_edges: false,
                // `package_patterns` is configured, so the symbol sidecar keeps
                // the configured patterns, rooted at each module root.
                symbol_rooted_patterns: vec![
                    "./cmd/service/cmd/...".to_string(),
                    "./cmd/service/pkg/...".to_string(),
                    "./libs/platform/cmd/...".to_string(),
                    "./libs/platform/pkg/...".to_string(),
                ],
                scope_files: Vec::new(),
                files_without_module_root: Vec::new(),
            }
        );
    }

    #[test]
    fn go_symbol_config_infers_nearest_module_roots_for_monorepos() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(temp.path().join("services/payments")).expect("mkdirs");
        std::fs::create_dir_all(temp.path().join("libs/money")).expect("mkdirs");
        std::fs::write(
            temp.path().join("services/payments/go.mod"),
            "module example.com/payments\n\ngo 1.24\n",
        )
        .expect("write service go.mod");
        std::fs::write(
            temp.path().join("libs/money/go.mod"),
            "module example.com/money\n\ngo 1.24\n",
        )
        .expect("write lib go.mod");
        let mut db = LocalFactDb::new();
        add_go_file(
            &mut db,
            temp.path(),
            "services/payments/main.go",
            "package payments\n",
        );
        add_go_file(
            &mut db,
            temp.path(),
            "libs/money/money.go",
            "package money\n",
        );
        let files = lifecycle::go_files(&db);

        let config = GoAnalysisConfig::from_settings_files(
            temp.path(),
            &go_settings(temp.path()),
            &files,
        )
        .unwrap();

        assert_eq!(
            config.module_roots,
            vec!["libs/money".to_string(), "services/payments".to_string()]
        );
        assert!(config.files_without_module_root.is_empty());
    }

    #[test]
    fn go_symbol_config_treats_files_outside_configured_roots_as_uncovered() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join(".polint.toml"),
            r#"
[languages.go]
module_roots = ["services/payments"]
"#,
        )
        .expect("write config");
        let mut db = LocalFactDb::new();
        add_go_file(
            &mut db,
            temp.path(),
            "services/payments/main.go",
            "package payments\n",
        );
        add_go_file(
            &mut db,
            temp.path(),
            "services/ledger/main.go",
            "package ledger\n",
        );
        let files = lifecycle::go_files(&db);

        let config = GoAnalysisConfig::from_settings_files(
            temp.path(),
            &go_settings(temp.path()),
            &files,
        )
        .unwrap();

        assert_eq!(config.module_roots, vec!["services/payments".to_string()]);
        assert_eq!(
            config.files_without_module_root,
            vec!["services/ledger/main.go".to_string()]
        );
    }

    #[test]
    fn missing_go_mod_reports_setup_missing_for_requested_capabilities() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut db = LocalFactDb::new();
        add_go_file(&mut db, temp.path(), "main.go", "package main\n");
        let mut builder = SymbolGraphBuilder::new(StableKeyInterner::default());

        let output = derive_go_symbols(
            &mut builder,
            &db,
            &options(temp.path(), SymbolGraphRequest::new(true, true)),
        );
        let graph = builder.finish();

        assert_eq!(
            output
                .capability_support
                .iter()
                .map(|entry| (entry.capability.as_str(), entry.status))
                .collect::<Vec<_>>(),
            vec![
                ("references", SymbolCapabilityStatus::SetupMissing),
                ("symbols", SymbolCapabilityStatus::SetupMissing),
            ]
        );
        assert!(graph.references.iter().all(|reference| {
            reference.status == SymbolResolutionStatus::SetupMissing
                && reference.precision == SymbolPrecision::SetupMissing
        }));
    }

    #[test]
    fn sidecar_command_failure_reports_setup_missing() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join("go.mod"),
            "module example.com/app\n\ngo 1.24.0\n",
        )
        .expect("write go.mod");
        let mut db = LocalFactDb::new();
        add_go_file(&mut db, temp.path(), "main.go", "package main\n");
        let mut builder = SymbolGraphBuilder::new(StableKeyInterner::default());

        let output = derive_go_symbols_with_runner(
            &mut builder,
            &db,
            &options(temp.path(), SymbolGraphRequest::new(true, true)),
            |_config| {
                Err(GoSidecarFailure::CommandFailed(
                    "sidecar failed".to_string(),
                ))
            },
        );

        assert!(
            output.capability_support.iter().all(|entry| {
                entry.status == SymbolCapabilityStatus::SetupMissing
                    && entry
                        .reason
                        .as_deref()
                        .is_some_and(|reason| reason.contains("sidecar failed"))
            }),
            "{:#?}",
            output.capability_support
        );
    }

    #[test]
    fn invalid_sidecar_json_reports_setup_missing() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join("go.mod"),
            "module example.com/app\n\ngo 1.24.0\n",
        )
        .expect("write go.mod");
        let mut db = LocalFactDb::new();
        add_go_file(&mut db, temp.path(), "main.go", "package main\n");
        let mut builder = SymbolGraphBuilder::new(StableKeyInterner::default());

        let output = derive_go_symbols_with_runner(
            &mut builder,
            &db,
            &options(temp.path(), SymbolGraphRequest::new(true, true)),
            |_config| Ok(br#"{"schema":"wrong"}"#.to_vec()),
        );

        assert!(
            output.capability_support.iter().all(|entry| {
                entry.status == SymbolCapabilityStatus::SetupMissing
                    && entry.reason.as_deref().is_some_and(|reason| {
                        reason.contains("invalid Go symbol sidecar JSON")
                            || reason.contains("unsupported Go symbol sidecar schema")
                    })
            }),
            "{:#?}",
            output.capability_support
        );
    }

    #[test]
    fn sidecar_null_sequence_fields_parse_as_empty_vectors() {
        let output = parse_sidecar_output(
            br#"{
  "schema":"polint-go-symbols-semantic-1",
  "go_version":"go1.24.13",
  "packages":[{"files":null}],
  "symbols":null,
  "definitions":null,
  "references":null,
  "scopes":null,
  "imports":null,
  "exports":null,
  "resolution_steps":null,
  "errors":null
}"#,
        )
        .expect("sidecar output parses");

        assert_eq!(output.packages.len(), 1);
        assert!(output.packages[0].files.is_empty());
        assert!(output.symbols.is_empty());
        assert!(output.definitions.is_empty());
        assert!(output.references.is_empty());
        assert!(output.scopes.is_empty());
        assert!(output.imports.is_empty());
        assert!(output.exports.is_empty());
        assert!(output.resolution_steps.is_empty());
        assert!(output.errors.is_empty());
    }

    #[test]
    fn repo_escaping_sidecar_file_path_is_skipped_instead_of_failing_the_run() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join("go.mod"),
            "module example.com/app\n\ngo 1.24.0\n",
        )
        .expect("write go.mod");
        let mut db = LocalFactDb::new();
        add_go_file(&mut db, temp.path(), "main.go", "package main\n");
        let mut builder = SymbolGraphBuilder::new(StableKeyInterner::default());

        let output = derive_go_symbols_with_runner(
            &mut builder,
            &db,
            &options(temp.path(), SymbolGraphRequest::new(true, true)),
            |_config| {
                Ok(
                    br#"{
  "schema":"polint-go-symbols-semantic-1",
  "go_version":"go1.26.2",
  "packages":[],
  "symbols":[{
    "key":"bad",
    "package_id":"example.com/app",
    "package_path":"example.com/app",
    "test_variant":"regular",
    "file":"../outside.go",
    "name":"Bad",
    "qualified_name":"Bad",
    "namespace":"value",
    "kind":"function",
    "span":{"start_byte":0,"end_byte":3,"start_line":1,"start_column":1,"end_line":1,"end_column":4},
    "exported":true
  }],
  "definitions":[],
  "references":[]
}"#
                    .to_vec(),
                )
            },
        );

        assert!(
            output
                .capability_support
                .iter()
                .all(|entry| entry.status == SymbolCapabilityStatus::Supported),
            "{:#?}",
            output.capability_support
        );
        assert!(builder.finish().symbols.is_empty());
    }

    fn discovered_db(root: &Path, relative_paths: &[&str]) -> LocalFactDb {
        let mut db = LocalFactDb::new();
        for relative_path in relative_paths {
            add_go_file(&mut db, root, relative_path, "package app\n");
        }
        db
    }

    fn validated(db: &LocalFactDb, payload: &str) -> GoSidecarOutput {
        validate_paths(
            parse_sidecar_output(payload.as_bytes()).expect("sidecar output parses"),
            db,
        )
    }

    #[test]
    fn validate_paths_keeps_only_the_package_files_this_scan_discovered() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = discovered_db(temp.path(), &["main.go"]);

        let output = validated(
            &db,
            r#"{
  "schema":"polint-go-symbols-semantic-1",
  "packages":[{"files":["./main.go","internal/helper.go"]}]
}"#,
        );

        assert_eq!(output.packages.len(), 1);
        assert_eq!(output.packages[0].files, vec!["main.go".to_string()]);
    }

    #[test]
    fn validate_paths_drops_a_package_whose_files_are_all_out_of_scope() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = discovered_db(temp.path(), &["main.go"]);

        let output = validated(
            &db,
            r#"{
  "schema":"polint-go-symbols-semantic-1",
  "packages":[
    {"files":["internal/helper.go","cmd/app/main.go"]},
    {"files":["main.go"]}
  ]
}"#,
        );

        assert_eq!(output.packages.len(), 1);
        assert_eq!(output.packages[0].files, vec!["main.go".to_string()]);
    }

    #[test]
    fn validate_paths_keeps_a_package_that_named_no_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = discovered_db(temp.path(), &["main.go"]);

        let output = validated(
            &db,
            r#"{
  "schema":"polint-go-symbols-semantic-1",
  "packages":[{"files":[]}]
}"#,
        );

        assert_eq!(output.packages.len(), 1);
        assert!(output.packages[0].files.is_empty());
    }

    #[test]
    fn validate_paths_drops_rows_for_files_this_scan_did_not_discover() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = discovered_db(temp.path(), &["main.go"]);

        let output = validated(
            &db,
            r#"{
  "schema":"polint-go-symbols-semantic-1",
  "symbols":[
    {"key":"kept","package_path":"example.com/app","file":"./main.go","name":"Kept","qualified_name":"Kept","namespace":"value","kind":"function","span":{"start_byte":0,"end_byte":4}},
    {"key":"dropped","package_path":"example.com/app","file":"internal/helper.go","name":"Dropped","qualified_name":"Dropped","namespace":"value","kind":"function","span":{"start_byte":0,"end_byte":7}}
  ],
  "definitions":[
    {"symbol_key":"kept","file":"main.go","name":"Kept","kind":"function","span":{"start_byte":0,"end_byte":4}},
    {"symbol_key":"dropped","file":"internal/helper.go","name":"Dropped","kind":"function","span":{"start_byte":0,"end_byte":7}}
  ],
  "references":[
    {"package_id":"example.com/app","file":"main.go","name":"Kept","kind":"call","span":{"start_byte":0,"end_byte":4},"precision":"exact"},
    {"package_id":"example.com/app","file":"internal/helper.go","name":"Dropped","kind":"call","span":{"start_byte":0,"end_byte":7},"precision":"exact"}
  ],
  "scopes":[
    {"key":"kept","kind":"file","package_path":"example.com/app","file":"main.go","span":{"start_byte":0,"end_byte":4}},
    {"key":"dropped","kind":"file","package_path":"example.com/app","file":"internal/helper.go","span":{"start_byte":0,"end_byte":7}}
  ],
  "imports":[
    {"path":"fmt","alias_kind":"named","file":"main.go","span":{"start_byte":0,"end_byte":4}},
    {"path":"errors","alias_kind":"named","file":"internal/helper.go","span":{"start_byte":0,"end_byte":7}}
  ]
}"#,
        );

        assert_eq!(
            output
                .symbols
                .iter()
                .map(|symbol| (symbol.key.as_str(), symbol.file.as_str()))
                .collect::<Vec<_>>(),
            vec![("kept", "main.go")]
        );
        assert_eq!(
            output
                .definitions
                .iter()
                .map(|definition| definition.symbol_key.as_str())
                .collect::<Vec<_>>(),
            vec!["kept"]
        );
        assert_eq!(
            output
                .references
                .iter()
                .map(|reference| reference.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Kept"]
        );
        assert_eq!(
            output
                .scopes
                .iter()
                .map(|scope| scope.key.as_str())
                .collect::<Vec<_>>(),
            vec!["kept"]
        );
        assert_eq!(
            output
                .imports
                .iter()
                .map(|import| import.path.as_str())
                .collect::<Vec<_>>(),
            vec!["fmt"]
        );
    }

    #[test]
    fn validate_paths_keeps_rows_that_name_no_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = discovered_db(temp.path(), &["main.go"]);

        let output = validated(
            &db,
            r#"{
  "schema":"polint-go-symbols-semantic-1",
  "symbols":[
    {"key":"package-level","package_path":"example.com/app","name":"App","qualified_name":"App","namespace":"value","kind":"package","span":{"start_byte":0,"end_byte":0}}
  ],
  "scopes":[
    {"key":"go:scope:package:example.com/app","kind":"package","package_path":"example.com/app","span":{"start_byte":0,"end_byte":0}}
  ]
}"#,
        );

        assert_eq!(output.symbols.len(), 1);
        assert!(output.symbols[0].file.is_empty());
        assert_eq!(output.scopes.len(), 1);
        assert!(output.scopes[0].file.is_empty());
    }

    #[test]
    fn validate_paths_drops_exports_and_steps_that_only_describe_dropped_rows() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = discovered_db(temp.path(), &["main.go"]);

        let output = validated(
            &db,
            r#"{
  "schema":"polint-go-symbols-semantic-1",
  "symbols":[
    {"key":"kept","package_path":"example.com/app","file":"main.go","name":"Kept","qualified_name":"Kept","namespace":"value","kind":"function","span":{"start_byte":0,"end_byte":4}},
    {"key":"dropped","package_path":"example.com/app","file":"internal/helper.go","name":"Dropped","qualified_name":"Dropped","namespace":"value","kind":"function","span":{"start_byte":0,"end_byte":7}}
  ],
  "references":[
    {"package_id":"example.com/app","file":"main.go","name":"Dropped","target_key":"dropped","kind":"call","span":{"start_byte":10,"end_byte":16},"precision":"exact"},
    {"package_id":"example.com/app","file":"internal/helper.go","name":"Kept","target_key":"kept","kind":"call","span":{"start_byte":20,"end_byte":25},"precision":"exact"}
  ],
  "exports":[
    {"symbol_key":"kept","export_name":"Kept","namespace":"value","object_path":"Kept","package_path":"example.com/app"},
    {"symbol_key":"dropped","export_name":"Dropped","namespace":"value","object_path":"Dropped","package_path":"example.com/app"}
  ],
  "resolution_steps":[
    {"reference_key":"example.com/app|main.go|Dropped|dropped|call|10|16","step":"LexicalLookup","status":"resolved","target_key":"dropped","candidate_keys":["dropped","kept"]},
    {"reference_key":"example.com/app|internal/helper.go|Kept|kept|call|20|25","step":"LexicalLookup","status":"resolved","target_key":"kept","candidate_keys":["kept"]}
  ]
}"#,
        );

        assert_eq!(
            output
                .exports
                .iter()
                .map(|export| export.symbol_key.as_str())
                .collect::<Vec<_>>(),
            vec!["kept"]
        );
        assert_eq!(output.resolution_steps.len(), 1);
        let step = &output.resolution_steps[0];
        assert_eq!(
            step.reference_key,
            "example.com/app|main.go|Dropped|dropped|call|10|16"
        );
        assert!(step.target_key.is_empty());
        assert_eq!(step.candidate_keys, vec!["kept".to_string()]);
    }

    #[test]
    fn validate_paths_leaves_keys_the_sidecar_emitted_no_row_for_untouched() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = discovered_db(temp.path(), &["main.go"]);

        let output = validated(
            &db,
            r#"{
  "schema":"polint-go-symbols-semantic-1",
  "symbols":[
    {"key":"kept","package_path":"example.com/app","file":"main.go","name":"Kept","qualified_name":"Kept","namespace":"value","kind":"function","span":{"start_byte":0,"end_byte":4}}
  ],
  "exports":[
    {"symbol_key":"never-emitted","export_name":"Elsewhere","namespace":"value","object_path":"Elsewhere","package_path":"example.com/dep"}
  ],
  "resolution_steps":[
    {"reference_key":"example.com/app|main.go|Elsewhere|never-emitted|call|30|39","step":"LexicalLookup","status":"resolved","target_key":"never-emitted","candidate_keys":["never-emitted","go:builtin|len"]}
  ]
}"#,
        );

        assert_eq!(output.exports.len(), 1);
        assert_eq!(output.resolution_steps.len(), 1);
        assert_eq!(output.resolution_steps[0].target_key, "never-emitted");
        assert_eq!(
            output.resolution_steps[0].candidate_keys,
            vec!["never-emitted".to_string(), "go:builtin|len".to_string()]
        );
    }

    #[test]
    fn validate_paths_drops_absolute_sidecar_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = discovered_db(temp.path(), &["main.go"]);
        let absolute = temp
            .path()
            .join("main.go")
            .to_string_lossy()
            .replace('\\', "/");

        let output = validated(
            &db,
            &format!(
                r#"{{
  "schema":"polint-go-symbols-semantic-1",
  "packages":[{{"files":["{absolute}"]}}],
  "symbols":[
    {{"key":"absolute","package_path":"example.com/app","file":"{absolute}","name":"Absolute","qualified_name":"Absolute","namespace":"value","kind":"function","span":{{"start_byte":0,"end_byte":8}}}}
  ]
}}"#
            ),
        );

        assert!(output.packages.is_empty());
        assert!(output.symbols.is_empty());
    }
}
