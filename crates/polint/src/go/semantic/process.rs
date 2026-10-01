use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[cfg(unix)]
use crate::go::embedded_cache::write_private_file;
use crate::go::embedded_cache::{
    materialize_embedded_sources, read_verified_private_file, verify_private_file,
};
use crate::go::lifecycle;
use crate::go::process_runner::{GO_SUBPROCESS_TIMEOUT, GoProcessError, run_bounded};
use crate::go::semantic::protocol::GO_SEMANTIC_SCHEMA;

pub const GO_SEMANTIC_FRONTEND_ENV: &str = "POLINT_GO_FRONTEND";
const EMBEDDED_GO_FRONTEND_FILES: &[(&str, &str)] = &[
    (
        "go.mod",
        include_str!("../../go-sidecar/polint-go-frontend/go.mod"),
    ),
    (
        "go.sum",
        include_str!("../../go-sidecar/polint-go-frontend/go.sum"),
    ),
    (
        "main.go",
        include_str!("../../go-sidecar/polint-go-frontend/main.go"),
    ),
    (
        "internal/semantic/emit.go",
        include_str!("../../go-sidecar/polint-go-frontend/internal/semantic/emit.go"),
    ),
    (
        "internal/semantic/rss_unix.go",
        include_str!("../../go-sidecar/polint-go-frontend/internal/semantic/rss_unix.go"),
    ),
    (
        "internal/semantic/rss_other.go",
        include_str!("../../go-sidecar/polint-go-frontend/internal/semantic/rss_other.go"),
    ),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoSemanticProcessError {
    CommandFailed(String),
    CommandUnavailable(String),
    VersionUnsupported(String),
    Timeout(String),
}

impl std::fmt::Display for GoSemanticProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CommandFailed(reason) => write!(f, "{reason}"),
            Self::CommandUnavailable(reason) => write!(f, "{reason}"),
            Self::VersionUnsupported(reason) => write!(f, "{reason}"),
            Self::Timeout(reason) => write!(f, "{reason}"),
        }
    }
}

impl std::error::Error for GoSemanticProcessError {}

#[derive(Debug, Clone)]
pub enum GoSemanticCommand {
    Binary(PathBuf),
    SourceDir(PathBuf),
}

pub fn resolve_go_semantic_frontend() -> Result<GoSemanticCommand, GoSemanticProcessError> {
    if let Ok(path) = std::env::var(GO_SEMANTIC_FRONTEND_ENV)
        && !path.trim().is_empty()
    {
        return command_for_path(PathBuf::from(path));
    }
    if let Some(path) = installed_frontend_binary()? {
        return Ok(GoSemanticCommand::Binary(path));
    }
    materialize_embedded_frontend().map(GoSemanticCommand::SourceDir)
}

pub fn command_for_path(path: PathBuf) -> Result<GoSemanticCommand, GoSemanticProcessError> {
    if path.is_file() {
        return Ok(GoSemanticCommand::Binary(path));
    }
    if path.join("go.mod").is_file() {
        return Ok(GoSemanticCommand::SourceDir(path));
    }
    Err(GoSemanticProcessError::CommandFailed(format!(
        "{GO_SEMANTIC_FRONTEND_ENV} must point to a polint-go-frontend binary or source directory."
    )))
}

fn installed_frontend_binary() -> Result<Option<PathBuf>, GoSemanticProcessError> {
    let executable = std::env::current_exe().map_err(|error| {
        GoSemanticProcessError::CommandFailed(format!(
            "failed to resolve current executable: {error}"
        ))
    })?;
    let Some(directory) = executable.parent() else {
        return Ok(None);
    };
    let candidate = directory.join(frontend_binary_name());
    Ok(candidate.is_file().then_some(candidate))
}

fn frontend_binary_name() -> &'static str {
    if cfg!(windows) {
        "polint-go-frontend.exe"
    } else {
        "polint-go-frontend"
    }
}

fn materialize_embedded_frontend() -> Result<PathBuf, GoSemanticProcessError> {
    materialize_embedded_sources(
        "semantic",
        env!("CARGO_PKG_VERSION"),
        &embedded_frontend_hash(),
        EMBEDDED_GO_FRONTEND_FILES,
    )
    .map_err(|reason| {
        GoSemanticProcessError::CommandFailed(format!(
            "failed to materialize embedded Go semantic frontend: {reason}"
        ))
    })
}

pub fn embedded_frontend_hash() -> String {
    let mut parts = Vec::new();
    parts.push(GO_SEMANTIC_SCHEMA.to_string());
    for (relative_path, contents) in EMBEDDED_GO_FRONTEND_FILES {
        parts.push(format!(
            "{relative_path}:{}",
            crate::go::hash::stable_hash(&[*contents])
        ));
    }
    let part_refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    crate::go::hash::stable_hash(&part_refs)
}

pub fn frontend_digest(frontend: &GoSemanticCommand) -> Result<String, GoSemanticProcessError> {
    match frontend {
        GoSemanticCommand::Binary(path) => {
            let bytes = fs::read(path).map_err(|error| {
                GoSemanticProcessError::CommandFailed(format!(
                    "failed to read Go semantic frontend binary `{}` for digest: {error}",
                    path.display()
                ))
            })?;
            Ok(stable_bytes_hash(&bytes))
        }
        GoSemanticCommand::SourceDir(path) => source_dir_digest(path),
    }
}

fn source_dir_digest(path: &Path) -> Result<String, GoSemanticProcessError> {
    let mut files = Vec::new();
    collect_frontend_source_files(path, path, &mut files)?;
    files.sort();
    let mut parts = vec![GO_SEMANTIC_SCHEMA.to_string()];
    for relative_path in files {
        let full_path = path.join(&relative_path);
        let bytes = fs::read(&full_path).map_err(|error| {
            GoSemanticProcessError::CommandFailed(format!(
                "failed to read Go semantic frontend source `{}` for digest: {error}",
                full_path.display()
            ))
        })?;
        parts.push(format!(
            "{}:{}",
            relative_path.to_string_lossy().replace('\\', "/"),
            stable_bytes_hash(&bytes)
        ));
    }
    let refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    Ok(crate::go::hash::stable_hash(&refs))
}

fn collect_frontend_source_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<PathBuf>,
) -> Result<(), GoSemanticProcessError> {
    let entries = fs::read_dir(directory).map_err(|error| {
        GoSemanticProcessError::CommandFailed(format!(
            "failed to read Go semantic frontend source directory `{}` for digest: {error}",
            directory.display()
        ))
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            GoSemanticProcessError::CommandFailed(format!(
                "failed to read Go semantic frontend source directory entry `{}` for digest: {error}",
                directory.display()
            ))
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| {
            GoSemanticProcessError::CommandFailed(format!(
                "failed to read Go semantic frontend source file type `{}` for digest: {error}",
                path.display()
            ))
        })?;
        if file_type.is_dir() && !skip_frontend_digest_dir(&path) {
            collect_frontend_source_files(root, &path, files)?;
        } else if file_type.is_file() && is_frontend_digest_source(&path) {
            let relative_path = path.strip_prefix(root).map_err(|error| {
                GoSemanticProcessError::CommandFailed(format!(
                    "failed to relativize Go semantic frontend source `{}` for digest: {error}",
                    path.display()
                ))
            })?;
            files.push(relative_path.to_path_buf());
        }
    }
    Ok(())
}

fn skip_frontend_digest_dir(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(".git" | ".hg" | "target" | "node_modules")
    )
}

fn is_frontend_digest_source(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("go.mod" | "go.sum")
    ) || path.extension().and_then(|extension| extension.to_str()) == Some("go")
}

fn stable_bytes_hash(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

pub fn command_for_frontend(
    frontend: &GoSemanticCommand,
    root: &Path,
    offline: bool,
) -> Result<Command, GoSemanticProcessError> {
    match frontend {
        GoSemanticCommand::Binary(path) => {
            let mut command = Command::new(path);
            command.current_dir(root);
            Ok(command)
        }
        GoSemanticCommand::SourceDir(path) => {
            let binary = ensure_frontend_binary(path, offline, GO_SUBPROCESS_TIMEOUT)?;
            let mut command = Command::new(binary);
            command.current_dir(root);
            Ok(command)
        }
    }
}

/// What tells one embedded Go sidecar's cached build apart from another's.
///
/// Every embedded sidecar is built the same way: once, with the local toolchain
/// pinned, into its private materialized source directory, and then executed
/// directly. Executing the built binary, rather than `go run`, matters for what
/// the sidecar does next: the toolchain pin stays on the build, so the go command
/// the sidecar starts to load the analyzed modules honours their own `go` and
/// `toolchain` lines exactly as a `go build` inside those modules would.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GoSidecarBinary {
    /// Program name of the sidecar; the cached build is a dot file named after it.
    program: &'static str,
    /// Version label of the cache key. Changing the build recipe changes it.
    cache_context: &'static str,
    /// Version label of the receipt published beside the cached build.
    receipt_label: &'static str,
    /// How build and cache errors name the sidecar.
    label: &'static str,
}

/// The Go semantic frontend.
pub(crate) const SEMANTIC_FRONTEND_BINARY: GoSidecarBinary = GoSidecarBinary {
    program: "polint-go-frontend",
    cache_context: "polint-go-frontend-binary-v2",
    receipt_label: "polint-go-frontend-receipt-v1",
    label: "embedded frontend",
};

/// The Go symbol sidecar.
pub(crate) const SYMBOLS_SIDECAR_BINARY: GoSidecarBinary = GoSidecarBinary {
    program: "polint-go-symbols",
    cache_context: "polint-go-symbols-binary-v1",
    receipt_label: "polint-go-symbols-receipt-v1",
    label: "embedded symbol sidecar",
};

impl GoSidecarBinary {
    fn file_name(self) -> String {
        if cfg!(windows) {
            format!("{}.exe", self.program)
        } else {
            self.program.to_string()
        }
    }
}

fn ensure_frontend_binary(
    source_dir: &Path,
    offline: bool,
    timeout: Duration,
) -> Result<PathBuf, GoSemanticProcessError> {
    ensure_go_sidecar_binary(&SEMANTIC_FRONTEND_BINARY, source_dir, offline, timeout)
}

/// Builds the embedded sidecar in `source_dir` once per toolchain and host target
/// and returns the cached, verified binary.
pub(crate) fn ensure_go_sidecar_binary(
    sidecar: &GoSidecarBinary,
    source_dir: &Path,
    offline: bool,
    timeout: Duration,
) -> Result<PathBuf, GoSemanticProcessError> {
    ensure_sidecar_binary_with_program_and_hooks(
        sidecar,
        source_dir,
        offline,
        timeout,
        OsStr::new("go"),
        || {},
        || {},
    )
}

#[cfg(test)]
fn ensure_frontend_binary_with_program(
    source_dir: &Path,
    offline: bool,
    timeout: Duration,
    go_program: &OsStr,
) -> Result<PathBuf, GoSemanticProcessError> {
    ensure_frontend_binary_with_program_and_hooks(
        source_dir,
        offline,
        timeout,
        go_program,
        || {},
        || {},
    )
}

#[cfg(test)]
fn ensure_frontend_binary_with_program_and_hooks<AfterPublish, OnContention>(
    source_dir: &Path,
    offline: bool,
    timeout: Duration,
    go_program: &OsStr,
    after_binary_publish: AfterPublish,
    on_lock_contention: OnContention,
) -> Result<PathBuf, GoSemanticProcessError>
where
    AfterPublish: FnOnce(),
    OnContention: FnOnce(),
{
    ensure_sidecar_binary_with_program_and_hooks(
        &SEMANTIC_FRONTEND_BINARY,
        source_dir,
        offline,
        timeout,
        go_program,
        after_binary_publish,
        on_lock_contention,
    )
}

fn ensure_sidecar_binary_with_program_and_hooks<AfterPublish, OnContention>(
    sidecar: &GoSidecarBinary,
    source_dir: &Path,
    offline: bool,
    timeout: Duration,
    go_program: &OsStr,
    after_binary_publish: AfterPublish,
    on_lock_contention: OnContention,
) -> Result<PathBuf, GoSemanticProcessError>
where
    AfterPublish: FnOnce(),
    OnContention: FnOnce(),
{
    let label = sidecar.label;
    let toolchain = local_go_toolchain_version_with_program(go_program)?;
    if !go_version_at_least(&toolchain, 1, 25) {
        return Err(GoSemanticProcessError::VersionUnsupported(format!(
            "{} source mode requires Go 1.25 or newer on PATH; found {toolchain}",
            sidecar.program
        )));
    }
    let (goos, goarch) = execution_host_go_target()?;
    verify_local_go_target_with_program(go_program, offline, goos, goarch)?;
    let cache_key = sidecar_binary_cache_key(
        sidecar,
        std::env::consts::OS,
        std::env::consts::ARCH,
        &toolchain,
        goos,
        goarch,
    );
    let binary = source_dir.join(format!(".{}-{cache_key}", sidecar.file_name()));
    let receipt_path = source_dir.join(format!(".binary-receipt-{cache_key}"));
    if cached_binary_matches(sidecar, &binary, &receipt_path, &cache_key) {
        return Ok(binary);
    }

    let _cache_lock =
        lock_frontend_binary_cache(sidecar, source_dir, &cache_key, on_lock_contention)?;
    if cached_binary_matches(sidecar, &binary, &receipt_path, &cache_key) {
        return Ok(binary);
    }
    if binary.exists() || receipt_path.exists() {
        return Err(GoSemanticProcessError::CommandFailed(format!(
            "{label} cache contains an unverifiable immutable entry `{}`",
            binary.display()
        )));
    }

    let staging = source_dir.join(format!(
        ".build-{cache_key}-{}-{}",
        std::process::id(),
        unique_build_suffix()
    ));
    let command = frontend_build_command(go_program, source_dir, &staging, offline, goos, goarch);
    let output = run_bounded(
        command,
        timeout,
        &format!("go build of {}", sidecar.program),
    )
    .map_err(map_process_error)?;
    if !output.status.success() {
        let _ = fs::remove_file(&staging);
        return Err(GoSemanticProcessError::CommandFailed(format!(
            "go build of {label} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    make_binary_private_executable(sidecar, &staging)?;

    // Platforms where cache-file ownership cannot be verified never execute a
    // persistent binary from the cache. The unique just-built output is used
    // once and will not be selected by a later invocation.
    #[cfg(not(unix))]
    {
        after_binary_publish();
        Ok(staging)
    }

    #[cfg(unix)]
    {
        let bytes = fs::read(&staging).map_err(|error| {
            GoSemanticProcessError::CommandFailed(format!(
                "failed to hash built {label} `{}`: {error}",
                staging.display()
            ))
        })?;
        let digest = stable_bytes_hash(&bytes);

        match fs::hard_link(&staging, &binary) {
            Ok(()) => {
                let _ = fs::remove_file(&staging);
                after_binary_publish();
                publish_binary_receipt(sidecar, &receipt_path, &cache_key, &digest)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = fs::remove_file(&staging);
                if cached_binary_matches(sidecar, &binary, &receipt_path, &cache_key) {
                    return Ok(binary);
                }
                return Err(GoSemanticProcessError::CommandFailed(format!(
                    "{label} cache contains an unverifiable immutable binary `{}`",
                    binary.display()
                )));
            }
            Err(error) => {
                let _ = fs::remove_file(&staging);
                return Err(GoSemanticProcessError::CommandFailed(format!(
                    "failed to publish {label} binary `{}`: {error}",
                    binary.display()
                )));
            }
        }
        if !cached_binary_matches(sidecar, &binary, &receipt_path, &cache_key) {
            return Err(GoSemanticProcessError::CommandFailed(format!(
                "published {label} binary `{}` failed receipt or ownership verification",
                binary.display()
            )));
        }
        Ok(binary)
    }
}

#[cfg(test)]
fn frontend_binary_cache_key(
    execution_os: &str,
    execution_arch: &str,
    toolchain: &str,
    goos: &str,
    goarch: &str,
) -> String {
    sidecar_binary_cache_key(
        &SEMANTIC_FRONTEND_BINARY,
        execution_os,
        execution_arch,
        toolchain,
        goos,
        goarch,
    )
}

fn sidecar_binary_cache_key(
    sidecar: &GoSidecarBinary,
    execution_os: &str,
    execution_arch: &str,
    toolchain: &str,
    goos: &str,
    goarch: &str,
) -> String {
    let build_environment = [
        "GOENV",
        "GOFLAGS",
        "CGO_ENABLED",
        "CC",
        "CXX",
        "GOTOOLDIR",
        "GOROOT",
    ]
    .map(|name| {
        format!(
            "{name}={}",
            std::env::var_os(name).unwrap_or_default().to_string_lossy()
        )
    });
    let mut context = vec![
        sidecar.cache_context,
        execution_os,
        execution_arch,
        toolchain,
        goos,
        goarch,
    ];
    context.extend(build_environment.iter().map(String::as_str));
    crate::go::hash::stable_hash(&context)
}

fn lock_frontend_binary_cache(
    sidecar: &GoSidecarBinary,
    source_dir: &Path,
    cache_context: &str,
    on_contention: impl FnOnce(),
) -> Result<fs::File, GoSemanticProcessError> {
    let label = sidecar.label;
    let lock_path = source_dir.join(format!(".binary-lock-{cache_context}"));
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(&lock_path).map_err(|error| {
        GoSemanticProcessError::CommandFailed(format!(
            "failed to open {label} binary cache lock `{}`: {error}",
            lock_path.display()
        ))
    })?;
    // Do not read the lock through a second handle: on Windows an existing
    // byte-range lock rejects that read with ERROR_LOCK_VIOLATION. The lock's
    // contents are irrelevant; only its private regular-file identity matters.
    verify_private_file(&lock_path).map_err(|reason| {
        GoSemanticProcessError::CommandFailed(format!(
            "refusing untrusted {label} binary cache lock `{}`: {reason}",
            lock_path.display()
        ))
    })?;

    match lock.try_lock() {
        Ok(()) => {}
        Err(fs::TryLockError::WouldBlock) => {
            on_contention();
            lock.lock().map_err(|error| {
                GoSemanticProcessError::CommandFailed(format!(
                    "failed to wait for {label} binary cache lock `{}`: {error}",
                    lock_path.display()
                ))
            })?;
        }
        Err(fs::TryLockError::Error(error)) => {
            return Err(GoSemanticProcessError::CommandFailed(format!(
                "failed to acquire {label} binary cache lock `{}`: {error}",
                lock_path.display()
            )));
        }
    }
    Ok(lock)
}

fn frontend_build_command(
    go_program: &OsStr,
    source_dir: &Path,
    staging: &Path,
    offline: bool,
    goos: &str,
    goarch: &str,
) -> Command {
    let mut command = Command::new(go_program);
    command
        .arg("build")
        .arg("-o")
        .arg(staging)
        .arg(".")
        .current_dir(source_dir)
        .env("GOWORK", "off")
        .env("GOTOOLCHAIN", "local")
        .env_remove("GOOS")
        .env_remove("GOARCH")
        .env("GOOS", goos)
        .env("GOARCH", goarch);
    lifecycle::apply_go_offline_env(&mut command, offline);
    command
}

#[cfg(unix)]
fn cached_binary_matches(
    sidecar: &GoSidecarBinary,
    binary: &Path,
    receipt_path: &Path,
    cache_key: &str,
) -> bool {
    let Ok(actual_receipt) = read_verified_private_file(receipt_path) else {
        return false;
    };
    let Ok(bytes) = read_verified_private_file(binary) else {
        return false;
    };
    actual_receipt == binary_receipt(sidecar, cache_key, &stable_bytes_hash(&bytes)).as_bytes()
}

#[cfg(not(unix))]
fn cached_binary_matches(
    _sidecar: &GoSidecarBinary,
    _binary: &Path,
    _receipt_path: &Path,
    _cache_key: &str,
) -> bool {
    false
}

fn binary_receipt(sidecar: &GoSidecarBinary, cache_key: &str, digest: &str) -> String {
    format!("{}\n{cache_key}\n{digest}\n", sidecar.receipt_label)
}

#[cfg(unix)]
fn publish_binary_receipt(
    sidecar: &GoSidecarBinary,
    path: &Path,
    cache_key: &str,
    digest: &str,
) -> Result<(), GoSemanticProcessError> {
    let label = sidecar.label;
    let staging = path.with_extension(format!(
        "receipt-{}-{}",
        std::process::id(),
        unique_build_suffix()
    ));
    write_private_file(
        &staging,
        binary_receipt(sidecar, cache_key, digest).as_bytes(),
    )
    .map_err(|reason| {
        GoSemanticProcessError::CommandFailed(format!(
            "failed to stage {label} binary receipt: {reason}"
        ))
    })?;
    let result = fs::hard_link(&staging, path).map_err(|error| {
        GoSemanticProcessError::CommandFailed(format!(
            "failed to publish immutable {label} binary receipt `{}`: {error}",
            path.display()
        ))
    });
    let _ = fs::remove_file(&staging);
    result
}

fn make_binary_private_executable(
    sidecar: &GoSidecarBinary,
    path: &Path,
) -> Result<(), GoSemanticProcessError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
            GoSemanticProcessError::CommandFailed(format!(
                "failed to secure {} binary `{}`: {error}",
                sidecar.label,
                path.display()
            ))
        })?;
    }
    #[cfg(not(unix))]
    let _ = (sidecar, path);
    Ok(())
}

fn unique_build_suffix() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{nanos}-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

fn map_process_error(error: GoProcessError) -> GoSemanticProcessError {
    match error {
        GoProcessError::Unavailable(reason) => GoSemanticProcessError::CommandUnavailable(reason),
        GoProcessError::Failed(reason) => GoSemanticProcessError::CommandFailed(reason),
        GoProcessError::Timeout(reason) => GoSemanticProcessError::Timeout(reason),
    }
}

pub fn local_go_toolchain_version() -> Result<String, GoSemanticProcessError> {
    local_go_toolchain_version_with_program(OsStr::new("go"))
}

fn local_go_toolchain_version_with_program(
    go_program: &OsStr,
) -> Result<String, GoSemanticProcessError> {
    let mut command = Command::new(go_program);
    command.arg("version").env("GOTOOLCHAIN", "local");
    lifecycle::apply_go_offline_env(&mut command, true);
    let output =
        run_bounded(command, GO_SUBPROCESS_TIMEOUT, "go version").map_err(map_process_error)?;
    if !output.status.success() {
        return Err(GoSemanticProcessError::CommandFailed(format!(
            "`go version` exited with status {}",
            output.status
        )));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_go_version(stdout.as_ref()).ok_or_else(|| {
        GoSemanticProcessError::CommandFailed(format!(
            "failed to parse Go toolchain version from `{}`",
            stdout.trim()
        ))
    })
}

fn execution_host_go_target() -> Result<(&'static str, &'static str), GoSemanticProcessError> {
    let goos = match std::env::consts::OS {
        "macos" => "darwin",
        "linux" => "linux",
        "windows" => "windows",
        "freebsd" => "freebsd",
        "openbsd" => "openbsd",
        "netbsd" => "netbsd",
        "dragonfly" => "dragonfly",
        "solaris" => "solaris",
        "illumos" => "illumos",
        "aix" => "aix",
        "android" => "android",
        "ios" => "ios",
        other => {
            return Err(GoSemanticProcessError::CommandFailed(format!(
                "cannot map execution host OS `{other}` to a Go build target"
            )));
        }
    };
    let goarch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "x86" => "386",
        "aarch64" => "arm64",
        "arm" => "arm",
        "powerpc64" => "ppc64",
        "powerpc64le" => "ppc64le",
        "s390x" => "s390x",
        "riscv64" => "riscv64",
        "loongarch64" => "loong64",
        other => {
            return Err(GoSemanticProcessError::CommandFailed(format!(
                "cannot map execution host architecture `{other}` to a Go build target"
            )));
        }
    };
    Ok((goos, goarch))
}

fn verify_local_go_target_with_program(
    go_program: &OsStr,
    offline: bool,
    host_goos: &str,
    host_goarch: &str,
) -> Result<(), GoSemanticProcessError> {
    let mut command = Command::new(go_program);
    command
        .args(["env", "GOOS", "GOARCH"])
        .env_remove("GOOS")
        .env_remove("GOARCH")
        .env("GOOS", host_goos)
        .env("GOARCH", host_goarch)
        .env("GOTOOLCHAIN", "local");
    lifecycle::apply_go_offline_env(&mut command, offline);
    let output = run_bounded(command, GO_SUBPROCESS_TIMEOUT, "go env GOOS GOARCH")
        .map_err(map_process_error)?;
    if !output.status.success() {
        return Err(GoSemanticProcessError::CommandFailed(format!(
            "`go env GOOS GOARCH` exited with status {}",
            output.status
        )));
    }
    let actual = parse_go_target(&String::from_utf8_lossy(&output.stdout)).ok_or_else(|| {
        GoSemanticProcessError::CommandFailed(
            "failed to parse GOOS and GOARCH from `go env GOOS GOARCH`".to_string(),
        )
    })?;
    if actual.0 != host_goos || actual.1 != host_goarch {
        return Err(GoSemanticProcessError::CommandFailed(format!(
            "refusing cross-target embedded frontend build for {}/{}; execution host requires {host_goos}/{host_goarch}",
            actual.0, actual.1
        )));
    }
    Ok(())
}

fn parse_go_target(output: &str) -> Option<(String, String)> {
    let mut values = output
        .lines()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let goos = values.next()?.to_string();
    let goarch = values.next()?.to_string();
    values.next().is_none().then_some((goos, goarch))
}

fn parse_go_version(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .find(|part| part.starts_with("go") && part.len() > 2)
        .map(str::to_string)
}

fn go_version_at_least(version: &str, min_major: u32, min_minor: u32) -> bool {
    let Some((major, minor)) = parse_go_version_numbers(version) else {
        return false;
    };
    major > min_major || (major == min_major && minor >= min_minor)
}

fn parse_go_version_numbers(version: &str) -> Option<(u32, u32)> {
    let version = version.trim().trim_start_matches("go");
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_for_path_accepts_source_directory() {
        let temp = tempfile::tempdir().expect("temp dir");
        fs::write(temp.path().join("go.mod"), "module test\n").expect("write go.mod");
        let command = command_for_path(temp.path().to_path_buf()).expect("source dir accepted");
        assert!(matches!(command, GoSemanticCommand::SourceDir(_)));
    }

    #[test]
    fn embedded_frontend_hash_includes_schema() {
        assert!(!embedded_frontend_hash().is_empty());
    }

    #[test]
    fn embedded_go_frontend_sources_match_workspace_sources() {
        let workspace_frontend =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/go-sidecar/polint-go-frontend");
        for (relative_path, embedded) in EMBEDDED_GO_FRONTEND_FILES {
            let workspace = fs::read_to_string(workspace_frontend.join(relative_path))
                .unwrap_or_else(|error| panic!("read workspace frontend {relative_path}: {error}"));
            assert_eq!(
                workspace, *embedded,
                "embedded Go semantic frontend drifted at {relative_path}"
            );
        }
    }

    /// The embedded list is what a release build compiles; a source file missing
    /// from it builds in the workspace and fails only at a user's first deep run.
    #[test]
    fn every_workspace_frontend_source_is_embedded() {
        let workspace_frontend =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/go-sidecar/polint-go-frontend");
        let mut pending = vec![workspace_frontend.clone()];
        let mut sources = Vec::new();
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(&directory).expect("read frontend directory") {
                let path = entry.expect("frontend entry").path();
                if path.is_dir() {
                    pending.push(path);
                    continue;
                }
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("");
                if name.ends_with(".go") && !name.ends_with("_test.go") {
                    let relative = path
                        .strip_prefix(&workspace_frontend)
                        .expect("under the frontend")
                        .to_string_lossy()
                        .replace('\\', "/");
                    sources.push(relative);
                }
            }
        }
        sources.sort();
        let mut embedded = EMBEDDED_GO_FRONTEND_FILES
            .iter()
            .map(|(relative, _)| (*relative).to_string())
            .filter(|relative| relative.ends_with(".go"))
            .collect::<Vec<_>>();
        embedded.sort();
        assert_eq!(sources, embedded);
    }

    #[test]
    fn source_dir_digest_changes_when_go_source_changes() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::write(temp.path().join("go.mod"), "module example.test/frontend\n")
            .expect("write go.mod");
        fs::write(
            temp.path().join("main.go"),
            "package main\nfunc main() {}\n",
        )
        .expect("write main.go");
        let first = frontend_digest(&GoSemanticCommand::SourceDir(temp.path().to_path_buf()))
            .expect("digest source dir");

        fs::write(
            temp.path().join("main.go"),
            "package main\nfunc main() { println(\"changed\") }\n",
        )
        .expect("rewrite main.go");
        let second = frontend_digest(&GoSemanticCommand::SourceDir(temp.path().to_path_buf()))
            .expect("digest source dir again");

        assert_ne!(first, second);
    }

    #[test]
    fn cold_frontend_build_honors_offline_policy() {
        use std::ffi::OsStr;

        let command = frontend_build_command(
            OsStr::new("go"),
            Path::new("source"),
            Path::new("binary"),
            true,
            "linux",
            "amd64",
        );
        let environment = command
            .get_envs()
            .collect::<std::collections::BTreeMap<_, _>>();
        for (key, expected) in [
            ("GOENV", "off"),
            ("GOPROXY", "off"),
            ("GOSUMDB", "off"),
            ("GOPRIVATE", "none"),
            ("GONOPROXY", "none"),
            ("GONOSUMDB", "none"),
            ("GOINSECURE", "none"),
            ("GOVCS", "*:off"),
            ("GOAUTH", "off"),
            ("GOTOOLCHAIN", "local"),
            ("GOOS", "linux"),
            ("GOARCH", "amd64"),
        ] {
            assert_eq!(
                environment.get(OsStr::new(key)).copied().flatten(),
                Some(OsStr::new(expected)),
                "unexpected offline value for {key}"
            );
        }
        assert_eq!(
            environment.get(OsStr::new("GOCACHEPROG")),
            Some(&None),
            "offline commands must remove inherited external cache helpers"
        );
    }

    #[cfg(unix)]
    #[test]
    fn inherited_windows_target_is_replaced_with_execution_host_target() {
        use std::os::unix::fs::PermissionsExt;

        const CHILD_MARKER: &str = "POLINT_OFFLINE_ENV_REGRESSION_CHILD";
        if std::env::var_os(CHILD_MARKER).is_none() {
            let current_thread = std::thread::current();
            let test_name = current_thread.name().expect("test harness thread name");
            let status = Command::new(std::env::current_exe().expect("current test executable"))
                .arg("--exact")
                .arg(test_name)
                .env(CHILD_MARKER, "1")
                .env("GOENV", "/fatal/inherited/go/env")
                .env("GOPROXY", "direct")
                .env("GOSUMDB", "sum.golang.org")
                .env("GOPRIVATE", "*")
                .env("GONOPROXY", "*")
                .env("GONOSUMDB", "*")
                .env("GOINSECURE", "*")
                .env("GOVCS", "*:all")
                .env("GOAUTH", "fatal-auth-helper")
                .env("GOTOOLCHAIN", "auto")
                .env("GOCACHEPROG", "fatal-cache-helper")
                .env("GOOS", "windows")
                .env("GOARCH", "386")
                .status()
                .expect("spawn isolated inherited-environment regression");
            assert!(
                status.success(),
                "isolated regression test failed: {status}"
            );
            return;
        }

        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        fs::create_dir(&source).expect("create source");
        fs::set_permissions(&source, fs::Permissions::from_mode(0o700)).expect("secure source");
        let fake_go = temp.path().join("fake-go");
        fs::write(
            &fake_go,
            r#"#!/bin/sh
[ "$GOENV" = "off" ] || exit 81
[ "$GOPROXY" = "off" ] || exit 82
[ "$GOSUMDB" = "off" ] || exit 83
[ "$GOPRIVATE" = "none" ] || exit 84
[ "$GONOPROXY" = "none" ] || exit 85
[ "$GONOSUMDB" = "none" ] || exit 86
[ "$GOINSECURE" = "none" ] || exit 87
[ "$GOVCS" = "*:off" ] || exit 88
[ "$GOAUTH" = "off" ] || exit 89
[ "$GOTOOLCHAIN" = "local" ] || exit 90
[ -z "$GOCACHEPROG" ] || exit 91
if [ "$1" = "version" ]; then
  echo 'go version go1.25.0 test/arch'
  exit 0
fi
if [ "$1" = "env" ]; then
  printf '%s\n%s\n' "$GOOS" "$GOARCH"
  exit 0
fi
out=''
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-o" ]; then shift; out="$1"; fi
  shift
done
[ -n "$out" ] || exit 92
printf '#!/bin/sh\nprintf "%%s\\n" "%s/%s"\n' "$GOOS" "$GOARCH" > "$out"
"#,
        )
        .expect("write fake go");
        fs::set_permissions(&fake_go, fs::Permissions::from_mode(0o700)).expect("chmod fake go");

        let binary = ensure_frontend_binary_with_program(
            &source,
            true,
            Duration::from_secs(5),
            fake_go.as_os_str(),
        )
        .expect("cold offline build");
        let output = Command::new(&binary)
            .output()
            .expect("execute host frontend binary");
        assert!(output.status.success());
        let (host_goos, host_goarch) = execution_host_go_target().expect("host Go target");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            format!("{host_goos}/{host_goarch}")
        );
    }

    #[cfg(unix)]
    #[test]
    fn competing_frontend_publisher_waits_for_binary_receipt_pair() {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::mpsc;

        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        fs::create_dir(&source).expect("create source");
        fs::set_permissions(&source, fs::Permissions::from_mode(0o700)).expect("secure source");
        let build_log = temp.path().join("build.log");
        let fake_go = temp.path().join("fake-go");
        fs::write(
            &fake_go,
            format!(
                r#"#!/bin/sh
if [ "$1" = "version" ]; then
  echo 'go version go1.25.0 test/arch'
  exit 0
fi
if [ "$1" = "env" ]; then
  printf '%s\n%s\n' "$GOOS" "$GOARCH"
  exit 0
fi
out=''
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-o" ]; then shift; out="$1"; fi
  shift
done
[ -n "$out" ] || exit 92
printf 'build\n' >> '{}'
printf 'fake concurrent frontend' > "$out"
"#,
                build_log.display()
            ),
        )
        .expect("write fake go");
        fs::set_permissions(&fake_go, fs::Permissions::from_mode(0o700)).expect("chmod fake go");

        let (published_tx, published_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let first_source = source.clone();
        let first_go = fake_go.clone();
        let first = std::thread::spawn(move || {
            ensure_frontend_binary_with_program_and_hooks(
                &first_source,
                true,
                Duration::from_secs(5),
                first_go.as_os_str(),
                move || {
                    published_tx.send(()).expect("signal binary publication");
                    release_rx.recv().expect("wait before receipt publication");
                },
                || {},
            )
        });
        published_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("first publisher reached binary/receipt interleaving");

        let (host_goos, host_goarch) = execution_host_go_target().expect("host Go target");
        let cache_key = frontend_binary_cache_key(
            std::env::consts::OS,
            std::env::consts::ARCH,
            "go1.25.0",
            host_goos,
            host_goarch,
        );
        let binary = source.join(format!(".{}-{cache_key}", frontend_binary_name()));
        let receipt = source.join(format!(".binary-receipt-{cache_key}"));
        assert!(binary.is_file());
        assert!(!receipt.exists());

        let (contention_tx, contention_rx) = mpsc::channel();
        let second_source = source;
        let second_go = fake_go;
        let second = std::thread::spawn(move || {
            ensure_frontend_binary_with_program_and_hooks(
                &second_source,
                true,
                Duration::from_secs(5),
                second_go.as_os_str(),
                || panic!("waiting publisher must reuse the completed pair"),
                move || contention_tx.send(()).expect("signal lock contention"),
            )
        });
        contention_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("second publisher observed the in-progress publication");

        assert!(binary.is_file());
        assert!(!receipt.exists());
        release_tx.send(()).expect("release first publisher");

        let first_binary = first
            .join()
            .expect("first publisher thread")
            .expect("publish");
        let second_binary = second
            .join()
            .expect("second publisher thread")
            .expect("reuse published pair");
        assert_eq!(first_binary, second_binary);
        assert!(cached_binary_matches(
            &SEMANTIC_FRONTEND_BINARY,
            &second_binary,
            &receipt,
            &cache_key
        ));
        assert_eq!(
            fs::read_to_string(build_log)
                .expect("read build log")
                .lines()
                .count(),
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn cached_frontend_binary_requires_matching_private_receipt() {
        let temp = tempfile::tempdir().expect("tempdir");
        let binary = temp.path().join("polint-go-frontend");
        let receipt = temp.path().join(".binary-receipt");
        let cache_key =
            frontend_binary_cache_key("test-os", "test-arch", "go1.25", "goos", "goarch");
        write_private_file(&binary, b"trusted binary").expect("write binary");
        make_binary_private_executable(&SEMANTIC_FRONTEND_BINARY, &binary).expect("secure binary");
        write_private_file(
            &receipt,
            binary_receipt(
                &SEMANTIC_FRONTEND_BINARY,
                &cache_key,
                &stable_bytes_hash(b"trusted binary"),
            )
            .as_bytes(),
        )
        .expect("write receipt");
        assert!(cached_binary_matches(
            &SEMANTIC_FRONTEND_BINARY,
            &binary,
            &receipt,
            &cache_key
        ));

        fs::write(&binary, b"changed binary").expect("tamper binary");
        assert!(!cached_binary_matches(
            &SEMANTIC_FRONTEND_BINARY,
            &binary,
            &receipt,
            &cache_key
        ));
    }

    #[cfg(not(unix))]
    #[test]
    fn adjacent_attacker_receipt_never_authorizes_binary_reuse() {
        let temp = tempfile::tempdir().expect("tempdir");
        let binary = temp.path().join(frontend_binary_name());
        let receipt = temp.path().join(".binary-receipt");
        fs::write(&binary, b"attacker binary").expect("write binary");
        fs::write(
            &receipt,
            binary_receipt(
                &SEMANTIC_FRONTEND_BINARY,
                "attacker-key",
                &stable_bytes_hash(b"attacker binary"),
            ),
        )
        .expect("write receipt");

        assert!(!cached_binary_matches(
            &SEMANTIC_FRONTEND_BINARY,
            &binary,
            &receipt,
            "attacker-key"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_frontend_contexts_publish_distinct_immutable_binaries() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        fs::create_dir(&source).expect("create source");
        fs::set_permissions(&source, fs::Permissions::from_mode(0o700)).expect("secure source");

        let write_fake_go = |name: &str, version: &str, ready: &str, peer: &str, payload: &str| {
            let path = temp.path().join(name);
            fs::write(
                &path,
                format!(
                    r#"#!/bin/sh
if [ "$1" = "version" ]; then
  echo 'go version {version} test/arch'
  exit 0
fi
if [ "$1" = "env" ]; then
  printf '%s\n%s\n' "$GOOS" "$GOARCH"
  exit 0
fi
out=''
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-o" ]; then shift; out="$1"; fi
  shift
done
touch '{}'
i=0
while [ ! -f '{}' ]; do
  i=$((i + 1)); [ "$i" -lt 500 ] || exit 93
  sleep 0.01
done
printf '{payload}' > "$out"
"#,
                    temp.path().join(ready).display(),
                    temp.path().join(peer).display(),
                ),
            )
            .expect("write fake go");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("chmod fake go");
            path
        };
        let first_go = write_fake_go("fake-go-a", "go1.25.0", "ready-a", "ready-b", "first");
        let second_go = write_fake_go("fake-go-b", "go1.26.0", "ready-b", "ready-a", "second");

        let first_source = source.clone();
        let first = std::thread::spawn(move || {
            ensure_frontend_binary_with_program(
                &first_source,
                true,
                Duration::from_secs(10),
                first_go.as_os_str(),
            )
        });
        let second_source = source;
        let second = std::thread::spawn(move || {
            ensure_frontend_binary_with_program(
                &second_source,
                true,
                Duration::from_secs(10),
                second_go.as_os_str(),
            )
        });

        let first_binary = first
            .join()
            .expect("first context thread")
            .expect("first build");
        let second_binary = second
            .join()
            .expect("second context thread")
            .expect("second build");
        assert_ne!(first_binary, second_binary);
        assert_eq!(
            fs::read(&first_binary).expect("read first binary"),
            b"first"
        );
        assert_eq!(
            fs::read(&second_binary).expect("read second binary"),
            b"second"
        );
        assert_eq!(
            fs::read(&first_binary).expect("reread first binary"),
            b"first"
        );
        assert!(
            first_binary
                .file_name()
                .expect("first filename")
                .to_string_lossy()
                .contains(&frontend_binary_cache_key(
                    std::env::consts::OS,
                    std::env::consts::ARCH,
                    "go1.25.0",
                    execution_host_go_target().expect("host target").0,
                    execution_host_go_target().expect("host target").1,
                ))
        );
    }

    #[test]
    fn frontend_binary_cache_key_covers_execution_and_go_context() {
        let baseline = frontend_binary_cache_key("linux", "x86_64", "go1.25.0", "linux", "amd64");
        for changed in [
            frontend_binary_cache_key("darwin", "x86_64", "go1.25.0", "linux", "amd64"),
            frontend_binary_cache_key("linux", "aarch64", "go1.25.0", "linux", "amd64"),
            frontend_binary_cache_key("linux", "x86_64", "go1.26.0", "linux", "amd64"),
            frontend_binary_cache_key("linux", "x86_64", "go1.25.0", "darwin", "amd64"),
            frontend_binary_cache_key("linux", "x86_64", "go1.25.0", "linux", "arm64"),
        ] {
            assert_ne!(baseline, changed);
        }
    }

    #[test]
    fn go_toolchain_version_parser_accepts_go_version_output() {
        assert_eq!(
            parse_go_version("go version go1.26.2 darwin/arm64"),
            Some("go1.26.2".to_string())
        );
        assert!(go_version_at_least("go1.25.0", 1, 25));
        assert!(!go_version_at_least("go1.24.9", 1, 25));
        assert_eq!(
            parse_go_target("darwin\narm64\n"),
            Some(("darwin".to_string(), "arm64".to_string()))
        );
        assert_eq!(parse_go_target("darwin\n"), None);
    }
}
