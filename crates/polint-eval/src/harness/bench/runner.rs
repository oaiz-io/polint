//! Whole-repo performance runner (BENCH-01, success criteria 2).
//!
//! Drives a `polint check`-equivalent (and, when a review ref is supplied, the
//! diff-sizing for a `polint review`) over a checked-out repo and captures a
//! single [`CurvePoint`] via the Plan 01 measurement substrate: cold/warm
//! wall-clock and real OS peak RSS from [`measure::cold_then_warm`], the on-disk
//! layer-cache size, and the budget-exhaustion counters folded from the live
//! `AnalysisDb`.
//!
//! Everything here is test-facing (`#[cfg(test)]`): it goes through the
//! capability-gated `AnalysisKernel::run` pipeline (PERF-01 discipline) rather
//! than eagerly reading the whole repo, and it exposes no public/SDK/CLI
//! surface. The per-point measurement is aggregated into a multi-point
//! [`CurveSeries`](crate::eval::bench::curve::CurveSeries) by the sweep
//! entry-point.

#![cfg(test)]

use std::collections::BTreeMap;
use std::path::Path;

use crate::analysis_kernel::{AnalysisKernel, KernelInput, KernelOutput};
use crate::core::AnalysisDb;
use crate::eval::bench::curve::{BudgetExhaustionCounters, CurvePoint, StoreSizeBytes};
use crate::measure;

/// Measure a single [`CurvePoint`] for `repo_root`.
///
/// The measurement:
/// 1. derives repo size (`repo_file_count`, `repo_source_bytes`) from the source
///    set the capability-gated pipeline actually loads — no separate whole-repo
///    eager read;
/// 2. drives the `polint check` equivalent through [`AnalysisKernel::run`],
///    wrapped in [`measure::cold_then_warm`] so the cold and warm wall-clock and
///    the real OS peak RSS are captured;
/// 3. when `review_ref` is `Some`, derives the diff size (`diff_files`,
///    `diff_hunk_lines`) via [`crate::git::changeset_for_ref`] as the
///    review-kind (diff-gated) measurement — the analysis cost the review shares
///    with `check` is what the timing captures, and the diff gate itself is a
///    cheap reporting-layer filter;
/// 4. reads layer-cache and semantic-store sizes into [`StoreSizeBytes`]
///    without double-counting store bytes;
/// 5. folds budget-exhaustion telemetry from the live `AnalysisDb` (see
///    [`budget_counters`]).
pub(crate) fn run_repo_perf_point(
    repo_root: &Path,
    review_ref: Option<&str>,
) -> anyhow::Result<CurvePoint> {
    run_repo_perf_point_with_store_mode(repo_root, review_ref, SemanticStoreBenchMode::Disabled)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SemanticStoreBenchMode {
    Disabled,
    Enabled,
}

fn run_repo_perf_point_with_store_mode(
    repo_root: &Path,
    review_ref: Option<&str>,
    store_mode: SemanticStoreBenchMode,
) -> anyhow::Result<CurvePoint> {
    // Key the point by the repo directory name only — never an absolute host
    // path (threat T-63-02-04: curve JSON must not leak `/Users/` or `/home/`).
    let repo_id = repo_root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".to_string());

    // Diff size for the review measurement. `changeset_for_ref` passes the ref as
    // a fixed positional arg to the git binary (no shell) — threat T-63-02-01.
    let (diff_files, diff_hunk_lines) = match review_ref {
        Some(target) => {
            let changeset = crate::git::changeset_for_ref(repo_root, target)?;
            let files = changeset.files.len() as u64;
            let hunk_lines: u64 = changeset
                .files
                .iter()
                .flat_map(|file| file.new_line_ranges.iter())
                // Count inclusive, non-empty ranges only: a degenerate or
                // inverted `end < start` entry contributes nothing. Widen to
                // u64 *before* the `+ 1` so a `u32::MAX`-wide range cannot
                // overflow-panic in a debug build.
                .filter(|(start, end)| end >= start)
                .map(|(start, end)| u64::from(end.saturating_sub(*start)) + 1)
                .sum();
            (files, hunk_lines)
        }
        None => (0, 0),
    };

    // Drive the check-equivalent kernel run twice (cold then warm) and keep the
    // warm run's output for the fact walk. Caching is enabled so the warm run
    // exercises the persistent layer cache and populates `.polint/cache`.
    // Keep BOTH runs' outcomes: the closure runs twice, so overwriting a single
    // slot would let a warm-run success mask a cold-run failure (and
    // `cold_wall_clock_ms` would then describe a run that actually errored).
    // The cold run keeps only its `Result<(), _>`: retaining its `KernelOutput`
    // pins a whole `AnalysisDb` (~8 GB on a repo the size of excalidraw) while
    // the warm run builds a second one, which doubles the measured peak and is
    // what turned an 8.9 GB engine into a 12 GB SIGKILL in the committed
    // scale-corpus artifact.
    let mut cold_result: Option<anyhow::Result<()>> = None;
    let mut warm_result: Option<anyhow::Result<KernelOutput>> = None;
    let cold_only = std::env::var_os(CHILD_COLD_ONLY_ENV).is_some();
    let timing = if cold_only {
        let single = measure::TimedRun::measure(|| {
            warm_result = Some(run_check_kernel_with_store_mode(repo_root, store_mode));
        });
        cold_result = Some(Ok(()));
        measure::ColdWarm {
            cold_ms: single.elapsed_ms,
            warm_ms: single.elapsed_ms,
            peak_rss_bytes: single.peak_rss_bytes,
            peak_rss_delta_bytes: single.peak_rss_delta_bytes,
        }
    } else {
        measure::cold_then_warm(|| {
            if cold_result.is_none() {
                // Drop the cold `AnalysisDb` here; only the outcome is needed.
                cold_result =
                    Some(run_check_kernel_with_store_mode(repo_root, store_mode).map(|_| ()));
            } else {
                warm_result = Some(run_check_kernel_with_store_mode(repo_root, store_mode));
            }
        })
    };
    // Propagate a cold-run failure before trusting any timing; on success keep
    // the warm run's output for the fact walk.
    cold_result.expect("cold_then_warm runs the closure at least once")?;
    let output = warm_result.expect("cold_then_warm runs the closure twice")?;

    // Repo size from the source set the pipeline loaded — not a separate eager
    // whole-repo read (PERF-01 discipline).
    let files = output.db.files();
    let repo_file_count = files.len() as u64;
    let repo_source_bytes: u64 = files.iter().map(|file| file.source.len() as u64).sum();

    let cache_layout = crate::cache::CacheLayout::for_repo(repo_root);
    let semantic_store_bytes = dir_size_bytes(&cache_layout.semantic_store_dir());
    let size = StoreSizeBytes {
        cache_bytes: dir_size_bytes(cache_layout.root()).saturating_sub(semantic_store_bytes),
        store_bytes: if store_mode == SemanticStoreBenchMode::Enabled {
            semantic_store_bytes
        } else {
            0
        },
    };

    // Diagnostics identity: the number the goal actually protects. Printed at
    // `info` so a measurement run can diff `polint check` output between two
    // engine revisions without a second harness.
    tracing::info!(
        target: "polint::kernel::stage",
        diagnostics = output.diagnostics.len(),
        diagnostics_digest = digest_diagnostics(&output.diagnostics),
        "run diagnostics"
    );
    tracing::debug!(
        target: "polint::kernel::stage",
        diagnostics = ?output.diagnostics,
        "run diagnostics details"
    );

    let budget = budget_counters(&output.db);

    Ok(CurvePoint {
        repo_id,
        repo_file_count,
        repo_source_bytes,
        diff_files,
        diff_hunk_lines,
        cold_wall_clock_ms: timing.cold_ms,
        warm_wall_clock_ms: timing.warm_ms,
        peak_rss_bytes: timing.peak_rss_bytes,
        peak_rss_delta_bytes: timing.peak_rss_delta_bytes,
        size,
        budget,
    })
}

/// Full libtest path of the child measurement entry [`tests::perf_child_measure_entry`].
/// `run_repo_perf_point_isolated` re-invokes this binary filtered to exactly this
/// test; keep it in sync with the module path if the test moves.
const CHILD_MEASURE_TEST: &str = "eval::bench::runner::tests::perf_child_measure_entry";
/// Env var carrying the repo path the child measurement entry should measure.
/// Presence of this var is what switches the child entry from a no-op into a
/// measuring run.
const CHILD_REPO_ENV: &str = "POLINT_PERF_CHILD_REPO";
/// Env var carrying the optional review ref (empty/absent == a `check` point).
const CHILD_REVIEW_ENV: &str = "POLINT_PERF_CHILD_REVIEW_REF";
/// Internal libtest-only mode selector, not a supported CLI/config contract.
const CHILD_SEMANTIC_STORE_ENV: &str = "POLINT_PERF_CHILD_SEMANTIC_STORE";
/// Comma-separated capability override for attribution runs (libtest-only).
/// Absent == the default `full_pipeline_for_test` capability set. Lets a
/// measurement bisect which capability owns the cost without rebuilding.
const CHILD_CAPABILITIES_ENV: &str = "POLINT_PERF_CHILD_CAPABILITIES";
/// When set, measure the cold run only and report it as both cold and warm
/// (libtest-only). Halves the cost of an attribution sweep.
const CHILD_COLD_ONLY_ENV: &str = "POLINT_PERF_CHILD_COLD_ONLY";
/// Stdout markers the child prints the serialized [`CurvePoint`] JSON between, so
/// the parent can extract it from libtest's own `--nocapture` output.
const CHILD_POINT_BEGIN: &str = "<<<POLINT_PERF_POINT_BEGIN>>>";
const CHILD_POINT_END: &str = "<<<POLINT_PERF_POINT_END>>>";

/// Measure a single [`CurvePoint`] for `repo_root` in a DEDICATED CHILD PROCESS
/// so its `peak_rss_delta_bytes` is genuinely run-attributable and
/// order-independent (HI-01R).
///
/// `peak_rss_bytes` is `getrusage(RUSAGE_SELF).ru_maxrss` — a process-global,
/// monotonic, whole-lifetime high-water mark. When several measurements share
/// one process (as the baseline regenerator does: a digest run, then a check
/// point, then a review point), the mark saturates on the first run and every
/// later per-run delta collapses to allocator jitter — the committed review
/// baseline captured a meaningless 16 KiB exactly this way, while a differently
/// a warmed process would report the true footprint (tens of MiB) and
/// false-block the gate.
///
/// Re-executing the measurement in a fresh child gives each run its own
/// unsaturated high-water mark, so the child-computed
/// [`ColdWarm::peak_rss_delta_bytes`](measure::ColdWarm) reflects the kernel
/// run's marginal footprint over a fixed process baseline regardless of what the
/// parent already allocated or the order the points are taken in. The child is
/// this same test binary re-invoked (`current_exe`) filtered to
/// [`tests::perf_child_measure_entry`], which runs [`run_repo_perf_point`] and
/// prints the [`CurvePoint`] as JSON between [`CHILD_POINT_BEGIN`]/
/// [`CHILD_POINT_END`] on stdout, then exits.
///
/// The committed store-disabled baselines are regenerated through THIS path; a
/// A measured run fed to `evaluate_regression_budget` MUST use the same
/// isolation for the peak-RSS delta to be comparable rather than an artifact of
/// process warm-up order.
pub(crate) fn run_repo_perf_point_isolated(
    repo_root: &Path,
    review_ref: Option<&str>,
) -> anyhow::Result<CurvePoint> {
    run_repo_perf_point_isolated_with_store_mode(
        repo_root,
        review_ref,
        SemanticStoreBenchMode::Disabled,
    )
}

pub(crate) fn run_repo_perf_point_isolated_with_store_mode(
    repo_root: &Path,
    review_ref: Option<&str>,
    store_mode: SemanticStoreBenchMode,
) -> anyhow::Result<CurvePoint> {
    let exe = std::env::current_exe().map_err(|error| {
        anyhow::anyhow!("locating test binary for isolated perf child: {error}")
    })?;
    let mut command = std::process::Command::new(&exe);
    command
        .arg("--exact")
        .arg(CHILD_MEASURE_TEST)
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env(CHILD_REPO_ENV, repo_root)
        // Never let the child inherit the regenerator switch (it would re-enter
        // regeneration and recurse) or a stale review ref from an outer spawn.
        .env_remove("POLINT_WRITE_STORE_DISABLED_BASELINE")
        .env_remove("POLINT_WRITE_SCALE_CORPUS")
        .env_remove(CHILD_REVIEW_ENV)
        .env_remove(CHILD_SEMANTIC_STORE_ENV);
    if let Some(reference) = review_ref {
        command.env(CHILD_REVIEW_ENV, reference);
    }
    if store_mode == SemanticStoreBenchMode::Enabled {
        command.env(CHILD_SEMANTIC_STORE_ENV, "enabled");
    }
    let output = command
        .output()
        .map_err(|error| anyhow::anyhow!("spawning isolated perf child: {error}"))?;
    if !output.status.success() {
        anyhow::bail!(
            "isolated perf child exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json = stdout
        .split_once(CHILD_POINT_BEGIN)
        .and_then(|(_, rest)| rest.split_once(CHILD_POINT_END))
        .map(|(json, _)| json.trim())
        .ok_or_else(|| {
            anyhow::anyhow!("isolated perf child emitted no curve point; stdout: {stdout}")
        })?;
    let point: CurvePoint = serde_json::from_str(json)
        .map_err(|error| anyhow::anyhow!("parsing isolated perf child curve point: {error}"))?;
    Ok(point)
}

/// Deterministic digest of the diagnostics a store-disabled (== current)
/// `polint check` produces over `repo_root`.
///
/// This is the diagnostics-parity marker a
/// [`StoreDisabledBaseline`](crate::eval::baseline::StoreDisabledBaseline)
/// records (BENCH-02): enabling the durable store must not change the
/// diagnostics polint emits, so a later run can assert this digest is unchanged.
/// It is the FNV stable-hash over the sorted, canonical-JSON-serialized
/// diagnostics of the check-equivalent kernel run. Clean code (no diagnostics)
/// still yields a stable, non-empty digest (the hash of the empty set).
pub(crate) fn diagnostics_digest_for_repo(repo_root: &Path) -> anyhow::Result<String> {
    diagnostics_digest_for_repo_with_store_mode(repo_root, SemanticStoreBenchMode::Disabled)
}

pub(crate) fn diagnostics_digest_for_repo_with_store_mode(
    repo_root: &Path,
    store_mode: SemanticStoreBenchMode,
) -> anyhow::Result<String> {
    let output = run_check_kernel_with_store_mode(repo_root, store_mode)?;
    Ok(digest_diagnostics(&output.diagnostics))
}

fn digest_diagnostics(diagnostics: &[crate::diagnostics::Diagnostic]) -> String {
    let mut rows: Vec<String> = diagnostics
        .iter()
        .map(|diagnostic| serde_json::to_string(diagnostic).unwrap_or_default())
        .collect();
    // Sort so the digest is independent of diagnostic emission order.
    rows.sort();
    let refs: Vec<&str> = rows.iter().map(String::as_str).collect();
    crate::cache::stable_hash(&refs)
}

/// Drive one `polint check`-equivalent run through the capability-gated kernel.
///
/// This mirrors `crate::eval::observed::run_kernel_for_repo_for_test`: it loads
/// the repo config, requests the full pipeline (so the deep providers whose
/// budget counters we fold actually run), and enables caching so the warm run
/// of [`measure::cold_then_warm`] exercises the persistent layer cache.
fn run_check_kernel_with_store_mode(
    repo_root: &Path,
    store_mode: SemanticStoreBenchMode,
) -> anyhow::Result<KernelOutput> {
    let loaded = crate::config::load_config(repo_root)?;
    let config_digest = crate::cache::keys::config_hash(&loaded);
    let rule_digest = crate::cache::keys::rule_hash(&[], None, &BTreeMap::new());
    let cache = crate::cache::Cache::default_for_repo(repo_root, true);
    let cache = if store_mode == SemanticStoreBenchMode::Enabled {
        cache.with_semantic_store_enabled_for_test()
    } else {
        cache
    };
    let plan = match std::env::var(CHILD_CAPABILITIES_ENV) {
        Ok(names) if !names.trim().is_empty() => {
            let names: Vec<&str> = names
                .split(',')
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .collect();
            crate::analysis_plan::AnalysisPlan::from_capability_names_for_test(&names)
        }
        _ => crate::analysis_plan::AnalysisPlan::full_pipeline_for_test(),
    };
    AnalysisKernel::run(KernelInput {
        loaded: &loaded,
        cache: &cache,
        config_digest: &config_digest,
        rule_digest: &rule_digest,
        plan: &plan,
        parallel: true,
    })
}

/// Fold budget-exhaustion telemetry out of the live `AnalysisDb`.
///
/// `KernelRunReport` does not expose budget/token/iteration counters; they live
/// as per-fact `*::BudgetExceeded` statuses. This walks the same fact families
/// `analysis_kernel::debug` counts, mapping each reachable budget source to the
/// [`BudgetExhaustionCounters`] field it most directly evidences:
/// - `budget_exceeded`: summary facts/events with `SummaryStatus::BudgetExceeded`
///   (the solver/summary budget ceiling);
/// - `tokens_exhausted`: call targets with `CallTargetStatus::BudgetExceeded`
///   (the token/points-to budget surfaced at call resolution);
/// - `iteration_capped`: abstract-domain observations/events with
///   `DomainStatus::BudgetExceeded` (the domain solver's iteration/round cap).
fn budget_counters(db: &AnalysisDb) -> BudgetExhaustionCounters {
    use crate::analysis::calls::facts::CallTargetStatus;
    use crate::analysis::summaries::facts::SummaryStatus;
    use crate::analysis_neutral::domains::facts::DomainStatus;

    let budget_exceeded = db
        .summary_facts()
        .iter()
        .filter(|fact| fact.status == SummaryStatus::BudgetExceeded)
        .count() as u64
        + db.summary_events()
            .iter()
            .filter(|event| event.status == SummaryStatus::BudgetExceeded)
            .count() as u64;

    let tokens_exhausted = db
        .call_targets()
        .iter()
        .filter(|target| target.status == CallTargetStatus::BudgetExceeded)
        .count() as u64;

    let iteration_capped = db
        .abstract_domain_observations()
        .iter()
        .filter(|observation| observation.status == DomainStatus::BudgetExceeded)
        .count() as u64
        + db.abstract_domain_events()
            .iter()
            .filter(|event| event.status == DomainStatus::BudgetExceeded)
            .count() as u64;

    BudgetExhaustionCounters {
        budget_exceeded,
        tokens_exhausted,
        iteration_capped,
    }
}

/// Total size in bytes of all regular files under `dir` (recursive), or 0 if the
/// directory is absent. Symlinks are not followed and directory metadata is not
/// counted, so the result reflects only the cache payload bytes.
fn dir_size_bytes(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut total = 0u64;
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            total = total.saturating_add(dir_size_bytes(&entry.path()));
        } else if file_type.is_file()
            && let Ok(metadata) = entry.metadata()
        {
            total = total.saturating_add(metadata.len());
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::tempdir;

    /// Returns `true` when `git` is usable; git-dependent tests skip otherwise.
    fn git_available() -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .expect("git invocation");
        assert!(
            status.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&status.stderr)
        );
    }

    fn write(dir: &Path, rel: &str, contents: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(path, contents).expect("write file");
    }

    /// Write a small but non-trivial Go + TS repo so the full pipeline does
    /// observable, measurable (> 0 ms) work.
    fn write_tiny_repo(dir: &Path) {
        write(
            dir,
            "src/router.go",
            "package app\n\nfunc handle() { helper() }\n\nfunc helper() { println(1) }\n",
        );
        write(
            dir,
            "src/util.ts",
            "export function add(a: number, b: number): number {\n  return a + b;\n}\n\nexport function twice(n: number): number {\n  return add(n, n);\n}\n",
        );
    }

    /// Child-process entry for [`super::run_repo_perf_point_isolated`] (HI-01R).
    ///
    /// A normal `cargo test` run executes this as an immediate no-op because
    /// `POLINT_PERF_CHILD_REPO` is absent. When `run_repo_perf_point_isolated`
    /// re-invokes this binary it sets that var, so this FRESH process measures
    /// the repo and prints the [`CurvePoint`] as JSON between the shared markers,
    /// then `exit(0)`s before libtest prints its summary. Measuring in an
    /// otherwise-empty process is what makes `peak_rss_delta_bytes`
    /// order-independent rather than a shared-process high-water artifact.
    #[test]
    fn perf_child_measure_entry() {
        use std::io::Write as _;
        let Some(repo) = std::env::var_os(super::CHILD_REPO_ENV) else {
            return;
        };
        // Stage attribution: `RUST_LOG=polint::kernel::stage=info` prints one
        // row per provider (elapsed, live RSS, peak RSS) to stderr.
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_writer(std::io::stderr)
            .try_init()
            .ok();
        let review = std::env::var(super::CHILD_REVIEW_ENV).ok();
        let store_mode = if std::env::var_os(super::CHILD_SEMANTIC_STORE_ENV).is_some() {
            super::SemanticStoreBenchMode::Enabled
        } else {
            super::SemanticStoreBenchMode::Disabled
        };
        let point = super::run_repo_perf_point_with_store_mode(
            Path::new(&repo),
            review.as_deref(),
            store_mode,
        )
        .expect("isolated perf child measurement");
        let json = serde_json::to_string(&point).expect("serialize child curve point");
        println!(
            "{}{json}{}",
            super::CHILD_POINT_BEGIN,
            super::CHILD_POINT_END
        );
        std::io::stdout().flush().ok();
        std::process::exit(0);
    }

    #[test]
    fn run_repo_perf_point_isolated_measures_in_a_fresh_child() {
        let repo = tempdir().unwrap();
        write_tiny_repo(repo.path());

        let point = run_repo_perf_point_isolated(repo.path(), None).unwrap();

        assert!(
            point.repo_file_count > 0,
            "isolated child must measure repo files: {point:?}"
        );
        assert!(
            point.cold_wall_clock_ms > 0,
            "isolated child must record cold wall-clock: {point:?}"
        );
        assert!(
            point.peak_rss_bytes > 0,
            "isolated child must record peak RSS: {point:?}"
        );
        assert_eq!(point.diff_files, 0, "no review ref -> no diff");
    }

    #[test]
    fn run_repo_perf_point_measures_tiny_repo() {
        let repo = tempdir().unwrap();
        write_tiny_repo(repo.path());

        let point = run_repo_perf_point(repo.path(), None).unwrap();

        assert!(
            point.cold_wall_clock_ms > 0,
            "cold wall-clock must be measurable and > 0: {point:?}"
        );
        assert!(
            point.peak_rss_bytes > 0,
            "peak RSS must be measurable and > 0: {point:?}"
        );
        assert!(
            point.repo_file_count > 0,
            "repo file count must be > 0: {point:?}"
        );
        assert!(point.repo_source_bytes > 0, "repo source bytes must be > 0");
        // No review ref -> no diff.
        assert_eq!(point.diff_files, 0);
        assert_eq!(point.diff_hunk_lines, 0);
        // repo_id is the directory name, never an absolute host path.
        assert!(!point.repo_id.contains('/'));
        assert!(!point.repo_id.starts_with("/Users"));
    }

    #[test]
    fn run_repo_perf_point_captures_review_diff_size() {
        if !git_available() {
            eprintln!("skipping review diff-size test; `git` not on PATH");
            return;
        }
        let repo = tempdir().unwrap();
        let dir = repo.path();
        git(dir, &["init", "--quiet"]);
        git(dir, &["config", "user.email", "t@example.com"]);
        git(dir, &["config", "user.name", "Test"]);
        git(dir, &["config", "commit.gpgsign", "false"]);

        write_tiny_repo(dir);
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "--quiet", "-m", "base"]);
        let base = {
            let out = Command::new("git")
                .current_dir(dir)
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };

        // Introduce a diff on the working side.
        write(
            dir,
            "src/util.ts",
            "export function add(a: number, b: number): number {\n  return a + b + 0;\n}\n\nexport function twice(n: number): number {\n  return add(n, n);\n}\n",
        );
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "--quiet", "-m", "change"]);

        let point = run_repo_perf_point(dir, Some(&base)).unwrap();

        assert!(
            point.diff_files > 0,
            "review diff must record at least one changed file: {point:?}"
        );
        assert!(
            point.diff_hunk_lines > 0,
            "review diff must record changed hunk lines: {point:?}"
        );
        assert!(point.cold_wall_clock_ms > 0);
        assert!(point.peak_rss_bytes > 0);
    }

    mod semantic_store {
        use super::*;

        #[test]
        fn isolated_modes_report_real_store_bytes_and_equal_diagnostics_digest() {
            let disabled_repo = tempdir().expect("disabled repo");
            write_tiny_repo(disabled_repo.path());
            let disabled = run_repo_perf_point_isolated_with_store_mode(
                disabled_repo.path(),
                None,
                SemanticStoreBenchMode::Disabled,
            )
            .expect("disabled isolated measurement");
            let disabled_store_path =
                crate::cache::CacheLayout::for_repo(disabled_repo.path()).semantic_store_path();

            assert_eq!(disabled.size.store_bytes, 0);
            assert!(!disabled_store_path.exists());

            let enabled_repo = tempdir().expect("enabled repo");
            write_tiny_repo(enabled_repo.path());
            let enabled = run_repo_perf_point_isolated_with_store_mode(
                enabled_repo.path(),
                None,
                SemanticStoreBenchMode::Enabled,
            )
            .expect("enabled isolated measurement");
            let enabled_store_path =
                crate::cache::CacheLayout::for_repo(enabled_repo.path()).semantic_store_path();

            assert!(enabled.size.store_bytes > 0);
            assert!(enabled_store_path.is_file());
            assert!(AnalysisKernel::semantic_store_schema_is_current_for_test(
                &enabled_store_path
            ));
            assert_eq!(
                serde_json::to_string(&enabled).expect("serialize point"),
                serde_json::to_string(&enabled).expect("serialize point again")
            );

            let digest_repo = tempdir().expect("digest repo");
            write_tiny_repo(digest_repo.path());
            let disabled_digest = diagnostics_digest_for_repo_with_store_mode(
                digest_repo.path(),
                SemanticStoreBenchMode::Disabled,
            )
            .expect("disabled digest");
            let enabled_digest = diagnostics_digest_for_repo_with_store_mode(
                digest_repo.path(),
                SemanticStoreBenchMode::Enabled,
            )
            .expect("enabled digest");
            assert_eq!(enabled_digest, disabled_digest);
        }
    }
}
