//! The `Shell` executor — run a local command and fold its output back (GAP-2 toward the self-hosting
//! harness: an agent that BUILDS the language must run `cargo`/`git`/`gh`). This is the HOST-side executor
//! (cdz-agent-host, behind `live-exec`); the kernel has its own `ShellExecutor` behind its `live-exec`, but
//! the host owns its executor SET (`LiveExecutorSet`) + wires this in with a host-side ALLOW-LIST authz layer
//! — a defense-in-depth complement to the Cedar `shell`-family capability the kernel already gates before
//! dispatch (SEC-F1). Two independent gates: Cedar (policy — may this session run shell at all / this target)
//! + this executor's program allow-list (a hard cap on WHICH programs, regardless of an over-broad policy).
//!
//! **Injection-safe (CWE-78, mirrors the kernel's PR#992 fix).** The command `target` is split on whitespace
//! into `program` + args and exec'd DIRECTLY via `Command::new(program).args(...)` — NEVER `sh -c`. So shell
//! metacharacters (`;`, `|`, `&&`, `$()`, backtick) are LITERAL arguments, not interpreted: `git status; rm
//! -rf /` runs `git` with `status;`/`rm`/`-rf`/`/` as literal args, never a second command. The allow-list
//! keys on the program's basename, so the defense is structural (no shell) AND scoped (allow-list), not
//! reliant on either alone.
//!
//! **Feature-gated (`live-exec`).** OFF by default so the hermetic gate NEVER spawns a subprocess — exactly
//! like `live-net` gates the network executors. A deployed daemon that wants an agent to run commands builds
//! `--features live-exec` and configures the allow-list.
//!
//! **Idempotency (§16c-S1).** A command is NOT generally idempotent; the kernel's `idempotency_key` is the
//! dedup handle a re-driven dispatch (post-crash) can key on. v0 runs the command each perform (documented);
//! a dedup cache is a later refinement.

use crate::retry;
use cdz_kernel::effect::{effect_ct, EffectRequest, Payload};
use cdz_kernel::event::EffectOutcome;
use cdz_kernel::executor::Executor;
use cdz_kernel::hash::Hash;

/// A host command executor with a deny-by-default program ALLOW-LIST. Serves the `shell` effect family.
/// Constructed with the exact set of program basenames a session may run (e.g. `["cargo", "git", "gh"]`);
/// a command whose program is not on the list is a PERMANENT reject (a policy/config problem, never retry).
pub struct ShellExecutor {
    /// The allow-listed program basenames. Deny-by-default: EMPTY = no command may run (a shell executor
    /// with no grants is inert, not wide-open — the fail-safe posture). Matched on the program's basename
    /// (the last path component), so `/usr/bin/git` and `git` both match an allow-listed `"git"`.
    allowed_programs: Vec<String>,
}

impl ShellExecutor {
    /// Build with the allow-listed program basenames a session may run. Deny-by-default: pass exactly the
    /// programs this session is permitted (an empty list = a fully-inert shell executor).
    pub fn new(allowed_programs: impl IntoIterator<Item = impl Into<String>>) -> Self {
        ShellExecutor {
            allowed_programs: allowed_programs.into_iter().map(Into::into).collect(),
        }
    }

    /// Is `program` allow-listed? Compares the program's BASENAME (last `/`-separated component) against the
    /// list, so an absolute path (`/usr/bin/cargo`) matches an allow-listed bare name (`cargo`). This is a
    /// hard structural cap complementing Cedar; it is NOT itself the path-scoping authz (that stays Cedar's).
    fn is_allowed(&self, program: &str) -> bool {
        let base = program.rsplit('/').next().unwrap_or(program);
        self.allowed_programs.iter().any(|p| p == base)
    }
}

#[async_trait::async_trait(?Send)]
impl Executor for ShellExecutor {
    async fn perform(&mut self, req: &EffectRequest, _idempotency_key: Hash) -> EffectOutcome {
        // Family guard (seq-39 family-string source of truth, same as Clock/Http/Model). A wrong family is
        // structural → PERMANENT (§17: an observable Err, never a panic; a supervisor must not retry it).
        if !req.content_type.matches_family(effect_ct::SHELL) {
            return EffectOutcome::Err(retry::permanent(format!(
                "ShellExecutor only handles the {} family, got {}",
                effect_ct::SHELL,
                req.content_type.family
            )));
        }
        // Split the target into program + args on whitespace and exec DIRECTLY — no shell, so metacharacters
        // are literal args, not interpreted (CWE-78 / kernel PR#992 parity).
        let mut parts = req.target.split_whitespace();
        let Some(program) = parts.next() else {
            return EffectOutcome::Err(retry::permanent("empty command"));
        };
        // Deny-by-default allow-list: a non-allow-listed program is a POLICY/config reject (PERMANENT —
        // retrying the same denied program never succeeds). This is host-side defense-in-depth on top of the
        // kernel's Cedar `shell`-family gate; a command reaches here only if Cedar already permitted it.
        if !self.is_allowed(program) {
            return EffectOutcome::Err(retry::permanent(format!(
                "program '{program}' is not on the shell allow-list"
            )));
        }
        let args: Vec<&str> = parts.collect();
        match std::process::Command::new(program).args(&args).output() {
            Ok(out) if out.status.success() => {
                EffectOutcome::Ok(Some(Payload::Inline(out.stdout.into())))
            }
            // A non-zero exit is a REAL command outcome the reducer folds (a failed build/test), not a
            // transient host fault → PERMANENT (retrying the identical command re-fails the same way; the
            // reducer decides what to do with the failure). Carries the exit code + stderr for the fold.
            Ok(out) => EffectOutcome::Err(retry::permanent(format!(
                "exit {}: {}",
                out.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&out.stderr).trim()
            ))),
            // A spawn failure (program missing on PATH, fork error) is a host/environment fault. Missing
            // program = PERMANENT (won't appear on a retry); the message carries the OS error either way.
            Err(e) => EffectOutcome::Err(retry::permanent(format!("spawn failed: {e}"))),
        }
    }

    /// This single-kind executor serves exactly the `shell` family (the capability-manifest mechanism
    /// dimension when used bare as a `dyn Executor`; a `CompositeExecutor`'s own map answers otherwise).
    fn handles_family(&self, family: &str) -> bool {
        family == effect_ct::SHELL
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_kernel::effect::{EffectKind, Timeliness};

    fn shell_req(target: &str) -> EffectRequest {
        EffectRequest::new(
            EffectKind::Shell,
            target.to_string(),
            None,
            Timeliness::Interactive,
        )
    }

    #[tokio::test]
    async fn an_allow_listed_command_runs_and_returns_its_stdout() {
        let mut exec = ShellExecutor::new(["echo"]);
        let out = exec.perform(&shell_req("echo hello"), Hash::of(b"k")).await;
        match out {
            EffectOutcome::Ok(Some(Payload::Inline(bytes))) => {
                assert_eq!(
                    String::from_utf8_lossy(&bytes).trim(),
                    "hello",
                    "stdout of the allow-listed command is folded back"
                );
            }
            other => panic!("expected Ok(stdout), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_non_allow_listed_program_is_rejected_permanent() {
        // Deny-by-default: `echo` is allowed, `rm` is NOT → the rm is a PERMANENT reject, never run.
        let mut exec = ShellExecutor::new(["echo"]);
        let out = exec
            .perform(&shell_req("rm -rf /tmp/nope"), Hash::of(b"k"))
            .await;
        assert!(
            matches!(&out, EffectOutcome::Err(r) if r.starts_with("PERMANENT:") && r.contains("not on the shell allow-list")),
            "a non-allow-listed program is rejected PERMANENT, got {out:?}"
        );
    }

    #[tokio::test]
    async fn an_empty_allow_list_denies_everything() {
        // The fail-safe posture: a ShellExecutor with no grants runs nothing.
        let mut exec = ShellExecutor::new(Vec::<String>::new());
        let out = exec.perform(&shell_req("echo hi"), Hash::of(b"k")).await;
        assert!(
            matches!(&out, EffectOutcome::Err(r) if r.contains("not on the shell allow-list")),
            "an empty allow-list denies every command, got {out:?}"
        );
    }

    #[tokio::test]
    async fn shell_metacharacters_are_literal_args_not_a_second_command() {
        // CWE-78: `echo` is allow-listed; the `;` + `rm` are LITERAL args to echo (direct exec, no sh -c), so
        // nothing is deleted and echo simply prints the tokens. Proves the allow-list can't be bypassed by
        // chaining — a `;`-injected `rm` never runs (rm isn't the program; echo is).
        let mut exec = ShellExecutor::new(["echo"]);
        let out = exec
            .perform(&shell_req("echo a ; rm -rf /"), Hash::of(b"k"))
            .await;
        match out {
            EffectOutcome::Ok(Some(Payload::Inline(bytes))) => {
                let s = String::from_utf8_lossy(&bytes);
                assert!(
                    s.contains(';') && s.contains("rm"),
                    "the ; and rm are literal echo args (printed), not an executed second command: {s:?}"
                );
            }
            other => panic!("expected echo to print the literal tokens, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_absolute_path_matches_an_allow_listed_basename() {
        // `/bin/echo` matches an allow-listed bare `echo` (basename compare) — an agent may pass a full path.
        let mut exec = ShellExecutor::new(["echo"]);
        let out = exec
            .perform(&shell_req("/bin/echo hi"), Hash::of(b"k"))
            .await;
        assert!(
            matches!(&out, EffectOutcome::Ok(_)),
            "an absolute path whose basename is allow-listed runs, got {out:?}"
        );
    }

    #[tokio::test]
    async fn a_non_zero_exit_is_a_permanent_error_with_the_stderr() {
        // `false` exits non-zero → a real command failure the reducer folds, PERMANENT (not a host fault).
        let mut exec = ShellExecutor::new(["false"]);
        let out = exec.perform(&shell_req("false"), Hash::of(b"k")).await;
        assert!(
            matches!(&out, EffectOutcome::Err(r) if r.starts_with("PERMANENT:") && r.contains("exit ")),
            "a non-zero exit is a PERMANENT command-failure, got {out:?}"
        );
    }

    #[tokio::test]
    async fn a_wrong_family_is_a_permanent_error() {
        let mut exec = ShellExecutor::new(["echo"]);
        let req = EffectRequest::new(
            EffectKind::Http,
            "echo hi".to_string(),
            None,
            Timeliness::Interactive,
        );
        let out = exec.perform(&req, Hash::of(b"k")).await;
        assert!(
            matches!(&out, EffectOutcome::Err(r) if r.starts_with("PERMANENT:") && r.contains("only handles")),
            "a non-Shell family is rejected PERMANENT, got {out:?}"
        );
        assert!(exec.handles_family(effect_ct::SHELL) && !exec.handles_family(effect_ct::HTTP));
    }

    #[tokio::test]
    async fn an_empty_command_target_is_a_permanent_error() {
        let mut exec = ShellExecutor::new(["echo"]);
        let out = exec.perform(&shell_req("   "), Hash::of(b"k")).await;
        assert!(
            matches!(&out, EffectOutcome::Err(r) if r.contains("empty command")),
            "an empty/whitespace target is PERMANENT, got {out:?}"
        );
    }
}
