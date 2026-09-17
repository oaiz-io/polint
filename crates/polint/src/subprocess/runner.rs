//! Wall-clock-bounded subprocess execution with process-tree cleanup.
//!
//! A sidecar is an external toolchain: it can hang, it can fork descendants
//! that inherit the output pipes, and on Windows it can be reparented away
//! from its spawner. This runner bounds the whole tree rather than the direct
//! child, and drains output through cancellable readers so a surviving
//! descendant cannot extend the cleanup deadline.

use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

const PROCESS_CLEANUP_GRACE: Duration = Duration::from_secs(2);
#[cfg(windows)]
const TREE_KILL_COMMAND_GRACE: Duration = Duration::from_secs(1);
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Debug)]
pub(crate) struct SubprocessOutput {
    pub(crate) status: ExitStatus,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

#[derive(Debug)]
pub(crate) enum SubprocessError {
    Unavailable(String),
    Failed(String),
    Timeout(String),
}

impl SubprocessError {
    pub(crate) fn reason(&self) -> &str {
        match self {
            Self::Unavailable(reason) | Self::Failed(reason) | Self::Timeout(reason) => reason,
        }
    }
}

/// Runs `command` to completion or to `timeout`, whichever comes first.
///
/// `label` names the program in every message. `timeout_code` is the caller's
/// category for an exceeded budget: a timeout is reported with the code of the
/// toolchain that timed out, so a Go budget and a Node budget never share a
/// diagnostic category.
pub(crate) fn run_bounded(
    mut command: Command,
    timeout: Duration,
    label: &str,
    timeout_code: &str,
) -> Result<SubprocessOutput, SubprocessError> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::jobs::apply_to_command(&mut command);
    configure_child_process_group(&mut command);
    let mut process_tree = ProcessTreeGuard::before_spawn();
    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            SubprocessError::Unavailable(format!("{label} executable was not found."))
        } else {
            SubprocessError::Failed(format!("failed to start {label}: {error}"))
        }
    })?;
    process_tree.after_spawn(&child);
    let cancel_readers = Arc::new(AtomicBool::new(false));
    let mut stdout_reader = child.stdout.take().map(|stdout| {
        let cancel_reader = Arc::clone(&cancel_readers);
        thread::spawn(move || read_all(stdout, &cancel_reader))
    });
    let mut stderr_reader = child.stderr.take().map(|stderr| {
        let cancel_reader = Arc::clone(&cancel_readers);
        thread::spawn(move || read_all(stderr, &cancel_reader))
    });

    let deadline = Instant::now() + timeout;
    loop {
        let poll_result = child.try_wait();
        let now = Instant::now();
        match poll_result {
            Ok(Some(status)) if now < deadline => {
                let cleanup_deadline = (now + PROCESS_CLEANUP_GRACE).min(deadline);
                terminate_child_process_tree(&mut child, cleanup_deadline, &mut process_tree);
                cancel_readers.store(true, Ordering::Release);
                let _ = wait_for_exit_until(&mut child, cleanup_deadline);
                let stdout = join_reader_until(stdout_reader.take(), label, cleanup_deadline);
                let stderr = join_reader_until(stderr_reader.take(), label, cleanup_deadline);
                return Ok(SubprocessOutput {
                    status,
                    stdout: stdout?,
                    stderr: stderr?,
                });
            }
            Ok(Some(_)) => {
                cleanup_process(
                    &mut child,
                    stdout_reader.take(),
                    stderr_reader.take(),
                    &cancel_readers,
                    &mut process_tree,
                );
                return Err(SubprocessError::Timeout(format!(
                    "{timeout_code}: {label} exceeded its {} ms timeout.",
                    timeout.as_millis()
                )));
            }
            Ok(None) if now < deadline => sleep_until_next_poll(deadline),
            Ok(None) => {
                cleanup_process(
                    &mut child,
                    stdout_reader.take(),
                    stderr_reader.take(),
                    &cancel_readers,
                    &mut process_tree,
                );
                return Err(SubprocessError::Timeout(format!(
                    "{timeout_code}: {label} exceeded its {} ms timeout.",
                    timeout.as_millis()
                )));
            }
            Err(error) => {
                cleanup_process(
                    &mut child,
                    stdout_reader.take(),
                    stderr_reader.take(),
                    &cancel_readers,
                    &mut process_tree,
                );
                return Err(SubprocessError::Failed(format!(
                    "failed to poll {label}: {error}"
                )));
            }
        }
    }
}

fn cleanup_process(
    child: &mut Child,
    stdout_reader: Option<thread::JoinHandle<std::io::Result<Vec<u8>>>>,
    stderr_reader: Option<thread::JoinHandle<std::io::Result<Vec<u8>>>>,
    cancel_readers: &AtomicBool,
    process_tree: &mut ProcessTreeGuard,
) {
    let cleanup_deadline = Instant::now() + PROCESS_CLEANUP_GRACE;
    terminate_child_process_tree(child, cleanup_deadline, process_tree);
    cancel_readers.store(true, Ordering::Release);
    let _ = wait_for_exit_until(child, cleanup_deadline);

    // Descendants can keep inherited pipe handles open even after the direct
    // child dies. Never let output-reader joins extend the cleanup deadline.
    discard_reader_until(stdout_reader, cleanup_deadline);
    discard_reader_until(stderr_reader, cleanup_deadline);
}

fn wait_for_exit_until(child: &mut Child, deadline: Instant) -> std::io::Result<bool> {
    loop {
        if child.try_wait()?.is_some() {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        sleep_until_next_poll(deadline);
    }
}

fn sleep_until_next_poll(deadline: Instant) {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if !remaining.is_zero() {
        thread::sleep(remaining.min(PROCESS_POLL_INTERVAL));
    }
}

fn discard_reader_until(
    reader: Option<thread::JoinHandle<std::io::Result<Vec<u8>>>>,
    deadline: Instant,
) {
    let Some(reader) = reader else {
        return;
    };
    while !reader.is_finished() && Instant::now() < deadline {
        sleep_until_next_poll(deadline);
    }
    if reader.is_finished() {
        let _ = reader.join();
    }
}

#[cfg(unix)]
fn read_all(
    mut reader: impl Read + std::os::fd::AsRawFd,
    cancel_reader: &AtomicBool,
) -> std::io::Result<Vec<u8>> {
    set_nonblocking(&reader)?;
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut cancellation_observed = None;
    loop {
        if cancel_reader.load(Ordering::Acquire) {
            let observed_at = cancellation_observed.get_or_insert_with(Instant::now);
            if observed_at.elapsed() >= PROCESS_POLL_INTERVAL {
                return Ok(bytes);
            }
        }
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(bytes),
            Ok(read) => bytes.extend_from_slice(&buffer[..read]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if cancellation_observed.is_some() {
                    return Ok(bytes);
                }
                thread::sleep(PROCESS_POLL_INTERVAL);
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(unix)]
fn set_nonblocking(reader: &impl std::os::fd::AsRawFd) -> std::io::Result<()> {
    let fd = reader.as_raw_fd();
    #[allow(unsafe_code)]
    // SAFETY: `fd` belongs to the borrowed live pipe. F_GETFL reads its flags,
    // and F_SETFL changes only that descriptor's nonblocking status.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags == -1 {
            return Err(std::io::Error::last_os_error());
        }
        if libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) == -1 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn read_all(mut reader: impl Read, _cancel_reader: &AtomicBool) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn join_reader_until(
    reader: Option<thread::JoinHandle<std::io::Result<Vec<u8>>>>,
    label: &str,
    deadline: Instant,
) -> Result<Vec<u8>, SubprocessError> {
    let Some(reader) = reader else {
        return Ok(Vec::new());
    };
    while !reader.is_finished() && Instant::now() < deadline {
        sleep_until_next_poll(deadline);
    }
    if !reader.is_finished() {
        return Err(SubprocessError::Failed(format!(
            "timed out draining {label} output after its process exited"
        )));
    }
    reader
        .join()
        .map_err(|_| SubprocessError::Failed(format!("failed to join {label} output reader")))?
        .map_err(|error| SubprocessError::Failed(format!("failed to read {label} output: {error}")))
}

#[cfg(windows)]
struct ProcessTreeGuard {
    job: Option<std::os::windows::io::OwnedHandle>,
}

#[cfg(windows)]
impl ProcessTreeGuard {
    fn before_spawn() -> Self {
        match create_kill_on_close_job() {
            Ok(job) => Self { job: Some(job) },
            Err(error) => {
                tracing::debug!(
                    %error,
                    "Windows Job Object setup failed; process-tree cleanup will use taskkill"
                );
                Self { job: None }
            }
        }
    }

    fn after_spawn(&mut self, child: &Child) {
        let assignment = self
            .job
            .as_ref()
            .map(|job| assign_process_to_job(job, child));
        if let Some(Err(error)) = assignment {
            self.job.take();
            tracing::debug!(
                process_id = child.id(),
                %error,
                "Windows Job Object assignment failed; process-tree cleanup will use taskkill"
            );
        }
        if let Err(error) = resume_suspended_process(child) {
            // Dropping an assigned job terminates the still-suspended child;
            // without assignment the ordinary cleanup path can kill it by PID.
            self.job.take();
            tracing::debug!(
                process_id = child.id(),
                %error,
                "failed to resume suspended Windows child"
            );
        }
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn create_kill_on_close_job() -> std::io::Result<std::os::windows::io::OwnedHandle> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::JobObjects::{
        CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };

    // SAFETY: the unnamed job handle is checked before ownership transfers to
    // OwnedHandle. The configuration call receives the live handle and a
    // correctly sized JOBOBJECT_EXTENDED_LIMIT_INFORMATION value.
    unsafe {
        let raw_job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if raw_job.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let job = OwnedHandle::from_raw_handle(raw_job);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let limits_size = u32::try_from(std::mem::size_of_val(&limits)).map_err(|_| {
            std::io::Error::other("Windows Job Object limit information is too large")
        })?;
        if SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            std::ptr::from_ref(&limits).cast(),
            limits_size,
        ) == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(job)
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn assign_process_to_job(
    job: &std::os::windows::io::OwnedHandle,
    child: &Child,
) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;

    // SAFETY: both handles are owned by live Rust values for the duration of
    // this call, and the process handle grants assignment access after spawn.
    unsafe {
        if AssignProcessToJobObject(job.as_raw_handle(), child.as_raw_handle()) == 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn resume_suspended_process(child: &Child) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;

    unsafe extern "system" {
        fn NtResumeProcess(process_handle: std::os::windows::io::RawHandle) -> i32;
    }

    // SAFETY: the process handle is owned by the live Child. NtResumeProcess
    // resumes the primary thread created with CREATE_SUSPENDED without a
    // pathname lookup or a race-prone system-wide thread enumeration.
    let status = unsafe { NtResumeProcess(child.as_raw_handle()) };
    if status >= 0 {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "NtResumeProcess failed with NTSTATUS {status:#010x}"
        )))
    }
}

#[cfg(not(windows))]
struct ProcessTreeGuard;

#[cfg(not(windows))]
impl ProcessTreeGuard {
    fn before_spawn() -> Self {
        Self
    }

    fn after_spawn(&mut self, _child: &Child) {}
}

#[cfg(unix)]
fn configure_child_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    // A fresh group contains ordinary forked descendants. A descendant that
    // deliberately creates a new session can escape any portable process-group
    // cleanup, so cancellable nonblocking readers also bound inherited pipes.
    command.process_group(0);
}

#[cfg(windows)]
fn configure_child_process_group(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::CREATE_SUSPENDED;

    command.creation_flags(CREATE_SUSPENDED);
}

#[cfg(not(any(unix, windows)))]
fn configure_child_process_group(_command: &mut Command) {}

#[cfg(unix)]
fn terminate_child_process_tree(
    child: &mut Child,
    _deadline: Instant,
    _process_tree: &mut ProcessTreeGuard,
) {
    #[allow(unsafe_code)]
    // SAFETY: the child PID is supplied by std::process and negating it targets
    // only the fresh process group configured immediately before spawning.
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
}

#[cfg(windows)]
fn terminate_child_process_tree(
    child: &mut Child,
    cleanup_deadline: Instant,
    process_tree: &mut ProcessTreeGuard,
) {
    // Once assignment succeeds, this handle remains live until every runner
    // exit path reaches cleanup. Closing it kills descendants even after the
    // direct child has exited and Windows has reparented them.
    if let Some(job) = process_tree.job.take() {
        drop(job);
        return;
    }

    let taskkill_deadline = (Instant::now() + TREE_KILL_COMMAND_GRACE).min(cleanup_deadline);
    let mut taskkill = Command::new("taskkill");
    taskkill
        .args(["/F", "/T", "/PID", &child.id().to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Ok(taskkill) = taskkill.spawn() {
        terminate_cleanup_command(taskkill, taskkill_deadline);
    }
    let _ = child.kill();
}

#[cfg(windows)]
fn terminate_cleanup_command(mut cleanup: Child, deadline: Instant) {
    if !wait_for_exit_until(&mut cleanup, deadline).unwrap_or(false) {
        let _ = cleanup.kill();
    }
}

#[cfg(all(not(unix), not(windows)))]
fn terminate_child_process_tree(
    child: &mut Child,
    _deadline: Instant,
    _process_tree: &mut ProcessTreeGuard,
) {
    let _ = child.kill();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_for_exit_until_returns_at_its_deadline() {
        #[cfg(windows)]
        let mut command = {
            let mut command = Command::new("cmd");
            command.args(["/C", "ping -n 6 127.0.0.1 > nul"]);
            command
        };
        #[cfg(not(windows))]
        let mut command = {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 5"]);
            command
        };

        let mut child = command.spawn().expect("spawn sleeping process");
        let started = Instant::now();
        let exited = wait_for_exit_until(&mut child, started + Duration::from_millis(30))
            .expect("poll sleeping process");

        assert!(!exited);
        assert!(started.elapsed() < Duration::from_millis(500));
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn timeout_returns_within_cleanup_grace_and_terminates_descendants() {
        let temp = tempfile::tempdir().expect("tempdir");
        let sentinel = temp.path().join("descendant-finished");
        let command = if cfg!(windows) {
            let mut command = Command::new("cmd");
            command.args([
                "/C",
                &format!(
                    r#"start /B cmd /C "ping -n 3 127.0.0.1 > nul & echo done > {}" & ping -n 6 127.0.0.1 > nul"#,
                    sentinel.display()
                ),
            ]);
            command
        } else {
            let mut command = Command::new("sh");
            command.args([
                "-c",
                &format!(
                    "(sleep 1; echo done > '{}') & i=0; while [ $i -lt 2000 ]; do echo output; echo error >&2; i=$((i+1)); done; sleep 5",
                    sentinel.display()
                ),
            ]);
            command
        };

        let timeout = Duration::from_millis(50);
        let started = Instant::now();
        let error = run_bounded(
            command,
            timeout,
            "sleeping test program",
            "TestSubprocessTimeout",
        )
        .expect_err("command should time out");
        assert!(matches!(error, SubprocessError::Timeout(_)));
        assert!(started.elapsed() < timeout + PROCESS_CLEANUP_GRACE + Duration::from_secs(1));
        thread::sleep(Duration::from_millis(1_200));
        assert!(!sentinel.exists(), "timed-out descendant survived");
    }

    #[cfg(unix)]
    #[test]
    fn successful_parent_exit_terminates_child_inheriting_output_pipe() {
        assert_successful_parent_exit_terminates_child(false);
    }

    #[cfg(unix)]
    #[test]
    fn successful_parent_exit_terminates_child_with_redirected_output() {
        assert_successful_parent_exit_terminates_child(true);
    }

    #[cfg(unix)]
    fn assert_successful_parent_exit_terminates_child(redirect_output: bool) {
        let temp = tempfile::tempdir().expect("tempdir");
        let sentinel = temp.path().join("background-child-finished");
        let redirection = if redirect_output {
            ">/dev/null 2>&1"
        } else {
            ""
        };
        let mut command = Command::new("sh");
        command.args([
            "-c",
            &format!(
                "printf 'parent output\n'; (sleep 0.5; printf done > '{}') {redirection} & exit 0",
                sentinel.display()
            ),
        ]);

        let started = Instant::now();
        let output = run_bounded(
            command,
            Duration::from_secs(5),
            "early-exit test program",
            "TestSubprocessTimeout",
        )
        .expect("parent should exit successfully");

        assert!(output.status.success());
        assert_eq!(output.stdout, b"parent output\n");
        assert!(started.elapsed() < Duration::from_millis(400));
        thread::sleep(Duration::from_millis(700));
        assert!(!sentinel.exists(), "background child survived parent exit");
    }

    #[cfg(windows)]
    #[test]
    fn successful_parent_exit_terminates_reparented_descendant() {
        let temp = tempfile::tempdir().expect("tempdir");
        let sentinel = temp.path().join("background-child-finished");
        let mut command = Command::new("powershell");
        command.current_dir(temp.path()).args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            r#"Start-Process -FilePath cmd.exe -ArgumentList '/C', 'ping -n 3 127.0.0.1 > nul & echo done > background-child-finished'; Write-Output parent"#,
        ]);

        // The descendant needs its full ping before it writes the sentinel, and
        // it only starts once PowerShell reaches `Start-Process`. Checking the
        // sentinel the moment the call returns therefore proves the run did not
        // block on the descendant, without pinning that proof to a wall-clock
        // bound: PowerShell startup on a shared runner varies by seconds, and a
        // slow start says nothing about whether the descendant was waited on.
        let output = run_bounded(
            command,
            Duration::from_secs(60),
            "early-exit test program",
            "TestSubprocessTimeout",
        )
        .expect("parent should exit successfully");

        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("parent"));
        assert!(
            !sentinel.exists(),
            "run returned only after the descendant finished"
        );
        thread::sleep(Duration::from_millis(2_300));
        assert!(!sentinel.exists(), "background child survived parent exit");
    }
}
