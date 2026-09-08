use crate::analysis_kernel::{AnalysisKernel, KernelInput};
use crate::analysis_plan::AnalysisPlan;
use crate::baseline::{
    BaselineConfig, BaselineSummary, DEFAULT_BASELINE_PATH, classify_diagnostics, load_baseline,
    render_baseline_summary, write_baseline,
};
use crate::cache::rules_store;
use crate::cache::{
    CacheCleanReport, CacheCleanSelection, CacheLayout, CacheManagedCategory, CachePruneOptions,
    CachePruneReport, CacheStatus, POLINT_CACHE_DIR_ENV,
};
use crate::config::{LoadedConfig, default_config_toml, load_config};
use crate::core::{
    AnalysisDb, Language, ResolutionStatus, Rule, RuleOptions, SymbolPrecision,
    SymbolResolutionStatus, rule_id_matches, run_rules,
};
use crate::diagnostics::{
    AiFriendlyReport, ColorChoice, Diagnostic, JsonReportMeta, OutputFormat, RenderOpts, Severity,
    apply_report_filters, build_ai_friendly_report,
    diagnostics_and_rule_execution_from_public_json_report, limit_report_diagnostics,
    render_ai_friendly_stdout, render_with_sarif_help,
};
use crate::fs::load_analysis_files_scoped;
use crate::ignores::{apply_ignores, filter_report, render_ignore_report_human};
use crate::rule_manifest::InspectRuleReport;
use crate::rule_test::{RuleTestOptions, render_rule_test_human, run_rule_tests};
use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

mod rules_host_error;
mod skill;

const POLINT_CACHE_STATUS_JSON_SCHEMA_V1_URL: &str = "https://raw.githubusercontent.com/oaiz-io/polint/main/docs/schemas/polint-cache-status-v1.json";
const POLINT_RULES_PROFILE_ENV: &str = "POLINT_RULES_PROFILE";
const AI_FRIENDLY_OUTPUT_DIR: &str = ".polint/output";
const AI_FRIENDLY_LATEST_OUTPUT: &str = ".polint/output/latest.json";
const REPORT_SOURCE_SNIPPET_MAX_BYTES: u64 = 1_048_576;

struct AiFriendlyOutput {
    report: AiFriendlyReport,
}

fn json_report_meta() -> JsonReportMeta<'static> {
    JsonReportMeta {
        tool_name: "polint",
        tool_version: env!("CARGO_PKG_VERSION"),
    }
}

fn render_opts<'a>(
    args: &CheckArgs,
    sources: Option<&'a BTreeMap<String, Arc<str>>>,
    rule_execution: &'a [crate::diagnostics::RuleExecutionRow],
) -> RenderOpts<'a> {
    RenderOpts {
        json: json_report_meta(),
        color: match args.color {
            ColorArg::Auto => ColorChoice::Auto,
            ColorArg::Always => ColorChoice::Always,
            ColorArg::Never => ColorChoice::Never,
        },
        sources,
        rule_execution,
    }
}

fn sarif_help_map(config: &LoadedConfig) -> Option<&BTreeMap<String, String>> {
    if config.config.sarif.rule_help_uri.is_empty() {
        None
    } else {
        Some(&config.config.sarif.rule_help_uri)
    }
}

fn write_ai_friendly_report(
    root: &Path,
    diagnostics: &[Diagnostic],
    persisted_diagnostics: &[Diagnostic],
    json_meta: JsonReportMeta<'_>,
    rule_execution: &[crate::diagnostics::RuleExecutionRow],
) -> Result<AiFriendlyOutput> {
    crate::repo_fs::ensure_repo_dir(root, AI_FRIENDLY_OUTPUT_DIR).with_context(|| {
        format!(
            "failed to create {}",
            root.join(AI_FRIENDLY_OUTPUT_DIR).display()
        )
    })?;
    ensure_polint_nested_gitignore(root)?;

    let generated_at = generated_at_label();
    let report = build_ai_friendly_report(
        diagnostics,
        persisted_diagnostics,
        json_meta,
        generated_at.clone(),
        rule_execution,
    );
    let json = serde_json::to_string_pretty(&report)?;
    let hash = crate::cache::stable_hash(&[&json]);
    let run_name = format!("check-{generated_at}-{}.json", &hash[..12]);
    let run_path = format!("{AI_FRIENDLY_OUTPUT_DIR}/{run_name}");
    crate::repo_fs::write_repo_file_atomic(root, &run_path, &json)
        .with_context(|| format!("failed to write {}", root.join(&run_path).display()))?;
    crate::repo_fs::write_repo_file_atomic(root, AI_FRIENDLY_LATEST_OUTPUT, json).with_context(
        || {
            format!(
                "failed to write {}",
                root.join(AI_FRIENDLY_LATEST_OUTPUT).display()
            )
        },
    )?;
    Ok(AiFriendlyOutput { report })
}

fn generated_at_label() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    secs.to_string()
}

fn read_sources_for_diagnostics(
    root: &Path,
    diagnostics: &[Diagnostic],
) -> BTreeMap<String, Arc<str>> {
    let mut map = BTreeMap::new();
    for diagnostic in diagnostics {
        if diagnostic.file.is_empty() || diagnostic.file == "<unknown>" {
            continue;
        }
        if map.contains_key(&diagnostic.file) {
            continue;
        }
        let rel = diagnostic.file.trim_start_matches("./");
        if let Ok(text) = crate::repo_fs::read_repo_file_to_string_with_limit(
            root,
            rel,
            REPORT_SOURCE_SNIPPET_MAX_BYTES,
        ) {
            map.insert(diagnostic.file.clone(), Arc::from(text.into_boxed_str()));
        }
    }
    map
}

#[derive(Debug, Parser)]
#[command(name = "polint")]
#[command(about = "Repo-local static analysis policy as code.")]
#[command(
    after_help = "AI agents: prefer `polint check --format ai-friendly --fail-on none` to avoid context overload."
)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create `.polint.toml`, `.polint/rules/src`, `.polint/cache`, `.polint/.gitignore`, and root
    /// `rust-toolchain.toml` when missing (matches polint's MSRV for building rule packs).
    Init,
    /// Install a repo-local AI-agent skill for using polint.
    AddSkill(skill::AddSkillArgs),
    /// Scaffold a repo-local Rust rule.
    NewRule(NewRuleArgs),
    /// Create or update a compact YAML diagnostic baseline.
    Baseline(BaselineArgs),
    /// Inspect, prune, or clean polint caches.
    Cache(CacheArgs),
    /// Inspect polint configuration and repo-local rule manifests.
    Inspect(InspectArgs),
    /// List or sample supported public fact views.
    Facts(FactsArgs),
    /// Report public setup and resolution unknowns.
    Unknowns(UnknownsArgs),
    /// Explain rule capability planning.
    Explain(ExplainArgs),
    /// Run repo-local rule fixture tests.
    Test(TestArgs),
    /// Run enabled rules.
    Check(CheckArgs),
    /// Run review-kind rules against a diff to a target branch/commit.
    Review(ReviewArgs),
    /// Inspect polint comment-ignore directives and suppression statistics.
    Ignores(IgnoresArgs),
}

#[derive(Debug, Args, Clone)]
struct NewRuleArgs {
    /// Rule language focus: go, ts, js, or generic.
    language: String,
    /// Rule directory/name.
    rule_name: String,
    /// Scaffold a review-kind rule (`kind = "review"`) with a `ChangedFiles<'_>`
    /// parameter, run via `polint review <ref>` instead of `polint check`.
    #[arg(long)]
    review: bool,
    /// Flagship policy template to scaffold.
    #[arg(long, value_enum)]
    template: Option<RuleTemplateKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[clap(rename_all = "kebab-case")]
enum RuleTemplateKind {
    RequestToShell,
    SecretToLog,
    PiiToAnalytics,
    SensitiveWriteGuard,
    TransactionCleanup,
    RawReachableApi,
    Ssrf,
    DangerousHtml,
    UnsafeDeserialization,
    UserFilePath,
}

impl RuleTemplateKind {
    fn as_kebab_case(self) -> &'static str {
        match self {
            Self::RequestToShell => "request-to-shell",
            Self::SecretToLog => "secret-to-log",
            Self::PiiToAnalytics => "pii-to-analytics",
            Self::SensitiveWriteGuard => "sensitive-write-guard",
            Self::TransactionCleanup => "transaction-cleanup",
            Self::RawReachableApi => "raw-reachable-api",
            Self::Ssrf => "ssrf",
            Self::DangerousHtml => "dangerous-html",
            Self::UnsafeDeserialization => "unsafe-deserialization",
            Self::UserFilePath => "user-file-path",
        }
    }
}

#[derive(Debug, Args, Clone)]
struct BaselineArgs {
    #[command(subcommand)]
    command: BaselineCommand,
}

#[derive(Debug, Args, Clone)]
struct CacheArgs {
    #[command(subcommand)]
    command: CacheCommand,
}

#[derive(Debug, Args, Clone)]
struct InspectArgs {
    #[command(subcommand)]
    command: InspectCommand,
}

#[derive(Debug, Args, Clone)]
struct FactsArgs {
    #[command(subcommand)]
    command: FactsCommand,
}

#[derive(Debug, Subcommand, Clone)]
enum FactsCommand {
    /// List supported and reserved public fact-view capabilities.
    List(FactsListArgs),
    /// Sample a bounded set of public facts for one capability.
    Sample(FactsSampleArgs),
}

#[derive(Debug, Args, Clone)]
struct FactsListArgs {
    /// Output format.
    #[arg(long, value_enum, default_value_t = AgentJsonFormatArg::Json)]
    format: AgentJsonFormatArg,
}

#[derive(Debug, Args, Clone)]
struct FactsSampleArgs {
    /// Capability to sample, such as resolved_imports, symbols, references, or file_metrics.
    #[arg(long = "cap", value_name = "CAPABILITY")]
    capability: String,
    /// Maximum number of rows to return.
    #[arg(long, value_name = "N", default_value_t = 10)]
    limit: usize,
    /// Output format.
    #[arg(long, value_enum, default_value_t = AgentJsonFormatArg::Json)]
    format: AgentJsonFormatArg,
    /// Files or directories to analyze. Defaults to workspace config.
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,
    /// Disable analysis/fact cache reads and writes.
    #[arg(long)]
    no_cache: bool,
}

#[derive(Debug, Args, Clone)]
struct UnknownsArgs {
    /// Capability to inspect, such as resolved_imports, symbols, references, or dataflow.
    #[arg(long = "cap", value_name = "CAPABILITY")]
    capability: String,
    /// Output format.
    #[arg(long, value_enum, default_value_t = AgentJsonFormatArg::Json)]
    format: AgentJsonFormatArg,
    /// Files or directories to analyze. Defaults to workspace config.
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,
    /// Disable analysis/fact cache reads and writes.
    #[arg(long)]
    no_cache: bool,
}

#[derive(Debug, Args, Clone)]
struct ExplainArgs {
    /// Rule id to explain. When omitted, all discovered rules are included.
    #[arg(long, value_name = "RULE_ID")]
    rule: Option<String>,
    /// Output format.
    #[arg(long, value_enum, default_value_t = AgentJsonFormatArg::Json)]
    format: AgentJsonFormatArg,
}

#[derive(Debug, Subcommand, Clone)]
enum InspectCommand {
    /// Show registered repo-local rule manifests.
    Rule(InspectRuleArgs),
    /// Show consolidated setup, unsupported, budget, model, and resolution unknowns.
    Unknowns(InspectUnknownsArgs),
}

#[derive(Debug, Args, Clone)]
struct InspectRuleArgs {
    /// Output format.
    #[arg(long, value_enum, default_value_t = InspectFormatArg::Human)]
    format: InspectFormatArg,
    /// Only include rules whose id matches this pattern.
    #[arg(long, value_name = "RULE_ID")]
    rule: Option<String>,
}

#[derive(Debug, Args, Clone)]
struct InspectUnknownsArgs {
    /// Optional capability filter, such as resolved_imports, symbols, or references.
    #[arg(long = "cap", value_name = "CAPABILITY")]
    capability: Option<String>,
    /// Output format.
    #[arg(long, value_enum, default_value_t = AgentJsonFormatArg::Json)]
    format: AgentJsonFormatArg,
    /// Files or directories to analyze. Defaults to workspace config.
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,
    /// Disable analysis/fact cache reads and writes.
    #[arg(long)]
    no_cache: bool,
}

#[derive(Debug, Args, Clone)]
struct TestArgs {
    /// Output format.
    #[arg(long, value_enum, default_value_t = TestFormatArg::Human)]
    format: TestFormatArg,
    /// Only run fixture cases whose manifest rule id matches this pattern.
    #[arg(long, value_name = "RULE_ID")]
    rule: Option<String>,
    /// Only run fixture cases with this case directory name.
    #[arg(long, value_name = "CASE_NAME")]
    case: Option<String>,
    /// Disable analysis/fact cache reads and writes while running fixture checks.
    #[arg(long)]
    no_cache: bool,
    /// Keep temporary fixture repositories on disk.
    #[arg(long)]
    keep_temp: bool,
}

#[derive(Debug, Subcommand, Clone)]
enum CacheCommand {
    /// Show cache paths, size, and file counts.
    Status(CacheStatusArgs),
    /// Remove cache files by size or age while preserving newer entries.
    Prune(CachePruneArgs),
    /// Remove a whole cache category.
    Clean(CacheCleanArgs),
}

#[derive(Debug, Args, Clone)]
struct CacheStatusArgs {
    /// Output format.
    #[arg(long, value_enum, default_value_t = CacheStatusFormatArg::Human)]
    format: CacheStatusFormatArg,
}

#[derive(Debug, Args, Clone)]
struct CachePruneArgs {
    /// Cache category to prune. Defaults to all managed categories.
    #[arg(long, value_enum)]
    category: Option<CacheCategoryArg>,
    /// Remove files older than this many days.
    #[arg(long, value_name = "DAYS")]
    max_age_days: Option<u64>,
    /// Keep selected categories at or below this many MiB.
    #[arg(long, value_name = "MIB")]
    max_size_mb: Option<u64>,
    /// Print what would be removed without deleting files.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Args, Clone)]
struct CacheCleanArgs {
    /// Cache category to clean. Defaults to all.
    #[arg(long, value_enum, default_value_t = CacheCategoryArg::All)]
    category: CacheCategoryArg,
}

#[derive(Debug, Subcommand, Clone)]
enum BaselineCommand {
    /// Write current diagnostics to .polint/baseline.yaml.
    Create(BaselineCreateArgs),
    /// Remove fixed entries from .polint/baseline.yaml and refresh moved paths.
    Update(BaselineUpdateArgs),
}

#[derive(Debug, Args, Clone)]
struct BaselineCreateArgs {
    /// Overwrite an existing baseline file.
    #[arg(long)]
    force: bool,
    /// Named profile from .polint.toml. When omitted, all discovered rules run.
    #[arg(long)]
    profile: Option<String>,
    /// Disable analysis/fact cache reads and writes while creating the baseline.
    #[arg(long)]
    no_cache: bool,
    /// Files or directories to baseline. Defaults to the workspace include config.
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,
}

#[derive(Debug, Args, Clone)]
struct BaselineUpdateArgs {
    /// Named profile from .polint.toml. When omitted, all discovered rules run.
    #[arg(long)]
    profile: Option<String>,
    /// Disable analysis/fact cache reads and writes while updating the baseline.
    #[arg(long)]
    no_cache: bool,
    /// Files or directories to check. Defaults to the workspace include config.
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,
}

#[derive(Debug, Args, Clone)]
#[command(
    after_help = "AI agents: use `--format ai-friendly` to print a compact summary and save queryable JSON under `.polint/output/`."
)]
struct CheckArgs {
    /// Files or directories to check. Defaults to the workspace include config.
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,
    /// Named profile from .polint.toml. When omitted, all discovered rules run.
    #[arg(long)]
    profile: Option<String>,
    /// Output format.
    #[arg(long, value_enum, default_value_t = FormatArg::Human)]
    format: FormatArg,
    /// ANSI colors for human output (no effect on JSON/SARIF). `auto` uses color only for a tty and when `NO_COLOR` is unset.
    #[arg(long, value_enum, default_value_t = ColorArg::Auto)]
    color: ColorArg,
    /// Disable analysis/fact cache reads and writes. Does not disable the repo-local rule-host Cargo target cache.
    #[arg(long)]
    no_cache: bool,
    /// Diagnostic level that fails the process.
    #[arg(long, value_enum, default_value_t = FailOn::Error)]
    fail_on: FailOn,
    /// Only include diagnostics whose `rule_id` matches this pattern (same rules as profiles: exact id, `prefix/*`, or `*`).
    #[arg(long, value_name = "PATTERN")]
    only_rule: Option<String>,
    /// Emit at most this many diagnostics after stable sort (applies after `--only-rule`).
    #[arg(long, value_name = "N")]
    max_diagnostics: Option<usize>,
    /// Print grouped scan statistics for human output.
    #[arg(long)]
    stat: bool,
    /// Print one scan summary line for human output.
    #[arg(long)]
    shortstat: bool,
    /// Suppress known diagnostics from .polint/baseline.yaml.
    #[arg(long)]
    baseline: bool,
    /// Emit and fail only on diagnostics not covered by `--baseline`.
    #[arg(long)]
    new_only: bool,
    /// Apply polint comment-ignore directives.
    #[arg(long, default_value_t = true, hide = true, action = clap::ArgAction::Set)]
    ignore_comments: bool,
}

#[derive(Debug, Args, Clone)]
struct ReviewArgs {
    /// Target branch or commit to diff against (e.g. origin/main, a SHA, or a...b).
    #[arg(value_name = "REF")]
    reff: String,
    /// Files or directories to review. Defaults to the workspace include config.
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,
    /// Named profile from .polint.toml. When omitted, all discovered rules run.
    #[arg(long)]
    profile: Option<String>,
    /// Output format.
    #[arg(long, value_enum, default_value_t = FormatArg::Human)]
    format: FormatArg,
    /// ANSI colors for human output (no effect on JSON/SARIF). `auto` uses color only for a tty and when `NO_COLOR` is unset.
    #[arg(long, value_enum, default_value_t = ColorArg::Auto)]
    color: ColorArg,
    /// Disable analysis/fact cache reads and writes. Does not disable the repo-local rule-host Cargo target cache.
    #[arg(long)]
    no_cache: bool,
    /// Diagnostic level that fails the process.
    #[arg(long, value_enum, default_value_t = FailOn::Error)]
    fail_on: FailOn,
    /// Only include diagnostics whose `rule_id` matches this pattern (same rules as profiles: exact id, `prefix/*`, or `*`).
    #[arg(long, value_name = "PATTERN")]
    only_rule: Option<String>,
    /// Emit at most this many diagnostics after stable sort (applies after `--only-rule`).
    #[arg(long, value_name = "N")]
    max_diagnostics: Option<usize>,
    /// Surface ALL review-rule findings, not just those intersecting the diff.
    #[arg(long)]
    no_diff_gate: bool,
    /// Gate on changed FILES only (ignore line ranges) when diff-gating.
    #[arg(long)]
    whole_file: bool,
}

#[derive(Debug, Args, Clone)]
struct IgnoresArgs {
    /// Files or directories to inspect. Defaults to the workspace include config.
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,
    /// Named profile from .polint.toml. When omitted, all discovered rules run.
    #[arg(long)]
    profile: Option<String>,
    /// Disable analysis/fact cache reads and writes while collecting diagnostics.
    #[arg(long)]
    no_cache: bool,
    /// Comma-separated rule selectors to show, e.g. `local/no-todo,local/*`.
    #[arg(long)]
    filter: Option<String>,
    /// Print grouped ignore statistics.
    #[arg(long)]
    stat: bool,
    /// Print one summary line.
    #[arg(long)]
    shortstat: bool,
    /// Output format.
    #[arg(long, value_enum, default_value_t = IgnoresFormatArg::Human)]
    format: IgnoresFormatArg,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ColorArg {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum FormatArg {
    Human,
    Github,
    Json,
    Sarif,
    AiFriendly,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum IgnoresFormatArg {
    Human,
    Json,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum InspectFormatArg {
    Human,
    Json,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum TestFormatArg {
    Human,
    Json,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum AgentJsonFormatArg {
    Json,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CacheStatusFormatArg {
    Human,
    Json,
}

/// Mirrors `CacheManagedCategory`; `cache_category_arg_covers_every_managed_category`
/// keeps the two lists, and their user-facing names, in step.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum CacheCategoryArg {
    All,
    Analysis,
    Layers,
    Derived,
    #[value(name = "semantic-store")]
    Semantic,
    RulesTarget,
    ExtensionsTarget,
    Review,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum FailOn {
    Warn,
    Error,
    None,
}

pub(crate) fn run() -> Result<u8> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .try_init()
        .ok();

    let cli = Cli::parse();
    match cli.command {
        Command::Init => {
            init_project(std::env::current_dir()?)?;
            Ok(0)
        }
        Command::AddSkill(args) => {
            skill::add_skill(std::env::current_dir()?, &args)?;
            Ok(0)
        }
        Command::NewRule(args) => {
            new_rule(std::env::current_dir()?, &args)?;
            Ok(0)
        }
        Command::Baseline(args) => baseline(std::env::current_dir()?, &args),
        Command::Cache(args) => cache_command(std::env::current_dir()?, &args),
        Command::Inspect(args) => inspect(std::env::current_dir()?, &args),
        Command::Facts(args) => facts(std::env::current_dir()?, &args),
        Command::Unknowns(args) => unknowns(std::env::current_dir()?, &args),
        Command::Explain(args) => explain(std::env::current_dir()?, &args),
        Command::Test(args) => test(std::env::current_dir()?, &args),
        Command::Check(args) => check(std::env::current_dir()?, &args),
        Command::Review(args) => review(std::env::current_dir()?, &args),
        Command::Ignores(args) => ignores(std::env::current_dir()?, &args),
    }
}

fn init_project(root: PathBuf) -> Result<()> {
    let config_path = root.join(".polint.toml");
    if !config_path.exists() {
        crate::repo_fs::write_repo_file_atomic(&root, ".polint.toml", default_config_toml())
            .with_context(|| format!("failed to write {}", config_path.display()))?;
    }

    let polint_dir = root.join(".polint");
    crate::repo_fs::ensure_repo_dir(&root, ".polint/rules/src")
        .with_context(|| format!("failed to create {}", polint_dir.display()))?;
    crate::repo_fs::ensure_repo_dir(&root, ".polint/cache")
        .with_context(|| format!("failed to create {}", polint_dir.join("cache").display()))?;
    crate::repo_fs::ensure_repo_dir(&root, ".polint/output")
        .with_context(|| format!("failed to create {}", polint_dir.join("output").display()))?;
    ensure_polint_nested_gitignore(&root)?;
    ensure_repo_rust_toolchain_shim(&root)?;

    println!("Initialized polint config at {}", config_path.display());
    Ok(())
}

/// Ensures `.polint/.gitignore` lists `cache/` so analysis cache stays local to each machine.
fn ensure_polint_nested_gitignore(root: &Path) -> Result<()> {
    let path = root.join(".polint/.gitignore");
    const ENTRIES: &[&str] = &["cache/", "output/"];

    if let Ok(existing) =
        crate::repo_fs::read_repo_file_to_string_with_limit(root, ".polint/.gitignore", 1_048_576)
    {
        if ENTRIES.iter().all(|entry| {
            existing
                .lines()
                .any(|line| gitignore_line_covers(line, entry))
        }) {
            return Ok(());
        }
        let mut out = existing;
        if !out.ends_with('\n') {
            out.push('\n');
        }
        for entry in ENTRIES {
            if !out.lines().any(|line| gitignore_line_covers(line, entry)) {
                out.push_str(entry);
                out.push('\n');
            }
        }
        crate::repo_fs::write_repo_file_atomic(root, ".polint/.gitignore", out)
            .with_context(|| format!("failed to update {}", path.display()))?;
    } else {
        let content = "# polint: local cache and agent output (not shared between checkouts)\ncache/\noutput/\n";
        crate::repo_fs::write_repo_file_atomic(root, ".polint/.gitignore", content)
            .with_context(|| format!("failed to write {}", path.display()))?;
    }
    Ok(())
}

/// Writes `rust-toolchain.toml` at the repo root when absent so `cargo` (invoked from `polint
/// check` with `--manifest-path .polint/rules/Cargo.toml`) picks a toolchain that can build `polint`.
fn ensure_repo_rust_toolchain_shim(root: &Path) -> Result<()> {
    let path = root.join("rust-toolchain.toml");
    if path.exists() {
        return Ok(());
    }
    let msrv = env!("CARGO_PKG_RUST_VERSION");
    fs::write(
        &path,
        format!(
            "# Polint rule packs compile the `polint` crate (MSRV Rust {msrv}).\n\
             # https://github.com/oaiz-io/polint#minimum-rust-version\n\
             [toolchain]\n\
             channel = \"{msrv}\"\n"
        ),
    )
    .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn gitignore_line_covers(line: &str, entry: &str) -> bool {
    let t = line.trim();
    if t.is_empty() || t.starts_with('#') {
        return false;
    }
    t.trim_end_matches('/') == entry.trim_end_matches('/')
}

#[derive(Debug)]
struct ScaffoldWrite {
    relative_path: PathBuf,
    contents: Vec<u8>,
    previous: Option<crate::repo_fs::RepoFileSnapshot>,
}

impl ScaffoldWrite {
    fn create(relative_path: impl Into<PathBuf>, contents: impl Into<Vec<u8>>) -> Self {
        Self {
            relative_path: relative_path.into(),
            contents: contents.into(),
            previous: None,
        }
    }

    fn replace(
        relative_path: impl Into<PathBuf>,
        contents: impl Into<Vec<u8>>,
        previous: crate::repo_fs::RepoFileSnapshot,
    ) -> Self {
        Self {
            relative_path: relative_path.into(),
            contents: contents.into(),
            previous: Some(previous),
        }
    }
}

fn new_rule(root: PathBuf, args: &NewRuleArgs) -> Result<()> {
    let rule_name = validate_rule_name(&args.rule_name)?;
    let language = RuleLanguage::parse(&args.language)?;
    if args.review && args.template.is_some() {
        anyhow::bail!("`--review` cannot be combined with `--template`");
    }
    if let Some(template) = args.template {
        validate_rule_template_language(language, template)?;
    }

    let module = rust_module_name(&rule_name);
    let writes = plan_new_rule_scaffold(&root, args, language, &rule_name, &module)?;
    commit_new_rule_scaffold(&root, &writes)?;

    let module_path = root.join(".polint/rules/src").join(format!("{module}.rs"));
    if args.review {
        // A review rule's diagnostics depend on a diff, so the static
        // clean/violating `polint check` fixtures do not apply. Review rules
        // are exercised with `polint review <ref>`.
        println!(
            "Created review rule module {} (run it with `polint review <ref>`; \
             no diff-based fixtures were generated)",
            module_path.display()
        );
    } else {
        println!("Created rule module {}", module_path.display());
    }
    Ok(())
}

fn plan_new_rule_scaffold(
    root: &Path,
    args: &NewRuleArgs,
    language: RuleLanguage,
    rule_name: &str,
    module: &str,
) -> Result<Vec<ScaffoldWrite>> {
    let module_relative = PathBuf::from(format!(".polint/rules/src/{module}.rs"));
    ensure_scaffold_destination_absent(root, &module_relative, "rule")?;

    let cargo_relative = Path::new(".polint/rules/Cargo.toml");
    let main_relative = Path::new(".polint/rules/src/main.rs");
    let cargo_previous = read_optional_scaffold_file(root, cargo_relative)?;
    let main_previous = read_optional_scaffold_file(root, main_relative)?;
    let mut writes = Vec::new();

    if cargo_previous.is_none() {
        writes.push(ScaffoldWrite::create(
            cargo_relative,
            pack_cargo_toml(&root.join(".polint/rules")),
        ));
    }

    let main_contents = match &main_previous {
        Some(previous) => {
            let existing = std::str::from_utf8(&previous.contents).with_context(|| {
                format!("{} is not valid UTF-8", root.join(main_relative).display())
            })?;
            register_rule_in_pack_main(existing, module)?
        }
        None => initial_pack_main(module),
    };
    if let Some(previous) = main_previous {
        writes.push(ScaffoldWrite::replace(
            main_relative,
            main_contents,
            previous,
        ));
    } else {
        writes.push(ScaffoldWrite::create(main_relative, main_contents));
    }

    writes.push(ScaffoldWrite::create(
        module_relative,
        rule_module_template(language.as_str(), rule_name, args.review, args.template),
    ));

    if !args.review {
        writes.extend(plan_rule_fixture_skeleton(
            root,
            language,
            rule_name,
            module,
            args.template,
        )?);
    }

    // Validate every destination before the first write. In particular, this
    // catches symlinked rule/fixture parents and regular-file ancestors before
    // Cargo.toml or main.rs can be changed.
    for write in &writes {
        let target = crate::repo_fs::repo_write_target(root, &write.relative_path)
            .with_context(|| format!("unsafe scaffold path {}", write.relative_path.display()))?;
        match fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                anyhow::bail!("refusing to write through symlink: {}", target.display());
            }
            Ok(metadata) if !metadata.is_file() => {
                anyhow::bail!("scaffold destination is not a file: {}", target.display());
            }
            Ok(_) if write.previous.is_none() => {
                anyhow::bail!("scaffold destination already exists: {}", target.display());
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if write.previous.is_some() {
                    anyhow::bail!("scaffold destination disappeared: {}", target.display());
                }
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to inspect {}", target.display()));
            }
        }
    }

    Ok(writes)
}

fn ensure_scaffold_destination_absent(root: &Path, relative_path: &Path, kind: &str) -> Result<()> {
    let target = crate::repo_fs::repo_write_target(root, relative_path)
        .with_context(|| format!("unsafe {kind} path {}", relative_path.display()))?;
    match fs::symlink_metadata(&target) {
        Ok(_) => anyhow::bail!("{kind} already exists: {}", target.display()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("failed to inspect {}", target.display())),
    }
}

fn read_optional_scaffold_file(
    root: &Path,
    relative_path: &Path,
) -> Result<Option<crate::repo_fs::RepoFileSnapshot>> {
    crate::repo_fs::read_optional_repo_file_snapshot(root, relative_path)
        .with_context(|| format!("failed to inspect {}", relative_path.display()))
}

fn commit_new_rule_scaffold(root: &Path, writes: &[ScaffoldWrite]) -> Result<()> {
    commit_new_rule_scaffold_with(root, writes, |root, write, created_directories| {
        if let Some(previous) = &write.previous {
            crate::repo_fs::write_repo_file_atomic_tracked(
                root,
                &write.relative_path,
                &write.contents,
                previous,
                created_directories,
            )
        } else {
            crate::repo_fs::write_repo_file_atomic_noclobber_tracked(
                root,
                &write.relative_path,
                &write.contents,
                created_directories,
            )
        }
    })
}

fn commit_new_rule_scaffold_with<F>(
    root: &Path,
    writes: &[ScaffoldWrite],
    mut write_file: F,
) -> Result<()>
where
    F: FnMut(
        &Path,
        &ScaffoldWrite,
        &mut Vec<crate::repo_fs::RepoCreatedDirectory>,
    ) -> std::result::Result<
        crate::repo_fs::RepoFileIdentity,
        crate::repo_fs::RepoFileReadError,
    >,
{
    let mut created_directories = Vec::new();
    let mut committed = Vec::new();
    for write in writes {
        match write_file(root, write, &mut created_directories) {
            Ok(identity) => committed.push((write, identity)),
            Err(error) => {
                let rollback = rollback_new_rule_scaffold(root, &committed, &created_directories);
                return match rollback {
                    Ok(()) => Err(error).with_context(|| {
                        format!(
                            "failed to write {}; scaffold was rolled back",
                            write.relative_path.display()
                        )
                    }),
                    Err(rollback_error) => Err(error).with_context(|| {
                        format!(
                            "failed to write {}; rollback refused a concurrent replacement or also failed: {rollback_error:#}",
                            write.relative_path.display()
                        )
                    }),
                };
            }
        }
    }
    Ok(())
}

fn rollback_new_rule_scaffold(
    root: &Path,
    committed: &[(&ScaffoldWrite, crate::repo_fs::RepoFileIdentity)],
    created_directories: &[crate::repo_fs::RepoCreatedDirectory],
) -> Result<()> {
    let mut failures = Vec::new();
    for (write, identity) in committed.iter().rev() {
        let committed_snapshot = crate::repo_fs::RepoFileSnapshot {
            identity: identity.clone(),
            contents: write.contents.clone(),
        };
        let result = if let Some(previous) = &write.previous {
            crate::repo_fs::restore_repo_file_if_matches(
                root,
                &write.relative_path,
                &committed_snapshot,
                &previous.contents,
            )
        } else {
            crate::repo_fs::remove_repo_file_if_matches(
                root,
                &write.relative_path,
                &committed_snapshot,
            )
        };
        match result {
            Ok(true) => {}
            Ok(false) => failures.push(format!(
                "{}: destination changed concurrently; preserved it",
                write.relative_path.display()
            )),
            Err(error) => failures.push(format!("{}: {error}", write.relative_path.display())),
        }
    }
    for created_directory in created_directories.iter().rev() {
        match crate::repo_fs::remove_created_repo_directory(root, created_directory) {
            Ok(true) => {}
            Ok(false) => failures.push(format!(
                "{}: directory changed concurrently or is not empty; preserved it",
                created_directory.relative_path.display()
            )),
            Err(error) => failures.push(format!(
                "{}: {error}",
                created_directory.relative_path.display()
            )),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        anyhow::bail!("{}", failures.join("; "))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuleLanguage {
    Go,
    Ts,
    Js,
    Generic,
}

impl RuleLanguage {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "go" => Ok(Self::Go),
            "ts" => Ok(Self::Ts),
            "js" => Ok(Self::Js),
            "generic" => Ok(Self::Generic),
            _ => anyhow::bail!(
                "unsupported rule language `{value}`; expected one of: go, ts, js, generic"
            ),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Go => "go",
            Self::Ts => "ts",
            Self::Js => "js",
            Self::Generic => "generic",
        }
    }

    fn fixture_file(self) -> &'static str {
        match self {
            Self::Go => "src/example.go",
            Self::Js => "src/example.js",
            Self::Ts | Self::Generic => "src/example.ts",
        }
    }
}

fn validate_rule_template_language(
    language: RuleLanguage,
    template: RuleTemplateKind,
) -> Result<()> {
    match language {
        RuleLanguage::Ts => Ok(()),
        RuleLanguage::Go
            if matches!(
                template,
                RuleTemplateKind::SensitiveWriteGuard
                    | RuleTemplateKind::TransactionCleanup
                    | RuleTemplateKind::RawReachableApi
            ) =>
        {
            Ok(())
        }
        RuleLanguage::Go => anyhow::bail!(
            "template `{}` is not available for Go yet; use `ts` or choose one of: sensitive-write-guard, transaction-cleanup, raw-reachable-api",
            template.as_kebab_case()
        ),
        RuleLanguage::Js | RuleLanguage::Generic => anyhow::bail!(
            "policy templates are currently available for `ts` and selected `go` templates, not `{}`",
            language.as_str()
        ),
    }
}

fn rust_module_name(rule_name: &str) -> String {
    rule_name.replace('-', "_")
}

fn polint_deps_path_prefix(rules_dir: &Path) -> Option<String> {
    let mut dir = rules_dir.to_path_buf();
    let mut up = 0usize;
    loop {
        if dir.join("crates/polint").is_dir() {
            return Some(format!("{}crates/", "../".repeat(up)));
        }
        if !dir.pop() {
            return None;
        }
        up += 1;
    }
}

fn enabled_language_features() -> Vec<&'static str> {
    [
        #[cfg(feature = "lang-go")]
        "lang-go",
        #[cfg(feature = "lang-typescript")]
        "lang-typescript",
    ]
    .into_iter()
    .collect()
}

fn pack_cargo_toml(rules_dir: &Path) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let features = enabled_language_features()
        .into_iter()
        .map(|feature| format!(r#""{feature}""#))
        .collect::<Vec<_>>()
        .join(", ");
    let polint_dep_line = match polint_deps_path_prefix(rules_dir) {
        Some(prefix) => format!(
            r#"polint = {{ path = "{prefix}polint", default-features = false, features = [{features}] }}"#
        ),
        None => format!(
            r#"polint = {{ version = "{version}", default-features = false, features = [{features}] }}"#
        ),
    };
    format!(
        r#"[package]
name = "polint-local-rules"
version = "{version}"
edition = "2024"
publish = false

[dependencies]
{polint_dep_line}

[workspace]
"#,
    )
}

fn initial_pack_main(module: &str) -> String {
    format!(
        r#"mod {module};

use std::process::ExitCode;

fn main() -> ExitCode {{
    polint::runner::run_cli(vec![
        {module}::{module}(),
    ])
}}
"#
    )
}

fn register_rule_in_pack_main(existing: &str, module: &str) -> Result<String> {
    let mut src = existing.to_string();
    let mod_decl = format!("mod {module};");
    if !src.contains(&mod_decl) {
        src.insert_str(0, &format!("{mod_decl}\n"));
    }

    let needle = "polint::runner::run_cli(vec![";
    let start = src
        .find(needle)
        .with_context(|| "main.rs must call polint::runner::run_cli(vec![...])")?;
    let inner_start = start + needle.len();
    let mut depth = 1u32;
    let mut end = None;
    for (i, ch) in src[inner_start..].char_indices() {
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(inner_start + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end.context("unclosed vec![ in main.rs")?;
    let inner = &src[inner_start..end];
    let trimmed = inner.trim_end();
    let new_inner = if trimmed.is_empty() {
        format!("\n        {module}::{module}(),\n    ")
    } else {
        format!("{trimmed}\n        {module}::{module}(),\n    ")
    };
    let mut new_src = String::with_capacity(src.len() + new_inner.len());
    new_src.push_str(&src[..inner_start]);
    new_src.push_str(&new_inner);
    new_src.push_str(&src[end..]);
    Ok(new_src)
}

fn validate_rule_name(name: &str) -> Result<String> {
    let sanitized = sanitize_name(name);
    let path = Path::new(name);
    if name.is_empty()
        || sanitized != name
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        anyhow::bail!(
            "rule name must be a non-empty safe path component using only ASCII letters, digits, and '-'"
        );
    }
    Ok(sanitized)
}

fn plan_rule_fixture_skeleton(
    root: &Path,
    language: RuleLanguage,
    rule_name: &str,
    module: &str,
    template: Option<RuleTemplateKind>,
) -> Result<Vec<ScaffoldWrite>> {
    let rule_tests_relative = PathBuf::from(format!(".polint/tests/rules/{module}"));
    ensure_scaffold_destination_absent(root, &rule_tests_relative, "rule fixture")?;

    let fixture_file = language.fixture_file();
    let mut writes = Vec::new();
    writes.extend(plan_rule_fixture_case(
        &rule_tests_relative.join("clean"),
        language.as_str(),
        &rule_fixture_clean_source_template(language.as_str(), template),
        &rule_fixture_manifest_template(
            rule_name,
            fixture_file,
            false,
            rule_fixture_message_contains(template),
            rule_fixture_severity(template),
        ),
    ));
    writes.extend(plan_rule_fixture_case(
        &rule_tests_relative.join("violating"),
        language.as_str(),
        &rule_fixture_violating_source_template(language.as_str(), template),
        &rule_fixture_manifest_template(
            rule_name,
            fixture_file,
            true,
            rule_fixture_message_contains(template),
            rule_fixture_severity(template),
        ),
    ));
    Ok(writes)
}

fn plan_rule_fixture_case(
    case_relative: &Path,
    language: &str,
    source: &str,
    manifest: &str,
) -> Vec<ScaffoldWrite> {
    let source_relative = match language {
        "go" => case_relative.join("src/example.go"),
        "js" => case_relative.join("src/example.js"),
        _ => case_relative.join("src/example.ts"),
    };
    let mut writes = vec![ScaffoldWrite::create(
        case_relative.join("polint-test.toml"),
        manifest.as_bytes().to_vec(),
    )];
    if language == "go" {
        writes.push(ScaffoldWrite::create(
            case_relative.join("go.mod"),
            b"module example.com/polint-rule-fixture\n\ngo 1.22\n".to_vec(),
        ));
    }
    writes.push(ScaffoldWrite::create(
        source_relative,
        source.as_bytes().to_vec(),
    ));
    writes
}

fn rule_fixture_manifest_template(
    rule_name: &str,
    fixture_file: &str,
    expect_diagnostic: bool,
    message_contains: &str,
    severity: &str,
) -> String {
    let expected = if expect_diagnostic {
        format!(
            r#"
[[expect.diagnostic]]
rule_id = "custom/{rule_name}"
file = "{fixture_file}"
severity = "{severity}"
message_contains = "{message_contains}"
"#
        )
    } else {
        String::new()
    };
    format!(
        r#"rule = "custom/{rule_name}"
paths = ["src/**"]

[expect]
{expected}
"#
    )
}

fn rule_fixture_message_contains(template: Option<RuleTemplateKind>) -> &'static str {
    template.map_or("Project-specific policy", |template| {
        policy_template_spec("ts", template).message
    })
}

fn rule_fixture_severity(template: Option<RuleTemplateKind>) -> &'static str {
    if template.is_some() { "error" } else { "warn" }
}

fn rule_fixture_clean_source_template(
    language: &str,
    template: Option<RuleTemplateKind>,
) -> String {
    if let Some(template) = template {
        return policy_template_spec(language, template)
            .clean_source
            .to_string();
    }

    match language {
        "go" => r#"package main

func main() {}
"#
        .to_string(),
        "generic" => r#"function keepMe() {
	return "allowed";
}
"#
        .to_string(),
        _ => r#"export const example = "allowed";
"#
        .to_string(),
    }
}

fn rule_fixture_violating_source_template(
    language: &str,
    template: Option<RuleTemplateKind>,
) -> String {
    if let Some(template) = template {
        return policy_template_spec(language, template)
            .violating_source
            .to_string();
    }

    match language {
        "go" => r#"package main

func replaceMe(err error) error {
	if err != nil {
		return err
	}
	return nil
}
"#
        .to_string(),
        "generic" => r#"function replaceMe() {
	return "blocked";
}
"#
        .to_string(),
        _ => r#"export const example = "replace me";
"#
        .to_string(),
    }
}

fn rule_module_template(
    language: &str,
    rule_name: &str,
    review: bool,
    template: Option<RuleTemplateKind>,
) -> String {
    if review {
        return review_rule_module_template(rule_name);
    }
    if let Some(template) = template {
        return policy_rule_module_template(language, rule_name, template);
    }

    let module = rust_module_name(rule_name);
    let fact_params = match language {
        "go" => "tests: GoTests<'_>, branches: BranchObligations<'_>",
        "ts" | "tsx" | "js" | "jsx" => "literals: StringLiterals<'_>, jsx: JsxAttributes<'_>",
        _ => "functions: Functions<'_>",
    };
    let query_example = match language {
        "go" => {
            r#"    for branch in branches.iter() {
        if branch.is_error_path && tests.related_for_file(branch.file).is_empty() {
            ctx.warn(
                &branch.decision_span,
                "Project-specific policy (heuristic): add nearby test evidence for this Go error branch.",
            );
        }
    }"#
        }
        "ts" | "tsx" | "js" | "jsx" => {
            r#"    for literal in literals.iter() {
        if literal.value == "replace me" {
            ctx.warn(&literal.span, "Project-specific policy: replace this placeholder literal.");
        }
    }
    let attribute_count = jsx.iter().count();
    let _ = attribute_count;"#
        }
        _ => {
            r#"    for function in functions.iter() {
        if function.name == "replaceMe" {
            ctx.warn(&function.span, "Project-specific policy: replace this placeholder function.");
        }
    }"#
        }
    };
    format!(
        r#"use polint::sdk::prelude::*;

#[polint::rule(
    id = "custom/{rule_name}",
    description = "Project-specific policy: {rule_name}.",
    severity = "warn"
)]
pub(crate) fn {module}(ctx: &mut RuleCtx<'_>, {fact_params}) -> RuleResult {{
{query_example}
    let _ = ctx.options().settings.len();
    Ok(())
}}
"#
    )
}

/// Scaffold a review-kind rule: `kind = "review"` plus a `ChangedFiles<'_>`
/// parameter and a glob-match loop. Run with `polint review <ref>`.
fn review_rule_module_template(rule_name: &str) -> String {
    let module = rust_module_name(rule_name);

    format!(
        r#"use polint::sdk::prelude::*;

#[polint::rule(
    id = "review/{rule_name}",
    description = "Review-only policy (heuristic): {rule_name}.",
    severity = "warn",
    kind = "review"
)]
pub(crate) fn {module}(ctx: &mut RuleCtx<'_>, changes: ChangedFiles<'_>) -> RuleResult {{
    // `changes` is the diff against the target ref passed to `polint review`.
    // It is empty under `polint check`, so this rule only fires on a review.
    let rule_id = ctx.rule_id().to_string();
    for changed in changes.iter() {{
        // Replace the glob with the paths your policy cares about.
        if changed.matches_glob("db/migrations/**") {{
            let line = changed.lines().first().map(|&(lo, _)| lo).unwrap_or(1);
            ctx.report(Diagnostic::warning(
                rule_id.clone(),
                changed.path().to_string(),
                DiagnosticRange::point(line, 1),
                "Reviewer attention required: a watched path changed.",
            ));
        }}
    }}
    Ok(())
}}
"#
    )
}

struct PolicyTemplateSpec {
    description: &'static str,
    view_param: &'static str,
    body: String,
    message: &'static str,
    clean_source: &'static str,
    violating_source: &'static str,
}

fn policy_rule_module_template(
    language: &str,
    rule_name: &str,
    template: RuleTemplateKind,
) -> String {
    let module = rust_module_name(rule_name);
    let spec = policy_template_spec(language, template);
    let PolicyTemplateSpec {
        description,
        view_param,
        body,
        ..
    } = spec;

    format!(
        r#"use polint::sdk::prelude::*;

#[polint::rule(
    id = "custom/{rule_name}",
    description = "{description}",
    severity = "error"
)]
pub(crate) fn {module}(ctx: &mut RuleCtx<'_>, {view_param}) -> RuleResult {{
{body}
    Ok(())
}}
"#
    )
}

fn policy_template_spec(language: &str, template: RuleTemplateKind) -> PolicyTemplateSpec {
    let go = matches!(language, "go");
    match template {
        RuleTemplateKind::RequestToShell => data_flow_policy_template(
            "Request data must not reach shell execution without validation.",
            "Request data reaches shell execution without validation.",
            "SourcePattern::http_request()",
            call_sink(if go { "execCommand" } else { "exec" }),
            r#"["validate_command", "allow_command"]"#,
            None,
            (
                if go {
                    GO_REQUEST_TO_SHELL_CLEAN
                } else {
                    TS_REQUEST_TO_SHELL_CLEAN
                },
                if go {
                    GO_REQUEST_TO_SHELL_VIOLATING
                } else {
                    TS_REQUEST_TO_SHELL_VIOLATING
                },
            ),
        ),
        RuleTemplateKind::SecretToLog => data_flow_policy_template(
            "Secret-like values must not reach logs without redaction.",
            "Secret-like value reaches logging without redaction.",
            r#"SourcePattern::secret_like(["token", "password", "apiKey"])"#,
            "SinkPattern::logger()",
            r#"["redact", "mask_secret"]"#,
            Some("Heuristic"),
            (
                if go {
                    GO_SECRET_TO_LOG_CLEAN
                } else {
                    TS_SECRET_TO_LOG_CLEAN
                },
                if go {
                    GO_SECRET_TO_LOG_VIOLATING
                } else {
                    TS_SECRET_TO_LOG_VIOLATING
                },
            ),
        ),
        RuleTemplateKind::PiiToAnalytics => data_flow_policy_template(
            "PII-like values must not reach analytics calls without anonymization.",
            "PII-like value reaches analytics without anonymization.",
            r#"SourcePattern::secret_like(["email", "user_id", "phone"])"#,
            call_sink(if go {
                "trackAnalytics"
            } else {
                "analyticsTrack"
            }),
            r#"["anonymize", "hash_user_id"]"#,
            Some("Heuristic"),
            (
                if go {
                    GO_PII_TO_ANALYTICS_CLEAN
                } else {
                    TS_PII_TO_ANALYTICS_CLEAN
                },
                if go {
                    GO_PII_TO_ANALYTICS_VIOLATING
                } else {
                    TS_PII_TO_ANALYTICS_VIOLATING
                },
            ),
        ),
        RuleTemplateKind::SensitiveWriteGuard => control_guard_policy_template(
            "Sensitive write calls require a validation or authorization guard first.",
            "Sensitive write requires validation or authorization first.",
            "writeBalance",
            r#"["authorize", "validate_payment"]"#,
            if go {
                GO_SENSITIVE_WRITE_GUARD_CLEAN
            } else {
                TS_SENSITIVE_WRITE_GUARD_CLEAN
            },
            if go {
                GO_SENSITIVE_WRITE_GUARD_VIOLATING
            } else {
                TS_SENSITIVE_WRITE_GUARD_VIOLATING
            },
        ),
        RuleTemplateKind::TransactionCleanup => control_cleanup_policy_template(
            "Transactions must be cleaned up after they are opened.",
            "Transaction begin requires rollback or cleanup.",
            if go { "Begin" } else { "beginTransaction" },
            if go { "Rollback" } else { "rollback" },
            if go {
                GO_TRANSACTION_CLEANUP_CLEAN
            } else {
                TS_TRANSACTION_CLEANUP_CLEAN
            },
            if go {
                GO_TRANSACTION_CLEANUP_VIOLATING
            } else {
                TS_TRANSACTION_CLEANUP_VIOLATING
            },
        ),
        RuleTemplateKind::RawReachableApi => calls_policy_template(
            "Raw internal APIs must not be reachable from production roots.",
            "Raw internal API is reachable from this root.",
            "dangerousAdmin",
            "main",
            if go {
                GO_RAW_REACHABLE_API_CLEAN
            } else {
                TS_RAW_REACHABLE_API_CLEAN
            },
            if go {
                GO_RAW_REACHABLE_API_VIOLATING
            } else {
                TS_RAW_REACHABLE_API_VIOLATING
            },
        ),
        RuleTemplateKind::Ssrf => data_flow_policy_template(
            "Request-controlled URLs must not reach outbound fetches without allowlisting.",
            "Request-controlled URL reaches outbound fetch without allowlisting.",
            "SourcePattern::http_request()",
            call_sink(if go { "fetchURL" } else { "fetchUrl" }),
            r#"["allowlist_url", "validate_url"]"#,
            None,
            (
                if go { GO_SSRF_CLEAN } else { TS_SSRF_CLEAN },
                if go {
                    GO_SSRF_VIOLATING
                } else {
                    TS_SSRF_VIOLATING
                },
            ),
        ),
        RuleTemplateKind::DangerousHtml => data_flow_policy_template(
            "Request data must not reach HTML sinks without escaping.",
            "Request data reaches an HTML sink without escaping.",
            "SourcePattern::http_request()",
            call_sink(if go { "renderHTML" } else { "setInnerHTML" }),
            r#"["escape_html", "sanitize_html"]"#,
            None,
            (
                if go {
                    GO_DANGEROUS_HTML_CLEAN
                } else {
                    TS_DANGEROUS_HTML_CLEAN
                },
                if go {
                    GO_DANGEROUS_HTML_VIOLATING
                } else {
                    TS_DANGEROUS_HTML_VIOLATING
                },
            ),
        ),
        RuleTemplateKind::UnsafeDeserialization => data_flow_policy_template(
            "Request data must not reach unsafe deserialization without validation.",
            "Request data reaches unsafe deserialization without validation.",
            "SourcePattern::http_request()",
            call_sink("unsafeDeserialize"),
            r#"["validate_payload", "verify_schema"]"#,
            None,
            (
                if go {
                    GO_UNSAFE_DESERIALIZATION_CLEAN
                } else {
                    TS_UNSAFE_DESERIALIZATION_CLEAN
                },
                if go {
                    GO_UNSAFE_DESERIALIZATION_VIOLATING
                } else {
                    TS_UNSAFE_DESERIALIZATION_VIOLATING
                },
            ),
        ),
        RuleTemplateKind::UserFilePath => data_flow_policy_template(
            "Request-controlled paths must not reach file APIs without validation.",
            "Request-controlled path reaches a file API without validation.",
            "SourcePattern::http_request()",
            call_sink("readFile"),
            r#"["validate_path", "safe_join"]"#,
            None,
            (
                if go {
                    GO_USER_FILE_PATH_CLEAN
                } else {
                    TS_USER_FILE_PATH_CLEAN
                },
                if go {
                    GO_USER_FILE_PATH_VIOLATING
                } else {
                    TS_USER_FILE_PATH_VIOLATING
                },
            ),
        ),
    }
}

fn data_flow_policy_template(
    description: &'static str,
    message: &'static str,
    source: &'static str,
    sink: impl Into<String>,
    barriers: &'static str,
    minimum_precision: Option<&'static str>,
    fixtures: (&'static str, &'static str),
) -> PolicyTemplateSpec {
    let sink = sink.into();
    let (clean_source, violating_source) = fixtures;
    let minimum_precision = minimum_precision
        .map(|precision| format!("    query.minimum_precision = PolicyPrecision::{precision};\n"))
        .unwrap_or_default();
    PolicyTemplateSpec {
        description,
        view_param: "flow: DataFlow<'_>",
        body: format!(
            r#"    let mut query = FlowQuery::new(
        {source},
        {sink},
    );
    query.barriers = BarrierPattern::call_any({barriers});
{minimum_precision}    query.max_depth = 24;
    query.max_paths = 128;

    for violation in flow.forbidden(query) {{
        ctx.report(violation.diagnostic(
            ctx.rule_id(),
            "{message}",
        ));
    }}
"#
        ),
        message,
        clean_source,
        violating_source,
    }
}

fn control_guard_policy_template(
    description: &'static str,
    message: &'static str,
    event: &'static str,
    guards: &'static str,
    clean_source: &'static str,
    violating_source: &'static str,
) -> PolicyTemplateSpec {
    PolicyTemplateSpec {
        description,
        view_param: "control: ControlFlow<'_>",
        body: format!(
            r#"    let mut query = GuardQuery::new(
        EventPattern::call("{event}"),
        GuardPattern::call_any({guards}),
    );
    query.max_paths = 10;

    for violation in control.missing_guard(query) {{
        ctx.report(violation.diagnostic(
            ctx.rule_id(),
            "{message}",
        ));
    }}
"#
        ),
        message,
        clean_source,
        violating_source,
    }
}

fn control_cleanup_policy_template(
    description: &'static str,
    message: &'static str,
    start: &'static str,
    cleanup: &'static str,
    clean_source: &'static str,
    violating_source: &'static str,
) -> PolicyTemplateSpec {
    PolicyTemplateSpec {
        description,
        view_param: "control: ControlFlow<'_>",
        body: format!(
            r#"    let mut query = LifecycleQuery::new(
        EventPattern::call("{start}"),
        EventPattern::call("{cleanup}"),
    );
    query.max_paths = 10;

    for violation in control.missing_cleanup(query) {{
        ctx.report(violation.diagnostic(
            ctx.rule_id(),
            "{message}",
        ));
    }}
"#
        ),
        message,
        clean_source,
        violating_source,
    }
}

fn calls_policy_template(
    description: &'static str,
    message: &'static str,
    target: &'static str,
    root: &'static str,
    clean_source: &'static str,
    violating_source: &'static str,
) -> PolicyTemplateSpec {
    PolicyTemplateSpec {
        description,
        view_param: "calls: Calls<'_>",
        body: format!(
            r#"    let completeness_status = ctx.completeness().status_for("calls");
    if completeness_status != CapabilityCompletenessStatus::Complete {{
        let reason = ctx
            .completeness()
            .reason_for("calls")
            .unwrap_or("completeness information is unavailable")
            .to_string();
        ctx.report(
            Diagnostic::warning(
                ctx.rule_id(),
                "<workspace>",
                DiagnosticRange::point(1, 1),
                format!(
                    "Calls analysis is incomplete ({{}}).",
                    completeness_status.as_str()
                ),
            )
            .with_evidence("analysis_completeness", completeness_status.as_str())
            .with_evidence("analysis_completeness_reason", reason),
        );
    }}

    let mut query = ReachQuery::new(EventPattern::call("{target}"));
    query.roots = vec![EventPattern::call("{root}")];
    query.max_depth = 8;
    query.max_paths = 10;

    for violation in calls.forbidden_reachable(query) {{
        ctx.report(violation.diagnostic(
            ctx.rule_id(),
            "{message}",
        ));
    }}
"#
        ),
        message,
        clean_source,
        violating_source,
    }
}

fn call_sink(target: &'static str) -> String {
    format!(r#"SinkPattern::call("{target}")"#)
}

const TS_REQUEST_TO_SHELL_CLEAN: &str = r#"import express from "express";
const app = express();

function validate_command(command: string): string { return command; }
function exec(command: string) {}

app.get("/run", function handler(req, res) {
  // Request input is validated before reaching the shell wrapper.
  exec(validate_command(String(req.query.cmd)));
});
"#;

const TS_REQUEST_TO_SHELL_VIOLATING: &str = r#"import express from "express";
const app = express();

function exec(command: string) {}

app.get("/run", function handler(req, res) {
  // Policy violation: raw query-string data reaches shell execution.
  exec(String(req.query.cmd));
});
"#;

const TS_SECRET_TO_LOG_CLEAN: &str = r#"function redact(value: string): string { return value; }

export function handler(token: string) {
  // Token-like input is redacted before it is logged.
  console.log(redact(token));
}
"#;

const TS_SECRET_TO_LOG_VIOLATING: &str = r#"export function handler(token: string) {
  // Policy violation: token-like input reaches a logger unchanged.
  console.log(token);
}
"#;

const TS_PII_TO_ANALYTICS_CLEAN: &str = r#"function analyticsTrack(value: string) {}
function anonymize(value: string): string { return value; }

export function handler(email: string) {
  // User identifiers are anonymized before analytics ingestion.
  analyticsTrack(anonymize(email));
}
"#;

const TS_PII_TO_ANALYTICS_VIOLATING: &str = r#"function analyticsTrack(value: string) {}

export function handler(email: string) {
  // Policy violation: raw PII-like input reaches analytics.
  analyticsTrack(email);
}
"#;

const TS_SENSITIVE_WRITE_GUARD_CLEAN: &str = r#"function authorize() {}
function writeBalance() {}

export function handler() {
  // Authorization happens before the sensitive write.
  authorize();
  writeBalance();
}
"#;

const TS_SENSITIVE_WRITE_GUARD_VIOLATING: &str = r#"function writeBalance() {}

export function handler() {
  // Policy violation: sensitive write has no prior guard in this function.
  writeBalance();
}
"#;

const TS_TRANSACTION_CLEANUP_CLEAN: &str = r#"function beginTransaction() {}
function rollback() {}

export function handler() {
  // The opened transaction is cleaned up before the function exits.
  beginTransaction();
  rollback();
}
"#;

const TS_TRANSACTION_CLEANUP_VIOLATING: &str = r#"function beginTransaction() {}

export function handler() {
  // Policy violation: the transaction is opened without cleanup.
  beginTransaction();
}
"#;

const TS_RAW_REACHABLE_API_CLEAN: &str = r#"function safeAdmin() {}

export function main() {
  // Production root reaches only the safe wrapper.
  safeAdmin();
}
"#;

const TS_RAW_REACHABLE_API_VIOLATING: &str = r#"function dangerousAdmin() {}
function handler() { dangerousAdmin(); }

export function main() {
  // Policy violation: production root reaches the raw admin API.
  handler();
}
"#;

const TS_SSRF_CLEAN: &str = r#"import express from "express";
const app = express();

function allowlist_url(url: string): string { return url; }
function fetchUrl(url: string) {}

app.get("/fetch", function handler(req, res) {
  // Request-controlled URL is allowlisted before outbound fetch.
  fetchUrl(allowlist_url(String(req.query.url)));
});
"#;

const TS_SSRF_VIOLATING: &str = r#"import express from "express";
const app = express();

function fetchUrl(url: string) {}

app.get("/fetch", function handler(req, res) {
  // Policy violation: request-controlled URL reaches outbound fetch.
  fetchUrl(String(req.query.url));
});
"#;

const TS_DANGEROUS_HTML_CLEAN: &str = r#"import express from "express";
const app = express();

function setInnerHTML(html: string) {}
function sanitize_html(html: string): string { return html; }

app.post("/preview", function handler(req, res) {
  // Request HTML is sanitized before it reaches the DOM sink.
  setInnerHTML(sanitize_html(String(req.body.html)));
});
"#;

const TS_DANGEROUS_HTML_VIOLATING: &str = r#"import express from "express";
const app = express();

function setInnerHTML(html: string) {}

app.post("/preview", function handler(req, res) {
  // Policy violation: request body reaches a raw HTML sink.
  setInnerHTML(String(req.body.html));
});
"#;

const TS_UNSAFE_DESERIALIZATION_CLEAN: &str = r#"import express from "express";
const app = express();

function unsafeDeserialize(raw: string) {}
function verify_schema(raw: string): string { return raw; }

app.post("/load", function handler(req, res) {
  // Request payload is schema-checked before deserialization.
  unsafeDeserialize(verify_schema(String(req.body.payload)));
});
"#;

const TS_UNSAFE_DESERIALIZATION_VIOLATING: &str = r#"import express from "express";
const app = express();

function unsafeDeserialize(raw: string) {}

app.post("/load", function handler(req, res) {
  // Policy violation: unverified request body reaches deserialization.
  unsafeDeserialize(String(req.body.payload));
});
"#;

const TS_USER_FILE_PATH_CLEAN: &str = r#"import express from "express";
const app = express();

function readFile(path: string) {}
function validate_path(path: string): string { return path; }

app.get("/file", function handler(req, res) {
  // Request path is normalized/validated before file access.
  readFile(validate_path(String(req.query.path)));
});
"#;

const TS_USER_FILE_PATH_VIOLATING: &str = r#"import express from "express";
const app = express();

function readFile(path: string) {}

app.get("/file", function handler(req, res) {
  // Policy violation: request-controlled path reaches file access.
  readFile(String(req.query.path));
});
"#;

const GO_REQUEST_TO_SHELL_CLEAN: &str = r#"package main

func validate_command(command string) string { return command }

func handler(command string) {
	safe := validate_command(command)
	_ = safe
}
"#;

const GO_REQUEST_TO_SHELL_VIOLATING: &str = r#"package main

func execCommand(command string) {}

func handler(command string) {
	execCommand(command)
}
"#;

const GO_SECRET_TO_LOG_CLEAN: &str = r#"package main

func log(value string) {}
func redact(value string) string { return value }

func handler(token string) {
	redacted := redact(token)
	log("redacted")
	_ = redacted
}
"#;

const GO_SECRET_TO_LOG_VIOLATING: &str = r#"package main

func log(value string) {}

func handler(token string) {
	log(token)
}
"#;

const GO_PII_TO_ANALYTICS_CLEAN: &str = r#"package main

func trackAnalytics(value string) {}
func anonymize(value string) string { return value }

func handler(email string) {
	anonymous := anonymize(email)
	trackAnalytics("anonymous")
	_ = anonymous
}
"#;

const GO_PII_TO_ANALYTICS_VIOLATING: &str = r#"package main

func trackAnalytics(value string) {}

func handler(email string) {
	trackAnalytics(email)
}
"#;

const GO_SENSITIVE_WRITE_GUARD_CLEAN: &str = r#"package main

func authorize() {}
func writeBalance() {}

func handler() {
	// Authorization happens before the sensitive write.
	authorize()
	writeBalance()
}
"#;

const GO_SENSITIVE_WRITE_GUARD_VIOLATING: &str = r#"package main

func writeBalance() {}

func handler() {
	// Policy violation: sensitive write has no prior guard in this function.
	writeBalance()
}
"#;

const GO_TRANSACTION_CLEANUP_CLEAN: &str = r#"package main

func Begin() {}
func Rollback() {}

func handler() {
	// The opened transaction is cleaned up before the function exits.
	Begin()
	Rollback()
}
"#;

const GO_TRANSACTION_CLEANUP_VIOLATING: &str = r#"package main

func Begin() {}

func handler() {
	// Policy violation: the transaction is opened without cleanup.
	Begin()
}
"#;

const GO_RAW_REACHABLE_API_CLEAN: &str = r#"package main

func main() {
	// Production root reaches only the safe wrapper.
	safeAdmin()
}

func safeAdmin() {}
"#;

const GO_RAW_REACHABLE_API_VIOLATING: &str = r#"package main

func main() {
	// Policy violation: production root reaches the raw admin API.
	handler()
}

func handler() {
	dangerousAdmin()
}

func dangerousAdmin() {}
"#;

const GO_SSRF_CLEAN: &str = r#"package main

func allowlist_url(url string) string { return url }

func handler(url string) {
	safe := allowlist_url(url)
	_ = safe
}
"#;

const GO_SSRF_VIOLATING: &str = r#"package main

func fetchURL(url string) {}

func handler(url string) {
	fetchURL(url)
}
"#;

const GO_DANGEROUS_HTML_CLEAN: &str = r#"package main

func renderHTML(html string) {}
func sanitize_html(html string) string { return html }

func handler(html string) {
	safe := sanitize_html(html)
	renderHTML("<p>safe preview</p>")
	_ = safe
}
"#;

const GO_DANGEROUS_HTML_VIOLATING: &str = r#"package main

func renderHTML(html string) {}

func handler(html string) {
	renderHTML(html)
}
"#;

const GO_UNSAFE_DESERIALIZATION_CLEAN: &str = r#"package main

func unsafeDeserialize(raw string) {}
func verify_schema(raw string) string { return raw }

func handler(payload string) {
	safe := verify_schema(payload)
	unsafeDeserialize("{}")
	_ = safe
}
"#;

const GO_UNSAFE_DESERIALIZATION_VIOLATING: &str = r#"package main

func unsafeDeserialize(raw string) {}

func handler(payload string) {
	unsafeDeserialize(payload)
}
"#;

const GO_USER_FILE_PATH_CLEAN: &str = r#"package main

func readFile(path string) {}
func validate_path(path string) string { return path }

func handler(path string) {
	safe := validate_path(path)
	readFile("/tmp/safe.txt")
	_ = safe
}
"#;

const GO_USER_FILE_PATH_VIOLATING: &str = r#"package main

func readFile(path string) {}

func handler(path string) {
	readFile(path)
}
"#;

fn baseline(root: PathBuf, args: &BaselineArgs) -> Result<u8> {
    match &args.command {
        BaselineCommand::Create(create_args) => create_baseline(root, create_args),
        BaselineCommand::Update(update_args) => update_baseline(root, update_args),
    }
}

fn create_baseline(root: PathBuf, args: &BaselineCreateArgs) -> Result<u8> {
    let output = baseline_path(&root);
    if output.exists() && !args.force {
        anyhow::bail!(
            "baseline file already exists: {}; use --force to overwrite or `polint baseline update`",
            output.display()
        );
    }

    let diagnostics = collect_diagnostics_for_baseline(
        &root,
        &args.paths,
        args.profile.as_deref(),
        args.no_cache,
    )?;
    let config = BaselineConfig::from_diagnostics(&diagnostics);
    write_baseline(&root, &config)?;
    println!(
        "Created baseline {} with {} entries",
        output.display(),
        config.baseline.len()
    );
    Ok(0)
}

fn update_baseline(root: PathBuf, args: &BaselineUpdateArgs) -> Result<u8> {
    let baseline_path = baseline_path(&root);
    let config = load_baseline(&root)?;
    let diagnostics = collect_diagnostics_for_baseline(
        &root,
        &args.paths,
        args.profile.as_deref(),
        args.no_cache,
    )?;
    let classification = classify_diagnostics(&diagnostics, &config);
    let updated = config.updated_for_current_diagnostics(&diagnostics);
    write_baseline(&root, &updated)?;
    println!(
        "Updated baseline {} with {} entries",
        baseline_path.display(),
        updated.baseline.len()
    );
    print!("{}", render_baseline_summary(&classification.summary));
    Ok(0)
}

fn cache_command(root: PathBuf, args: &CacheArgs) -> Result<u8> {
    let layout = CacheLayout::for_repo(&root);
    match &args.command {
        CacheCommand::Status(status_args) => cache_status(&layout, status_args),
        CacheCommand::Prune(prune_args) => cache_prune(&layout, prune_args),
        CacheCommand::Clean(clean_args) => cache_clean(&layout, clean_args),
    }
}

fn inspect(root: PathBuf, args: &InspectArgs) -> Result<u8> {
    match &args.command {
        InspectCommand::Rule(rule_args) => inspect_rule(&root, rule_args),
        InspectCommand::Unknowns(unknowns_args) => inspect_unknowns(root, unknowns_args),
    }
}

fn facts(root: PathBuf, args: &FactsArgs) -> Result<u8> {
    match &args.command {
        FactsCommand::List(list_args) => facts_list(list_args),
        FactsCommand::Sample(sample_args) => facts_sample(&root, sample_args),
    }
}

fn facts_list(_args: &FactsListArgs) -> Result<u8> {
    println!("{}", serde_json::to_string_pretty(&FactsListReport::new())?);
    Ok(0)
}

fn facts_sample(root: &Path, args: &FactsSampleArgs) -> Result<u8> {
    let limit = args.limit.min(100);
    let support = public_fact_view(args.capability.as_str())
        .ok_or_else(|| anyhow::anyhow!("unknown public fact capability `{}`", args.capability))?;
    if !support.sampling {
        anyhow::bail!(
            "public fact capability `{}` is reserved and does not support sampling yet; see {}",
            args.capability,
            support.docs_path
        );
    }
    let analysis = analyze_for_agent_json(
        root,
        &args.paths,
        args.no_cache,
        &[args.capability.as_str()],
    )?;
    let db = &analysis.db;
    let mut rows = match args.capability.as_str() {
        "resolved_imports" => db
            .resolved_imports()
            .iter()
            .map(|fact| {
                PublicFactSampleRow::new(
                    db.path_for(fact.from_file),
                    None,
                    status_label(fact.status),
                    Some(resolution_precision_label(fact.precision).to_string()),
                    Some(format!("resolved_import:{}", fact.id.0)),
                )
            })
            .collect(),
        "module_graph" => db
            .module_edges()
            .iter()
            .map(|edge| {
                PublicFactSampleRow::new(
                    "<module-graph>".to_string(),
                    None,
                    status_label(edge.status),
                    None,
                    Some(format!("module_edge:{}", edge.id.0)),
                )
            })
            .collect(),
        "symbols" => db
            .symbols()
            .iter()
            .map(|symbol| {
                PublicFactSampleRow::new(
                    symbol
                        .file
                        .map(|file| db.path_for(file))
                        .unwrap_or_else(|| "<workspace>".to_string()),
                    symbol.primary_span.as_ref().map(span_start),
                    "present",
                    Some(symbol_precision_label(symbol.precision).to_string()),
                    Some(db.resolve_stable_key(symbol.stable_key).to_string()),
                )
            })
            .collect(),
        "references" => db
            .references()
            .iter()
            .map(|reference| {
                PublicFactSampleRow::new(
                    reference
                        .file
                        .map(|file| db.path_for(file))
                        .unwrap_or_else(|| "<workspace>".to_string()),
                    reference.primary_span.as_ref().map(span_start),
                    symbol_status_label(reference.status),
                    Some(symbol_precision_label(reference.precision).to_string()),
                    Some(db.resolve_stable_key(reference.stable_key).to_string()),
                )
            })
            .collect(),
        "file_metrics" => db
            .file_metrics()
            .iter()
            .map(|metric| {
                PublicFactSampleRow::new(
                    db.path_for(metric.file),
                    None,
                    "present",
                    None,
                    Some(format!(
                        "lines:{};bytes:{};functions:{}",
                        metric.line_count, metric.byte_count, metric.function_count
                    )),
                )
            })
            .collect(),
        "function_metrics" => db
            .function_metrics()
            .iter()
            .map(|metric| {
                PublicFactSampleRow::new(
                    db.path_for(metric.file),
                    Some(span_start(&metric.span)),
                    "present",
                    None,
                    Some(format!(
                        "function:{};lines:{};bytes:{}",
                        metric.name, metric.line_count, metric.byte_count
                    )),
                )
            })
            .collect(),
        "complexity_metrics" => db
            .complexity_metrics()
            .iter()
            .map(|metric| {
                PublicFactSampleRow::new(
                    db.path_for(metric.file),
                    Some(span_start(&metric.span)),
                    "present",
                    None,
                    Some(format!(
                        "function:{};cyclomatic_complexity:{}",
                        metric.name, metric.cyclomatic_complexity
                    )),
                )
            })
            .collect(),
        _ => Vec::new(),
    };
    rows.sort_by(|left, right| {
        (
            left.file.as_str(),
            left.span.as_ref().map(|span| (span.line, span.column)),
            left.stable_id.as_deref().unwrap_or_default(),
        )
            .cmp(&(
                right.file.as_str(),
                right.span.as_ref().map(|span| (span.line, span.column)),
                right.stable_id.as_deref().unwrap_or_default(),
            ))
    });
    rows.truncate(limit);
    let report = FactsSampleReport {
        version: 1,
        schema: POLINT_FACTS_JSON_SCHEMA_V1_URL.to_string(),
        tool: polint_tool_info(),
        capability: args.capability.clone(),
        limit,
        sampled: rows.len(),
        rows,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(0)
}

fn unknowns(root: PathBuf, args: &UnknownsArgs) -> Result<u8> {
    let support = public_fact_view(&args.capability);
    if support
        .as_ref()
        .is_none_or(|view| !view_supports_unknowns(view))
    {
        let row = crate::analysis::unknown_taxonomy::collect::unsupported_capability_row(
            &args.capability,
            support.map(|view| view.docs_path),
        );
        let report = UnknownsReport {
            version: 1,
            schema: POLINT_UNKNOWNS_JSON_SCHEMA_V1_URL.to_string(),
            tool: polint_tool_info(),
            capability: args.capability.clone(),
            rows: vec![UnknownsRow::from_taxonomy_compat(row)],
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(2);
    }

    let analysis = analyze_for_agent_json(
        &root,
        &args.paths,
        args.no_cache,
        &[args.capability.as_str()],
    )?;
    // A resource-budget stop is run-level: it is why the run could not finish,
    // so it belongs in every capability's answer, not only the
    // all-capabilities view.
    let mut taxonomy_rows = crate::analysis::unknown_taxonomy::collect::public_capability_unknowns(
        &analysis.db,
        &args.capability,
    );
    taxonomy_rows.extend(
        crate::analysis::unknown_taxonomy::collect::resource_budget_unknowns(&analysis.diagnostics),
    );
    let rows = crate::analysis::unknown_taxonomy::facts::normalize_rows(taxonomy_rows)
        .into_iter()
        .map(UnknownsRow::from_taxonomy_compat)
        .collect();
    let report = UnknownsReport {
        version: 1,
        schema: POLINT_UNKNOWNS_JSON_SCHEMA_V1_URL.to_string(),
        tool: polint_tool_info(),
        capability: args.capability.clone(),
        rows,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(0)
}

fn inspect_unknowns(root: PathBuf, args: &InspectUnknownsArgs) -> Result<u8> {
    match args.format {
        AgentJsonFormatArg::Json => {}
    }
    if let Some(capability) = &args.capability {
        let support = public_fact_view(capability);
        if support
            .as_ref()
            .is_none_or(|view| !view_supports_unknowns(view))
        {
            let row = crate::analysis::unknown_taxonomy::collect::unsupported_capability_row(
                capability,
                support.map(|view| view.docs_path),
            );
            let report = UnknownsReport {
                version: 1,
                schema: POLINT_UNKNOWNS_JSON_SCHEMA_V1_URL.to_string(),
                tool: polint_tool_info(),
                capability: capability.clone(),
                rows: vec![UnknownsRow::from_taxonomy_full(row)],
            };
            println!("{}", serde_json::to_string_pretty(&report)?);
            return Ok(2);
        }
    }

    let requested_caps = args
        .capability
        .as_deref()
        .map(|capability| vec![capability])
        .unwrap_or_else(|| {
            crate::analysis::unknown_taxonomy::collect::PUBLIC_UNKNOWN_CAPABILITIES.to_vec()
        });
    let analysis = analyze_for_agent_json(&root, &args.paths, args.no_cache, &requested_caps)?;
    let rows = if let Some(capability) = &args.capability {
        // A resource-budget stop is run-level: it is why the run could not
        // finish, so it belongs in every capability's answer, not only the
        // all-capabilities view.
        let mut rows = crate::analysis::unknown_taxonomy::collect::public_capability_unknowns(
            &analysis.db,
            capability,
        );
        rows.extend(
            crate::analysis::unknown_taxonomy::collect::resource_budget_unknowns(
                &analysis.diagnostics,
            ),
        );
        crate::analysis::unknown_taxonomy::facts::normalize_rows(rows)
    } else {
        crate::analysis::unknown_taxonomy::collect::all_unknowns_with_diagnostics(
            &analysis.db,
            &analysis.diagnostics,
        )
    }
    .into_iter()
    .map(UnknownsRow::from_taxonomy_full)
    .collect();
    let report = UnknownsReport {
        version: 1,
        schema: POLINT_UNKNOWNS_JSON_SCHEMA_V1_URL.to_string(),
        tool: polint_tool_info(),
        capability: args.capability.clone().unwrap_or_else(|| "all".to_string()),
        rows,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(0)
}

fn view_supports_unknowns(view: &PublicFactView) -> bool {
    view.unknowns && matches!(view.stability, "stable" | "preview")
}

fn explain(root: PathBuf, args: &ExplainArgs) -> Result<u8> {
    let manifests = discover_local_rule_hosts(&root)?;
    let mut rules = Vec::new();
    for manifest in &manifests {
        let host_path = local_manifest_path(&root, manifest);
        let report = run_local_rule_host_inspect(&root, manifest)?;
        rules.extend(
            report
                .rules
                .into_iter()
                .map(|rule| rule.with_host_path(host_path.clone())),
        );
    }
    if let Some(rule) = &args.rule {
        rules.retain(|candidate| candidate.rule_id == *rule);
        if rules.is_empty() {
            anyhow::bail!("no registered rule matched `{rule}`");
        }
    }
    let report = ExplainReport {
        version: 1,
        schema: POLINT_EXPLAIN_JSON_SCHEMA_V1_URL.to_string(),
        tool: polint_tool_info(),
        scope: args.rule.clone().unwrap_or_else(|| "all_rules".to_string()),
        rules: rules
            .into_iter()
            .map(|rule| ExplainRuleRow {
                rule_id: rule.rule_id,
                host_path: rule.host_path,
                fact_views: rule
                    .fact_views
                    .into_iter()
                    .map(|view| view.view_type)
                    .collect(),
                capabilities: rule.capability_support,
            })
            .collect(),
        public_capabilities: FactsListReport::new().views,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(0)
}

/// Internal (NON-public) view of a derived edge's provenance, surfaced through the
/// existing private plumbing reached from the [`explain`] command (D-10). This is
/// deliberately NOT added to the public `ExplainReport`/`ExplainRuleRow` JSON schema
/// — the only new public CLI surface in v1.3 is `polint inspect unknowns`.
/// All fields mirror the `pub(crate)` `DerivedEdgeProvenance`; nothing here reaches
/// `polint::sdk::prelude` (the leak gate stays green).
///
/// Today this is a `cfg(test)`-facing internal accessor: no PRODUCTION explain path
/// consumes derived edges yet. The test-exercised internal seam keeps the
/// unknown-taxonomy public surface unchanged. D-10 explicitly sanctions a
/// test-exercised internal seam — the plumbing exists and is locked by a unit test.
// `allow` (not `expect`): the struct is only constructed by the test-exercised
// `explain_derived_edge_provenance` seam (D-10); whether dead_code fires
// varies by build config, so an `expect` would be reported unfulfilled.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DerivedEdgeProvenanceView {
    /// The derived edge's stable key.
    pub(crate) edge_stable_key_text: String,
    /// The contributing fact stable keys, totally ordered by stable ID (D-08).
    pub(crate) contributing_fact_keys: Vec<String>,
    /// The producing constraint kind (`ConstraintKind::as_str()` label).
    pub(crate) constraint_kind: String,
    /// The monotonic solver step at which the edge was derived.
    pub(crate) solver_step: u64,
}

/// Private plumbing (D-10): for a derived edge identified by its `edge_stable_key`,
/// surface its contributing facts + constraint kind + solver step from the unified
/// [`crate::analysis::solver::store::SolverStore`]. Returns `None` if no derived edge
/// with that stable key exists.
///
/// This extends the EXISTING private/explain plumbing reached from [`explain`]; it
/// adds NO public JSON field. The view is `pub(crate)` and is exercised by a unit
/// test, keeping the provenance types internal (leak gate green).
//
// `allow` (not `expect`): whether dead_code fires for this `pub(crate)` fn varies by
// build config (it is live under `cfg(test)`), so an `expect` would be reported
// unfulfilled in test builds. D-10 sanctions a test-exercised internal seam
// until a production explain path consumes it.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn explain_derived_edge_provenance(
    store: &crate::analysis::solver::store::SolverStore,
    interner: &crate::core::StableKeyInterner,
    edge_stable_key: &str,
) -> Option<DerivedEdgeProvenanceView> {
    store
        .derived_edges()
        .iter()
        .find(|edge| interner.resolve(edge.stable_key).as_ref() == edge_stable_key)
        .map(|edge| DerivedEdgeProvenanceView {
            edge_stable_key_text: interner.resolve(edge.stable_key).to_string(),
            contributing_fact_keys: edge
                .provenance
                .contributing_facts
                .iter()
                .map(|fact| interner.resolve(fact.stable_key).to_string())
                .collect(),
            constraint_kind: edge.provenance.constraint_kind.clone(),
            solver_step: edge.provenance.solver_step,
        })
}

fn inspect_rule(root: &Path, args: &InspectRuleArgs) -> Result<u8> {
    let manifests = discover_local_rule_hosts(root)?;
    let mut rules = Vec::new();
    for manifest in &manifests {
        let host_path = local_manifest_path(root, manifest);
        let report = run_local_rule_host_inspect(root, manifest)?;
        rules.extend(
            report
                .rules
                .into_iter()
                .map(|rule| rule.with_host_path(host_path.clone())),
        );
    }
    if let Some(pattern) = &args.rule {
        rules.retain(|rule| rule_id_matches(pattern, &rule.rule_id));
        if rules.is_empty() {
            anyhow::bail!("no registered rule matched `{pattern}`");
        }
    }

    let report = InspectRuleReport::new("polint", env!("CARGO_PKG_VERSION"), rules);
    match args.format {
        InspectFormatArg::Human => print!("{}", render_inspect_rule_human(&report)),
        InspectFormatArg::Json => println!("{}", serde_json::to_string_pretty(&report)?),
    }
    Ok(0)
}

fn test(root: PathBuf, args: &TestArgs) -> Result<u8> {
    let report = run_rule_tests(
        &root,
        RuleTestOptions {
            rule: args.rule.clone(),
            case: args.case.clone(),
            no_cache: args.no_cache,
            keep_temp: args.keep_temp,
            rule_host_manifests: discover_local_rule_hosts(&root)?,
        },
    )?;
    match args.format {
        TestFormatArg::Human => print!("{}", render_rule_test_human(&report)),
        TestFormatArg::Json => println!("{}", serde_json::to_string_pretty(&report)?),
    }
    if report.summary.failed > 0 {
        Ok(1)
    } else {
        Ok(0)
    }
}

fn render_inspect_rule_human(report: &InspectRuleReport) -> String {
    if report.rules.is_empty() {
        return "No repo-local rules found.\n".to_string();
    }
    let mut out = String::new();
    for rule in &report.rules {
        let capabilities = if rule.capabilities.is_empty() {
            "none".to_string()
        } else {
            rule.capabilities
                .iter()
                .map(|capability| capability.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let host = rule.host_path.as_deref().unwrap_or("<built-in>");
        out.push_str(&format!(
            "{} [{}] {}: {} (capabilities: {})\n",
            rule.rule_id, rule.severity, host, rule.description, capabilities
        ));
    }
    out
}

fn cache_status(layout: &CacheLayout, args: &CacheStatusArgs) -> Result<u8> {
    let status = layout.status()?;
    match args.format {
        CacheStatusFormatArg::Human => {
            print!("{}", render_cache_status_human(&status));
        }
        CacheStatusFormatArg::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&CacheStatusWire {
                    schema: POLINT_CACHE_STATUS_JSON_SCHEMA_V1_URL,
                    status: &status,
                })?
            );
        }
    }
    Ok(0)
}

fn cache_prune(layout: &CacheLayout, args: &CachePruneArgs) -> Result<u8> {
    let options = CachePruneOptions {
        categories: category_arg_to_prune_categories(args.category),
        max_age: args
            .max_age_days
            .map(|days| Duration::from_secs(days.saturating_mul(24 * 60 * 60))),
        max_bytes: args.max_size_mb.map(mib_to_bytes),
        dry_run: args.dry_run,
    };
    if options.max_age.is_none() && options.max_bytes.is_none() {
        anyhow::bail!("cache prune requires --max-age-days or --max-size-mb");
    }
    let report = layout.prune(&options)?;
    print!("{}", render_cache_prune_human(&report));
    Ok(0)
}

fn cache_clean(layout: &CacheLayout, args: &CacheCleanArgs) -> Result<u8> {
    let selection = category_arg_to_clean_selection(args.category);
    let report = layout.clean(selection)?;
    print!("{}", render_cache_clean_human(&report));
    Ok(0)
}

#[derive(Serialize)]
struct CacheStatusWire<'a> {
    #[serde(rename = "schema")]
    schema: &'static str,
    #[serde(flatten)]
    status: &'a CacheStatus,
}

const POLINT_FACTS_JSON_SCHEMA_V1_URL: &str =
    "https://raw.githubusercontent.com/oaiz-io/polint/main/docs/schemas/polint-facts-v1.json";
const POLINT_UNKNOWNS_JSON_SCHEMA_V1_URL: &str =
    "https://raw.githubusercontent.com/oaiz-io/polint/main/docs/schemas/polint-unknowns-v1.json";
const POLINT_EXPLAIN_JSON_SCHEMA_V1_URL: &str =
    "https://raw.githubusercontent.com/oaiz-io/polint/main/docs/schemas/polint-explain-v1.json";

#[derive(Debug, Clone, Serialize)]
struct PublicFactView {
    capability: &'static str,
    view_type: &'static str,
    canonical_path: &'static str,
    stability: &'static str,
    docs_path: &'static str,
    sampling: bool,
    unknowns: bool,
}

#[derive(Debug, Clone, Serialize)]
struct FactsListReport {
    version: u32,
    schema: String,
    tool: crate::diagnostics::PolintToolInfo,
    views: Vec<PublicFactView>,
}

#[derive(Debug, Clone, Serialize)]
struct FactsSampleReport {
    version: u32,
    schema: String,
    tool: crate::diagnostics::PolintToolInfo,
    capability: String,
    limit: usize,
    sampled: usize,
    rows: Vec<PublicFactSampleRow>,
}

#[derive(Debug, Clone, Serialize)]
struct PublicFactSampleRow {
    file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    span: Option<PublicSpan>,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    precision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stable_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct UnknownsReport {
    version: u32,
    schema: String,
    tool: crate::diagnostics::PolintToolInfo,
    capability: String,
    rows: Vec<UnknownsRow>,
}

#[derive(Debug, Clone, Serialize)]
struct UnknownsRow {
    file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    span: Option<PublicSpan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    family: Option<String>,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    precision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    docs_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    suggested_artifact: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_stable_key: Option<String>,
}

impl UnknownsRow {
    fn from_taxonomy_compat(row: crate::analysis::unknown_taxonomy::facts::UnknownRow) -> Self {
        let mut rendered = Self::from_taxonomy_full(row);
        rendered.category = None;
        rendered.provider = None;
        rendered.family = None;
        rendered.source_stable_key = None;
        rendered
    }

    fn from_taxonomy_full(row: crate::analysis::unknown_taxonomy::facts::UnknownRow) -> Self {
        Self {
            category: Some(row.category.as_str().to_string()),
            provider: None,
            family: None,
            file: row.file,
            span: row.span.map(|span| PublicSpan {
                line: span.line,
                column: span.column,
            }),
            status: row.status,
            reason: row.reason,
            precision: row.precision,
            docs_path: row.docs_path,
            suggested_artifact: row.suggested_artifact,
            source_stable_key: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct ExplainReport {
    version: u32,
    schema: String,
    tool: crate::diagnostics::PolintToolInfo,
    scope: String,
    rules: Vec<ExplainRuleRow>,
    public_capabilities: Vec<PublicFactView>,
}

#[derive(Debug, Clone, Serialize)]
struct ExplainRuleRow {
    rule_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    host_path: Option<String>,
    fact_views: Vec<String>,
    capabilities: Vec<crate::rule_manifest::CapabilitySupportWire>,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct PublicSpan {
    line: u32,
    column: u32,
}

impl PublicFactSampleRow {
    fn new(
        file: String,
        span: Option<PublicSpan>,
        status: impl Into<String>,
        precision: Option<String>,
        stable_id: Option<String>,
    ) -> Self {
        Self {
            file,
            span,
            status: status.into(),
            precision,
            stable_id,
        }
    }
}

impl FactsListReport {
    fn new() -> Self {
        let mut views = vec![
            public_fact_view("resolved_imports").unwrap(),
            public_fact_view("module_graph").unwrap(),
            public_fact_view("symbols").unwrap(),
            public_fact_view("references").unwrap(),
            public_fact_view("file_metrics").unwrap(),
            public_fact_view("function_metrics").unwrap(),
            public_fact_view("complexity_metrics").unwrap(),
            public_fact_view("events").unwrap(),
            public_fact_view("calls").unwrap(),
            public_fact_view("control_flow").unwrap(),
            public_fact_view("cfg").unwrap(),
            public_fact_view("call_graph").unwrap(),
            public_fact_view("dataflow").unwrap(),
            public_fact_view("coverage_facts").unwrap(),
            public_fact_view("test_suite_metrics").unwrap(),
        ];
        views.sort_by(|left, right| left.capability.cmp(right.capability));
        Self {
            version: 1,
            schema: POLINT_FACTS_JSON_SCHEMA_V1_URL.to_string(),
            tool: polint_tool_info(),
            views,
        }
    }
}

fn public_fact_view(capability: &str) -> Option<PublicFactView> {
    let view = match capability {
        "resolved_imports" => PublicFactView {
            capability: "resolved_imports",
            view_type: "ResolvedImports",
            canonical_path: "polint::sdk::facts::ResolvedImports<'_>",
            stability: "stable",
            docs_path: "docs/facts/resolved-imports.md",
            sampling: true,
            unknowns: true,
        },
        "module_graph" => PublicFactView {
            capability: "module_graph",
            view_type: "ModuleGraphFacts",
            canonical_path: "polint::sdk::facts::ModuleGraphFacts<'_>",
            stability: "stable",
            docs_path: "docs/facts/resolved-imports.md",
            sampling: true,
            unknowns: false,
        },
        "symbols" => PublicFactView {
            capability: "symbols",
            view_type: "Symbols",
            canonical_path: "polint::sdk::facts::Symbols<'_>",
            stability: "stable",
            docs_path: "docs/facts/symbols-and-references.md",
            sampling: true,
            unknowns: true,
        },
        "references" => PublicFactView {
            capability: "references",
            view_type: "References",
            canonical_path: "polint::sdk::facts::References<'_>",
            stability: "stable",
            docs_path: "docs/facts/symbols-and-references.md",
            sampling: true,
            unknowns: true,
        },
        "file_metrics" => PublicFactView {
            capability: "file_metrics",
            view_type: "FileMetrics",
            canonical_path: "polint::sdk::facts::FileMetrics<'_>",
            stability: "stable",
            docs_path: "docs/facts/metrics.md",
            sampling: true,
            unknowns: false,
        },
        "function_metrics" => PublicFactView {
            capability: "function_metrics",
            view_type: "FunctionMetrics",
            canonical_path: "polint::sdk::facts::FunctionMetrics<'_>",
            stability: "stable",
            docs_path: "docs/facts/metrics.md",
            sampling: true,
            unknowns: false,
        },
        "complexity_metrics" => PublicFactView {
            capability: "complexity_metrics",
            view_type: "ComplexityMetrics",
            canonical_path: "polint::sdk::facts::ComplexityMetrics<'_>",
            stability: "stable",
            docs_path: "docs/facts/metrics.md",
            sampling: true,
            unknowns: false,
        },
        "cfg" => PublicFactView {
            capability: "cfg",
            view_type: "Cfg",
            canonical_path: "polint::sdk::facts::Cfg<'_>",
            stability: "reserved",
            docs_path: "docs/facts/capability-plans.md",
            sampling: false,
            unknowns: false,
        },
        "events" => PublicFactView {
            capability: "events",
            view_type: "Events",
            canonical_path: "polint::sdk::facts::Events<'_>",
            stability: "preview",
            docs_path: "docs/facts/events.md",
            sampling: false,
            unknowns: true,
        },
        "calls" => PublicFactView {
            capability: "calls",
            view_type: "Calls",
            canonical_path: "polint::sdk::facts::Calls<'_>",
            stability: "preview",
            docs_path: "docs/facts/calls.md",
            sampling: false,
            unknowns: true,
        },
        "control_flow" => PublicFactView {
            capability: "control_flow",
            view_type: "ControlFlow",
            canonical_path: "polint::sdk::facts::ControlFlow<'_>",
            stability: "preview",
            docs_path: "docs/facts/control-flow.md",
            sampling: false,
            unknowns: true,
        },
        "call_graph" => PublicFactView {
            capability: "call_graph",
            view_type: "CallGraph",
            canonical_path: "polint::sdk::facts::CallGraph<'_>",
            stability: "reserved",
            docs_path: "docs/facts/capability-plans.md",
            sampling: false,
            unknowns: false,
        },
        "dataflow" => PublicFactView {
            capability: "dataflow",
            view_type: "DataFlow",
            canonical_path: "polint::sdk::facts::DataFlow<'_>",
            stability: "preview",
            docs_path: "docs/facts/data-flow.md",
            sampling: false,
            unknowns: true,
        },
        "coverage_facts" => PublicFactView {
            capability: "coverage_facts",
            view_type: "CoverageFacts",
            canonical_path: "polint::sdk::facts::CoverageFacts<'_>",
            stability: "reserved",
            docs_path: "docs/facts/capability-plans.md",
            sampling: false,
            unknowns: false,
        },
        "test_suite_metrics" => PublicFactView {
            capability: "test_suite_metrics",
            view_type: "TestSuiteMetrics",
            canonical_path: "polint::sdk::facts::TestSuiteMetrics<'_>",
            stability: "reserved",
            docs_path: "docs/facts/capability-plans.md",
            sampling: false,
            unknowns: false,
        },
        _ => return None,
    };
    Some(view)
}

fn polint_tool_info() -> crate::diagnostics::PolintToolInfo {
    crate::diagnostics::PolintToolInfo {
        name: "polint".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

struct AgentJsonAnalysis {
    db: AnalysisDb,
    diagnostics: Vec<Diagnostic>,
}

fn analyze_for_agent_json(
    root: &Path,
    paths: &[PathBuf],
    no_cache: bool,
    capabilities: &[&str],
) -> Result<AgentJsonAnalysis> {
    let args = CheckArgs {
        paths: paths.to_vec(),
        profile: None,
        format: FormatArg::Json,
        color: ColorArg::Never,
        no_cache,
        fail_on: FailOn::None,
        only_rule: None,
        max_diagnostics: None,
        stat: false,
        shortstat: false,
        baseline: false,
        new_only: false,
        ignore_comments: false,
    };
    let loaded = load_config_for_check(root, &args.paths)?;
    let cache = crate::cache::Cache::default_for_repo(root, !args.no_cache);
    let config_digest = crate::cache::keys::config_hash(&loaded);
    let rules: Vec<Rule> = Vec::new();
    let enabled = selected_rule_patterns(&loaded, args.profile.as_deref())?;
    let options = BTreeMap::<String, RuleOptions>::new();
    let rule_digest = crate::cache::keys::rule_hash(&rules, enabled.as_ref(), &options);
    let plan = AnalysisPlan::from_capability_names(capabilities);
    let output = AnalysisKernel::run(KernelInput {
        loaded: &loaded,
        cache: &cache,
        config_digest: &config_digest,
        rule_digest: &rule_digest,
        plan: &plan,
        parallel: true,
    })?;
    Ok(AgentJsonAnalysis {
        db: output.db,
        diagnostics: output.diagnostics,
    })
}

fn span_start(span: &crate::core::Span) -> PublicSpan {
    let range = span.diagnostic_range();
    PublicSpan {
        line: range.start_line,
        column: range.start_col,
    }
}

fn status_label(status: ResolutionStatus) -> &'static str {
    match status {
        ResolutionStatus::Resolved => "resolved",
        ResolutionStatus::External => "external",
        ResolutionStatus::Unresolved => "unresolved",
        ResolutionStatus::SetupMissing => "setup_missing",
        ResolutionStatus::Dynamic => "dynamic",
        ResolutionStatus::Unsupported => "unsupported",
        _ => "unknown",
    }
}

fn resolution_precision_label(precision: crate::core::ResolutionPrecision) -> &'static str {
    match precision {
        crate::core::ResolutionPrecision::ExactFile => "exact_file",
        crate::core::ResolutionPrecision::Package => "package",
        crate::core::ResolutionPrecision::ExternalPackage => "external_package",
        crate::core::ResolutionPrecision::Heuristic => "heuristic",
        crate::core::ResolutionPrecision::None => "none",
        _ => "unknown",
    }
}

fn symbol_status_label(status: SymbolResolutionStatus) -> &'static str {
    match status {
        SymbolResolutionStatus::Resolved => "resolved",
        SymbolResolutionStatus::Unresolved => "unresolved",
        SymbolResolutionStatus::Ambiguous => "ambiguous",
        SymbolResolutionStatus::SetupMissing => "setup_missing",
        SymbolResolutionStatus::Unsupported => "unsupported",
        _ => "unknown",
    }
}

fn symbol_precision_label(precision: SymbolPrecision) -> &'static str {
    match precision {
        SymbolPrecision::ExactSemantic => "exact_semantic",
        SymbolPrecision::ExactLocal => "exact_local",
        SymbolPrecision::ModuleLinked => "module_linked",
        SymbolPrecision::Heuristic => "heuristic",
        SymbolPrecision::Unresolved => "unresolved",
        SymbolPrecision::Ambiguous => "ambiguous",
        SymbolPrecision::SetupMissing => "setup_missing",
        SymbolPrecision::Unsupported => "unsupported",
        _ => "unknown",
    }
}

/// The managed category a `--category` value selects, or `None` for `all`.
fn category_arg_to_managed(category: CacheCategoryArg) -> Option<CacheManagedCategory> {
    match category {
        CacheCategoryArg::All => None,
        CacheCategoryArg::Analysis => Some(CacheManagedCategory::Analysis),
        CacheCategoryArg::Layers => Some(CacheManagedCategory::Layers),
        CacheCategoryArg::Derived => Some(CacheManagedCategory::Derived),
        CacheCategoryArg::Semantic => Some(CacheManagedCategory::Semantic),
        CacheCategoryArg::RulesTarget => Some(CacheManagedCategory::RulesTarget),
        CacheCategoryArg::ExtensionsTarget => Some(CacheManagedCategory::ExtensionsTarget),
        CacheCategoryArg::Review => Some(CacheManagedCategory::Review),
    }
}

fn category_arg_to_prune_categories(
    category: Option<CacheCategoryArg>,
) -> Vec<CacheManagedCategory> {
    match category.and_then(category_arg_to_managed) {
        None => Vec::new(),
        Some(category) => vec![category],
    }
}

fn category_arg_to_clean_selection(category: CacheCategoryArg) -> CacheCleanSelection {
    match category_arg_to_managed(category) {
        None => CacheCleanSelection::All,
        Some(category) => CacheCleanSelection::Category(category),
    }
}

fn mib_to_bytes(value: u64) -> u64 {
    value.saturating_mul(1024 * 1024)
}

fn render_cache_status_human(status: &CacheStatus) -> String {
    let mut out = String::new();
    out.push_str(&format!("Cache root: {}\n", status.root));
    out.push_str(&format!(
        "Total: {} across {} files\n",
        human_bytes(status.total_bytes),
        status.total_files
    ));
    for category in &status.categories {
        out.push_str(&format!(
            "- {} [{}]: {} files, {} ({})\n  {}\n",
            category.name,
            category.role,
            category.files,
            human_bytes(category.bytes),
            if category.exists {
                "present"
            } else {
                "missing"
            },
            category.path
        ));
    }
    out
}

fn render_cache_clean_human(report: &CacheCleanReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Removed {} across {} files\n",
        human_bytes(report.removed_bytes),
        report.removed_files
    ));
    for category in &report.categories {
        out.push_str(&format!(
            "- {}: removed {} files, {} from {}\n",
            category.name,
            category.removed_files,
            human_bytes(category.removed_bytes),
            category.path
        ));
    }
    out
}

fn render_cache_prune_human(report: &CachePruneReport) -> String {
    let action = if report.dry_run {
        "Would remove"
    } else {
        "Removed"
    };
    let mut out = String::new();
    out.push_str(&format!(
        "{action} {} across {} files\n",
        human_bytes(report.removed_bytes),
        report.removed_files
    ));
    for category in &report.categories {
        out.push_str(&format!(
            "- {}: {} files, {} before; {} files, {} selected from {}\n",
            category.name,
            category.before_files,
            human_bytes(category.before_bytes),
            category.removed_files,
            human_bytes(category.removed_bytes),
            category.path
        ));
    }
    out
}

fn human_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let value = bytes as f64;
    if value >= GIB {
        format!("{:.1} GiB", value / GIB)
    } else if value >= MIB {
        format!("{:.1} MiB", value / MIB)
    } else if value >= KIB {
        format!("{:.1} KiB", value / KIB)
    } else {
        format!("{bytes} B")
    }
}

fn check(root: PathBuf, args: &CheckArgs) -> Result<u8> {
    if args.new_only && !args.baseline {
        anyhow::bail!("--new-only requires --baseline");
    }
    let local_rule_hosts = discover_local_rule_hosts(&root)?;
    if !local_rule_hosts.is_empty() {
        return check_local_rule_hosts(&root, args, &local_rule_hosts);
    }

    let (mut diagnostics, db, loaded) = analyze_and_run(&root, args, true)?;
    let mut ignore_report = None;
    if args.ignore_comments {
        let ignore_application = apply_ignores(&db, diagnostics, &loaded.config.ignores);
        diagnostics = ignore_application.diagnostics;
        ignore_report = Some(ignore_application.report);
    }
    let source_map = match args.format {
        FormatArg::Human => db.sources_by_relative_path(),
        _ => BTreeMap::new(),
    };
    let format = match args.format {
        FormatArg::Human => OutputFormat::Human,
        FormatArg::Github => OutputFormat::Github,
        FormatArg::Json => OutputFormat::Json,
        FormatArg::Sarif => OutputFormat::Sarif,
        FormatArg::AiFriendly => OutputFormat::AiFriendly,
    };
    diagnostics = apply_report_filters(diagnostics, args.only_rule.as_deref());
    let baseline = apply_baseline(&root, args, diagnostics)?;
    diagnostics = baseline.diagnostics;
    let rendered_diagnostics = limit_report_diagnostics(diagnostics.clone(), args.max_diagnostics);
    let stats = check_stats(
        &db,
        &diagnostics,
        rendered_diagnostics.len(),
        ignore_report.as_ref(),
    );
    let sources = match args.format {
        FormatArg::Human => Some(&source_map),
        _ => None,
    };
    if matches!(args.format, FormatArg::AiFriendly) {
        let output = write_ai_friendly_report(
            &root,
            &diagnostics,
            &rendered_diagnostics,
            json_report_meta(),
            &[],
        )?;
        print!(
            "{}",
            render_ai_friendly_stdout(&output.report, AI_FRIENDLY_LATEST_OUTPUT)
        );
    } else {
        print!(
            "{}",
            render_with_sarif_help(
                format,
                &rendered_diagnostics,
                render_opts(args, sources, &[]),
                sarif_help_map(&loaded),
            )
        );
    }

    if loaded.missing && matches!(args.format, FormatArg::Human) {
        println!("Config not found. Run `polint init` to create .polint.toml.");
    }
    if should_render_check_stats(args) {
        print!("{}", render_check_stats(&stats, args.stat, args.shortstat));
    }
    if matches!(args.format, FormatArg::Human)
        && let Some(summary) = &baseline.summary
    {
        print!("{}", render_baseline_summary(summary));
    }

    Ok(exit_code_for(&baseline.failure_diagnostics, args.fail_on))
}

fn ignores(root: PathBuf, args: &IgnoresArgs) -> Result<u8> {
    let report = collect_ignore_report(&root, args)?;
    let filters = parse_ignore_filters(args.filter.as_deref());
    let report = filter_report(&report, &filters);
    match args.format {
        IgnoresFormatArg::Human => {
            print!(
                "{}",
                render_ignore_report_human(&report, args.stat, args.shortstat)
            );
        }
        IgnoresFormatArg::Json => {
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
    }
    Ok(0)
}

fn collect_ignore_report(root: &Path, args: &IgnoresArgs) -> Result<crate::ignores::IgnoreReport> {
    let check_args = CheckArgs {
        paths: args.paths.clone(),
        profile: args.profile.clone(),
        format: FormatArg::Json,
        color: ColorArg::Never,
        no_cache: args.no_cache,
        fail_on: FailOn::None,
        only_rule: None,
        max_diagnostics: None,
        stat: false,
        shortstat: false,
        baseline: false,
        new_only: false,
        ignore_comments: false,
    };
    let local_rule_hosts = discover_local_rule_hosts(root)?;
    if local_rule_hosts.is_empty() {
        let (diagnostics, db, loaded) = analyze_and_run(root, &check_args, true)?;
        return Ok(apply_ignores(&db, diagnostics, &loaded.config.ignores).report);
    }

    let config = load_config_for_check(root, &args.paths)?;
    let enabled = selected_rule_patterns(&config, args.profile.as_deref())?;
    let mut diagnostics = Vec::new();
    for manifest in &local_rule_hosts {
        let (host_diagnostics, _) = run_local_rule_host(root, manifest, &check_args, false)?;
        diagnostics.extend(host_diagnostics);
    }
    // Same scope narrowing as `check_local_rule_hosts`: the db only feeds
    // ignore-directive scanning, which is limited to the configured rule scopes
    // plus any files diagnostics actually landed in.
    let rule_scope = config_rule_scope_globset(&config, enabled.as_ref());
    let (mut db, _) = load_analysis_files_scoped(&config, rule_scope.as_ref())?;
    backfill_diagnostic_files(&mut db, &diagnostics, root);
    Ok(apply_ignores(&db, diagnostics, &config.config.ignores).report)
}

fn parse_ignore_filters(filter: Option<&str>) -> Vec<String> {
    filter
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|filter| !filter.is_empty())
        .map(ToString::to_string)
        .collect()
}

#[derive(Debug, Clone, Default)]
struct CheckStats {
    scanned_files: usize,
    by_language: BTreeMap<&'static str, usize>,
    diagnostics: usize,
    emitted_diagnostics: usize,
    by_severity: BTreeMap<&'static str, usize>,
    by_rule: BTreeMap<String, usize>,
    ignore_directives: usize,
    active_ignores: usize,
    unused_ignores: usize,
    malformed_ignores: usize,
    missing_reason_ignores: usize,
    suppressed_diagnostics: usize,
}

fn check_stats(
    db: &AnalysisDb,
    diagnostics: &[Diagnostic],
    emitted_diagnostics: usize,
    ignore_report: Option<&crate::ignores::IgnoreReport>,
) -> CheckStats {
    let mut stats = CheckStats {
        scanned_files: db.files().len(),
        diagnostics: diagnostics.len(),
        emitted_diagnostics,
        ..CheckStats::default()
    };
    for file in db.files() {
        *stats
            .by_language
            .entry(language_label(file.language))
            .or_default() += 1;
    }
    for diagnostic in diagnostics {
        *stats
            .by_severity
            .entry(severity_label(diagnostic.severity))
            .or_default() += 1;
        *stats.by_rule.entry(diagnostic.rule_id.clone()).or_default() += 1;
    }
    if let Some(report) = ignore_report {
        stats.ignore_directives = report.summary.directives;
        stats.active_ignores = report.summary.active;
        stats.unused_ignores = report.summary.unused;
        stats.malformed_ignores = report.summary.malformed;
        stats.missing_reason_ignores = report.summary.missing_reasons;
        stats.suppressed_diagnostics = report.summary.suppressed_diagnostics;
    }
    stats
}

fn should_render_check_stats(args: &CheckArgs) -> bool {
    matches!(args.format, FormatArg::Human) && (args.stat || args.shortstat)
}

fn render_check_stats(stats: &CheckStats, stat: bool, shortstat: bool) -> String {
    let mut out = String::new();
    if shortstat {
        out.push_str(&format_check_shortstat(stats));
        out.push('\n');
    }
    if stat {
        if !shortstat {
            out.push_str(&format_check_shortstat(stats));
            out.push('\n');
        }
        out.push_str(&format_check_stat_tables(stats));
    }
    out
}

fn format_check_shortstat(stats: &CheckStats) -> String {
    let emitted = if stats.emitted_diagnostics == stats.diagnostics {
        String::new()
    } else {
        format!(", {} emitted", stats.emitted_diagnostics)
    };
    format!(
        "Scanned {} {}; {} diagnostics{}; {} suppressed by {} ignore directives",
        stats.scanned_files,
        plural(stats.scanned_files, "file", "files"),
        stats.diagnostics,
        emitted,
        stats.suppressed_diagnostics,
        stats.ignore_directives
    )
}

fn format_check_stat_tables(stats: &CheckStats) -> String {
    let mut out = String::new();
    if !stats.by_language.is_empty() {
        out.push_str("\nBy language\n");
        for (language, count) in &stats.by_language {
            out.push_str(&format!("  {language}: {count}\n"));
        }
    }
    if !stats.by_severity.is_empty() {
        out.push_str("\nBy severity\n");
        for (severity, count) in &stats.by_severity {
            out.push_str(&format!("  {severity}: {count}\n"));
        }
    }
    if !stats.by_rule.is_empty() {
        out.push_str("\nBy rule\n");
        for (rule_id, count) in &stats.by_rule {
            out.push_str(&format!("  {rule_id}: {count}\n"));
        }
    }
    if stats.ignore_directives > 0 || stats.suppressed_diagnostics > 0 {
        out.push_str("\nIgnores\n");
        out.push_str(&format!(
            "  directives={}, active={}, unused={}, malformed={}, missing_reasons={}, suppressed={}\n",
            stats.ignore_directives,
            stats.active_ignores,
            stats.unused_ignores,
            stats.malformed_ignores,
            stats.missing_reason_ignores,
            stats.suppressed_diagnostics
        ));
    }
    out
}

fn language_label(language: Language) -> &'static str {
    match language {
        Language::Go => "go",
        Language::TypeScript => "ts",
        Language::Tsx => "tsx",
        Language::JavaScript => "js",
        Language::Jsx => "jsx",
        Language::Unknown => "unknown",
        _ => "unknown",
    }
}

fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Info => "info",
        Severity::Warn => "warn",
        Severity::Error => "error",
        _ => unreachable!(),
    }
}

fn plural(count: usize, singular: &'static str, plural: &'static str) -> &'static str {
    if count == 1 { singular } else { plural }
}

struct BaselineApplied {
    diagnostics: Vec<Diagnostic>,
    failure_diagnostics: Vec<Diagnostic>,
    summary: Option<BaselineSummary>,
}

fn apply_baseline(
    root: &Path,
    args: &CheckArgs,
    diagnostics: Vec<Diagnostic>,
) -> Result<BaselineApplied> {
    if !args.baseline {
        return Ok(BaselineApplied {
            failure_diagnostics: diagnostics.clone(),
            diagnostics,
            summary: None,
        });
    }

    let config = load_baseline(root)?;
    let classification = classify_diagnostics(&diagnostics, &config);
    let failure_diagnostics = classification.new_diagnostics.clone();
    let diagnostics = if args.new_only {
        classification.new_diagnostics
    } else {
        classification.visible_diagnostics
    };
    Ok(BaselineApplied {
        diagnostics,
        failure_diagnostics,
        summary: Some(classification.summary),
    })
}

fn baseline_path(root: &Path) -> PathBuf {
    root.join(DEFAULT_BASELINE_PATH)
}

fn collect_diagnostics_for_baseline(
    root: &Path,
    paths: &[PathBuf],
    profile: Option<&str>,
    no_cache: bool,
) -> Result<Vec<Diagnostic>> {
    let check_args = CheckArgs {
        paths: paths.to_vec(),
        profile: profile.map(ToString::to_string),
        format: FormatArg::Json,
        color: ColorArg::Never,
        no_cache,
        fail_on: FailOn::None,
        only_rule: None,
        max_diagnostics: None,
        stat: false,
        shortstat: false,
        baseline: false,
        new_only: false,
        ignore_comments: true,
    };
    let local_rule_hosts = discover_local_rule_hosts(root)?;
    let mut diagnostics = if local_rule_hosts.is_empty() {
        let (diagnostics, db, loaded) = analyze_and_run(root, &check_args, true)?;
        apply_ignores(&db, diagnostics, &loaded.config.ignores).diagnostics
    } else {
        let config = load_config_for_check(root, paths)?;
        let enabled = selected_rule_patterns(&config, profile)?;
        let mut diagnostics = Vec::new();
        for manifest in &local_rule_hosts {
            let (host_diagnostics, _) = run_local_rule_host(root, manifest, &check_args, false)?;
            diagnostics.extend(host_diagnostics);
        }
        // Same scope narrowing as `check_local_rule_hosts`.
        let rule_scope = config_rule_scope_globset(&config, enabled.as_ref());
        let (mut loaded_db, _) = load_analysis_files_scoped(&config, rule_scope.as_ref())?;
        backfill_diagnostic_files(&mut loaded_db, &diagnostics, root);
        apply_ignores(&loaded_db, diagnostics, &config.config.ignores).diagnostics
    };
    diagnostics = apply_report_filters(diagnostics, None);
    Ok(diagnostics)
}

fn analyze_and_run(
    root: &Path,
    args: &CheckArgs,
    parallel: bool,
) -> Result<(
    Vec<crate::diagnostics::Diagnostic>,
    crate::core::AnalysisDb,
    LoadedConfig,
)> {
    let loaded = load_config_for_check(root, &args.paths)?;
    let cache = crate::cache::Cache::default_for_repo(root, !args.no_cache);
    let config_digest = crate::cache::keys::config_hash(&loaded);
    let rules: Vec<Rule> = Vec::new();
    let enabled = selected_rule_patterns(&loaded, args.profile.as_deref())?;
    let options = BTreeMap::<String, RuleOptions>::new();
    let rule_digest = crate::cache::keys::rule_hash(&rules, enabled.as_ref(), &options);
    let plan = AnalysisPlan::empty();

    let output = AnalysisKernel::run(KernelInput {
        loaded: &loaded,
        cache: &cache,
        config_digest: &config_digest,
        rule_digest: &rule_digest,
        plan: &plan,
        parallel,
    })?;
    let mut diagnostics = output.diagnostics;
    diagnostics.extend(run_rules(
        &output.db,
        &rules,
        &options,
        enabled.as_ref(),
        parallel,
    ));
    Ok((diagnostics, output.db, loaded))
}

fn selected_rule_patterns(
    config: &LoadedConfig,
    profile: Option<&str>,
) -> Result<Option<BTreeSet<String>>> {
    Ok(config
        .profile_rules(profile)?
        .map(|rules| rules.into_iter().collect()))
}

fn load_config_for_check(root: &Path, paths: &[PathBuf]) -> Result<LoadedConfig> {
    let mut config = load_config(root)?;
    if !paths.is_empty() {
        config.config.workspace.include = paths
            .iter()
            .map(|path| check_path_pattern(root, path))
            .collect();
    }
    Ok(config)
}

fn check_path_pattern(root: &Path, path: &Path) -> String {
    let display_path = if path.is_absolute() {
        path.strip_prefix(root).unwrap_or(path)
    } else {
        path
    };
    let normalized = display_path
        .to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string();
    if normalized.contains(['*', '?', '[', ']', '{', '}']) {
        return normalized;
    }
    if root.join(&normalized).is_dir() || normalized.ends_with('/') {
        format!("{}/**", normalized.trim_end_matches('/'))
    } else {
        normalized
    }
}

fn discover_local_rule_hosts(root: &Path) -> Result<Vec<PathBuf>> {
    let config = load_config(root)?;
    let mut manifests = BTreeSet::new();
    for rule_path in &config.config.rules.paths {
        let manifest = root.join(rule_path).join("Cargo.toml");
        if manifest.is_file() {
            manifests.insert(manifest);
        }
    }
    Ok(manifests.into_iter().collect())
}

/// Union of the configured rules' `files` scopes (optionally filtered to the
/// enabled profile patterns) as a glob set, or `None` when narrowing is unsafe:
/// no configured rules, or any enabled rule entry without a `files` scope (which
/// matches every file). `None` falls back to full workspace loading.
///
/// This mirrors `AnalysisKernel::rule_scope_globset`, but reads scopes from
/// `.polint.toml` instead of the in-process plan, because the outer CLI never
/// registers the repo-local rules itself — they live in the rule-host subprocess.
fn config_rule_scope_globset(
    config: &LoadedConfig,
    enabled: Option<&BTreeSet<String>>,
) -> Option<globset::GlobSet> {
    let entries = config
        .config
        .rules
        .config
        .iter()
        .filter(|entry| {
            enabled.is_none_or(|patterns| {
                patterns
                    .iter()
                    .any(|pattern| rule_id_matches(pattern, &entry.id))
            })
        })
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return None;
    }
    let mut patterns = Vec::new();
    for entry in &entries {
        if entry.files.is_empty() {
            return None;
        }
        patterns.extend(entry.files.iter().cloned());
    }
    crate::config::build_glob_set(&patterns).ok()
}

/// Ensures every diagnostic's file is present in `db` so ignore-comment
/// directives in those files still apply when the db was loaded with a narrowed
/// scope (e.g. a rule registered in the host without a `[[rules.config]]` entry
/// can emit outside the configured scopes).
fn backfill_diagnostic_files(
    db: &mut AnalysisDb,
    diagnostics: &[crate::diagnostics::Diagnostic],
    root: &Path,
) {
    let known = db
        .files()
        .iter()
        .map(|file| file.relative_path.clone())
        .collect::<BTreeSet<_>>();
    let missing = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.file.clone())
        .filter(|file| !file.is_empty() && !known.contains(file))
        .collect::<BTreeSet<_>>();
    for relative_path in missing {
        let path = root.join(&relative_path);
        if let Ok(source) = fs::read_to_string(&path) {
            db.add_file(path, relative_path, source);
        }
    }
}

fn check_local_rule_hosts(root: &Path, args: &CheckArgs, manifests: &[PathBuf]) -> Result<u8> {
    let config = load_config_for_check(root, &args.paths)?;
    let enabled = selected_rule_patterns(&config, args.profile.as_deref())?;

    let child_applies_ignores =
        args.ignore_comments && manifests.len() == 1 && !should_render_check_stats(args);
    let mut diagnostics = Vec::new();
    let mut rule_execution = Vec::new();
    for manifest in manifests {
        let (host_diagnostics, host_rules) =
            run_local_rule_host(root, manifest, args, child_applies_ignores)?;
        diagnostics.extend(host_diagnostics);
        rule_execution.extend(host_rules);
    }
    merge_rule_execution_rows(&mut rule_execution);

    let mut db = None;
    let mut ignore_report = None;
    // The outer db only feeds ignore-comment application and `--stat`
    // rendering, so load just the configured rule scopes instead of the whole
    // workspace — on large monorepos this is the difference between reading
    // a few thousand in-scope files and every file in the repo. Any diagnostic
    // outside those scopes gets its file backfilled so its ignore directives
    // still apply.
    let rule_scope = config_rule_scope_globset(&config, enabled.as_ref());
    if args.ignore_comments && !child_applies_ignores {
        let (mut loaded_db, _) = load_analysis_files_scoped(&config, rule_scope.as_ref())?;
        backfill_diagnostic_files(&mut loaded_db, &diagnostics, root);
        let ignore_application = apply_ignores(&loaded_db, diagnostics, &config.config.ignores);
        diagnostics = ignore_application.diagnostics;
        ignore_report = Some(ignore_application.report);
        db = Some(loaded_db);
    } else if should_render_check_stats(args) {
        let (mut loaded_db, _) = load_analysis_files_scoped(&config, rule_scope.as_ref())?;
        backfill_diagnostic_files(&mut loaded_db, &diagnostics, root);
        db = Some(loaded_db);
    }
    diagnostics = apply_report_filters(diagnostics, args.only_rule.as_deref());
    let baseline = apply_baseline(root, args, diagnostics)?;
    diagnostics = baseline.diagnostics;
    let rendered_diagnostics = limit_report_diagnostics(diagnostics.clone(), args.max_diagnostics);
    let source_map = if matches!(args.format, FormatArg::Human) {
        read_sources_for_diagnostics(root, &rendered_diagnostics)
    } else {
        BTreeMap::new()
    };
    let format = match args.format {
        FormatArg::Human => OutputFormat::Human,
        FormatArg::Github => OutputFormat::Github,
        FormatArg::Json => OutputFormat::Json,
        FormatArg::Sarif => OutputFormat::Sarif,
        FormatArg::AiFriendly => OutputFormat::AiFriendly,
    };
    let sources = match args.format {
        FormatArg::Human => Some(&source_map),
        _ => None,
    };
    if matches!(args.format, FormatArg::AiFriendly) {
        let output = write_ai_friendly_report(
            root,
            &diagnostics,
            &rendered_diagnostics,
            json_report_meta(),
            &rule_execution,
        )?;
        print!(
            "{}",
            render_ai_friendly_stdout(&output.report, AI_FRIENDLY_LATEST_OUTPUT)
        );
    } else {
        print!(
            "{}",
            render_with_sarif_help(
                format,
                &rendered_diagnostics,
                render_opts(args, sources, &rule_execution),
                sarif_help_map(&config),
            )
        );
    }
    if should_render_check_stats(args)
        && let Some(db) = &db
    {
        let stats = check_stats(
            db,
            &diagnostics,
            rendered_diagnostics.len(),
            ignore_report.as_ref(),
        );
        print!("{}", render_check_stats(&stats, args.stat, args.shortstat));
    }
    if matches!(args.format, FormatArg::Human)
        && let Some(summary) = &baseline.summary
    {
        print!("{}", render_baseline_summary(summary));
    }
    Ok(exit_code_for(&baseline.failure_diagnostics, args.fail_on))
}

/// `polint review <ref>`: run review-kind rules against the diff to `<ref>`.
///
/// A near-clone of [`check_local_rule_hosts`] with three additions: it requires
/// repo-local rule hosts, builds + serializes a changeset before running them,
/// runs each host with `--kind review --changed-files <file>`, and (by default)
/// gates the findings to the diff. `polint check` is unaffected.
fn review(root: PathBuf, args: &ReviewArgs) -> Result<u8> {
    let manifests = discover_local_rule_hosts(&root)?;
    if manifests.is_empty() {
        anyhow::bail!(
            "`polint review` requires repo-local rules under `[rules] paths` in .polint.toml; \
             none were found. Scaffold one with `polint new-rule generic <name> --review`."
        );
    }

    // Build the diff and serialize it to a cache-dir file (a file, not an env
    // var, so large diffs are not bound by env-size limits). The host reads it
    // via `--changed-files` and injects it before rules run.
    let changeset = crate::git::changeset_for_ref(&root, &args.reff)?;
    let changeset_file = write_review_changeset(&root, &changeset)?;

    // Map the review args onto the shared `CheckArgs` shape so the rendering,
    // baseline, and report-filter helpers are reused verbatim. Review has no
    // baseline/stat surface; those default off. Comment ignores stay enabled by
    // default, matching `polint check`.
    let check_args = CheckArgs {
        paths: args.paths.clone(),
        profile: args.profile.clone(),
        format: args.format,
        color: args.color,
        no_cache: args.no_cache,
        fail_on: args.fail_on,
        only_rule: args.only_rule.clone(),
        max_diagnostics: args.max_diagnostics,
        stat: false,
        shortstat: false,
        baseline: false,
        new_only: false,
        ignore_comments: true,
    };

    let config = load_config_for_check(&root, &check_args.paths)?;
    let enabled = selected_rule_patterns(&config, check_args.profile.as_deref())?;
    let child_applies_ignores = check_args.ignore_comments && manifests.len() == 1;
    let mut diagnostics = Vec::new();
    let mut rule_execution = Vec::new();
    for manifest in &manifests {
        let (host_diagnostics, host_rules) = run_local_rule_host_kind(
            &root,
            manifest,
            &check_args,
            child_applies_ignores,
            "review",
            Some(changeset_file.as_path()),
        )?;
        diagnostics.extend(host_diagnostics);
        rule_execution.extend(host_rules);
    }
    merge_rule_execution_rows(&mut rule_execution);

    if check_args.ignore_comments && !child_applies_ignores {
        let rule_scope = config_rule_scope_globset(&config, enabled.as_ref());
        let (mut loaded_db, _) = load_analysis_files_scoped(&config, rule_scope.as_ref())?;
        backfill_diagnostic_files(&mut loaded_db, &diagnostics, &root);
        diagnostics = apply_ignores(&loaded_db, diagnostics, &config.config.ignores).diagnostics;
    }

    // Default finding-level diff gate: keep only diagnostics that intersect the
    // changeset. `--no-diff-gate` surfaces every review finding; `--whole-file`
    // gates by changed file only, ignoring line ranges.
    if !args.no_diff_gate {
        diagnostics = gate_to_changeset(diagnostics, &changeset, args.whole_file);
    }

    diagnostics = apply_report_filters(diagnostics, check_args.only_rule.as_deref());
    let baseline = apply_baseline(&root, &check_args, diagnostics)?;
    diagnostics = baseline.diagnostics;
    let rendered_diagnostics =
        limit_report_diagnostics(diagnostics.clone(), check_args.max_diagnostics);
    let source_map = if matches!(check_args.format, FormatArg::Human) {
        read_sources_for_diagnostics(&root, &rendered_diagnostics)
    } else {
        BTreeMap::new()
    };
    let format = match check_args.format {
        FormatArg::Human => OutputFormat::Human,
        FormatArg::Github => OutputFormat::Github,
        FormatArg::Json => OutputFormat::Json,
        FormatArg::Sarif => OutputFormat::Sarif,
        FormatArg::AiFriendly => OutputFormat::AiFriendly,
    };
    let sources = match check_args.format {
        FormatArg::Human => Some(&source_map),
        _ => None,
    };
    if matches!(check_args.format, FormatArg::AiFriendly) {
        let output = write_ai_friendly_report(
            &root,
            &diagnostics,
            &rendered_diagnostics,
            json_report_meta(),
            &rule_execution,
        )?;
        print!(
            "{}",
            render_ai_friendly_stdout(&output.report, AI_FRIENDLY_LATEST_OUTPUT)
        );
    } else {
        print!(
            "{}",
            render_with_sarif_help(
                format,
                &rendered_diagnostics,
                render_opts(&check_args, sources, &rule_execution),
                sarif_help_map(&config),
            )
        );
    }
    if matches!(check_args.format, FormatArg::Human)
        && let Some(summary) = &baseline.summary
    {
        print!("{}", render_baseline_summary(summary));
    }
    Ok(exit_code_for(
        &baseline.failure_diagnostics,
        check_args.fail_on,
    ))
}

/// Serialize a review changeset to a stable-named JSON file under the cache dir.
///
/// The file name hashes the JSON so re-runs with the same diff reuse the name.
/// Returns the absolute path passed to the host as `--changed-files`.
fn write_review_changeset(
    root: &Path,
    changeset: &crate::core::ReviewChangeset,
) -> Result<PathBuf> {
    let json =
        serde_json::to_string(changeset).context("failed to serialize review changeset to JSON")?;
    let cache_layout = CacheLayout::for_repo(root);
    let review_dir = cache_layout.review_dir();
    std::fs::create_dir_all(&review_dir)
        .with_context(|| format!("failed to create review cache dir {}", review_dir.display()))?;
    let name = format!(
        "changeset-{}.json",
        &crate::cache::stable_hash(&[&json])[..16]
    );
    let path = review_dir.join(name);
    std::fs::write(&path, json)
        .with_context(|| format!("failed to write review changeset {}", path.display()))?;
    Ok(path)
}

/// Retain only diagnostics whose file (and, unless `whole_file`, line span)
/// intersects the review changeset.
///
/// File match is exact against the normalized changeset paths (which share the
/// `Diagnostic.file` form). Line match overlaps `[start_line, end_line]` with
/// any new-side range for that file. Deleted files carry no new-side ranges, so
/// line-gating drops their diagnostics (a review rule normally would not fire on
/// a deleted file anyway).
fn gate_to_changeset(
    diagnostics: Vec<Diagnostic>,
    changeset: &crate::core::ReviewChangeset,
    whole_file: bool,
) -> Vec<Diagnostic> {
    diagnostics
        .into_iter()
        .filter(|diagnostic| {
            let Some(file) = changeset
                .files
                .iter()
                .find(|file| file.path == diagnostic.file)
            else {
                return false;
            };
            if whole_file {
                return true;
            }
            let start = diagnostic.range.start_line;
            let end = diagnostic.range.end_line;
            file.new_line_ranges
                .iter()
                .any(|&(lo, hi)| start <= hi && lo <= end)
        })
        .collect()
}

fn run_local_rule_host(
    root: &Path,
    manifest: &Path,
    args: &CheckArgs,
    apply_ignore_comments: bool,
) -> Result<(Vec<Diagnostic>, Vec<crate::diagnostics::RuleExecutionRow>)> {
    // The outer `check` path always runs Check-kind rules and injects no diff.
    run_local_rule_host_kind(root, manifest, args, apply_ignore_comments, "check", None)
}

/// Run a repo-local rule host, selecting the rule `kind` and optionally
/// injecting a `polint review` changeset file via `--changed-files`.
///
/// `check` calls this with `kind = "check"` and `changed_files = None`;
/// `polint review` calls it with `kind = "review"` and the serialized changeset
/// path. The host runs the same inner `check` subcommand either way.
///
/// The host binary is obtained in whichever of three ways is cheapest and can be
/// proven correct: the one this checkout already compiled, the one this machine
/// compiled in another checkout, or a fresh compile. Which one answered changes
/// nothing about what the host reports — the fingerprint names the build's whole
/// input surface, and the bytes are verified against it before anything runs.
fn run_local_rule_host_kind(
    root: &Path,
    manifest: &Path,
    args: &CheckArgs,
    apply_ignore_comments: bool,
    kind: &str,
    changed_files: Option<&Path>,
) -> Result<(Vec<Diagnostic>, Vec<crate::diagnostics::RuleExecutionRow>)> {
    let cargo = local_rule_host_cargo();
    let cache_layout = CacheLayout::for_repo(root);
    let rules_target_dir = cache_layout.rules_target_dir();
    let host_args = local_rule_host_arguments(args, apply_ignore_comments, kind, changed_files)?;

    if let Some(plan) = local_rule_host_plan(root, manifest, &cargo, &rules_target_dir) {
        // The host this checkout compiled last, when it is still the host these
        // inputs name. No Cargo build or run is needed.
        if let Some(binary) =
            rules_store::binary_recorded_by_stamp(&rules_target_dir, &plan.fingerprint.complete)
            && let Some(result) =
                run_local_rule_host_binary(root, manifest, &binary, &cache_layout, &host_args)
        {
            return result;
        }
        // The store answers a question asked from another checkout, so it may
        // only be reached when this fingerprint names the same bytes there.
        // Decided once, past the stamp a warm run returns on, and used by both
        // the restore below and the publish after the build, which therefore
        // cannot disagree about it.
        let store = plan.store.as_ref().filter(|_| {
            rules_store::inputs_are_the_same_from_every_checkout(&plan.rule_package_dir, root)
        });
        if let Some(store) = store {
            if let Some(binary) =
                rules_store::restore(store, &rules_target_dir, &plan.fingerprint.complete)
                && let Some(result) =
                    run_local_rule_host_binary(root, manifest, &binary, &cache_layout, &host_args)
            {
                tracing::info!(target: "polint::rules", "rule host restored from store");
                return result;
            }
            // Compiling and running are separated here so the binary can be
            // named, recorded, and shared. A build that cannot be attributed to
            // one binary falls through to the combined `cargo run` below, which
            // needs no such answer.
            if let Ok(Some(binary)) =
                build_local_rule_host_binary(root, manifest, &cargo, &cache_layout)
            {
                record_built_rule_host(root, &plan, store, &rules_target_dir, &binary);
                if let Some(result) =
                    run_local_rule_host_binary(root, manifest, &binary, &cache_layout, &host_args)
                {
                    return result;
                }
            }
        }
    }

    run_local_rule_host_through_cargo(root, manifest, &cargo, &cache_layout, &host_args)
}

/// The cargo binary polint builds repo-local Rust with.
fn local_rule_host_cargo() -> String {
    std::env::var("POLINT_CARGO")
        .or_else(|_| std::env::var("CARGO"))
        .unwrap_or_else(|_| "cargo".to_string())
}

/// How a rule host's build inputs are named, and where a build already done may
/// be found.
struct LocalRuleHostPlan {
    /// Serializes this target directory through verification and execution.
    _target_lock: rules_store::TargetLock,
    /// The digests over every input the build reads, taken before it runs.
    fingerprint: rules_store::BuildFingerprint,
    /// The directory holding the rule package, whose sources the fingerprint
    /// covers and whose manifests decide whether it may be shared.
    rule_package_dir: PathBuf,
    /// What the fingerprint was taken against, so it can be taken again once the
    /// build has finished.
    environment: rules_store::BuildEnvironment,
    /// The machine-global store, when sharing is on for this run.
    store: Option<rules_store::RuleHostStore>,
}

/// Name this rule host's build inputs, or `None` when they cannot be named.
///
/// `None` is not a failure: it means this run compiles and runs the host the one
/// way it always could. Resolving the compiler costs two process starts, so it
/// is not paid at all when there is neither a stamp to check nor a store to
/// look in.
fn local_rule_host_plan(
    root: &Path,
    manifest: &Path,
    cargo: &str,
    rules_target_dir: &Path,
) -> Option<LocalRuleHostPlan> {
    let store = rules_store::RuleHostStore::from_env();
    if store.is_none() && !rules_store::is_stamped(rules_target_dir) {
        return None;
    }
    let target_lock = rules_store::TargetLock::acquire(rules_target_dir)?;
    let rule_package_dir = manifest.parent()?.to_path_buf();
    let environment = rules_store::BuildEnvironment::new(
        LocalRuleHostProfile::from_env().name(),
        local_rule_host_toolchain(root, cargo)?,
    )?;
    Some(LocalRuleHostPlan {
        _target_lock: target_lock,
        fingerprint: rules_store::build_fingerprint(root, &rule_package_dir, &environment)?,
        rule_package_dir,
        environment,
        store,
    })
}

/// Record a freshly built rule host, under the identity it has now that cargo
/// has finished.
///
/// The key is taken again because the build writes one of its own inputs: cargo
/// creates or updates the rule package's lockfile, and the key every later run
/// computes is the one that includes it. Nothing is recorded when the sources
/// changed while cargo was reading them — that binary is still correct to run,
/// but it is not the answer to the question the new sources ask.
fn record_built_rule_host(
    root: &Path,
    plan: &LocalRuleHostPlan,
    store: &rules_store::RuleHostStore,
    rules_target_dir: &Path,
    binary: &Path,
) {
    if let Some(after) =
        rules_store::build_fingerprint(root, &plan.rule_package_dir, &plan.environment)
        && after.authored == plan.fingerprint.authored
    {
        rules_store::record(Some(store), rules_target_dir, &after.complete, binary);
    }
}

/// The compiler and cargo a rule-host build in `root` would use.
///
/// Both are read in `root` with the same toolchain override the build gets, so
/// they report what would actually compile the host: a `rust-toolchain.toml`
/// there, a pinned `POLINT_RULES_TOOLCHAIN`, or a floating `stable` that moved.
/// They are started together because each is a rustup shim away and a warm run
/// waits for both before it can decide anything.
fn local_rule_host_toolchain(root: &Path, cargo: &str) -> Option<rules_store::ToolchainIdentity> {
    let toolchain = std::env::var(rules_host_error::POLINT_RULES_TOOLCHAIN)
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var("RUSTUP_TOOLCHAIN")
                .ok()
                .filter(|value| !value.is_empty())
        });
    let start = |program: &str, argument: &str| {
        let mut command = ProcessCommand::new(program);
        command
            .current_dir(root)
            .arg(argument)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        if let Some(toolchain) = &toolchain {
            command.env("RUSTUP_TOOLCHAIN", toolchain);
        }
        command.spawn().ok()
    };
    let rustc_program = match std::env::var_os("RUSTC") {
        Some(value) => value.into_string().ok()?,
        None => "rustc".to_string(),
    };
    let rustc = start(&rustc_program, "-vV")?;
    let cargo = start(cargo, "-V")?;
    let rustc = rustc.wait_with_output().ok()?;
    let cargo = cargo.wait_with_output().ok()?;
    if !rustc.status.success() || !cargo.status.success() {
        return None;
    }
    Some(rules_store::ToolchainIdentity {
        rustc: String::from_utf8(rustc.stdout).ok()?,
        cargo: String::from_utf8(cargo.stdout).ok()?,
        rustup_toolchain: toolchain,
    })
}

/// The arguments the rule host itself is invoked with — everything cargo would
/// pass after `--`.
fn local_rule_host_arguments(
    args: &CheckArgs,
    apply_ignore_comments: bool,
    kind: &str,
    changed_files: Option<&Path>,
) -> Result<Vec<OsString>> {
    let mut out: Vec<OsString> = [
        "check",
        "--format",
        "json",
        "--fail-on",
        "none",
        "--ignore-comments",
        if apply_ignore_comments {
            "true"
        } else {
            "false"
        },
        "--kind",
        kind,
    ]
    .iter()
    .map(OsString::from)
    .collect();
    if let Some(changed_files) = changed_files {
        out.push(OsString::from("--changed-files"));
        out.push(OsString::from(changed_files.to_str().ok_or_else(|| {
            anyhow::anyhow!("non-UTF-8 changeset path: {}", changed_files.display())
        })?));
    }
    if let Some(profile) = &args.profile {
        out.push(OsString::from("--profile"));
        out.push(OsString::from(profile));
    }
    if args.no_cache {
        out.push(OsString::from("--no-cache"));
    }
    if let Some(pattern) = &args.only_rule {
        out.push(OsString::from("--only-rule"));
        out.push(OsString::from(pattern));
    }
    out.extend(args.paths.iter().map(|path| path.as_os_str().to_owned()));
    Ok(out)
}

/// The environment every repo-local rule host process runs with, however it was
/// obtained: polint's cache root, the cargo target directory polint pins for
/// repo-local Rust, and the toolchain override when one is set.
fn apply_local_rule_host_env(command: &mut ProcessCommand, cache_layout: &CacheLayout) {
    command
        .env(POLINT_CACHE_DIR_ENV, cache_layout.root())
        .env("CARGO_TARGET_DIR", cache_layout.rules_target_dir());
    if let Ok(toolchain) = std::env::var(rules_host_error::POLINT_RULES_TOOLCHAIN)
        && !toolchain.is_empty()
    {
        command.env("RUSTUP_TOOLCHAIN", toolchain);
    }
}

/// Compile the rule host and answer where its binary is, or `None` when the
/// build produced no binary this can attribute to it.
///
/// The machine-readable stream is what names the binary; it is cargo's own
/// answer rather than a path this reconstructs, so a target directory laid out
/// for a cross-compilation target or a named profile needs no special case.
/// The caller treats every error as a miss and invokes the original `cargo run`
/// path, so a failed speculative build cannot change user-facing behavior.
///
/// # Errors
///
/// Errors are internal to the speculative store path and must be degraded by the
/// caller.
fn build_local_rule_host_binary(
    root: &Path,
    manifest: &Path,
    cargo: &str,
    cache_layout: &CacheLayout,
) -> Result<Option<PathBuf>> {
    let mut command = ProcessCommand::new(cargo);
    command.current_dir(root).args([
        "build",
        "--quiet",
        "--message-format=json-render-diagnostics",
    ]);
    apply_local_rule_host_profile(&mut command);
    command.args(["--manifest-path", manifest_path_argument(manifest)?]);
    apply_local_rule_host_env(&mut command, cache_layout);

    let output = command.output().with_context(|| {
        format!(
            "failed to build local rule host {} with `{cargo}`",
            manifest.display()
        )
    })?;
    if !output.status.success() {
        anyhow::bail!("speculative local rule-host build failed");
    }
    Ok(built_rule_host_binary(
        &output.stdout,
        &cache_layout.rules_target_dir(),
        &LocalRuleHostProfile::from_env().target_subdirectory(),
    ))
}

/// The one executable a rule-host build produced, or `None` when the build
/// cannot be attributed to exactly one.
///
/// A build script is an executable cargo builds and never the host, so only
/// binary targets are considered, and only those under the profile directory of
/// the target directory polint pinned. A package declaring several binaries is
/// one `cargo run` could not have resolved either, so it is refused rather than
/// guessed at.
fn built_rule_host_binary(
    stdout: &[u8],
    rules_target_dir: &Path,
    profile_directory: &str,
) -> Option<PathBuf> {
    let mut executables = std::str::from_utf8(stdout)
        .ok()?
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|message| message["reason"] == "compiler-artifact")
        .filter(|message| {
            message["target"]["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "bin"))
        })
        .filter_map(|message| Some(PathBuf::from(message["executable"].as_str()?)))
        .filter(|executable| {
            executable.starts_with(rules_target_dir)
                && executable
                    .parent()
                    .and_then(Path::file_name)
                    .is_some_and(|directory| directory == profile_directory)
        })
        .collect::<Vec<_>>();
    executables.sort();
    executables.dedup();
    match executables.as_slice() {
        [only] => Some(only.clone()),
        _ => None,
    }
}

/// Run an already-compiled rule host.
fn run_local_rule_host_binary(
    root: &Path,
    manifest: &Path,
    binary: &Path,
    cache_layout: &CacheLayout,
    host_args: &[OsString],
) -> Option<Result<(Vec<Diagnostic>, Vec<crate::diagnostics::RuleExecutionRow>)>> {
    let mut command = ProcessCommand::new(binary);
    command.current_dir(root).args(host_args);
    apply_local_rule_host_env(&mut command, cache_layout);
    let output = command.output().ok()?;
    // Cargo adds its own process-failure diagnostic when a host exits nonzero.
    // Re-enter the original path so that stderr and the final error remain
    // byte-for-byte identical instead of reporting only the direct child's
    // streams. A successful process has no Cargo wrapper output to reproduce.
    if !output.status.success() {
        return None;
    }
    Some(local_rule_host_report(manifest, &output))
}

/// Compile and run the rule host in one cargo invocation.
///
/// This is what runs when the build's inputs cannot be named — an unresolvable
/// toolchain, a rule package this cannot read, a cargo config that redirects the
/// build — and it is the behavior polint has always had.
fn run_local_rule_host_through_cargo(
    root: &Path,
    manifest: &Path,
    cargo: &str,
    cache_layout: &CacheLayout,
    host_args: &[OsString],
) -> Result<(Vec<Diagnostic>, Vec<crate::diagnostics::RuleExecutionRow>)> {
    let mut command = ProcessCommand::new(cargo);
    command.current_dir(root).args(["run", "--quiet"]);
    apply_local_rule_host_profile(&mut command);
    command.args(["--manifest-path", manifest_path_argument(manifest)?, "--"]);
    command.args(host_args);
    apply_local_rule_host_env(&mut command, cache_layout);

    let output = command.output().with_context(|| {
        format!(
            "failed to run local rule host {} with `{cargo}`",
            manifest.display()
        )
    })?;
    local_rule_host_report(manifest, &output)
}

/// A rule host's manifest as a `--manifest-path` argument.
fn manifest_path_argument(manifest: &Path) -> Result<&str> {
    manifest
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("non-UTF-8 manifest path: {}", manifest.display()))
}

/// The diagnostics and rule rows a finished rule host reported.
fn local_rule_host_report(
    manifest: &Path,
    output: &std::process::Output,
) -> Result<(Vec<Diagnostic>, Vec<crate::diagnostics::RuleExecutionRow>)> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        anyhow::bail!(rules_host_error::rules_host_error_message(
            &manifest.display().to_string(),
            output.status,
            stdout.as_ref(),
            stderr.as_ref(),
        ));
    }

    let stdout = std::str::from_utf8(&output.stdout).with_context(|| {
        format!(
            "local rule host emitted non-UTF-8 output: {}",
            manifest.display()
        )
    })?;
    diagnostics_and_rule_execution_from_public_json_report(stdout).with_context(|| {
        format!(
            "local rule host did not emit polint JSON report: {}",
            manifest.display()
        )
    })
}

fn merge_rule_execution_rows(rows: &mut Vec<crate::diagnostics::RuleExecutionRow>) {
    rows.sort_by(|left, right| left.rule_id.cmp(&right.rule_id));
    rows.dedup_by(|left, right| left.rule_id == right.rule_id);
}

fn run_local_rule_host_inspect(root: &Path, manifest: &Path) -> Result<InspectRuleReport> {
    let cargo = std::env::var("POLINT_CARGO")
        .or_else(|_| std::env::var("CARGO"))
        .unwrap_or_else(|_| "cargo".to_string());
    let cache_layout = CacheLayout::for_repo(root);
    let mut command = ProcessCommand::new(&cargo);
    command.current_dir(root).args(["run", "--quiet"]);
    apply_local_rule_host_profile(&mut command);
    command.args([
        "--manifest-path",
        manifest
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("non-UTF-8 manifest path: {}", manifest.display()))?,
        "--",
        "inspect",
        "rule",
        "--format",
        "json",
    ]);
    command
        .env(POLINT_CACHE_DIR_ENV, cache_layout.root())
        .env("CARGO_TARGET_DIR", cache_layout.rules_target_dir());
    if let Ok(toolchain) = std::env::var(rules_host_error::POLINT_RULES_TOOLCHAIN)
        && !toolchain.is_empty()
    {
        command.env("RUSTUP_TOOLCHAIN", toolchain);
    }

    let output = command.output().with_context(|| {
        format!(
            "failed to inspect local rule host {} with `{cargo}`",
            manifest.display()
        )
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        anyhow::bail!(rules_host_error::rules_host_error_message(
            &manifest.display().to_string(),
            output.status,
            stdout.as_ref(),
            stderr.as_ref(),
        ));
    }

    let stdout = String::from_utf8(output.stdout).with_context(|| {
        format!(
            "local rule host emitted non-UTF-8 inspect output: {}",
            manifest.display()
        )
    })?;
    serde_json::from_str(&stdout).with_context(|| {
        format!(
            "local rule host did not emit polint inspect JSON: {}",
            manifest.display()
        )
    })
}

fn local_manifest_path(root: &Path, manifest: &Path) -> String {
    manifest
        .strip_prefix(root)
        .unwrap_or(manifest)
        .to_string_lossy()
        .replace('\\', "/")
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LocalRuleHostProfile {
    Dev,
    Release,
    Custom(String),
}

impl LocalRuleHostProfile {
    fn from_env() -> Self {
        Self::from_env_value(std::env::var(POLINT_RULES_PROFILE_ENV).ok())
    }

    /// The profile's cargo name, as it appears in a build fingerprint.
    fn name(&self) -> String {
        match self {
            Self::Dev => "dev".to_string(),
            Self::Release => "release".to_string(),
            Self::Custom(profile) => profile.clone(),
        }
    }

    /// The directory cargo puts this profile's output in, under the target
    /// directory. Every profile uses its own name except `dev`, whose output
    /// cargo has always written to `debug`.
    fn target_subdirectory(&self) -> String {
        match self {
            Self::Dev => "debug".to_string(),
            other => other.name(),
        }
    }

    fn from_env_value(value: Option<String>) -> Self {
        let Some(value) = value else {
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

fn exit_code_for(diagnostics: &[crate::diagnostics::Diagnostic], fail_on: FailOn) -> u8 {
    let threshold = match fail_on {
        FailOn::Warn => Some(Severity::Warn),
        FailOn::Error => Some(Severity::Error),
        FailOn::None => None,
    };
    if let Some(threshold) = threshold
        && diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity.is_at_least(threshold))
    {
        return 1;
    }
    0
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::{
        Cli, FactsListReport, LocalRuleHostProfile, enabled_language_features,
        explain_derived_edge_provenance, public_fact_view,
    };
    #[cfg(unix)]
    use super::{ScaffoldWrite, commit_new_rule_scaffold_with};

    /// `--category` is how a user reaches a cache directory, so a directory
    /// polint manages but the flag cannot name is a directory nobody can clean,
    /// and a flag value that does not match the directory name is a directory
    /// nobody can find.
    #[test]
    fn cache_category_arg_covers_every_managed_category() {
        use super::{CacheCategoryArg, CacheManagedCategory, category_arg_to_managed};
        use clap::ValueEnum;

        let selectable = CacheCategoryArg::value_variants()
            .iter()
            .filter_map(|variant| {
                let category = category_arg_to_managed(*variant)?;
                let value = variant
                    .to_possible_value()
                    .expect("every category is selectable");
                Some((value.get_name().to_string(), category.name()))
            })
            .collect::<Vec<_>>();

        for (value, category) in &selectable {
            assert_eq!(
                value, category,
                "the --category value must be the cache directory's own name"
            );
        }
        assert_eq!(
            selectable
                .iter()
                .map(|(_, category)| *category)
                .collect::<Vec<_>>(),
            CacheManagedCategory::ALL
                .iter()
                .map(|category| category.name())
                .collect::<Vec<_>>(),
            "cache --category must name every managed cache category, in the same order"
        );
    }

    #[test]
    fn rule_host_dependency_features_match_the_cli_build() {
        let features = enabled_language_features();
        assert_eq!(features.contains(&"lang-go"), cfg!(feature = "lang-go"));
        assert_eq!(
            features.contains(&"lang-typescript"),
            cfg!(feature = "lang-typescript")
        );
    }

    #[cfg(unix)]
    #[test]
    fn new_rule_transaction_rolls_back_failure_at_every_write_boundary() {
        let sentinel = b"fn main() { polint::runner::run_cli(vec![]) }\n";
        for failed_boundary in 0..5 {
            let repo = tempfile::tempdir().expect("repo");
            let main = repo.path().join(".polint/rules/src/main.rs");
            std::fs::create_dir_all(main.parent().expect("main parent"))
                .expect("create existing pack");
            std::fs::write(&main, sentinel).expect("write existing main");
            let main_previous = crate::repo_fs::read_optional_repo_file_snapshot(
                repo.path(),
                ".polint/rules/src/main.rs",
            )
            .expect("snapshot main")
            .expect("main exists");
            let writes = vec![
                ScaffoldWrite::create(".polint/rules/Cargo.toml", b"[workspace]\n".to_vec()),
                ScaffoldWrite::replace(
                    ".polint/rules/src/main.rs",
                    b"updated main\n".to_vec(),
                    main_previous,
                ),
                ScaffoldWrite::create(".polint/rules/src/demo.rs", b"rule module\n".to_vec()),
                ScaffoldWrite::create(
                    ".polint/tests/rules/demo/clean/polint-test.toml",
                    b"fixture manifest\n".to_vec(),
                ),
                ScaffoldWrite::create(
                    ".polint/tests/rules/demo/clean/src/example.ts",
                    b"fixture source\n".to_vec(),
                ),
            ];
            let mut boundary = 0usize;

            let error = commit_new_rule_scaffold_with(
                repo.path(),
                &writes,
                |root, write, created_directories| {
                    let current = boundary;
                    boundary += 1;
                    if current == failed_boundary {
                        return Err(crate::repo_fs::RepoFileReadError::Write);
                    }
                    if let Some(previous) = &write.previous {
                        crate::repo_fs::write_repo_file_atomic_tracked(
                            root,
                            &write.relative_path,
                            &write.contents,
                            previous,
                            created_directories,
                        )
                    } else {
                        crate::repo_fs::write_repo_file_atomic_noclobber_tracked(
                            root,
                            &write.relative_path,
                            &write.contents,
                            created_directories,
                        )
                    }
                },
            )
            .expect_err("injected boundary failure must fail the transaction");

            assert!(
                error.to_string().contains("scaffold was rolled back"),
                "boundary {failed_boundary}: {error:#}"
            );
            assert_eq!(
                std::fs::read(&main).expect("restored main"),
                sentinel,
                "boundary {failed_boundary}"
            );
            assert!(
                !repo.path().join(".polint/rules/Cargo.toml").exists(),
                "boundary {failed_boundary}"
            );
            assert!(
                !repo.path().join(".polint/rules/src/demo.rs").exists(),
                "boundary {failed_boundary}"
            );
            assert!(
                !repo.path().join(".polint/tests").exists(),
                "boundary {failed_boundary}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn new_rule_rollback_preserves_concurrent_destination_replacement() {
        let repo = tempfile::tempdir().expect("repo");
        let main = repo.path().join(".polint/rules/src/main.rs");
        std::fs::create_dir_all(main.parent().expect("main parent")).expect("create pack");
        std::fs::write(&main, b"original main\n").expect("write original");
        let previous = crate::repo_fs::read_optional_repo_file_snapshot(
            repo.path(),
            ".polint/rules/src/main.rs",
        )
        .expect("snapshot main")
        .expect("main exists");
        let writes = vec![
            ScaffoldWrite::replace(
                ".polint/rules/src/main.rs",
                b"transaction main\n".to_vec(),
                previous,
            ),
            ScaffoldWrite::create(
                ".polint/rules/src/demo.rs",
                b"transaction module\n".to_vec(),
            ),
        ];
        let mut boundary = 0usize;

        let error = commit_new_rule_scaffold_with(
            repo.path(),
            &writes,
            |root, write, created_directories| {
                let current = boundary;
                boundary += 1;
                if current == 1 {
                    std::fs::write(&main, b"concurrent replacement\n")
                        .expect("replace committed destination concurrently");
                    return Err(crate::repo_fs::RepoFileReadError::Write);
                }
                let previous = write.previous.as_ref().expect("first write replaces main");
                crate::repo_fs::write_repo_file_atomic_tracked(
                    root,
                    &write.relative_path,
                    &write.contents,
                    previous,
                    created_directories,
                )
            },
        )
        .expect_err("injected later failure must fail transaction");

        assert!(
            error
                .to_string()
                .contains("rollback refused a concurrent replacement"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read(&main).expect("concurrent main remains"),
            b"concurrent replacement\n"
        );
        assert!(!repo.path().join(".polint/rules/src/demo.rs").exists());
    }

    #[cfg(unix)]
    #[test]
    fn new_rule_rollback_does_not_delete_concurrently_replaced_created_file() {
        let repo = tempfile::tempdir().expect("repo");
        let destination = repo.path().join(".polint/rules/src/demo.rs");
        let writes = vec![
            ScaffoldWrite::create(
                ".polint/rules/src/demo.rs",
                b"transaction module\n".to_vec(),
            ),
            ScaffoldWrite::create(
                ".polint/tests/rules/demo/polint-test.toml",
                b"later write\n".to_vec(),
            ),
        ];
        let mut boundary = 0usize;

        let error = commit_new_rule_scaffold_with(
            repo.path(),
            &writes,
            |root, write, created_directories| {
                let current = boundary;
                boundary += 1;
                if current == 1 {
                    let replacement = destination.with_extension("replacement");
                    std::fs::write(&replacement, b"concurrent replacement\n")
                        .expect("write replacement inode");
                    std::fs::rename(&replacement, &destination)
                        .expect("replace created destination concurrently");
                    return Err(crate::repo_fs::RepoFileReadError::Write);
                }
                crate::repo_fs::write_repo_file_atomic_noclobber_tracked(
                    root,
                    &write.relative_path,
                    &write.contents,
                    created_directories,
                )
            },
        )
        .expect_err("injected later failure must fail transaction");

        assert!(
            error
                .to_string()
                .contains("rollback refused a concurrent replacement"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read(&destination).expect("concurrent file remains"),
            b"concurrent replacement\n"
        );
    }

    #[test]
    fn explain_private_plumbing_surfaces_derived_edge_provenance() {
        // D-10: the private plumbing reached from `explain` surfaces, for a derived
        // edge, its contributing facts + constraint kind + solver step — WITHOUT a
        // new public JSON field.
        use crate::analysis::ids::SemanticConstraintId;
        use crate::analysis::points_to::facts::{PointsToPrecision, PointsToStatus};
        use crate::analysis::semantic_graph::constraints::{ConstraintFact, ConstraintKind};
        use crate::analysis::solver::engine::derive_edges;
        use crate::analysis::solver::store::SolverStore;

        fn copy(stable_key: &str, src: u64, dst: u64) -> ConstraintFact {
            ConstraintFact {
                id: SemanticConstraintId(0),
                kind: ConstraintKind::CopyEdge {
                    dst: crate::analysis::ids::SemanticNodeId(dst),
                    src: crate::analysis::ids::SemanticNodeId(src),
                },
                status: PointsToStatus::Present,
                precision: PointsToPrecision::FlowInsensitive,
                stable_key: crate::core::stable_key_for_test(stable_key),
            }
        }

        let constraints = vec![copy("copy|a-b", 1, 2), copy("copy|b-c", 2, 3)];
        let budget = crate::analysis::solver::budget::SolverBudget::default();
        let interner = crate::core::test_stable_key_interner();
        let output = derive_edges(&interner, &constraints, &budget);
        let store = SolverStore::from_output(output, &interner).expect("store");

        // Pick the transitive edge (the one with 2 contributing facts).
        let transitive = store
            .derived_edges()
            .iter()
            .find(|e| e.provenance.contributing_facts.len() == 2)
            .expect("transitive derived edge");

        let view = explain_derived_edge_provenance(
            &store,
            &interner,
            interner.resolve(transitive.stable_key).as_ref(),
        )
        .expect("provenance surfaced via private plumbing");
        assert_eq!(view.constraint_kind, "copy_edge");
        assert_eq!(view.contributing_fact_keys.len(), 2);
        assert!(view.solver_step > 0);
        // A missing edge key yields None (no panic, no public surface).
        assert!(
            explain_derived_edge_provenance(&store, &interner, "edge|does|not|exist").is_none()
        );
    }

    #[test]
    fn local_rule_host_profile_defaults_to_release() {
        assert_eq!(
            LocalRuleHostProfile::from_env_value(None),
            LocalRuleHostProfile::Release
        );
    }

    #[test]
    fn local_rule_host_profile_accepts_dev_aliases_for_fast_local_builds() {
        assert_eq!(
            LocalRuleHostProfile::from_env_value(Some("dev".to_string())),
            LocalRuleHostProfile::Dev
        );
        assert_eq!(
            LocalRuleHostProfile::from_env_value(Some("debug".to_string())),
            LocalRuleHostProfile::Dev
        );
        assert_eq!(
            LocalRuleHostProfile::from_env_value(Some(String::new())),
            LocalRuleHostProfile::Dev
        );
    }

    #[test]
    fn local_rule_host_profile_accepts_custom_cargo_profiles() {
        assert_eq!(
            LocalRuleHostProfile::from_env_value(Some("profiling".to_string())),
            LocalRuleHostProfile::Custom("profiling".to_string())
        );
    }

    #[test]
    fn facts_list_reports_phase55_preview_capabilities() {
        let report = FactsListReport::new();
        let preview = report
            .views
            .iter()
            .filter(|view| {
                matches!(
                    view.capability,
                    "events" | "calls" | "control_flow" | "dataflow"
                )
            })
            .map(|view| {
                (
                    view.capability,
                    view.view_type,
                    view.canonical_path,
                    view.stability,
                    view.docs_path,
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            preview,
            vec![
                (
                    "calls",
                    "Calls",
                    "polint::sdk::facts::Calls<'_>",
                    "preview",
                    "docs/facts/calls.md"
                ),
                (
                    "control_flow",
                    "ControlFlow",
                    "polint::sdk::facts::ControlFlow<'_>",
                    "preview",
                    "docs/facts/control-flow.md"
                ),
                (
                    "dataflow",
                    "DataFlow",
                    "polint::sdk::facts::DataFlow<'_>",
                    "preview",
                    "docs/facts/data-flow.md"
                ),
                (
                    "events",
                    "Events",
                    "polint::sdk::facts::Events<'_>",
                    "preview",
                    "docs/facts/events.md"
                ),
            ]
        );

        assert_eq!(public_fact_view("cfg").unwrap().stability, "reserved");
        assert_eq!(
            public_fact_view("call_graph").unwrap().stability,
            "reserved"
        );
    }

    #[test]
    fn public_help_does_not_expose_phase33_internal_markers() {
        let mut command = Cli::command();
        let help = command
            .find_subcommand_mut("check")
            .expect("check subcommand exists")
            .render_long_help()
            .to_string();

        for marker in [
            ["de", "mand"].concat(),
            ["s", "cc"].concat(),
            ["quaran", "tine"].concat(),
            ["back", "dating"].concat(),
        ] {
            assert!(
                !help.contains(marker.as_str()),
                "public help leaked internal implementation marker `{marker}`"
            );
        }
    }
}
