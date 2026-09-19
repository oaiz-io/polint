use std::collections::BTreeMap;
use std::path::Path;

use toml::Value;

use crate::analysis_api::{Digest, ProviderManifest};
use crate::core::AnalysisDb;

pub(crate) use crate::ts::types::provider::{
    TsTypesProviderRunOutput, TsTypesSidecarAccess, derive_ts_types_with_cache_stats,
};

/// Entry point the kernel provider calls.
///
/// The indirection exists so the frontend module never names `AnalysisDb`.
pub(crate) fn derive_ts_types(
    db: &mut AnalysisDb,
    root: &Path,
    ts_settings: &BTreeMap<String, Value>,
    config_digest: &str,
    manifest: &ProviderManifest,
    ts_syntax_output_digest: Digest,
    sidecar: TsTypesSidecarAccess<'_>,
) -> TsTypesProviderRunOutput {
    derive_ts_types_with_cache_stats(
        db,
        root,
        ts_settings,
        config_digest,
        manifest,
        ts_syntax_output_digest,
        sidecar,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis_api::{
        CachePolicy, PrecisionCeiling, ProviderExecution, ProviderFailureReason,
        ProviderFailureStage, ProviderKind, SchemaVersion,
    };
    use crate::internal_core::Language;
    use crate::ts::types::client::{TsTypesClientError, TsTypesClientRun};
    use crate::ts::types::process::{ResolvedTypeScript, TsTypesProcessError, TypeScriptSource};
    use crate::ts::types::protocol::decode_ndjson_str;
    use crate::ts::types::provider::derive_ts_types_with_runner_for_test;

    const MANIFEST: ProviderManifest = ProviderManifest {
        id: "polint.ts.types",
        kind: ProviderKind::WholeRepoDerived,
        inputs: &["source_files"],
        outputs: &["ts_type_callsites"],
        language_ids: &[],
        cache_policy: CachePolicy::InMemoryDerived,
        schema_versions: &[SchemaVersion {
            name: "ts-type-facts-1",
            version: 1,
        }],
        precision_ceiling: PrecisionCeiling::SetupAware,
    };

    fn db_with_ts_file() -> AnalysisDb {
        let mut db = AnalysisDb::default();
        let source = "export function run() { helper(); }\n";
        db.add_source_file(
            "src/app.ts".into(),
            "src/app.ts".to_string(),
            Language::TypeScript,
            std::sync::Arc::from(source),
            "hash".to_string(),
        );
        db
    }

    fn settings(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect()
    }

    fn typescript() -> ResolvedTypeScript {
        ResolvedTypeScript {
            directory: std::path::PathBuf::from("/repo/node_modules/typescript"),
            version: "5.9.3".to_string(),
            source: TypeScriptSource::Repository,
        }
    }

    fn successful_run(rows: &str) -> TsTypesClientRun {
        let framed = format!(
            "{{\"schema\":\"polint-ts-types-1\",\"kind\":\"session_begin\",\
             \"typescript_version\":\"5.9.3\",\"node_version\":\"v22.0.0\"}}\n{rows}\n\
             {{\"schema\":\"polint-ts-types-1\",\"kind\":\"session_end\",\"elapsed_ms\":12}}\n"
        );
        TsTypesClientRun {
            output: decode_ndjson_str(&framed).expect("scripted output decodes"),
            sidecar_digest: "sidecar".to_string(),
            typescript: typescript(),
        }
    }

    fn temp_repo_with_tsconfig() -> tempfile::TempDir {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("tsconfig.json"), "{}").expect("write tsconfig");
        temp
    }

    #[test]
    fn a_repository_that_never_asked_for_the_tier_skips_it_without_a_diagnostic() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut db = db_with_ts_file();

        let output = derive_ts_types_with_runner_for_test(
            &mut db,
            temp.path(),
            &BTreeMap::new(),
            "config",
            &MANIFEST,
            Digest::absent(crate::analysis_api::DigestKind::ProviderOutput, "ts"),
            |_| {
                panic!("the sidecar must not run without a tsconfig");
            },
        );

        assert_eq!(output.execution, ProviderExecution::Succeeded);
        assert!(output.diagnostics.is_empty());
        assert!(db.ts_type_callsites().is_empty());
        assert_eq!(output.counts.get("ts_types.setup_missing"), Some(&1));
    }

    #[test]
    fn a_repository_that_asked_for_the_tier_is_told_why_it_did_not_run() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut db = db_with_ts_file();

        let output = derive_ts_types_with_runner_for_test(
            &mut db,
            temp.path(),
            &settings(&[("type_sidecar", Value::Boolean(true))]),
            "config",
            &MANIFEST,
            Digest::absent(crate::analysis_api::DigestKind::ProviderOutput, "ts"),
            |_| panic!("the sidecar must not run without a tsconfig"),
        );

        assert_eq!(
            output.execution,
            ProviderExecution::Failed {
                stage: ProviderFailureStage::Setup,
                reason: ProviderFailureReason::SetupMissing,
            }
        );
        assert_eq!(output.diagnostics.len(), 1);
        assert!(
            output.diagnostics[0]
                .message
                .contains("TsTypesSetupMissing")
        );
    }

    #[test]
    fn a_missing_typescript_compiler_is_a_quiet_skip_not_a_panic() {
        let temp = temp_repo_with_tsconfig();
        let mut db = db_with_ts_file();

        let output = derive_ts_types_with_runner_for_test(
            &mut db,
            temp.path(),
            &BTreeMap::new(),
            "config",
            &MANIFEST,
            Digest::absent(crate::analysis_api::DigestKind::ProviderOutput, "ts"),
            |_| {
                Err(TsTypesClientError::Process(
                    TsTypesProcessError::SetupMissing("no TypeScript compiler".to_string()),
                ))
            },
        );

        assert_eq!(output.execution, ProviderExecution::Succeeded);
        assert!(output.diagnostics.is_empty());
        assert!(db.ts_type_callsites().is_empty());
    }

    #[test]
    fn an_unsupported_typescript_major_is_reported_when_the_tier_was_requested() {
        let temp = temp_repo_with_tsconfig();
        let mut db = db_with_ts_file();

        let output = derive_ts_types_with_runner_for_test(
            &mut db,
            temp.path(),
            &settings(&[("type_sidecar", Value::Boolean(true))]),
            "config",
            &MANIFEST,
            Digest::absent(crate::analysis_api::DigestKind::ProviderOutput, "ts"),
            |_| {
                Err(TsTypesClientError::Process(
                    TsTypesProcessError::VersionUnsupported(
                        "TypeScript 7.0.0 exposes no supported programmatic API".to_string(),
                    ),
                ))
            },
        );

        assert!(matches!(
            output.execution,
            ProviderExecution::Failed {
                reason: ProviderFailureReason::SetupMissing,
                ..
            }
        ));
        assert!(output.diagnostics[0].message.contains("TypeScript 7.0.0"));
    }

    #[test]
    fn a_sidecar_timeout_is_always_reported_even_when_the_tier_was_not_requested() {
        let temp = temp_repo_with_tsconfig();
        let mut db = db_with_ts_file();

        let output = derive_ts_types_with_runner_for_test(
            &mut db,
            temp.path(),
            &BTreeMap::new(),
            "config",
            &MANIFEST,
            Digest::absent(crate::analysis_api::DigestKind::ProviderOutput, "ts"),
            |_| {
                Err(TsTypesClientError::Process(TsTypesProcessError::Timeout(
                    "TsTypesSidecarTimeout: exceeded its 300000 ms timeout.".to_string(),
                )))
            },
        );

        assert_eq!(
            output.execution,
            ProviderExecution::Failed {
                stage: ProviderFailureStage::Execution,
                reason: ProviderFailureReason::ExecutionFailed,
            }
        );
        assert!(
            output.diagnostics[0]
                .message
                .contains("TsTypesSidecarTimeout")
        );
        assert!(output.output_digest.is_none());
    }

    #[test]
    fn a_malformed_wire_is_a_reported_failure_and_leaves_the_store_empty() {
        let temp = temp_repo_with_tsconfig();
        let mut db = db_with_ts_file();

        let output = derive_ts_types_with_runner_for_test(
            &mut db,
            temp.path(),
            &BTreeMap::new(),
            "config",
            &MANIFEST,
            Digest::absent(crate::analysis_api::DigestKind::ProviderOutput, "ts"),
            |_| {
                Err(TsTypesClientError::Protocol(
                    crate::ts::types::protocol::TsTypesProtocolError::MissingEnd,
                ))
            },
        );

        assert!(matches!(
            output.execution,
            ProviderExecution::Failed {
                stage: ProviderFailureStage::Execution,
                ..
            }
        ));
        assert!(db.ts_type_callsites().is_empty());
    }

    #[test]
    fn a_successful_run_stores_rows_and_issues_a_digest() {
        let temp = temp_repo_with_tsconfig();
        let mut db = db_with_ts_file();

        let output = derive_ts_types_with_runner_for_test(
            &mut db,
            temp.path(),
            &BTreeMap::new(),
            "config",
            &MANIFEST,
            Digest::absent(crate::analysis_api::DigestKind::ProviderOutput, "ts"),
            |_| {
                Ok(successful_run(
                    "{\"schema\":\"polint-ts-types-1\",\"kind\":\"callsite\",\
                     \"project\":\"tsconfig.json\",\"callsite\":\"src/app.ts:24\",\
                     \"file\":\"src/app.ts\",\"call_kind\":\"call\",\"status\":\"resolved\",\
                     \"stable_key\":\"site\",\"span\":{\"start_byte\":24,\"end_byte\":32,\
                     \"start_line\":1,\"start_column\":25,\"end_line\":1,\"end_column\":33}}",
                ))
            },
        );

        assert_eq!(output.execution, ProviderExecution::Succeeded);
        assert!(output.output_digest.is_some());
        assert_eq!(db.ts_type_callsites().len(), 1);
        assert_eq!(output.counts.get("ts_types.elapsed_ms"), Some(&12));
    }

    #[test]
    fn dropped_rows_are_counted_in_the_run_report_and_not_only_warned_about() {
        let temp = temp_repo_with_tsconfig();
        let mut db = db_with_ts_file();
        let row = "{\"schema\":\"polint-ts-types-1\",\"kind\":\"callsite\",\
                   \"project\":\"tsconfig.json\",\"callsite\":\"src/app.ts:24\",\
                   \"file\":\"src/app.ts\",\"call_kind\":\"call\",\"status\":\"resolved\",\
                   \"stable_key\":\"site\"}";

        let output = derive_ts_types_with_runner_for_test(
            &mut db,
            temp.path(),
            &BTreeMap::new(),
            "config",
            &MANIFEST,
            Digest::absent(crate::analysis_api::DigestKind::ProviderOutput, "ts"),
            |_| Ok(successful_run(&format!("{row}\n{row}"))),
        );

        assert_eq!(output.counts.get("ts_types.dropped_rows"), Some(&1));
        assert_eq!(output.counts.get("ts_types.dangling_callees"), Some(&0));
    }

    #[test]
    fn the_output_digest_changes_when_a_row_changes() {
        let temp = temp_repo_with_tsconfig();
        let row = |status: &str| {
            format!(
                "{{\"schema\":\"polint-ts-types-1\",\"kind\":\"callsite\",\
                 \"project\":\"tsconfig.json\",\"callsite\":\"src/app.ts:24\",\
                 \"file\":\"src/app.ts\",\"call_kind\":\"call\",\"status\":\"{status}\",\
                 \"stable_key\":\"site\"}}"
            )
        };
        let digest_for = |status: &str| {
            let mut db = db_with_ts_file();
            derive_ts_types_with_runner_for_test(
                &mut db,
                temp.path(),
                &BTreeMap::new(),
                "config",
                &MANIFEST,
                Digest::absent(crate::analysis_api::DigestKind::ProviderOutput, "ts"),
                |_| Ok(successful_run(&row(status))),
            )
            .output_digest
            .expect("digest issued")
        };

        assert_ne!(digest_for("resolved"), digest_for("any_receiver"));
    }

    #[test]
    fn a_repository_with_no_typescript_sources_succeeds_without_touching_the_sidecar() {
        let temp = temp_repo_with_tsconfig();
        let mut db = AnalysisDb::default();

        let output = derive_ts_types_with_runner_for_test(
            &mut db,
            temp.path(),
            &BTreeMap::new(),
            "config",
            &MANIFEST,
            Digest::absent(crate::analysis_api::DigestKind::ProviderOutput, "ts"),
            |_| panic!("the sidecar must not run for a repository with no TS sources"),
        );

        assert_eq!(output.execution, ProviderExecution::Succeeded);
        assert!(output.diagnostics.is_empty());
    }

    #[test]
    fn turning_the_tier_off_is_a_successful_no_op() {
        let temp = temp_repo_with_tsconfig();
        let mut db = db_with_ts_file();

        let output = derive_ts_types_with_runner_for_test(
            &mut db,
            temp.path(),
            &settings(&[("type_sidecar", Value::Boolean(false))]),
            "config",
            &MANIFEST,
            Digest::absent(crate::analysis_api::DigestKind::ProviderOutput, "ts"),
            |_| panic!("a disabled tier must not run the sidecar"),
        );

        assert_eq!(output.execution, ProviderExecution::Succeeded);
        assert!(output.diagnostics.is_empty());
        assert!(db.ts_type_callsites().is_empty());
    }

    #[test]
    fn an_invalid_setting_fails_setup_rather_than_defaulting_silently() {
        let temp = temp_repo_with_tsconfig();
        let mut db = db_with_ts_file();

        let output = derive_ts_types_with_runner_for_test(
            &mut db,
            temp.path(),
            &settings(&[("type_timeout_ms", Value::Integer(-1))]),
            "config",
            &MANIFEST,
            Digest::absent(crate::analysis_api::DigestKind::ProviderOutput, "ts"),
            |_| panic!("an unusable lifecycle must not reach the sidecar"),
        );

        assert!(matches!(
            output.execution,
            ProviderExecution::Failed {
                stage: ProviderFailureStage::Setup,
                ..
            }
        ));
        assert!(output.diagnostics[0].message.contains("type_timeout_ms"));
    }
}
