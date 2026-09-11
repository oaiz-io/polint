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
    fn embedded_go_sidecar_keeps_go_1_24_minimum() {
        let go_mod = EMBEDDED_GO_SIDECAR_FILES
            .iter()
            .find_map(|(relative_path, contents)| (*relative_path == "go.mod").then_some(*contents))
            .expect("embedded go.mod exists");

        assert!(
            go_mod.lines().any(|line| line == "go 1.24.0"),
            "embedded sidecar should keep Go 1.24 as its minimum supported toolchain: {go_mod:?}"
        );
        assert!(
            go_mod
                .lines()
                .map(str::trim)
                .any(|line| line == "golang.org/x/tools v0.42.0"),
            "embedded sidecar should stay on the Go 1.24-compatible x/tools line: {go_mod:?}"
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
    fn repo_escaping_sidecar_file_path_reports_setup_missing() {
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
            output.capability_support.iter().all(|entry| {
                entry.status == SymbolCapabilityStatus::SetupMissing
                    && entry
                        .reason
                        .as_deref()
                        .is_some_and(|reason| reason.contains("escapes repository"))
            }),
            "{:#?}",
            output.capability_support
        );
    }
}
