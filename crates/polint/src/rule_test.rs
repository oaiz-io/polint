use crate::cache::{CacheLayout, POLINT_CACHE_DIR_ENV};
use crate::core::rule_id_matches;
use crate::diagnostics::{
    Diagnostic, PolintReport, PolintToolInfo, Severity, diagnostics_from_json_report,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

/// Version of the `polint test` report body.
///
/// Tracks [`crate::diagnostics::POLINT_REPORT_JSON_SCHEMA_V`] so a consumer
/// reading both reports sees one version vocabulary.
pub(crate) const POLINT_TEST_REPORT_JSON_SCHEMA_V: u32 =
    crate::diagnostics::POLINT_REPORT_JSON_SCHEMA_V;

pub(crate) const POLINT_TEST_REPORT_JSON_SCHEMA_V1_URL: &str =
    "https://raw.githubusercontent.com/oaiz-io/polint/main/docs/schemas/polint-test-report-v1.json";

#[derive(Debug, Clone)]
pub(crate) struct RuleTestCase {
    pub(crate) rule: String,
    pub(crate) case: String,
    pub(crate) dir: PathBuf,
    pub(crate) manifest: RuleTestManifest,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RuleTestManifest {
    pub(crate) rule: String,
    #[serde(default)]
    pub(crate) paths: Vec<String>,
    #[serde(default)]
    pub(crate) expect: RuleTestExpect,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RuleTestExpect {
    #[serde(default)]
    pub(crate) diagnostic: Vec<ExpectedDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ExpectedDiagnostic {
    pub(crate) rule_id: String,
    pub(crate) file: String,
    pub(crate) severity: Severity,
    #[serde(default)]
    pub(crate) message_contains: Option<String>,
    #[serde(default)]
    pub(crate) range_start_line: Option<u32>,
    #[serde(default)]
    pub(crate) range_start_column: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RuleTestReport {
    pub(crate) version: u32,
    pub(crate) schema: String,
    pub(crate) tool: PolintToolInfo,
    pub(crate) summary: RuleTestSummary,
    pub(crate) cases: Vec<RuleTestCaseResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RuleTestSummary {
    pub(crate) passed: u32,
    pub(crate) failed: u32,
    pub(crate) total: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RuleTestCaseResult {
    pub(crate) rule: String,
    pub(crate) case: String,
    pub(crate) status: String,
    pub(crate) expected_diagnostics: Vec<ExpectedDiagnostic>,
    pub(crate) observed_diagnostics: Vec<ObservedDiagnostic>,
    pub(crate) failures: Vec<RuleTestFailure>,
    #[serde(skip)]
    pub(crate) kept_temp_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DiagnosticMatchResult {
    pub(crate) expected_index: usize,
    pub(crate) observed_index: Option<usize>,
    pub(crate) matched: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RuleTestFailure {
    pub(crate) message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ObservedDiagnostic {
    pub(crate) rule_id: String,
    pub(crate) file: String,
    pub(crate) severity: Severity,
    pub(crate) message: String,
    pub(crate) range_start_line: u32,
    pub(crate) range_start_column: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct RuleTestOptions {
    pub(crate) rule: Option<String>,
    pub(crate) case: Option<String>,
    pub(crate) no_cache: bool,
    pub(crate) keep_temp: bool,
    pub(crate) rule_host_manifests: Vec<PathBuf>,
}

pub(crate) fn discover_cases(
    root: &Path,
    rule_filter: Option<&str>,
    case_filter: Option<&str>,
) -> Result<Vec<RuleTestCase>> {
    let tests_root = root.join(".polint/tests/rules");
    let Some(tests_root_type) = fixture_path_type(&tests_root)? else {
        return Ok(Vec::new());
    };
    if !tests_root_type.is_dir() {
        return Ok(Vec::new());
    }

    let mut cases = Vec::new();
    for rule_entry in fs::read_dir(&tests_root)
        .with_context(|| format!("failed to read {}", tests_root.display()))?
    {
        let rule_entry = rule_entry?;
        let rule_dir = rule_entry.path();
        let rule_type = fixture_entry_type(&rule_entry)?;
        if !rule_type.is_dir() {
            continue;
        }
        for case_entry in fs::read_dir(&rule_dir)
            .with_context(|| format!("failed to read {}", rule_dir.display()))?
        {
            let case_entry = case_entry?;
            let case_dir = case_entry.path();
            let case_type = fixture_entry_type(&case_entry)?;
            if !case_type.is_dir() {
                continue;
            }
            let manifest_path = case_dir.join("polint-test.toml");
            let Some(manifest_type) = fixture_path_type(&manifest_path)? else {
                continue;
            };
            if !manifest_type.is_file() {
                continue;
            }
            let raw = fs::read_to_string(&manifest_path)
                .with_context(|| format!("failed to read {}", manifest_path.display()))?;
            let manifest: RuleTestManifest = toml::from_str(&raw)
                .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
            if let Some(pattern) = rule_filter
                && !rule_id_matches(pattern, &manifest.rule)
            {
                continue;
            }
            let case_name = case_dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string();
            if let Some(filter) = case_filter
                && filter != case_name
            {
                continue;
            }
            cases.push(RuleTestCase {
                rule: manifest.rule.clone(),
                case: case_name,
                dir: case_dir,
                manifest,
            });
        }
    }

    cases.sort_by(|left, right| {
        (left.rule.as_str(), left.case.as_str()).cmp(&(right.rule.as_str(), right.case.as_str()))
    });
    Ok(cases)
}

pub(crate) fn run_rule_tests(root: &Path, options: RuleTestOptions) -> Result<RuleTestReport> {
    let cases = discover_cases(root, options.rule.as_deref(), options.case.as_deref())?;
    let mut results = Vec::new();
    for case in cases {
        results.push(run_case(root, &case, &options)?);
    }
    Ok(RuleTestReport::new(
        "polint",
        env!("CARGO_PKG_VERSION"),
        results,
    ))
}

impl RuleTestReport {
    pub(crate) fn new(
        tool_name: impl Into<String>,
        tool_version: impl Into<String>,
        mut cases: Vec<RuleTestCaseResult>,
    ) -> Self {
        cases.sort_by(|left, right| {
            (left.rule.as_str(), left.case.as_str())
                .cmp(&(right.rule.as_str(), right.case.as_str()))
        });
        let total = cases.len() as u32;
        let failed = cases.iter().filter(|case| case.status == "failed").count() as u32;
        Self {
            version: POLINT_TEST_REPORT_JSON_SCHEMA_V,
            schema: POLINT_TEST_REPORT_JSON_SCHEMA_V1_URL.to_string(),
            tool: PolintToolInfo {
                name: tool_name.into(),
                version: tool_version.into(),
            },
            summary: RuleTestSummary {
                passed: total - failed,
                failed,
                total,
            },
            cases,
        }
    }
}

pub(crate) fn render_rule_test_human(report: &RuleTestReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Rule tests: {} passed, {} failed, {} total\n",
        report.summary.passed, report.summary.failed, report.summary.total
    ));
    for case in &report.cases {
        out.push_str(&format!("{} {}: {}\n", case.status, case.rule, case.case));
        if let Some(path) = &case.kept_temp_path {
            out.push_str(&format!("  temp: {}\n", path.display()));
        }
        for failure in &case.failures {
            out.push_str(&format!("  - {}\n", failure.message));
        }
    }
    out
}

fn run_case(
    root: &Path,
    case: &RuleTestCase,
    options: &RuleTestOptions,
) -> Result<RuleTestCaseResult> {
    if options.rule_host_manifests.is_empty() {
        return Ok(failed_case(
            case,
            Vec::new(),
            "no repo-local rule hosts were discovered",
        ));
    }

    let temp = tempfile::tempdir().context("failed to create rule test temp repo")?;
    copy_fixture_tree(&case.dir, temp.path())?;
    ensure_case_config(temp.path(), &case.manifest)?;

    let mut diagnostics = Vec::new();
    for manifest in &options.rule_host_manifests {
        match run_rule_host_check(root, temp.path(), manifest, &case.rule, options.no_cache) {
            Ok(mut observed) => diagnostics.append(&mut observed),
            Err(error) => {
                let mut result = failed_case(
                    case,
                    normalize_diagnostics(&diagnostics),
                    "local rule host check failed",
                );
                result.failures.push(RuleTestFailure {
                    message: error.to_string(),
                });
                if options.keep_temp {
                    result.kept_temp_path = Some(temp.keep());
                }
                return Ok(result);
            }
        }
    }

    let observed = normalize_diagnostics(&diagnostics);
    let (matches, mut failures) = match_expected(&case.manifest.expect.diagnostic, &observed);
    let _match_results: Vec<DiagnosticMatchResult> = matches;
    let status = if failures.is_empty() {
        "passed"
    } else {
        "failed"
    };
    let mut result = RuleTestCaseResult {
        rule: case.rule.clone(),
        case: case.case.clone(),
        status: status.to_string(),
        expected_diagnostics: case.manifest.expect.diagnostic.clone(),
        observed_diagnostics: observed,
        failures: std::mem::take(&mut failures),
        kept_temp_path: None,
    };
    if options.keep_temp {
        result.kept_temp_path = Some(temp.keep());
    }
    Ok(result)
}

fn failed_case(
    case: &RuleTestCase,
    observed_diagnostics: Vec<ObservedDiagnostic>,
    message: &str,
) -> RuleTestCaseResult {
    RuleTestCaseResult {
        rule: case.rule.clone(),
        case: case.case.clone(),
        status: "failed".to_string(),
        expected_diagnostics: case.manifest.expect.diagnostic.clone(),
        observed_diagnostics,
        failures: vec![RuleTestFailure {
            message: message.to_string(),
        }],
        kept_temp_path: None,
    }
}

fn run_rule_host_check(
    root: &Path,
    temp_root: &Path,
    manifest: &Path,
    rule_id: &str,
    no_cache: bool,
) -> Result<Vec<Diagnostic>> {
    let cargo = std::env::var("POLINT_CARGO")
        .or_else(|_| std::env::var("CARGO"))
        .unwrap_or_else(|_| "cargo".to_string());
    let test_cache = CacheLayout::for_repo(temp_root);
    let host_cache = CacheLayout::for_repo(root);
    let mut command = ProcessCommand::new(&cargo);
    command.current_dir(temp_root).args(["run", "--quiet"]);
    apply_local_rule_host_profile(&mut command);
    command.args([
        "--manifest-path",
        manifest
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("non-UTF-8 manifest path: {}", manifest.display()))?,
        "--",
        "check",
        "--format",
        "json",
        "--only-rule",
        rule_id,
        "--fail-on",
        "none",
    ]);
    if no_cache {
        command.arg("--no-cache");
    }
    command
        .env(POLINT_CACHE_DIR_ENV, test_cache.root())
        .env("CARGO_TARGET_DIR", host_cache.rules_target_dir());
    crate::jobs::apply_to_command(&mut command);
    if let Ok(toolchain) = std::env::var("POLINT_RULES_TOOLCHAIN")
        && !toolchain.is_empty()
    {
        command.env("RUSTUP_TOOLCHAIN", toolchain);
    }

    let output = command.output().context("failed to run local rule host")?;
    if !output.status.success() {
        anyhow::bail!("local rule host exited unsuccessfully");
    }
    let stdout =
        String::from_utf8(output.stdout).context("local rule host emitted non-UTF-8 output")?;
    let _: PolintReport =
        serde_json::from_str(&stdout).context("local rule host did not emit polint JSON report")?;
    diagnostics_from_json_report(&stdout).context("local rule host diagnostics were invalid")
}

fn copy_fixture_tree(source: &Path, destination: &Path) -> Result<()> {
    let Some(source_type) = fixture_path_type(source)? else {
        anyhow::bail!("missing rule test fixture path: {}", source.display());
    };
    if !source_type.is_dir() {
        anyhow::bail!(
            "rule test fixture path is not a directory: {}",
            source.display()
        );
    }
    for entry in
        fs::read_dir(source).with_context(|| format!("failed to read {}", source.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let file_name = entry.file_name();
        if file_name == "polint-test.toml" {
            continue;
        }
        let target = destination.join(&file_name);
        let file_type = fixture_entry_type(&entry)?;
        if file_type.is_dir() {
            fs::create_dir_all(&target)
                .with_context(|| format!("failed to create {}", target.display()))?;
            copy_fixture_tree(&path, &target)?;
        } else if file_type.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            fs::copy(&path, &target).with_context(|| {
                format!("failed to copy {} to {}", path.display(), target.display())
            })?;
        } else {
            anyhow::bail!("unsupported rule test fixture path: {}", path.display());
        }
    }
    Ok(())
}

fn fixture_entry_type(entry: &fs::DirEntry) -> Result<fs::FileType> {
    let path = entry.path();
    let file_type = entry
        .file_type()
        .with_context(|| format!("failed to inspect {}", path.display()))?;
    reject_symlink_fixture_path(&path, file_type)?;
    Ok(file_type)
}

fn fixture_path_type(path: &Path) -> Result<Option<fs::FileType>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            let file_type = metadata.file_type();
            reject_symlink_fixture_path(path, file_type)?;
            Ok(Some(file_type))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("failed to inspect {}", path.display())),
    }
}

fn reject_symlink_fixture_path(path: &Path, file_type: fs::FileType) -> Result<()> {
    if file_type.is_symlink() {
        anyhow::bail!(
            "rule test fixture paths may not be symlinks: {}",
            path.display()
        );
    }
    Ok(())
}

#[derive(Serialize)]
struct TempPolintConfig<'a> {
    workspace: TempWorkspaceConfig<'a>,
}

#[derive(Serialize)]
struct TempWorkspaceConfig<'a> {
    include: &'a [String],
    exclude: Vec<String>,
}

fn ensure_case_config(root: &Path, manifest: &RuleTestManifest) -> Result<()> {
    let config_path = root.join(".polint.toml");
    if config_path.is_file() {
        return Ok(());
    }
    let include = if manifest.paths.is_empty() {
        vec!["**".to_string()]
    } else {
        manifest.paths.clone()
    };
    let config = TempPolintConfig {
        workspace: TempWorkspaceConfig {
            include: &include,
            exclude: Vec::new(),
        },
    };
    let toml = toml::to_string(&config).context("failed to serialize fixture .polint.toml")?;
    fs::write(&config_path, toml)
        .with_context(|| format!("failed to write {}", config_path.display()))?;
    Ok(())
}

fn normalize_diagnostics(diagnostics: &[Diagnostic]) -> Vec<ObservedDiagnostic> {
    let mut observed = diagnostics
        .iter()
        .map(|diagnostic| ObservedDiagnostic {
            rule_id: diagnostic.rule_id.clone(),
            file: diagnostic.file.trim_start_matches("./").to_string(),
            severity: diagnostic.severity,
            message: diagnostic.message.clone(),
            range_start_line: diagnostic.range.start_line,
            range_start_column: diagnostic.range.start_col,
        })
        .collect::<Vec<_>>();
    observed.sort_by(|left, right| {
        (
            left.rule_id.as_str(),
            left.file.as_str(),
            left.severity.to_string(),
            left.range_start_line,
            left.range_start_column,
            left.message.as_str(),
        )
            .cmp(&(
                right.rule_id.as_str(),
                right.file.as_str(),
                right.severity.to_string(),
                right.range_start_line,
                right.range_start_column,
                right.message.as_str(),
            ))
    });
    observed
}

fn match_expected(
    expected: &[ExpectedDiagnostic],
    observed: &[ObservedDiagnostic],
) -> (Vec<DiagnosticMatchResult>, Vec<RuleTestFailure>) {
    let mut used = BTreeSet::new();
    let mut matches = Vec::new();
    let mut failures = Vec::new();

    for (expected_index, expected_diagnostic) in expected.iter().enumerate() {
        let observed_index = observed
            .iter()
            .enumerate()
            .find(|(index, diagnostic)| {
                !used.contains(index) && expected_matches_observed(expected_diagnostic, diagnostic)
            })
            .map(|(index, _)| index);
        if let Some(index) = observed_index {
            used.insert(index);
        } else {
            failures.push(RuleTestFailure {
                message: format!(
                    "expected diagnostic {} for rule `{}` in `{}` was not observed",
                    expected_index, expected_diagnostic.rule_id, expected_diagnostic.file
                ),
            });
        }
        matches.push(DiagnosticMatchResult {
            expected_index,
            observed_index,
            matched: observed_index.is_some(),
        });
    }

    for (index, diagnostic) in observed.iter().enumerate() {
        if !used.contains(&index) {
            failures.push(RuleTestFailure {
                message: format!(
                    "unexpected diagnostic for rule `{}` in `{}`: {}",
                    diagnostic.rule_id, diagnostic.file, diagnostic.message
                ),
            });
        }
    }

    (matches, failures)
}

fn expected_matches_observed(expected: &ExpectedDiagnostic, observed: &ObservedDiagnostic) -> bool {
    expected.rule_id == observed.rule_id
        && expected.file.trim_start_matches("./") == observed.file
        && expected.severity == observed.severity
        && expected
            .message_contains
            .as_deref()
            .is_none_or(|needle| observed.message.contains(needle))
        && expected
            .range_start_line
            .is_none_or(|line| line == observed.range_start_line)
        && expected
            .range_start_column
            .is_none_or(|column| column == observed.range_start_column)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LocalRuleHostProfile {
    Dev,
    Release,
    Custom(String),
}

impl LocalRuleHostProfile {
    fn from_env() -> Self {
        let Some(value) = std::env::var("POLINT_RULES_PROFILE").ok() else {
            return Self::Release;
        };
        let trimmed = value.trim();
        match trimmed.to_ascii_lowercase().as_str() {
            "" | "dev" | "debug" => Self::Dev,
            "release" => Self::Release,
            _ => Self::Custom(trimmed.to_string()),
        }
    }
}

fn apply_local_rule_host_profile(command: &mut ProcessCommand) {
    match LocalRuleHostProfile::from_env() {
        LocalRuleHostProfile::Dev => {}
        LocalRuleHostProfile::Release => {
            command.arg("--release");
        }
        LocalRuleHostProfile::Custom(profile) => {
            command.args(["--profile", &profile]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{Diagnostic, TextRange};

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn discover_cases_finds_sorted_rule_case_manifests() {
        let temp = tempfile::tempdir().unwrap();
        write(
            &temp
                .path()
                .join(".polint/tests/rules/z_rule/second/polint-test.toml"),
            r#"
rule = "local/z"
paths = ["src/**"]
"#,
        );
        write(
            &temp
                .path()
                .join(".polint/tests/rules/a_rule/first/polint-test.toml"),
            r#"
rule = "local/a"
paths = ["src/**"]
"#,
        );

        let cases = discover_cases(temp.path(), None, None).unwrap();

        assert_eq!(
            cases
                .iter()
                .map(|case| (case.rule.as_str(), case.case.as_str()))
                .collect::<Vec<_>>(),
            [("local/a", "first"), ("local/z", "second")]
        );
    }

    #[test]
    fn expected_diagnostic_matching_supports_message_and_range_fields() {
        let expected = ExpectedDiagnostic {
            rule_id: "local/no-raw-colors".to_string(),
            file: "src/example.ts".to_string(),
            severity: Severity::Warn,
            message_contains: Some("design token".to_string()),
            range_start_line: Some(2),
            range_start_column: Some(10),
        };
        let observed = normalize_diagnostics(&[Diagnostic::warning(
            "local/no-raw-colors",
            "src/example.ts",
            TextRange::new(2, 10, 2, 16),
            "Use a design token instead.",
        )]);

        let (matches, failures) = match_expected(&[expected], &observed);

        assert!(failures.is_empty(), "{failures:#?}");
        assert_eq!(
            matches,
            [DiagnosticMatchResult {
                expected_index: 0,
                observed_index: Some(0),
                matched: true,
            }]
        );
    }

    #[cfg(unix)]
    #[test]
    fn copy_fixture_tree_rejects_symlink_paths() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("case");
        let destination = temp.path().join("repo");
        write(
            &source.join("polint-test.toml"),
            r#"rule = "local/example""#,
        );
        write(&temp.path().join("outside.ts"), "const leaked = true;\n");
        std::os::unix::fs::symlink(temp.path().join("outside.ts"), source.join("src.ts")).unwrap();

        let error = copy_fixture_tree(&source, &destination).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("rule test fixture paths may not be symlinks"),
            "{error:#}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn discover_cases_rejects_symlink_case_dirs() {
        let temp = tempfile::tempdir().unwrap();
        let external_case = temp.path().join("external-case");
        write(
            &external_case.join("polint-test.toml"),
            r#"rule = "local/example""#,
        );
        let rule_dir = temp.path().join(".polint/tests/rules/example");
        fs::create_dir_all(&rule_dir).unwrap();
        std::os::unix::fs::symlink(&external_case, rule_dir.join("escape")).unwrap();

        let error = discover_cases(temp.path(), None, None).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("rule test fixture paths may not be symlinks"),
            "{error:#}"
        );
    }

    #[test]
    fn test_report_json_uses_stable_top_level_fields() {
        let report = RuleTestReport::new(
            "polint",
            "0.0.0",
            vec![RuleTestCaseResult {
                rule: "local/no-raw-colors".to_string(),
                case: "basic".to_string(),
                status: "passed".to_string(),
                expected_diagnostics: Vec::new(),
                observed_diagnostics: Vec::new(),
                failures: Vec::new(),
                kept_temp_path: Some(PathBuf::from("/tmp/not-serialized")),
            }],
        );
        let json = serde_json::to_string(&report).unwrap();

        assert!(json.contains(r#""version":2"#));
        assert!(json.contains(r#""schema":"https://raw.githubusercontent.com/oaiz-io/polint/main/docs/schemas/polint-test-report-v1.json""#));
        assert!(json.contains(r#""summary":{"passed":1,"failed":0,"total":1}"#));
        assert!(json.contains(r#""cases":["#));
        assert!(!json.contains("/tmp/not-serialized"));
    }
}
