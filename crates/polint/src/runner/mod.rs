use crate::analysis_kernel::{AnalysisKernel, KernelInput};
use crate::analysis_plan::{AnalysisPlan, RulePlanInputs};
use crate::config::{LoadedConfig, load_config};
use crate::core::{Rule, RuleKind, RuleRuntimeViews, run_rules_with_runtime_provider_blockers};
use crate::diagnostics::{
    ColorChoice, JsonReportMeta, OutputFormat, RenderOpts, Severity, apply_report_filters,
    build_ai_friendly_report, limit_report_diagnostics, render_ai_friendly_stdout,
    render_with_sarif_help,
};
use crate::ignores::apply_ignores;
use crate::rule_manifest::{InspectRuleReport, RuleManifestWire};
use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

const AI_FRIENDLY_OUTPUT_DIR: &str = ".polint/output";
const AI_FRIENDLY_LATEST_OUTPUT: &str = ".polint/output/latest.json";

#[derive(Debug, Parser)]
#[command(name = "polint-local-rules")]
#[command(about = "Run a repo-local native polint rule host.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the registered local rules against the current directory.
    Check(CheckArgs),
    /// Inspect registered local rules without running analysis.
    Inspect(InspectArgs),
}

#[derive(Debug, Args, Clone)]
struct InspectArgs {
    #[command(subcommand)]
    command: InspectCommand,
}

#[derive(Debug, Subcommand, Clone)]
enum InspectCommand {
    /// Show registered rule manifests.
    Rule(InspectRuleArgs),
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
#[command(
    after_help = "AI agents: use `--format ai-friendly` to print a compact summary and save queryable JSON under `.polint/output/`."
)]
struct CheckArgs {
    /// Files or directories to check. Defaults to the workspace include config.
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,
    /// Named profile from .polint.toml. When omitted, all registered rules run.
    #[arg(long)]
    profile: Option<String>,
    /// Output format.
    #[arg(long, value_enum, default_value_t = FormatArg::Human)]
    format: FormatArg,
    /// ANSI colors for human output (no effect on JSON/SARIF).
    #[arg(long, value_enum, default_value_t = ColorArg::Auto)]
    color: ColorArg,
    /// Disable analysis/fact cache reads and writes.
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
    /// Apply polint comment-ignore directives.
    #[arg(long, default_value_t = true, hide = true, action = clap::ArgAction::Set)]
    ignore_comments: bool,
    /// Which rule kind to run (internal; set by `polint review`).
    #[arg(long, value_enum, default_value_t = KindArg::Check, hide = true)]
    kind: KindArg,
    /// Internal: path to a JSON changeset injected by `polint review`.
    #[arg(long, value_name = "FILE", hide = true)]
    changed_files: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum KindArg {
    Check,
    Review,
}

impl From<KindArg> for RuleKind {
    fn from(value: KindArg) -> Self {
        match value {
            KindArg::Check => RuleKind::Check,
            KindArg::Review => RuleKind::Review,
        }
    }
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
enum InspectFormatArg {
    Human,
    Json,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum FailOn {
    Warn,
    Error,
    None,
}

pub fn run_cli(rules: Vec<Rule>) -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .try_init()
        .ok();
    match run(rules) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("polint-local-rules: {error:?}");
            ExitCode::from(2)
        }
    }
}

fn run(rules: Vec<Rule>) -> Result<u8> {
    let cli = Cli::parse();
    match cli.command {
        Command::Check(args) => crate::golden_cost::run_with_optional_cost(|| {
            check(std::env::current_dir()?, &args, &rules)
        }),
        Command::Inspect(args) => inspect(std::env::current_dir()?, &args, &rules),
    }
}

fn inspect(root: PathBuf, args: &InspectArgs, rules: &[Rule]) -> Result<u8> {
    match &args.command {
        InspectCommand::Rule(rule_args) => inspect_rule(root, rule_args, rules),
    }
}

fn inspect_rule(root: PathBuf, args: &InspectRuleArgs, rules: &[Rule]) -> Result<u8> {
    let loaded = load_config_for_check(&root, &[])?;
    let mut plan_inputs = RulePlanInputs::collect(rules, None);
    plan_inputs.retain_rule_pattern(args.rule.as_deref());
    let selected_rule_ids = plan_inputs.exact_rule_ids();
    if args.rule.is_some() && selected_rule_ids.is_empty() {
        anyhow::bail!(
            "no registered rule matched `{}`",
            args.rule.as_deref().unwrap_or_default()
        );
    }

    let options = plan_inputs.rule_options_from_config(&loaded);
    let plan = AnalysisPlan::from_inputs(&plan_inputs, &options);
    let mut manifests = Vec::new();
    for rule in rules {
        let meta = rule.meta();
        if !selected_rule_ids.contains(&meta.id) {
            continue;
        }
        let manifest = rule.manifest(options.get(&meta.id));
        manifests.push(RuleManifestWire::from_manifest(
            manifest,
            None,
            plan.support_view(),
        ));
    }
    let report = InspectRuleReport::new("polint-local-rules", env!("CARGO_PKG_VERSION"), manifests);
    match args.format {
        InspectFormatArg::Human => print!("{}", render_inspect_rule_human(&report)),
        InspectFormatArg::Json => println!("{}", serde_json::to_string_pretty(&report)?),
    }
    Ok(0)
}

fn render_inspect_rule_human(report: &InspectRuleReport) -> String {
    if report.rules.is_empty() {
        return "No registered local rules.\n".to_string();
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
        out.push_str(&format!(
            "{} [{}]: {} (capabilities: {})\n",
            rule.rule_id, rule.severity, rule.description, capabilities
        ));
    }
    out
}

fn check(root: PathBuf, args: &CheckArgs, rules: &[Rule]) -> Result<u8> {
    let (mut diagnostics, db, loaded, rule_execution) = analyze_and_run(&root, args, rules)?;
    if args.ignore_comments {
        diagnostics = apply_ignores(&db, diagnostics, &loaded.config.ignores).diagnostics;
    }
    diagnostics = apply_report_filters(diagnostics, args.only_rule.as_deref());
    let rendered_diagnostics = limit_report_diagnostics(diagnostics.clone(), args.max_diagnostics);
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
    let sources = match args.format {
        FormatArg::Human => Some(&source_map),
        _ => None,
    };
    let sarif_map = if loaded.config.sarif.rule_help_uri.is_empty() {
        None
    } else {
        Some(&loaded.config.sarif.rule_help_uri)
    };
    let opts = RenderOpts {
        json: JsonReportMeta {
            tool_name: "polint-local-rules",
            tool_version: env!("CARGO_PKG_VERSION"),
        },
        color: match args.color {
            ColorArg::Auto => ColorChoice::Auto,
            ColorArg::Always => ColorChoice::Always,
            ColorArg::Never => ColorChoice::Never,
        },
        sources,
        rule_execution: &rule_execution,
    };
    if matches!(args.format, FormatArg::AiFriendly) {
        let report =
            write_ai_friendly_report(&root, &diagnostics, &rendered_diagnostics, &rule_execution)?;
        print!(
            "{}",
            render_ai_friendly_stdout(&report, AI_FRIENDLY_LATEST_OUTPUT)
        );
    } else {
        print!(
            "{}",
            render_with_sarif_help(format, &rendered_diagnostics, opts, sarif_map)
        );
    }

    // The fact database is by far the largest structure in the process, and the
    // report it fed has already been written. Walking millions of facts to free
    // them one at a time is pure latency ahead of an exit that reclaims the whole
    // address space anyway, so hand it to the operating system.
    //
    // This is sound only here, at the end of the command: nothing reads the
    // database afterwards, and everything with a side effect on drop — the cache
    // handles and report writers — is owned elsewhere and still drops normally.
    // A library helper, which the rule host also links, must keep freeing.
    std::mem::forget(db);

    Ok(exit_code_for(&diagnostics, args.fail_on))
}

fn write_ai_friendly_report(
    root: &Path,
    diagnostics: &[crate::diagnostics::Diagnostic],
    persisted_diagnostics: &[crate::diagnostics::Diagnostic],
    rule_execution: &[crate::diagnostics::RuleExecutionRow],
) -> Result<crate::diagnostics::AiFriendlyReport> {
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
        JsonReportMeta {
            tool_name: "polint-local-rules",
            tool_version: env!("CARGO_PKG_VERSION"),
        },
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
    Ok(report)
}

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

fn gitignore_line_covers(line: &str, entry: &str) -> bool {
    let t = line.trim();
    if t.is_empty() || t.starts_with('#') {
        return false;
    }
    t.trim_end_matches('/') == entry.trim_end_matches('/')
}

fn generated_at_label() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    secs.to_string()
}

fn analyze_and_run(
    root: &Path,
    args: &CheckArgs,
    rules: &[Rule],
) -> Result<(
    Vec<crate::diagnostics::Diagnostic>,
    crate::core::AnalysisDb,
    LoadedConfig,
    Vec<crate::diagnostics::RuleExecutionRow>,
)> {
    let loaded = load_config_for_check(root, &args.paths)?;
    let cache = crate::cache::Cache::default_for_repo(root, !args.no_cache);
    let config_digest = crate::cache::keys::config_hash(&loaded);
    let enabled = selected_rule_patterns(&loaded, args.profile.as_deref())?;
    // Run only the requested rule kind. `polint check` runs `Check` rules;
    // `polint review` runs `Review` rules. Filtering here (rather than in the
    // frozen `run_rules_with_capability_support` signature) keeps capability
    // planning honest: only the selected kind's facts are planned and built.
    let wanted_kind: RuleKind = args.kind.into();
    let rules: Vec<Rule> = rules
        .iter()
        .filter(|rule| rule.meta().kind == wanted_kind)
        .cloned()
        .collect();
    let rules: &[Rule] = &rules;
    let mut plan_inputs = RulePlanInputs::collect(rules, enabled.as_ref());
    plan_inputs.retain_rule_pattern(args.only_rule.as_deref());
    let exact_enabled = plan_inputs.exact_rule_ids();
    let options = plan_inputs.rule_options_from_config(&loaded);
    let rule_digest = plan_inputs.rule_digest(&options);
    let plan = AnalysisPlan::from_inputs(&plan_inputs, &options);

    let mut diagnostics = plan_inputs.diagnostics();
    diagnostics.extend(plan.diagnostics());
    let mut output = AnalysisKernel::run(KernelInput {
        loaded: &loaded,
        cache: &cache,
        config_digest: &config_digest,
        rule_digest: &rule_digest,
        plan: &plan,
        parallel: true,
    })?;
    diagnostics.extend(std::mem::take(&mut output.diagnostics));
    // Inject the `polint review` changeset (if any) AFTER the kernel runs and
    // BEFORE rules execute, so the `ChangedFiles` fact view sees the diff. The
    // changeset is set post-kernel, so it never participates in cache identity.
    if let Some(path) = &args.changed_files {
        let changeset = read_changeset(path)?;
        output.db.set_changeset(changeset);
    }
    let runtime = RuleRuntimeViews::new(
        &output.capability_support,
        &output.completeness,
        &output.runtime_blocked_rules,
    );
    diagnostics.extend(run_rules_with_runtime_provider_blockers(
        &output.db,
        rules,
        &options,
        Some(&exact_enabled),
        true,
        &runtime,
    ));
    let file_paths = output
        .db
        .files()
        .iter()
        .map(|file| file.relative_path.as_str())
        .collect::<Vec<_>>();
    let rule_execution = plan.rule_execution_rows(&options, &file_paths, &diagnostics);
    Ok((diagnostics, output.db, loaded, rule_execution))
}

/// Read a `polint review` changeset JSON file injected via `--changed-files`.
///
/// The file is polint-internal (written by the outer `review` command), so a
/// missing or malformed file is a loud error rather than a silent empty diff.
fn read_changeset(path: &Path) -> Result<crate::core::ReviewChangeset> {
    let raw = std::fs::read_to_string(path).with_context(|| {
        format!(
            "failed to read injected changeset file {} (polint review internal)",
            path.display()
        )
    })?;
    serde_json::from_str(&raw).with_context(|| {
        format!(
            "failed to parse injected changeset file {} (polint review internal)",
            path.display()
        )
    })
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
