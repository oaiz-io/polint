use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use super::discovery::DiscoveredExtension;
use super::manifest::{ExtensionActivationStatus, FactFamilyLabel};
use super::protocol::{
    ExtensionHandshakeRequest, ExtensionHandshakeResponse, ExtensionProtocolError,
    ExtensionProviderRunRequest, ExtensionProviderRunResponse, HANDSHAKE_SCHEMA,
    PROVIDER_RUN_SCHEMA,
};
use crate::diagnostics::{Diagnostic, extension_setup_diagnostic};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_STDOUT_LIMIT: usize = 1_048_576;
const DEFAULT_STDERR_LIMIT: usize = 16_384;

#[derive(Debug, Clone)]
pub(crate) struct ExtensionHost<R = StdCommandRunner> {
    repo_root: PathBuf,
    runner: R,
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
}

impl ExtensionHost<StdCommandRunner> {
    pub(crate) fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self::with_runner(repo_root, StdCommandRunner)
    }
}

impl<R> ExtensionHost<R>
where
    R: ExtensionCommandRunner,
{
    pub(crate) fn with_runner(repo_root: impl Into<PathBuf>, runner: R) -> Self {
        Self {
            repo_root: repo_root.into(),
            runner,
            timeout: DEFAULT_TIMEOUT,
            stdout_limit: DEFAULT_STDOUT_LIMIT,
            stderr_limit: DEFAULT_STDERR_LIMIT,
        }
    }

    #[cfg(test)]
    fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub(crate) fn handshake(
        &self,
        extension: &DiscoveredExtension,
    ) -> ExtensionHostResult<ExtensionHandshakeResponse> {
        let request = ExtensionHandshakeRequest {
            schema_version: HANDSHAKE_SCHEMA.to_string(),
            extension_id: extension.extension_id.clone(),
            manifest_path: extension.manifest_path.clone(),
        };
        let spec = self.command_spec(
            &extension.manifest_path,
            vec!["handshake".to_string()],
            serde_json::to_vec(&request).expect("handshake request should serialize"),
        );
        let outcome = self.runner.run(spec);
        let response = self.parse_json_response::<ExtensionHandshakeResponse>(
            outcome,
            &extension.extension_id,
            None,
            ExtensionHostFailureKind::HandshakeFailed,
        )?;
        response.validate_schema().map_err(|error| {
            ExtensionHostError::from_protocol_error(&extension.extension_id, None, error)
        })?;
        ensure_extension_identity(&response.extension_id, &extension.extension_id, None)?;
        Ok(response)
    }

    pub(crate) fn run_provider(
        &self,
        extension: &DiscoveredExtension,
        provider_id: &str,
        declared_inputs: Vec<FactFamilyLabel>,
        declared_outputs: Vec<FactFamilyLabel>,
        input_digest_labels: Vec<String>,
    ) -> ExtensionHostResult<ExtensionProviderRunResponse> {
        let request = ExtensionProviderRunRequest {
            schema_version: PROVIDER_RUN_SCHEMA.to_string(),
            extension_id: extension.extension_id.clone(),
            provider_id: provider_id.to_string(),
            declared_inputs,
            declared_outputs,
            input_digest_labels,
        };
        let spec = self.command_spec(
            &extension.manifest_path,
            vec!["run-provider".to_string(), provider_id.to_string()],
            serde_json::to_vec(&request).expect("provider request should serialize"),
        );
        let outcome = self.runner.run(spec);
        let response = self.parse_json_response::<ExtensionProviderRunResponse>(
            outcome,
            &extension.extension_id,
            Some(provider_id),
            ExtensionHostFailureKind::ProviderFailed,
        )?;
        response.validate_schema().map_err(|error| {
            ExtensionHostError::from_protocol_error(
                &extension.extension_id,
                Some(provider_id),
                error,
            )
        })?;
        ensure_extension_identity(
            &response.extension_id,
            &extension.extension_id,
            Some(provider_id),
        )?;
        ensure_provider_identity(&response.provider_id, &extension.extension_id, provider_id)?;
        Ok(response)
    }

    fn command_spec(
        &self,
        manifest_path: &str,
        extension_args: Vec<String>,
        stdin: Vec<u8>,
    ) -> ExtensionCommandSpec {
        let manifest_path = self.repo_root.join(manifest_path);
        let mut args = vec![
            "run".to_string(),
            "--manifest-path".to_string(),
            manifest_path.to_string_lossy().to_string(),
            "--".to_string(),
        ];
        args.extend(extension_args);
        let mut env = BTreeMap::new();
        // Extension builds are compiler output under the cache root, so they
        // follow the same layout - and the same `POLINT_CACHE_DIR` override -
        // as every other cache directory.
        env.insert(
            "CARGO_TARGET_DIR".to_string(),
            crate::cache::CacheLayout::for_repo(&self.repo_root)
                .extensions_target_dir()
                .to_string_lossy()
                .to_string(),
        );
        ExtensionCommandSpec {
            program: "cargo".to_string(),
            args,
            env,
            stdin,
            timeout: self.timeout,
            stdout_limit: self.stdout_limit,
            stderr_limit: self.stderr_limit,
        }
    }

    fn parse_json_response<T>(
        &self,
        outcome: ExtensionCommandOutcome,
        extension_id: &str,
        provider_id: Option<&str>,
        nonzero_kind: ExtensionHostFailureKind,
    ) -> ExtensionHostResult<T>
    where
        T: serde::de::DeserializeOwned,
    {
        if outcome.timed_out {
            return Err(ExtensionHostError::new(
                ExtensionHostFailureKind::Timeout,
                extension_id,
                provider_id,
                "extension command timed out",
            ));
        }
        if let Some(spawn_error) = outcome.spawn_error {
            return Err(ExtensionHostError::new(
                ExtensionHostFailureKind::BuildFailed,
                extension_id,
                provider_id,
                spawn_error,
            ));
        }
        if outcome.status != Some(0) {
            return Err(ExtensionHostError::new(
                classify_nonzero(nonzero_kind, &outcome.stderr),
                extension_id,
                provider_id,
                bounded_summary(&outcome.stderr, self.stderr_limit),
            ));
        }

        serde_json::from_slice(&outcome.stdout).map_err(|error| {
            ExtensionHostError::new(
                ExtensionHostFailureKind::MalformedResponse,
                extension_id,
                provider_id,
                error.to_string(),
            )
        })
    }
}

pub(crate) type ExtensionHostResult<T> = Result<T, ExtensionHostError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExtensionHostError {
    pub(crate) kind: ExtensionHostFailureKind,
    pub(crate) extension_id: String,
    pub(crate) provider_id: Option<String>,
    pub(crate) summary: String,
}

impl ExtensionHostError {
    fn new(
        kind: ExtensionHostFailureKind,
        extension_id: impl Into<String>,
        provider_id: Option<&str>,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            extension_id: extension_id.into(),
            provider_id: provider_id.map(str::to_string),
            summary: sanitize_summary(&summary.into()),
        }
    }

    fn from_protocol_error(
        extension_id: &str,
        provider_id: Option<&str>,
        error: ExtensionProtocolError,
    ) -> Self {
        match error {
            ExtensionProtocolError::UnsupportedProtocol { expected, actual } => Self::new(
                ExtensionHostFailureKind::UnsupportedProtocol,
                extension_id,
                provider_id,
                format!("unsupported protocol schema {actual}; expected {expected}"),
            ),
        }
    }

    pub(crate) fn activation_status(&self) -> ExtensionActivationStatus {
        match self.kind {
            ExtensionHostFailureKind::MalformedResponse
            | ExtensionHostFailureKind::IdentityMismatch
            | ExtensionHostFailureKind::UnsupportedProtocol => {
                ExtensionActivationStatus::ValidationFailed
            }
            ExtensionHostFailureKind::BuildFailed
            | ExtensionHostFailureKind::HandshakeFailed
            | ExtensionHostFailureKind::ProviderFailed
            | ExtensionHostFailureKind::Timeout => ExtensionActivationStatus::Failed,
        }
    }

    pub(crate) fn diagnostic(&self) -> Diagnostic {
        extension_setup_diagnostic(
            self.kind.as_str(),
            &self.extension_id,
            self.provider_id.as_deref(),
            &self.summary,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExtensionHostFailureKind {
    BuildFailed,
    HandshakeFailed,
    ProviderFailed,
    Timeout,
    MalformedResponse,
    IdentityMismatch,
    UnsupportedProtocol,
}

impl ExtensionHostFailureKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::BuildFailed => "build_failed",
            Self::HandshakeFailed => "handshake_failed",
            Self::ProviderFailed => "provider_failed",
            Self::Timeout => "timeout",
            Self::MalformedResponse => "malformed_response",
            Self::IdentityMismatch => "identity_mismatch",
            Self::UnsupportedProtocol => "unsupported_protocol",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExtensionCommandSpec {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) stdin: Vec<u8>,
    pub(crate) timeout: Duration,
    pub(crate) stdout_limit: usize,
    pub(crate) stderr_limit: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExtensionCommandOutcome {
    pub(crate) status: Option<i32>,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) timed_out: bool,
    pub(crate) spawn_error: Option<String>,
}

pub(crate) trait ExtensionCommandRunner {
    fn run(&self, spec: ExtensionCommandSpec) -> ExtensionCommandOutcome;
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct StdCommandRunner;

impl ExtensionCommandRunner for StdCommandRunner {
    fn run(&self, spec: ExtensionCommandSpec) -> ExtensionCommandOutcome {
        run_std_command(&spec)
    }
}

const EXTENSION_ENV_ALLOWLIST: &[&str] = &[
    // Cross-platform
    "PATH",
    "LANG",
    "TERM",
    // Unix / macOS
    "HOME",
    "USER",
    "SHELL",
    "TMPDIR",
    "SDKROOT",
    "MACOSX_DEPLOYMENT_TARGET",
    // Windows — required for cargo, rustc, and MSVC linker
    "USERPROFILE",
    "SystemRoot",
    "TEMP",
    "TMP",
    "PATHEXT",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "COMSPEC",
    "WINDIR",
    // Rust toolchain
    "RUSTUP_HOME",
    "RUSTUP_TOOLCHAIN",
    "CARGO_HOME",
    "RUSTC",
    "RUSTFLAGS",
    "RUSTDOCFLAGS",
    // Native build tools
    "CC",
    "CXX",
    "CFLAGS",
    "CXXFLAGS",
    "PKG_CONFIG_PATH",
];

fn run_std_command(spec: &ExtensionCommandSpec) -> ExtensionCommandOutcome {
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .env_clear()
        .envs(
            EXTENSION_ENV_ALLOWLIST
                .iter()
                .filter_map(|key| std::env::var(key).ok().map(|val| (*key, val))),
        )
        .envs(&spec.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::jobs::apply_to_command(&mut command);
    configure_child_process_group(&mut command);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return ExtensionCommandOutcome {
                status: None,
                stdout: Vec::new(),
                stderr: Vec::new(),
                timed_out: false,
                spawn_error: Some(error.to_string()),
            };
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        let input = spec.stdin.clone();
        thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
    }

    let stdout_limit = spec.stdout_limit;
    let stderr_limit = spec.stderr_limit;
    let stdout_reader = child
        .stdout
        .take()
        .map(|stdout| thread::spawn(move || read_limited(stdout, stdout_limit)));
    let stderr_reader = child
        .stderr
        .take()
        .map(|stderr| thread::spawn(move || read_limited(stderr, stderr_limit)));

    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() >= spec.timeout => {
                terminate_child_process_tree(&mut child);
                let status = child.wait().ok();
                return ExtensionCommandOutcome {
                    status: status.and_then(|status| status.code()),
                    stdout: join_reader(stdout_reader),
                    stderr: join_reader(stderr_reader),
                    timed_out: true,
                    spawn_error: None,
                };
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                return ExtensionCommandOutcome {
                    status: None,
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                    timed_out: false,
                    spawn_error: Some(error.to_string()),
                };
            }
        }
    }

    match child.wait() {
        Ok(status) => ExtensionCommandOutcome {
            status: status.code(),
            stdout: join_reader(stdout_reader),
            stderr: join_reader(stderr_reader),
            timed_out: false,
            spawn_error: None,
        },
        Err(error) => ExtensionCommandOutcome {
            status: None,
            stdout: Vec::new(),
            stderr: Vec::new(),
            timed_out: false,
            spawn_error: Some(error.to_string()),
        },
    }
}

fn ensure_extension_identity(
    actual: &str,
    expected: &str,
    provider_id: Option<&str>,
) -> ExtensionHostResult<()> {
    if actual == expected {
        return Ok(());
    }
    Err(ExtensionHostError::new(
        ExtensionHostFailureKind::IdentityMismatch,
        expected,
        provider_id,
        format!("extension identity mismatch: expected {expected}, got {actual}"),
    ))
}

fn ensure_provider_identity(
    actual: &str,
    extension_id: &str,
    expected: &str,
) -> ExtensionHostResult<()> {
    if actual == expected {
        return Ok(());
    }
    Err(ExtensionHostError::new(
        ExtensionHostFailureKind::IdentityMismatch,
        extension_id,
        Some(expected),
        format!("provider identity mismatch: expected {expected}, got {actual}"),
    ))
}

#[cfg(unix)]
fn configure_child_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_child_process_group(_command: &mut Command) {}

#[cfg(unix)]
fn terminate_child_process_tree(child: &mut Child) {
    let process_group = format!("-{}", child.id());
    let _ = Command::new("kill")
        .args(["-KILL", "--", &process_group])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
}

#[cfg(not(unix))]
fn terminate_child_process_tree(child: &mut Child) {
    let _ = child.kill();
}

fn classify_nonzero(
    default_kind: ExtensionHostFailureKind,
    stderr: &[u8],
) -> ExtensionHostFailureKind {
    let stderr = String::from_utf8_lossy(stderr);
    if stderr.contains("could not compile") || stderr.contains("failed to compile") {
        ExtensionHostFailureKind::BuildFailed
    } else {
        default_kind
    }
}

fn bounded_summary(bytes: &[u8], limit: usize) -> String {
    sanitize_summary(&String::from_utf8_lossy(&truncate_bytes(bytes, limit)))
}

fn sanitize_summary(summary: &str) -> String {
    summary.lines().take(4).collect::<Vec<_>>().join(" ")
}

fn truncate_bytes(bytes: &[u8], limit: usize) -> Vec<u8> {
    bytes[..bytes.len().min(limit)].to_vec()
}

fn read_limited(mut reader: impl Read, limit: usize) -> Vec<u8> {
    let mut output = Vec::with_capacity(limit.min(8192));
    let mut buffer = [0_u8; 8192];
    while let Ok(read) = reader.read(&mut buffer) {
        if read == 0 {
            break;
        }
        if output.len() < limit {
            let remaining = limit - output.len();
            output.extend_from_slice(&buffer[..read.min(remaining)]);
        }
    }
    output
}

fn join_reader(reader: Option<thread::JoinHandle<Vec<u8>>>) -> Vec<u8> {
    reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::extensions::manifest::ExtensionManifest;
    use std::cell::RefCell;
    use tempfile::TempDir;

    #[derive(Debug)]
    struct FakeRunner {
        outcomes: RefCell<Vec<ExtensionCommandOutcome>>,
        specs: RefCell<Vec<ExtensionCommandSpec>>,
    }

    impl FakeRunner {
        fn new(outcomes: Vec<ExtensionCommandOutcome>) -> Self {
            Self {
                outcomes: RefCell::new(outcomes),
                specs: RefCell::new(Vec::new()),
            }
        }
    }

    impl ExtensionCommandRunner for FakeRunner {
        fn run(&self, spec: ExtensionCommandSpec) -> ExtensionCommandOutcome {
            self.specs.borrow_mut().push(spec);
            self.outcomes.borrow_mut().remove(0)
        }
    }

    fn extension() -> DiscoveredExtension {
        DiscoveredExtension {
            extension_id: "demo".to_string(),
            manifest_path: ".polint/extensions/demo/Cargo.toml".to_string(),
            activation_status: ExtensionActivationStatus::Discovered,
            manifest: ExtensionManifest::repo_local("demo").unwrap(),
            source_digest: crate::analysis_kernel::incremental::Digest::absent(
                crate::analysis_kernel::incremental::DigestKind::ExtensionCode,
                "source",
            ),
            dependency_digest: crate::analysis_kernel::incremental::Digest::absent(
                crate::analysis_kernel::incremental::DigestKind::ExtensionCode,
                "dependency",
            ),
            digest_input_paths: Vec::new(),
        }
    }

    fn outcome(status: i32, stdout: &str, stderr: &str) -> ExtensionCommandOutcome {
        ExtensionCommandOutcome {
            status: Some(status),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
            timed_out: false,
            spawn_error: None,
        }
    }

    #[test]
    fn successful_handshake_uses_cargo_run_without_shell_string() {
        let temp = TempDir::new().expect("create temp repo");
        let runner = FakeRunner::new(vec![outcome(
            0,
            r#"{"schema_version":"polint-extension-handshake-v1","extension_id":"demo","activation_status":"handshake_ok","providers":[],"diagnostics":[]}"#,
            "",
        )]);
        let host = ExtensionHost::with_runner(temp.path(), runner);

        let response = host.handshake(&extension()).unwrap();

        assert_eq!(
            response.activation_status,
            ExtensionActivationStatus::HandshakeOk
        );
        let spec = host.runner.specs.borrow();
        assert_eq!(spec[0].program, "cargo");
        assert!(spec[0].args.contains(&"run".to_string()));
        assert!(spec[0].args.contains(&"--manifest-path".to_string()));
        assert!(spec[0].args.contains(&"handshake".to_string()));
        assert!(spec[0].env.contains_key("CARGO_TARGET_DIR"));
        assert_eq!(spec[0].timeout, Duration::from_secs(30));
    }

    #[test]
    fn nonzero_exit_is_classified_without_emitted_facts() {
        let temp = TempDir::new().expect("create temp repo");
        let runner = FakeRunner::new(vec![outcome(1, "", "thread panicked at absolute/path")]);
        let host = ExtensionHost::with_runner(temp.path(), runner);

        let error = host.handshake(&extension()).unwrap_err();

        assert_eq!(error.kind, ExtensionHostFailureKind::HandshakeFailed);
        assert_eq!(error.activation_status(), ExtensionActivationStatus::Failed);
        assert_eq!(error.diagnostic().rule_id, "polint/extension");
    }

    #[test]
    fn invalid_json_is_malformed_response() {
        let temp = TempDir::new().expect("create temp repo");
        let runner = FakeRunner::new(vec![outcome(0, "not-json", "")]);
        let host = ExtensionHost::with_runner(temp.path(), runner);

        let error = host.handshake(&extension()).unwrap_err();

        assert_eq!(error.kind, ExtensionHostFailureKind::MalformedResponse);
        assert_eq!(
            error.activation_status(),
            ExtensionActivationStatus::ValidationFailed
        );
    }

    #[test]
    fn handshake_rejects_mismatched_extension_id() {
        let temp = TempDir::new().expect("create temp repo");
        let runner = FakeRunner::new(vec![outcome(
            0,
            r#"{"schema_version":"polint-extension-handshake-v1","extension_id":"spoof","activation_status":"handshake_ok","providers":[],"diagnostics":[]}"#,
            "",
        )]);
        let host = ExtensionHost::with_runner(temp.path(), runner);

        let error = host.handshake(&extension()).unwrap_err();

        assert_eq!(error.kind, ExtensionHostFailureKind::IdentityMismatch);
        assert_eq!(
            error.activation_status(),
            ExtensionActivationStatus::ValidationFailed
        );
    }

    #[test]
    fn provider_run_rejects_mismatched_identity() {
        let temp = TempDir::new().expect("create temp repo");
        let mismatched_extension = outcome(
            0,
            r#"{"schema_version":"polint-extension-provider-run-v1","extension_id":"spoof","provider_id":"routes","activation_status":"active","diagnostics":[],"facts":[],"output_digest_inputs":[]}"#,
            "",
        );
        let mismatched_provider = outcome(
            0,
            r#"{"schema_version":"polint-extension-provider-run-v1","extension_id":"demo","provider_id":"other","activation_status":"active","diagnostics":[],"facts":[],"output_digest_inputs":[]}"#,
            "",
        );
        let host = ExtensionHost::with_runner(
            temp.path(),
            FakeRunner::new(vec![mismatched_extension, mismatched_provider]),
        );

        let extension = extension();
        let first = host
            .run_provider(&extension, "routes", Vec::new(), Vec::new(), Vec::new())
            .unwrap_err();
        let second = host
            .run_provider(&extension, "routes", Vec::new(), Vec::new(), Vec::new())
            .unwrap_err();

        assert_eq!(first.kind, ExtensionHostFailureKind::IdentityMismatch);
        assert_eq!(second.kind, ExtensionHostFailureKind::IdentityMismatch);
    }

    #[test]
    fn timeout_is_classified() {
        let temp = TempDir::new().expect("create temp repo");
        let runner = FakeRunner::new(vec![ExtensionCommandOutcome {
            status: None,
            stdout: Vec::new(),
            stderr: Vec::new(),
            timed_out: true,
            spawn_error: None,
        }]);
        let host =
            ExtensionHost::with_runner(temp.path(), runner).with_timeout(Duration::from_millis(1));

        let error = host.handshake(&extension()).unwrap_err();

        assert_eq!(error.kind, ExtensionHostFailureKind::Timeout);
    }

    #[cfg(unix)]
    #[test]
    fn timeout_terminates_descendant_processes() {
        let temp = TempDir::new().expect("create temp repo");
        let pid_file = temp.path().join("sleep.pid");
        let mut env = BTreeMap::new();
        env.insert(
            "PID_FILE".to_string(),
            pid_file.to_string_lossy().to_string(),
        );
        let spec = ExtensionCommandSpec {
            program: "sh".to_string(),
            args: vec![
                "-c".to_string(),
                "sleep 3 & echo $! > \"$PID_FILE\"; wait".to_string(),
            ],
            env,
            stdin: Vec::new(),
            timeout: Duration::from_millis(50),
            stdout_limit: DEFAULT_STDOUT_LIMIT,
            stderr_limit: DEFAULT_STDERR_LIMIT,
        };

        let started = Instant::now();
        let outcome = run_std_command(&spec);

        assert!(outcome.timed_out);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "timeout waited for descendant process to finish"
        );
        if let Ok(pid) = std::fs::read_to_string(pid_file)
            && let Ok(pid) = pid.trim().parse::<i32>()
        {
            assert_process_exits(pid);
        }
    }

    #[cfg(unix)]
    #[test]
    fn large_stdout_is_drained_while_child_runs() {
        let spec = ExtensionCommandSpec {
            program: "sh".to_string(),
            args: vec![
                "-c".to_string(),
                "i=0; while [ $i -lt 5000 ]; do printf 0123456789abcdef0123456789abcdef; i=$((i + 1)); done".to_string(),
            ],
            env: BTreeMap::new(),
            stdin: Vec::new(),
            timeout: Duration::from_secs(2),
            stdout_limit: 1024,
            stderr_limit: DEFAULT_STDERR_LIMIT,
        };

        let outcome = run_std_command(&spec);

        assert!(!outcome.timed_out);
        assert_eq!(outcome.status, Some(0));
        assert_eq!(outcome.stdout.len(), 1024);
    }

    #[cfg(unix)]
    fn assert_process_exits(pid: i32) {
        for _ in 0..20 {
            if !process_exists(pid) {
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
        // Best-effort cleanup if the regression leaves the descendant around.
        let _ = Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        panic!("descendant process {pid} was still running after extension timeout");
    }

    #[cfg(unix)]
    fn process_exists(pid: i32) -> bool {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
}
