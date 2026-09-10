//! Process orchestration for the driver (`DESIGN-http-outpost-conformance-harness.md` §3.2).
//!
//! The driver runs each SUT as a REAL child process (nothing internal mocked — operator directive: "a stock
//! gateway tested end-to-end"). This module is the reusable primitive underneath that: [`ServerProcess`]
//! spawns a bin, waits for its "ready" line on stderr — each SUT bin binds an ephemeral `:0` port and prints
//! the REAL bound address to stderr (`cdz-cas-http: listening on <addr> …`, `cdz-http-control-mock: admin=…
//! control=… …`, `gateway: listen=… control=…`) so the driver never guesses a port — and KILLS the child on
//! drop. The per-server wiring (which bin, which args, which ready-line parser) sits on top in a later slice;
//! this is the spawn/ready/teardown mechanism all three share.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

/// A spawned SUT child, killed when this guard drops (`kill_on_drop`). Spawn it, [`wait_for_ready`] on its
/// stderr to learn the bound address, then drive it; dropping the guard tears the process down so a scenario
/// never leaks a server. stderr is piped (read by `wait_for_ready`, then drained); stdout/stdin are null.
///
/// [`wait_for_ready`]: ServerProcess::wait_for_ready
#[derive(Debug)]
pub struct ServerProcess {
    child: Child,
}

impl ServerProcess {
    /// Spawn `program` with `args`, killed on drop. Errs (a `String`) if the process cannot be spawned (a
    /// missing bin / exec failure) — surfaced so the driver reports it as a scenario setup error.
    ///
    /// # Errors
    /// The program cannot be spawned (not found, not executable, …).
    pub fn spawn(program: &Path, args: &[&str]) -> Result<Self, String> {
        Self::spawn_with_env(program, args, &[])
    }

    /// Like [`spawn`], additionally setting the given environment variables on the child (e.g. the CAS's
    /// `CDZ_CAS_STORE_DIR` / `CDZ_CAS_WRITE_CREDENTIAL`, which it reads only at startup). The child otherwise
    /// inherits the parent environment.
    ///
    /// [`spawn`]: ServerProcess::spawn
    ///
    /// # Errors
    /// The program cannot be spawned (not found, not executable, …).
    pub fn spawn_with_env(
        program: &Path,
        args: &[&str],
        envs: &[(&str, &str)],
    ) -> Result<Self, String> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let child = cmd
            .spawn()
            .map_err(|e| format!("spawn {}: {e}", program.display()))?;
        Ok(Self { child })
    }

    /// Read the child's stderr line by line until `parse` returns `Some` (the ready line — e.g. parse the
    /// bound address out of it), then keep draining stderr in the background so a chatty server never stalls
    /// on a full pipe. Returns what `parse` extracted.
    ///
    /// # Errors
    /// The `timeout` elapses before a ready line, the child closes stderr / exits first, stderr was already
    /// consumed by an earlier call, or the stderr read fails.
    pub async fn wait_for_ready<T, F>(&mut self, timeout: Duration, parse: F) -> Result<T, String>
    where
        F: Fn(&str) -> Option<T>,
    {
        let stderr = self
            .child
            .stderr
            .take()
            .ok_or_else(|| "server stderr already consumed".to_string())?;
        let mut lines = BufReader::new(stderr).lines();
        // Retain the stderr seen before the ready line so a server that dies during boot reports WHY (its own
        // error) — not just "closed before ready". Timeout is enforced per read (a computed remaining budget)
        // so `recent` stays owned here + is available on both the timeout and the closed-stderr paths.
        let deadline = tokio::time::Instant::now() + timeout;
        let mut recent: Vec<String> = Vec::new();
        let found = loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(format!(
                    "timed out waiting for server ready line{}",
                    fmt_recent(&recent)
                ));
            }
            match tokio::time::timeout(remaining, lines.next_line()).await {
                Err(_) => {
                    return Err(format!(
                        "timed out waiting for server ready line{}",
                        fmt_recent(&recent)
                    ));
                }
                Ok(Ok(Some(line))) => {
                    if let Some(v) = parse(&line) {
                        break v;
                    }
                    recent.push(line);
                    if recent.len() > 50 {
                        recent.remove(0);
                    }
                }
                Ok(Ok(None)) => {
                    return Err(format!(
                        "server closed stderr / exited before its ready line{}",
                        fmt_recent(&recent)
                    ));
                }
                Ok(Err(e)) => return Err(format!("reading server stderr: {e}")),
            }
        };
        // Drain the rest of stderr in the background: the pipe has a bounded buffer, and a server that keeps
        // logging after boot would eventually block on a full pipe if nothing reads it.
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
        Ok(found)
    }

    /// Kill the child now (rather than waiting for drop) and reap it. Idempotent-ish: a second kill of an
    /// already-dead child is a no-op success on most platforms.
    ///
    /// # Errors
    /// The kill signal / wait fails at the OS level.
    pub async fn kill(&mut self) -> Result<(), String> {
        self.child.kill().await.map_err(|e| format!("kill: {e}"))
    }
}

/// Format the stderr lines seen before a failed ready-wait, for the error message — empty string when there
/// was none (so a clean "closed before ready" stays terse), else the captured lines under a label.
fn fmt_recent(recent: &[String]) -> String {
    if recent.is_empty() {
        String::new()
    } else {
        format!(" — server stderr:\n{}", recent.join("\n"))
    }
}

/// Parse the value of a `key=<value>` field out of a whitespace-delimited status line, e.g. the token after
/// `admin=` in `cdz-http-control-mock: admin=127.0.0.1:5 control=127.0.0.1:6 …`. Returns the raw value token
/// (no trailing whitespace); the caller parses it into a `SocketAddr` etc. Used by the per-server ready-line
/// parsers the orchestration layer supplies to [`ServerProcess::wait_for_ready`].
#[must_use]
pub fn field_after_eq<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split_whitespace().find_map(|tok| {
        let (k, v) = tok.split_once('=')?;
        (k == key).then_some(v)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    #[test]
    fn field_after_eq_extracts_the_named_field() {
        let line =
            "cdz-http-control-mock: admin=127.0.0.1:45678 control=127.0.0.1:45679 cas-url=http://x";
        assert_eq!(field_after_eq(line, "admin"), Some("127.0.0.1:45678"));
        assert_eq!(field_after_eq(line, "control"), Some("127.0.0.1:45679"));
        assert_eq!(field_after_eq(line, "cas-url"), Some("http://x"));
        // Absent key → None; a bare (no `=`) token is skipped, not mismatched.
        assert_eq!(field_after_eq(line, "listen"), None);
        assert_eq!(
            field_after_eq("gateway: listen=127.0.0.1:9 control=127.0.0.1:8", "listen"),
            Some("127.0.0.1:9")
        );
        assert!(
            field_after_eq(line, "admin")
                .unwrap()
                .parse::<SocketAddr>()
                .is_ok()
        );
    }

    #[tokio::test]
    async fn spawn_reads_the_ready_line_then_the_child_can_be_killed() {
        // A stand-in server: prints a mock-style ready line to stderr, then lingers (like a real server).
        let mut proc = ServerProcess::spawn(
            Path::new("/bin/sh"),
            &[
                "-c",
                "echo 'ready: admin=127.0.0.1:45678 control=127.0.0.1:45679' 1>&2; sleep 30",
            ],
        )
        .expect("spawn sh");
        let admin: SocketAddr = proc
            .wait_for_ready(Duration::from_secs(5), |line| {
                field_after_eq(line, "admin").and_then(|v| v.parse().ok())
            })
            .await
            .expect("ready line parsed");
        assert_eq!(admin, "127.0.0.1:45678".parse().unwrap());
        // Teardown is explicit here (drop would also kill it via kill_on_drop).
        proc.kill().await.expect("kill");
    }

    #[tokio::test]
    async fn wait_for_ready_times_out_when_no_ready_line_arrives() {
        // A server that never prints the awaited line: wait_for_ready must give up, not hang.
        let mut proc =
            ServerProcess::spawn(Path::new("/bin/sh"), &["-c", "sleep 30"]).expect("spawn sh");
        let r: Result<SocketAddr, _> = proc
            .wait_for_ready(Duration::from_millis(300), |line| {
                field_after_eq(line, "admin").and_then(|v| v.parse().ok())
            })
            .await;
        assert!(r.is_err(), "expected a timeout error, got {r:?}");
        // Dropping `proc` here kills the lingering `sleep` (kill_on_drop).
    }

    #[tokio::test]
    async fn a_dying_server_surfaces_its_stderr_in_the_error() {
        // A server that prints an error to stderr then exits before its ready line: the error must include
        // that stderr so the failure is diagnosable (not a bare "closed before ready").
        let mut proc = ServerProcess::spawn(
            Path::new("/bin/sh"),
            &["-c", "echo 'fatal: bad config at line 3' 1>&2; exit 1"],
        )
        .expect("spawn sh");
        let r: Result<SocketAddr, _> = proc
            .wait_for_ready(Duration::from_secs(5), |line| {
                field_after_eq(line, "admin").and_then(|v| v.parse().ok())
            })
            .await;
        let err = r.expect_err("a dying server errs");
        assert!(err.contains("closed stderr"), "got: {err}");
        assert!(
            err.contains("fatal: bad config at line 3"),
            "error should surface the server's stderr, got: {err}"
        );
    }

    #[tokio::test]
    async fn wait_for_ready_errors_when_the_child_exits_before_the_line() {
        // A server that exits immediately without ever printing the line: stderr closes → a clear error.
        let mut proc =
            ServerProcess::spawn(Path::new("/bin/sh"), &["-c", "exit 0"]).expect("spawn sh");
        let r: Result<SocketAddr, _> = proc
            .wait_for_ready(Duration::from_secs(5), |line| {
                field_after_eq(line, "admin").and_then(|v| v.parse().ok())
            })
            .await;
        assert!(r.is_err(), "expected a closed-stderr error, got {r:?}");
    }
}
