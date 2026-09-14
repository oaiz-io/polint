//! Parallel job cap for Rayon workers and polint-spawned child processes.
//!
//! Unset, polint uses 80% of [`std::thread::available_parallelism`]. `--jobs`
//! and `POLINT_JOBS` accept a core count, a percentage such as `80%`, or `0`
//! (or `0%`) to use every available CPU. The resolved count is not a cache
//! input: it must not change diagnostics.

use anyhow::{Context, Result, bail};
use std::process::Command;
use std::str::FromStr;
use std::sync::OnceLock;
use thiserror::Error;

/// Environment override, same grammar as `--jobs`.
pub(crate) const JOBS_ENV: &str = "POLINT_JOBS";

/// Default fraction of available CPUs when neither `--jobs` nor `POLINT_JOBS` is set.
pub(crate) const DEFAULT_PERCENT: u32 = 80;

static RESOLVED: OnceLock<usize> = OnceLock::new();

/// How the user asked to size the worker pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JobsSpec {
    /// 80% of available CPUs.
    Auto,
    /// Every available CPU (`0`, `0%`, or `100%`).
    All,
    /// An explicit core count, later capped at available CPUs.
    Count(usize),
    /// A percentage of available CPUs in `1..=99`.
    Percent(u32),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum JobsError {
    #[error(
        "invalid --jobs value `{value}`: use a core count, a percentage such as `80%`, or `0` to use every available CPU"
    )]
    Invalid { value: String },
    #[error("invalid --jobs percentage `{value}`: use an integer from 0 to 100")]
    InvalidPercent { value: String },
}

impl FromStr for JobsSpec {
    type Err = JobsError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        parse_jobs_spec(raw)
    }
}

pub(crate) fn parse_jobs_spec(raw: &str) -> Result<JobsSpec, JobsError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(JobsError::Invalid {
            value: raw.to_string(),
        });
    }
    if let Some(percent) = trimmed.strip_suffix('%') {
        let percent = percent.trim();
        let value = percent
            .parse::<u32>()
            .map_err(|_| JobsError::InvalidPercent {
                value: raw.to_string(),
            })?;
        return match value {
            0 | 100 => Ok(JobsSpec::All),
            1..=99 => Ok(JobsSpec::Percent(value)),
            _ => Err(JobsError::InvalidPercent {
                value: raw.to_string(),
            }),
        };
    }
    let value = trimmed.parse::<usize>().map_err(|_| JobsError::Invalid {
        value: raw.to_string(),
    })?;
    if value == 0 {
        Ok(JobsSpec::All)
    } else {
        Ok(JobsSpec::Count(value))
    }
}

/// Reads `POLINT_JOBS` when `--jobs` was omitted.
pub(crate) fn spec_from_env() -> Result<JobsSpec> {
    match std::env::var(JOBS_ENV) {
        Ok(raw) if !raw.trim().is_empty() => {
            parse_jobs_spec(&raw).with_context(|| format!("invalid {JOBS_ENV} value `{raw}`"))
        }
        Ok(_) | Err(std::env::VarError::NotPresent) => Ok(JobsSpec::Auto),
        Err(error) => bail!("invalid {JOBS_ENV}: {error}"),
    }
}

pub(crate) fn available_cpus() -> usize {
    std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1)
}

pub(crate) fn resolve_job_count(spec: JobsSpec, available: usize) -> usize {
    let available = available.max(1);
    match spec {
        JobsSpec::Auto => percent_of(available, DEFAULT_PERCENT),
        JobsSpec::All => available,
        JobsSpec::Count(count) => count.max(1).min(available),
        JobsSpec::Percent(percent) => percent_of(available, percent),
    }
}

fn percent_of(available: usize, percent: u32) -> usize {
    available
        .saturating_mul(percent as usize)
        .saturating_div(100)
        .max(1)
        .min(available)
}

/// Resolves `--jobs` / `POLINT_JOBS` / the 80% default and installs the Rayon pool.
pub(crate) fn install_from_cli_flag(cli_jobs: Option<&str>) -> Result<usize> {
    let spec = match cli_jobs {
        Some(raw) => {
            parse_jobs_spec(raw).with_context(|| format!("invalid --jobs value `{raw}`"))?
        }
        None => spec_from_env()?,
    };
    Ok(install(spec))
}

pub(crate) fn install(spec: JobsSpec) -> usize {
    let count = resolve_job_count(spec, available_cpus());
    let count = *RESOLVED.get_or_init(|| count);
    if let Err(error) = rayon::ThreadPoolBuilder::new()
        .num_threads(count)
        .build_global()
    {
        tracing::debug!(
            error = %error,
            jobs = count,
            "rayon global pool already initialized"
        );
    }
    count
}

pub(crate) fn resolved_job_count() -> usize {
    *RESOLVED.get_or_init(|| resolve_job_count(JobsSpec::Auto, available_cpus()))
}

/// Caps Cargo, Go, Rayon, and nested polint processes at the resolved job count.
pub(crate) fn apply_to_command(command: &mut Command) {
    let jobs = resolved_job_count().to_string();
    command
        .env(JOBS_ENV, &jobs)
        .env("CARGO_BUILD_JOBS", &jobs)
        .env("GOMAXPROCS", &jobs)
        .env("RAYON_NUM_THREADS", &jobs);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_jobs_spec_should_treat_zero_as_all_cpus() {
        assert_eq!(parse_jobs_spec("0").unwrap(), JobsSpec::All);
        assert_eq!(parse_jobs_spec("0%").unwrap(), JobsSpec::All);
        assert_eq!(parse_jobs_spec("100%").unwrap(), JobsSpec::All);
    }

    #[test]
    fn parse_jobs_spec_should_accept_core_counts() {
        assert_eq!(parse_jobs_spec("1").unwrap(), JobsSpec::Count(1));
        assert_eq!(parse_jobs_spec(" 8 ").unwrap(), JobsSpec::Count(8));
    }

    #[test]
    fn parse_jobs_spec_should_accept_percentages() {
        assert_eq!(parse_jobs_spec("80%").unwrap(), JobsSpec::Percent(80));
        assert_eq!(parse_jobs_spec("50 %").unwrap(), JobsSpec::Percent(50));
    }

    #[test]
    fn parse_jobs_spec_should_reject_empty_and_non_numeric_values() {
        assert!(matches!(
            parse_jobs_spec(""),
            Err(JobsError::Invalid { .. })
        ));
        assert!(matches!(
            parse_jobs_spec("max"),
            Err(JobsError::Invalid { .. })
        ));
        assert!(matches!(
            parse_jobs_spec("-1"),
            Err(JobsError::Invalid { .. })
        ));
    }

    #[test]
    fn parse_jobs_spec_should_reject_percentages_above_100() {
        assert!(matches!(
            parse_jobs_spec("101%"),
            Err(JobsError::InvalidPercent { .. })
        ));
        assert!(matches!(
            parse_jobs_spec("12.5%"),
            Err(JobsError::InvalidPercent { .. })
        ));
    }

    #[test]
    fn resolve_job_count_should_use_eighty_percent_by_default() {
        assert_eq!(resolve_job_count(JobsSpec::Auto, 10), 8);
        assert_eq!(resolve_job_count(JobsSpec::Auto, 8), 6);
        assert_eq!(resolve_job_count(JobsSpec::Auto, 1), 1);
        assert_eq!(resolve_job_count(JobsSpec::Auto, 2), 1);
    }

    #[test]
    fn resolve_job_count_should_use_every_cpu_for_all() {
        assert_eq!(resolve_job_count(JobsSpec::All, 8), 8);
        assert_eq!(resolve_job_count(JobsSpec::All, 1), 1);
    }

    #[test]
    fn resolve_job_count_should_cap_requested_cores_at_available() {
        assert_eq!(resolve_job_count(JobsSpec::Count(3), 8), 3);
        assert_eq!(resolve_job_count(JobsSpec::Count(99), 4), 4);
    }

    #[test]
    fn resolve_job_count_should_keep_at_least_one_worker() {
        assert_eq!(resolve_job_count(JobsSpec::Percent(1), 1), 1);
        assert_eq!(resolve_job_count(JobsSpec::Auto, 0), 1);
    }
}
