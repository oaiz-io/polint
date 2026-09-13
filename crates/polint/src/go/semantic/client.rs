use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::go::lifecycle::{self, GoAnalysisConfig};
use crate::go::process_runner::{GoProcessError, run_bounded};
use crate::go::semantic::budget::semantic_timeout;
use crate::go::semantic::diagnostics::GO_SIDECAR_TIMEOUT;
use crate::go::semantic::cache_key::go_semantic_sidecar_cache_key;
use crate::go::semantic::process::{
    GoSemanticProcessError, command_for_frontend, frontend_digest, local_go_toolchain_version,
    resolve_go_semantic_frontend,
};
use crate::go::semantic::protocol::{GoSemanticOutput, GoSemanticProtocolError, decode_ndjson};

#[derive(Debug)]
pub enum GoSemanticClientError {
    Process(GoSemanticProcessError),
    Protocol(GoSemanticProtocolError),
}

impl std::fmt::Display for GoSemanticClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Process(error) => write!(f, "{error}"),
            Self::Protocol(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for GoSemanticClientError {}

impl From<GoSemanticProcessError> for GoSemanticClientError {
    fn from(error: GoSemanticProcessError) -> Self {
        Self::Process(error)
    }
}

impl From<GoSemanticProtocolError> for GoSemanticClientError {
    fn from(error: GoSemanticProtocolError) -> Self {
        Self::Protocol(error)
    }
}

#[derive(Debug, Clone)]
pub struct GoSemanticClient {
    root: PathBuf,
    timeout: Duration,
}

#[derive(Debug)]
pub struct GoSemanticClientRun {
    pub output: GoSemanticOutput,
    pub frontend_digest: String,
}

impl GoSemanticClient {
    /// Builds a client whose budget is resolved from `config`, the
    /// `POLINT_GO_SEMANTIC_TIMEOUT_MS` environment override, and the shared Go
    /// subprocess default, in that precedence order.
    pub fn new(root: PathBuf, config: &GoAnalysisConfig) -> Self {
        Self {
            root,
            timeout: semantic_timeout(config.semantic_timeout_ms),
        }
    }

    #[cfg(test)]
    pub fn with_timeout(root: PathBuf, timeout: Duration) -> Self {
        Self { root, timeout }
    }

    pub fn run(
        &self,
        config: &GoAnalysisConfig,
    ) -> Result<GoSemanticClientRun, GoSemanticClientError> {
        let frontend = resolve_go_semantic_frontend()?;
        let digest = frontend_digest(&frontend)?;
        let mut command = command_for_frontend(&frontend, &self.root, config.offline)?;
        append_request_args(&mut command, &self.root, config);
        let stdout = run_with_timeout(command, self.timeout, &self.root)?;
        tracing::debug!(
            target: "polint::kernel::stage",
            provider = "polint.go.semantic",
            timeout_ms = self.timeout.as_millis() as u64,
            "go semantic sidecar returned"
        );
        let output = decode_ndjson(&stdout).map_err(GoSemanticClientError::from)?;
        Ok(GoSemanticClientRun {
            output,
            frontend_digest: digest,
        })
    }

    pub fn run_cached(
        &self,
        config: &GoAnalysisConfig,
        cache_dir: &Path,
        upstream_digest: &str,
    ) -> Result<GoSemanticClientRun, GoSemanticClientError> {
        let frontend = resolve_go_semantic_frontend()?;
        let digest = frontend_digest(&frontend)?;
        let go_version = local_go_toolchain_version().unwrap_or_default();
        let cache_key =
            go_semantic_sidecar_cache_key(&digest, &go_version, upstream_digest, config);
        let cache_path = cache_dir.join(format!("{cache_key}.ndjson"));

        if let Ok(cached_bytes) = std::fs::read(&cache_path) {
            match decode_ndjson(&cached_bytes) {
                Ok(output) => {
                    tracing::info!(
                        target: "polint::kernel::stage",
                        provider = "polint.go.semantic",
                        "sidecar cache hit"
                    );
                    return Ok(GoSemanticClientRun {
                        output,
                        frontend_digest: digest,
                    });
                }
                Err(_) => {
                    let _ = std::fs::remove_file(&cache_path);
                }
            }
        }

        let mut command = command_for_frontend(&frontend, &self.root, config.offline)?;
        append_request_args(&mut command, &self.root, config);
        let stdout = run_with_timeout(command, self.timeout, &self.root)?;
        tracing::debug!(
            target: "polint::kernel::stage",
            provider = "polint.go.semantic",
            timeout_ms = self.timeout.as_millis() as u64,
            "go semantic sidecar returned"
        );

        let _ = std::fs::create_dir_all(cache_dir);
        let _ = std::fs::write(&cache_path, &stdout);

        let output = decode_ndjson(&stdout).map_err(GoSemanticClientError::from)?;
        Ok(GoSemanticClientRun {
            output,
            frontend_digest: digest,
        })
    }
}

fn append_request_args(
    command: &mut std::process::Command,
    root: &Path,
    config: &GoAnalysisConfig,
) {
    command
        .arg("semantic")
        .arg("--root")
        .arg(root.as_os_str())
        .arg("--module-roots")
        .arg(config.module_roots.join(","))
        .arg("--patterns")
        .arg(config.package_patterns.join(","))
        .arg("--tests")
        .arg(config.include_tests.to_string())
        .arg("--build-tags")
        .arg(config.build_tags.join(","))
        .arg("--ndjson");
    lifecycle::apply_go_offline_env(command, config.offline);
}

fn run_with_timeout(
    command: std::process::Command,
    timeout: Duration,
    root: &Path,
) -> Result<Vec<u8>, GoSemanticProcessError> {
    let _ = root;
    let output =
        run_bounded(command, timeout, "go semantic frontend").map_err(|error| match error {
            GoProcessError::Unavailable(reason) => {
                GoSemanticProcessError::CommandUnavailable(reason)
            }
            GoProcessError::Failed(reason) => GoSemanticProcessError::CommandFailed(reason),
            GoProcessError::Timeout(reason) => {
                GoSemanticProcessError::Timeout(format!("{GO_SIDECAR_TIMEOUT}: {reason}"))
            }
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let reason = if stderr.is_empty() {
            format!("go semantic frontend exited with status {}.", output.status)
        } else {
            format!(
                "go semantic frontend exited with status {}: {stderr}",
                output.status
            )
        };
        return Err(GoSemanticProcessError::CommandFailed(reason));
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    static FAKE_STDOUT_FILE_COUNTER: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn timeout_error_uses_go_sidecar_timeout_category() {
        let command = if cfg!(windows) {
            let mut command = std::process::Command::new("cmd");
            command.args(["/C", "ping -n 3 127.0.0.1 > nul"]);
            command
        } else {
            let mut command = std::process::Command::new("sh");
            command.args(["-c", "sleep 2"]);
            command
        };
        let err = run_with_timeout(command, Duration::from_millis(1), Path::new(".")).unwrap_err();
        assert!(err.to_string().contains("GoSidecarTimeout"));
    }

    #[test]
    fn command_failure_captures_stderr() {
        let command = if cfg!(windows) {
            let mut command = std::process::Command::new("cmd");
            command.args(["/C", "echo semantic boom 1>&2 && exit /B 7"]);
            command
        } else {
            let mut command = std::process::Command::new("sh");
            command.args(["-c", "echo semantic boom >&2; exit 7"]);
            command
        };
        let err = run_with_timeout(command, Duration::from_secs(5), Path::new(".")).unwrap_err();
        assert!(err.to_string().contains("semantic boom"));
    }

    #[test]
    fn schema_mismatch_from_fake_sidecar_is_typed_protocol_error() {
        let command = fake_stdout_command("{\"schema\":\"wrong\",\"kind\":\"session_begin\"}\n");
        let stdout = run_with_timeout(command, Duration::from_secs(5), Path::new("."))
            .expect("fake sidecar exits");
        let err = decode_ndjson(&stdout).unwrap_err();
        assert!(matches!(err, GoSemanticProtocolError::UnsupportedSchema(_)));
    }

    #[test]
    fn missing_terminator_from_fake_sidecar_is_typed_protocol_error() {
        let command = fake_stdout_command(
            "{\"schema\":\"polint-go-semantic-3\",\"kind\":\"session_begin\"}\n",
        );
        let stdout = run_with_timeout(command, Duration::from_secs(5), Path::new("."))
            .expect("fake sidecar exits");
        let err = decode_ndjson(&stdout).unwrap_err();
        assert_eq!(err, GoSemanticProtocolError::MissingEnd);
    }

    #[test]
    fn no_surviving_go_frontend_process_after_timeout() {
        let command = if cfg!(windows) {
            let mut command = std::process::Command::new("cmd");
            command.args(["/C", "ping -n 10 127.0.0.1 > nul"]);
            command
        } else {
            let mut command = std::process::Command::new("sleep");
            command.arg("10");
            command
        };
        let started = Instant::now();
        let err = run_with_timeout(command, Duration::from_millis(1), Path::new(".")).unwrap_err();
        assert!(err.to_string().contains("GoSidecarTimeout"));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "timeout cleanup should not wait for the fake frontend to finish naturally"
        );
    }

    #[cfg(unix)]
    #[test]
    fn timeout_terminates_descendant_processes() {
        let temp = tempfile::tempdir().expect("tempdir");
        let pid_file = temp.path().join("sleep.pid");
        let mut command = std::process::Command::new("sh");
        command.arg("-c").arg(format!(
            "sleep 10 & echo $! > {}; wait",
            shell_quote(pid_file.to_string_lossy().as_ref())
        ));

        let err = run_with_timeout(command, Duration::from_millis(25), Path::new(".")).unwrap_err();

        assert!(err.to_string().contains("GoSidecarTimeout"));
        if let Ok(pid) = std::fs::read_to_string(pid_file)
            && let Ok(pid) = pid.trim().parse::<i32>()
        {
            assert_process_exits(pid);
        }
    }

    #[cfg(unix)]
    #[test]
    fn large_stdout_is_drained_while_child_runs() {
        let mut command = std::process::Command::new("sh");
        command.arg("-c").arg(
            "i=0; while [ $i -lt 5000 ]; do printf 0123456789abcdef0123456789abcdef; i=$((i + 1)); done",
        );

        let stdout = run_with_timeout(command, Duration::from_secs(2), Path::new("."))
            .expect("large stdout exits");

        assert_eq!(stdout.len(), 160_000);
    }

    fn fake_stdout_command(stdout: &str) -> std::process::Command {
        let path = std::env::temp_dir().join(format!(
            "polint-fake-sidecar-{}-{}.ndjson",
            std::process::id(),
            FAKE_STDOUT_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, stdout).expect("write fake sidecar stdout fixture");
        if cfg!(windows) {
            let mut command = std::process::Command::new("cmd");
            command.arg("/C").arg("type").arg(path);
            command
        } else {
            let mut command = std::process::Command::new("cat");
            command.arg(path);
            command
        }
    }

    #[cfg(unix)]
    fn shell_quote(value: &str) -> String {
        format!("'{}'", value.replace('\'', "'\\''"))
    }

    #[cfg(unix)]
    fn assert_process_exits(pid: i32) {
        for _ in 0..20 {
            if !process_exists(pid) {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let _ = std::process::Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        panic!("descendant process {pid} was still running after Go semantic timeout");
    }

    #[cfg(unix)]
    fn process_exists(pid: i32) -> bool {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
}
