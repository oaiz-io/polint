//! Go's parameters for the shared bounded-subprocess runner.
//!
//! The mechanics live in [`crate::subprocess`]; what belongs to Go is the
//! default budget and the timeout category the Go diagnostics use.

use std::process::Command;
use std::time::Duration;

pub(crate) use crate::subprocess::{SubprocessError as GoProcessError, SubprocessOutput};

pub(crate) const GO_SUBPROCESS_TIMEOUT: Duration = Duration::from_secs(120);
const GO_SUBPROCESS_TIMEOUT_CODE: &str = "GoSubprocessTimeout";

pub(crate) fn run_bounded(
    command: Command,
    timeout: Duration,
    label: &str,
) -> Result<SubprocessOutput, GoProcessError> {
    crate::subprocess::run_bounded(command, timeout, label, GO_SUBPROCESS_TIMEOUT_CODE)
}

/// [`run_bounded`], writing standard output to `stdout_file` instead of
/// collecting it.
pub(crate) fn run_bounded_to_file(
    command: Command,
    timeout: Duration,
    label: &str,
    stdout_file: std::fs::File,
) -> Result<SubprocessOutput, GoProcessError> {
    crate::subprocess::run_bounded_to_file(
        command,
        timeout,
        label,
        GO_SUBPROCESS_TIMEOUT_CODE,
        stdout_file,
    )
}
