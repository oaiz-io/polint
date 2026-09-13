//! Wall-clock budget for the Go semantic sidecar.
//!
//! The sidecar type-checks the full dependency graph, builds SSA over every
//! package, and runs RTA. It is the heaviest Go subprocess, so it defaults to
//! the same [`GO_SUBPROCESS_TIMEOUT`] every other Go subprocess gets rather
//! than to a tighter constant of its own.
//!
//! A timeout is a *reported outcome*, never a silent scope reduction: exceeding
//! it fails the provider, which blocks the requesting rules with capability
//! diagnostics. Raising the budget is the wrong first move — read the sidecar's
//! phase telemetry first and see which stage the time went to.

use std::time::Duration;

use crate::go::process_runner::GO_SUBPROCESS_TIMEOUT;

/// `[languages.go] semantic_timeout_ms` in `.polint.toml`.
pub const SEMANTIC_TIMEOUT_SETTING: &str = "semantic_timeout_ms";
/// Environment override for [`SEMANTIC_TIMEOUT_SETTING`].
pub const SEMANTIC_TIMEOUT_ENV: &str = "POLINT_GO_SEMANTIC_TIMEOUT_MS";

/// Resolves the sidecar budget from the setting, then the environment, then the
/// shared Go subprocess default.
///
/// The environment wins so a one-off diagnostic run can widen the budget
/// without editing committed configuration. Unparseable or zero values warn and
/// fall through rather than silently disabling the bound.
pub fn semantic_timeout(setting_ms: Option<u64>) -> Duration {
    resolve_timeout(
        std::env::var(SEMANTIC_TIMEOUT_ENV).ok().as_deref(),
        setting_ms,
    )
}

/// The resolution itself, separated from the environment so it is testable
/// without mutating process-wide state.
fn resolve_timeout(env_ms: Option<&str>, setting_ms: Option<u64>) -> Duration {
    if let Some(raw) = env_ms {
        match raw.trim().parse::<u64>() {
            Ok(millis) if millis > 0 => return Duration::from_millis(millis),
            _ => tracing::warn!(
                target: "polint::kernel",
                value = raw,
                "ignoring unusable {SEMANTIC_TIMEOUT_ENV}; using the configured budget"
            ),
        }
    }
    match setting_ms {
        Some(millis) if millis > 0 => Duration::from_millis(millis),
        Some(_) => {
            tracing::warn!(
                target: "polint::kernel",
                setting = SEMANTIC_TIMEOUT_SETTING,
                "ignoring a zero Go semantic timeout; using the default budget"
            );
            GO_SUBPROCESS_TIMEOUT
        }
        None => GO_SUBPROCESS_TIMEOUT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unset_setting_uses_the_shared_go_subprocess_budget() {
        assert_eq!(resolve_timeout(None, None), GO_SUBPROCESS_TIMEOUT);
    }

    #[test]
    fn a_setting_overrides_the_default() {
        assert_eq!(
            resolve_timeout(None, Some(4_500)),
            Duration::from_millis(4_500)
        );
    }

    #[test]
    fn the_environment_overrides_the_setting() {
        assert_eq!(
            resolve_timeout(Some("300000"), Some(4_500)),
            Duration::from_millis(300_000)
        );
    }

    #[test]
    fn an_unparseable_environment_value_falls_back_to_the_setting() {
        assert_eq!(
            resolve_timeout(Some("soon"), Some(4_500)),
            Duration::from_millis(4_500)
        );
    }

    #[test]
    fn a_zero_environment_value_falls_back_to_the_setting() {
        assert_eq!(
            resolve_timeout(Some("0"), Some(4_500)),
            Duration::from_millis(4_500)
        );
    }

    #[test]
    fn a_zero_setting_falls_back_to_the_default() {
        assert_eq!(resolve_timeout(None, Some(0)), GO_SUBPROCESS_TIMEOUT);
    }

    #[test]
    fn the_default_matches_every_other_go_subprocess() {
        assert_eq!(GO_SUBPROCESS_TIMEOUT, Duration::from_secs(120));
    }
}
