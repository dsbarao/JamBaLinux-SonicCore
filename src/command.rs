//! Timeout-bounded execution of the short-lived external audio clients.
//!
//! `pactl`, `pw-dump`, `pw-cli`, and especially `pw-link --wait` can block for
//! an unbounded time when the session graph is wedged. The equalizer and
//! spatial routing paths call them while holding the route lock, so a single
//! unbounded wait freezes every other control operation and the supervisor
//! loops behind it. Every short-lived client therefore runs through
//! [`run_default`]: it drains `stdout` and `stderr` on dedicated threads so a
//! full pipe buffer can never deadlock the child, enforces a per-program
//! budget, and kills *and reaps* the child when that budget expires.
//!
//! The long-running `pipewire` filter-chain hosts are deliberately not routed
//! through this module. They are supervised as child processes for the lifetime
//! of the graph and have no output to collect.
//!
//! This module never opens a HID device, and it neither knows nor inspects the
//! arguments it forwards: the allowlisted command vocabulary stays in the
//! callers.

use std::cell::RefCell;
use std::io::Read;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

/// Budget for the short-lived PipeWire/PulseAudio clients, including
/// `pw-link --wait`. A healthy session answers all of them in a few
/// milliseconds; five seconds is long enough to absorb a loaded graph and
/// short enough that the route lock is never held for a human-visible pause.
pub const AUDIO_CLIENT_TIMEOUT: Duration = Duration::from_secs(5);

/// Budget for `systemctl --user`. Enabling or disabling a unit queues a job
/// whose completion depends on the unit itself, so it gets a wider budget than
/// a graph query while still being bounded.
pub const SERVICE_TIMEOUT: Duration = Duration::from_secs(15);

/// Budget for any other program routed through this module.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// Polling bounds for the exit check. The equalizer sends four `pw-cli` updates
/// in a row, so the detection delay is deliberately kept near a millisecond:
/// waking up to a thousand times a second costs one `waitpid` each, which is
/// immaterial next to the client it is waiting for.
const POLL_MIN: Duration = Duration::from_micros(200);
const POLL_MAX: Duration = Duration::from_millis(1);

/// Upper bound on collecting the output of a child that has already exited.
/// It is reached only when a grandchild inherited the pipes, so it never
/// extends a healthy call.
const COLLECT_GRACE: Duration = Duration::from_millis(500);

/// Injection point for tests: an alternative execution backend for the calling
/// thread. Keeping it thread-local means concurrently running tests cannot
/// observe each other's scripted commands, while the code under test — which
/// runs the routing sequence on the calling thread — sees the substitute.
pub trait CommandRunner: Send + Sync {
    fn run(&self, program: &str, args: &[&str], timeout: Duration) -> Result<Output, String>;
}

thread_local! {
    static RUNNER: RefCell<Option<Arc<dyn CommandRunner>>> = const { RefCell::new(None) };
}

/// Clones the override out before it is used so a runner may itself call back
/// into this module without re-entrant borrow panics.
fn current_runner() -> Option<Arc<dyn CommandRunner>> {
    RUNNER
        .try_with(|slot| slot.borrow().clone())
        .unwrap_or(None)
}

/// Per-program budget. Matching on the file name keeps an absolute path and a
/// `PATH` lookup of the same client on the same budget.
pub fn timeout_for(program: &str) -> Duration {
    match program.rsplit('/').next().unwrap_or(program) {
        "pactl" | "pw-dump" | "pw-cli" | "pw-link" | "pw-metadata" => AUDIO_CLIENT_TIMEOUT,
        "systemctl" => SERVICE_TIMEOUT,
        _ => DEFAULT_TIMEOUT,
    }
}

/// Runs `program` with the budget [`timeout_for`] assigns to it.
pub fn run_default(program: &str, args: &[&str]) -> Result<Output, String> {
    run(program, args, timeout_for(program))
}

/// Runs `program` with an explicit budget, honouring a thread-local
/// [`CommandRunner`] override when one is installed.
pub fn run(program: &str, args: &[&str], timeout: Duration) -> Result<Output, String> {
    if let Some(runner) = current_runner() {
        return runner.run(program, args, timeout);
    }
    run_process(program, args, timeout)
}

/// Shared rendering of a non-zero exit. `program` is the label the caller wants
/// in the message, which may include the distinguishing subcommand.
pub fn failure(program: &str, output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if stderr.is_empty() {
        format!("{program} exited with {}", output.status)
    } else {
        format!("{program} failed: {stderr}")
    }
}

fn run_process(program: &str, args: &[&str], timeout: Duration) -> Result<Output, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not start {program}: {error}"))?;
    // Both pipes are drained concurrently with the exit check. Without this a
    // client writing more than one pipe buffer (a large `pw-dump`) would block
    // in `write` while this thread blocked waiting for it to exit.
    let stdout = child.stdout.take().map(drain);
    let stderr = child.stderr.take().map(drain);

    let exited =
        wait_bounded(&mut child, timeout).map_err(|error| format!("{program}: {error}"))?;
    match exited {
        Some(status) => {
            // The child has exited, so both pipes are normally at EOF already.
            // Collection is still bounded: a grandchild that inherited them
            // could hold them open, and no caller may be parked on that.
            Ok(Output {
                status,
                stdout: collect(stdout),
                stderr: collect(stderr),
            })
        }
        None => {
            // `kill` may race with an exit that happened after the deadline, so
            // its error is not fatal; the `wait` below is what prevents a
            // zombie either way.
            let _ = child.kill();
            let _reaped = child.wait().map_err(|error| {
                format!("could not reap {program} after its {timeout:?} timeout: {error}")
            })?;
            // Dropping the receivers detaches the reader threads, which end when
            // the pipes close. Nothing here waits for them: the caller needs the
            // error now, not the output of a command that never finished.
            drop(stdout);
            drop(stderr);
            Err(format!(
                "{program} did not finish within {} ms and was killed",
                timeout.as_millis()
            ))
        }
    }
}

fn drain<R: Read + Send + 'static>(mut source: R) -> Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    // The handle is dropped, which detaches the reader: it ends on its own when
    // the pipe closes, and no caller ever blocks joining it.
    let _reader = thread::spawn(move || {
        let mut buffer = Vec::new();
        // A read error keeps whatever was already collected: the exit status and
        // stderr are what the callers report on, and truncated output is
        // rejected by their JSON parsing.
        let _ = source.read_to_end(&mut buffer);
        let _ = sender.send(buffer);
    });
    receiver
}

/// Takes one drained stream. The child has already exited, so the bytes are
/// normally waiting in the channel; [`COLLECT_GRACE`] only bounds the pathological
/// case of an inherited pipe, and losing output there is preferable to hanging.
fn collect(stream: Option<Receiver<Vec<u8>>>) -> Vec<u8> {
    stream
        .and_then(|receiver| receiver.recv_timeout(COLLECT_GRACE).ok())
        .unwrap_or_default()
}

/// Waits for `child` for at most `timeout`. `Ok(None)` means the budget
/// expired while the child was still running.
fn wait_bounded(child: &mut Child, timeout: Duration) -> Result<Option<ExitStatus>, String> {
    let deadline = Instant::now() + timeout;
    let mut interval = POLL_MIN;
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("could not wait for the child process: {error}"))?
        {
            return Ok(Some(status));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(None);
        }
        thread::sleep(interval.min(remaining));
        interval = (interval * 2).min(POLL_MAX);
    }
}

#[cfg(test)]
pub use testing::{ScriptedRunner, set_runner};

#[cfg(test)]
mod testing {
    //! Substitute backend for unit tests of command sequences. It lets a test
    //! drive the routing and supervisor paths without a PipeWire session.

    use super::{CommandRunner, RUNNER};
    use std::collections::VecDeque;
    use std::os::unix::process::ExitStatusExt as _;
    use std::process::Output;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// One recorded call, in the form assertions want to read it.
    #[derive(Debug, Clone, PartialEq)]
    pub struct Invocation {
        pub program: String,
        pub args: Vec<String>,
        pub timeout: Duration,
    }

    impl Invocation {
        /// `"pactl --format=json list sinks"`: a single string is far easier to
        /// assert a whole sequence against.
        pub fn command_line(&self) -> String {
            let mut line = self.program.clone();
            for argument in &self.args {
                line.push(' ');
                line.push_str(argument);
            }
            line
        }
    }

    /// Replays queued responses in order and records every call. An
    /// unscripted extra call is an error rather than a silent success, so a
    /// test cannot pass while the code issues commands it did not expect.
    #[derive(Debug, Default)]
    pub struct ScriptedRunner {
        responses: Mutex<VecDeque<Result<Output, String>>>,
        calls: Mutex<Vec<Invocation>>,
    }

    impl ScriptedRunner {
        pub fn new() -> Self {
            Self::default()
        }

        /// Queues a successful run with `stdout`.
        pub fn push_output(&self, stdout: &str) -> &Self {
            self.push(Ok(Output {
                status: std::process::ExitStatus::from_raw(0),
                stdout: stdout.as_bytes().to_vec(),
                stderr: Vec::new(),
            }))
        }

        /// Queues a run that exited with `code` and wrote `stderr`.
        pub fn push_failure(&self, code: i32, stderr: &str) -> &Self {
            self.push(Ok(Output {
                // `from_raw` takes the raw wait status, where the exit code
                // lives in the second byte.
                status: std::process::ExitStatus::from_raw(code << 8),
                stdout: Vec::new(),
                stderr: stderr.as_bytes().to_vec(),
            }))
        }

        /// Queues a run that could not be executed at all, such as a timeout.
        pub fn push_error(&self, error: &str) -> &Self {
            self.push(Err(error.to_owned()))
        }

        fn push(&self, response: Result<Output, String>) -> &Self {
            self.responses
                .lock()
                .expect("scripted responses")
                .push_back(response);
            self
        }

        pub fn calls(&self) -> Vec<Invocation> {
            self.calls.lock().expect("recorded calls").clone()
        }

        pub fn command_lines(&self) -> Vec<String> {
            self.calls()
                .iter()
                .map(Invocation::command_line)
                .collect::<Vec<_>>()
        }
    }

    impl CommandRunner for ScriptedRunner {
        fn run(&self, program: &str, args: &[&str], timeout: Duration) -> Result<Output, String> {
            self.calls.lock().expect("recorded calls").push(Invocation {
                program: program.to_owned(),
                args: args.iter().map(|argument| (*argument).to_owned()).collect(),
                timeout,
            });
            self.responses
                .lock()
                .expect("scripted responses")
                .pop_front()
                .unwrap_or_else(|| Err(format!("unscripted command: {program} {args:?}")))
        }
    }

    /// Installs `runner` for the calling thread until the returned guard is
    /// dropped, restoring whatever was installed before.
    pub fn set_runner(runner: Arc<dyn CommandRunner>) -> RunnerGuard {
        let previous = RUNNER.with(|slot| slot.borrow_mut().replace(runner));
        RunnerGuard { previous }
    }

    pub struct RunnerGuard {
        previous: Option<Arc<dyn CommandRunner>>,
    }

    impl Drop for RunnerGuard {
        fn drop(&mut self) {
            let previous = self.previous.take();
            // `try_with` keeps a guard dropped during thread teardown from
            // turning a failing test into an abort.
            let _ = RUNNER.try_with(|slot| *slot.borrow_mut() = previous);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn sh(script: &str) -> Result<Output, String> {
        run("/bin/sh", &["-c", script], Duration::from_secs(20))
    }

    /// Direct children of this process that are still unreaped.
    fn zombie_children() -> Vec<String> {
        let me = std::process::id().to_string();
        let mut zombies = Vec::new();
        let Ok(entries) = fs::read_dir("/proc") else {
            return zombies;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.chars().all(|character| character.is_ascii_digit()) {
                continue;
            }
            let Ok(stat) = fs::read_to_string(entry.path().join("stat")) else {
                continue;
            };
            // The command name is parenthesised and may contain spaces, so the
            // state and parent pid are read after the final ')'.
            let Some((_, rest)) = stat.rsplit_once(')') else {
                continue;
            };
            let mut fields = rest.split_whitespace();
            let state = fields.next().unwrap_or_default();
            let parent = fields.next().unwrap_or_default();
            if state == "Z" && parent == me {
                zombies.push(name);
            }
        }
        zombies
    }

    /// A leaked zombie survives until this process exits, while a child another
    /// test is about to reap disappears within one polling interval. Waiting for
    /// the set to empty therefore catches a real leak without flaking on tests
    /// running in parallel.
    fn assert_no_leaked_zombie_children() {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let zombies = zombie_children();
            if zombies.is_empty() {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "unreaped child processes remained: {zombies:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn large_concurrent_output_on_both_pipes_does_not_deadlock() {
        // Each stream is far beyond a single pipe buffer and both are written
        // at the same time, which is exactly the shape that deadlocks a
        // sequential read-then-wait implementation.
        let output = sh("{ yes 0123456789abcdef | head -c 200000; } & \
             { yes 0123456789abcdef | head -c 200000 >&2; } & \
             wait; exit 0")
        .expect("the shell ran");
        assert!(output.status.success());
        assert_eq!(output.stdout.len(), 200_000);
        assert_eq!(output.stderr.len(), 200_000);
        assert!(output.stdout.len() > 128 * 1024);
        assert!(output.stderr.len() > 128 * 1024);
    }

    #[test]
    fn a_non_zero_exit_is_preserved_with_both_streams() {
        let output = sh("printf out; printf err >&2; exit 7").expect("the shell ran");
        assert!(!output.status.success());
        assert_eq!(output.status.code(), Some(7));
        assert_eq!(output.stdout, b"out");
        assert_eq!(output.stderr, b"err");
        assert_eq!(
            failure("test-client", &output),
            "test-client failed: err".to_owned()
        );
    }

    /// Loose ceiling, not a benchmark: it catches a per-call fixed sleep being
    /// reintroduced into the wait, which the equalizer would pay four times for
    /// every band change.
    #[test]
    fn a_fast_client_is_not_delayed_by_the_wait_itself() {
        let started = Instant::now();
        for _ in 0..4 {
            sh("exit 0").expect("the shell ran");
        }
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_millis(250),
            "four trivial runs took {elapsed:?}"
        );
    }

    #[test]
    fn an_empty_stderr_falls_back_to_the_exit_status() {
        let output = sh("exit 3").expect("the shell ran");
        let message = failure("pactl get-default-sink", &output);
        assert!(
            message.starts_with("pactl get-default-sink exited with"),
            "unexpected message: {message}"
        );
    }

    #[test]
    fn a_timeout_kills_and_reaps_the_child_and_names_the_limit() {
        let limit = Duration::from_millis(300);
        let started = Instant::now();
        let error =
            run("/bin/sh", &["-c", "sleep 5"], limit).expect_err("a sleeping client must time out");
        let elapsed = started.elapsed();
        assert!(
            elapsed < limit * 2,
            "the timeout took {elapsed:?}, more than twice the {limit:?} limit"
        );
        assert!(error.contains("/bin/sh"), "unexpected error: {error}");
        assert!(error.contains("300 ms"), "unexpected error: {error}");
        assert_no_leaked_zombie_children();
    }

    /// A grandchild can outlive the killed child while still holding the pipes.
    /// The timeout path must not wait for the readers in that case.
    #[test]
    fn a_timeout_after_a_full_pipe_buffer_still_returns_within_the_budget() {
        let limit = Duration::from_millis(400);
        let started = Instant::now();
        let error = run(
            "/bin/sh",
            &["-c", "yes 0123456789abcdef | head -c 200000; sleep 5"],
            limit,
        )
        .expect_err("a stalled client must time out");
        let elapsed = started.elapsed();
        assert!(
            elapsed < limit * 2,
            "the timeout took {elapsed:?}, more than twice the {limit:?} limit"
        );
        assert!(error.contains("400 ms"), "unexpected error: {error}");
        assert_no_leaked_zombie_children();
    }

    #[test]
    fn a_missing_program_reports_that_it_could_not_start() {
        let error = run(
            "jambalinux-soniccore-absent-client",
            &[],
            Duration::from_secs(1),
        )
        .expect_err("a missing program cannot run");
        assert!(
            error.starts_with("could not start jambalinux-soniccore-absent-client"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn every_audio_client_and_service_call_has_a_bounded_budget() {
        for program in ["pactl", "pw-dump", "pw-cli", "pw-link", "/usr/bin/pactl"] {
            assert_eq!(timeout_for(program), AUDIO_CLIENT_TIMEOUT);
        }
        assert_eq!(timeout_for("systemctl"), SERVICE_TIMEOUT);
        assert_eq!(timeout_for("something-else"), DEFAULT_TIMEOUT);
    }

    #[test]
    fn an_injected_runner_replaces_execution_and_records_the_budget() {
        let runner = Arc::new(ScriptedRunner::new());
        runner.push_output("[]").push_failure(1, "no such sink");
        let guard = set_runner(runner.clone());

        let first = run_default("pactl", &["--format=json", "list", "sinks"]).expect("scripted");
        assert_eq!(first.stdout, b"[]");
        let second = run_default("pw-link", &["--wait", "a", "b"]).expect("scripted");
        assert_eq!(second.status.code(), Some(1));
        assert_eq!(failure("pw-link", &second), "pw-link failed: no such sink");
        // An unscripted third call must fail loudly instead of looking healthy.
        assert!(run_default("pw-dump", &[]).is_err());

        drop(guard);
        assert_eq!(
            runner.command_lines(),
            vec![
                "pactl --format=json list sinks".to_owned(),
                "pw-link --wait a b".to_owned(),
                "pw-dump".to_owned(),
            ]
        );
        assert!(
            runner
                .calls()
                .iter()
                .all(|call| call.timeout == AUDIO_CLIENT_TIMEOUT)
        );
    }

    #[test]
    fn a_scripted_failure_to_run_reaches_the_caller_unchanged() {
        let runner = Arc::new(ScriptedRunner::new());
        runner.push_error("pw-link did not finish within 5000 ms and was killed");
        let _guard = set_runner(runner.clone());
        let error = run_default("pw-link", &["--wait", "a", "b"])
            .expect_err("a scripted timeout is an error");
        assert_eq!(
            error,
            "pw-link did not finish within 5000 ms and was killed"
        );
        assert_eq!(
            runner.command_lines(),
            vec!["pw-link --wait a b".to_owned()]
        );
    }

    #[test]
    fn dropping_the_guard_restores_real_execution() {
        let runner = Arc::new(ScriptedRunner::new());
        runner.push_output("scripted");
        {
            let _guard = set_runner(runner.clone());
            let output = run_default("pactl", &["info"]).expect("scripted");
            assert_eq!(output.stdout, b"scripted");
        }
        let output = sh("printf real").expect("the shell ran");
        assert_eq!(output.stdout, b"real");
        assert_eq!(runner.calls().len(), 1);
    }
}
