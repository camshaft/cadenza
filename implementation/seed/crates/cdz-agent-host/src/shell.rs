//! The `Shell` executor — run a local command and fold its output back (GAP-2 toward the self-hosting
//! harness: an agent that BUILDS the language must run `cargo`/`git`/`gh`). Behind `live-exec`.
//!
//! **THIN MECHANISM ONLY (operator standing-order: minimize host logic — the host is INEVOLVABLE).** This
//! executor is the irreducible MECHANISM: spawn a process + fold its outcome. It carries NO command policy
//! (no allow-list) — WHICH commands a session may run is EVOLVABLE POLICY that lives in the Cedar WASM policy
//! on the log, NOT baked into this inevolvable host code. The kernel authorizes the `shell` effect on its
//! resolved `target` (the command string) via the Cedar authorizer BEFORE dispatch (SEC-F1), so a policy like
//! `permit(principal, action == Action::"shell", resource) when { resource.target like "cargo *" }` is the
//! command allow-list — and it can be swapped by shipping a new policy component, never a host redeploy. This
//! executor runs only what Cedar already permitted; it does not (and must not) re-decide policy.
//!
//! **Injection-safe (CWE-78, mirrors the kernel's PR#992 fix) — this IS irreducible mechanism, not policy.**
//! The command `target` is split on whitespace into `program` + args and exec'd DIRECTLY via
//! `Command::new(program).args(...)` — NEVER `sh -c`. So shell metacharacters (`;`, `|`, `&&`, `$()`,
//! backtick) are LITERAL arguments, not interpreted: a `target` of `git status; rm -rf /` runs `git` with
//! `status;`/`rm`/`-rf`/`/` as literal args, never a second command. This is a property of HOW the process is
//! spawned (mechanism), so it belongs in the host — it's not a policy decision about what's allowed, it's the
//! safe way to execute the one thing Cedar authorized.
//!
//! **Feature-gated (`live-exec`).** OFF by default so the hermetic gate NEVER spawns a subprocess — exactly
//! like `live-net` gates the network executors. A deployed daemon that wants agents to run commands builds
//! `--features live-exec`; WHICH commands is then the Cedar policy's call, not a host config.
//!
//! **Idempotency (§16c-S1).** A command is NOT generally idempotent; the kernel's `idempotency_key` is the
//! dedup handle a re-driven dispatch (post-crash) can key on. v0 runs the command each perform (documented);
//! a dedup cache is a later refinement.

use cdz_kernel::effect::{effect_ct, EffectRequest, Payload};
use cdz_kernel::event::EffectOutcome;
use cdz_kernel::executor::Executor;
use cdz_kernel::hash::Hash;

/// A THIN host command executor: spawn the (already-Cedar-authorized) command + fold its outcome. Serves the
/// `shell` effect family. Carries NO policy — the command allow-list is the Cedar WASM policy's job (operator
/// standing-order: host = inevolvable mechanism; policy = evolvable wasm on the log). A unit struct: it holds
/// no state because it makes no decisions.
pub struct ShellExecutor;

impl ShellExecutor {
    /// Construct the shell executor. No configuration — WHICH commands run is decided by the Cedar policy
    /// that authorizes the `shell` effect's target before this executor is ever reached, not here.
    pub fn new() -> Self {
        ShellExecutor
    }
}

impl Default for ShellExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait(?Send)]
impl Executor for ShellExecutor {
    async fn perform(&mut self, req: &EffectRequest, _idempotency_key: Hash) -> EffectOutcome {
        // Family guard (seq-39 family-string source of truth, same as Clock/Http/Model). A wrong family is
        // structural → PERMANENT (§17: an observable Err, never a panic; a supervisor must not retry it).
        if !req.content_type.matches_family(effect_ct::SHELL) {
            return EffectOutcome::err(format!(
                "ShellExecutor only handles the {} family, got {}",
                effect_ct::SHELL,
                req.content_type.family
            ));
        }
        // SECURITY (authz-bypass guard, reviewer HIGH): the kernel's shell-pipeline fan-out (#2596) authorizes
        // a `(shell-pipeline …)` payload STAGE-BY-STAGE and does NOT gate `req.target` for such an effect. This
        // executor does not yet EXECUTE pipelines (the piped-stdio multi-stage runner + its outcome codec are a
        // following slice), and it runs `req.target` directly. If a pipeline-payload effect reached here we
        // would run the UN-GATED `target` — a privilege escalation (a session with one allowed stage program
        // could smuggle an arbitrary denied command in `target`). So REJECT any effect whose payload decodes as
        // a `(shell-pipeline …)`, using the SAME discriminant the kernel fan-out keys on (payload decodes as a
        // pipeline), until the pipeline executor lands and runs the authorized stages instead of `target`.
        // PERMANENT: a pipeline payload isn't a transient fault, and re-driving must not run the un-gated target.
        if let Some(Payload::Inline(bytes)) = &req.payload {
            if cdz_kernel::event_ast::decode_shell_pipeline(bytes).is_ok() {
                return EffectOutcome::err(
                    "ShellExecutor: a shell-pipeline payload is not yet executable by this host \
                     (the per-stage pipeline runner is a following slice); refusing so the un-gated \
                     target is never run",
                );
            }
        }
        // Split the target into program + args on whitespace and exec DIRECTLY — no shell, so metacharacters
        // are literal args, not interpreted (CWE-78 / kernel PR#992 parity). This is the irreducible spawn
        // mechanism; the command itself was already authorized by the Cedar policy (SEC-F1) before dispatch.
        let mut parts = req.target.split_whitespace();
        let Some(program) = parts.next() else {
            return EffectOutcome::err("empty command");
        };
        let args: Vec<&str> = parts.collect();
        match std::process::Command::new(program).args(&args).output() {
            Ok(out) if out.status.success() => {
                EffectOutcome::Ok(Some(Payload::Inline(out.stdout.into())))
            }
            // A non-zero exit is a REAL command outcome the reducer folds (a failed build/test), not a
            // transient host fault → PERMANENT (retrying the identical command re-fails the same way; the
            // reducer decides what to do with the failure). Carries the exit code + stderr for the fold.
            Ok(out) => EffectOutcome::err(format!(
                "exit {}: {}",
                out.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&out.stderr).trim()
            )),
            // A spawn failure (program missing on PATH, fork error) is a host/environment fault → PERMANENT
            // (a missing program won't appear on a retry); the message carries the OS error.
            Err(e) => EffectOutcome::err(format!("spawn failed: {e}")),
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
    use cdz_kernel::event::Retryability;

    fn shell_req(target: &str) -> EffectRequest {
        EffectRequest::new(
            EffectKind::Shell,
            target.to_string(),
            None,
            Timeliness::Interactive,
        )
    }

    #[tokio::test]
    async fn a_shell_pipeline_payload_is_refused_so_the_ungated_target_never_runs() {
        // SECURITY regression (reviewer HIGH): the kernel fan-out (#2596) gates a shell-pipeline payload's
        // STAGES, not `req.target`. Until this host runs the authorized stages, a pipeline-payload effect must
        // be REFUSED — otherwise we'd exec the un-gated `target` (a priv-esc: smuggle a denied command in
        // `target` behind an allowed one-stage pipeline). Here `target` is a would-be-dangerous command; the
        // payload decodes as a one-stage pipeline; the executor must reject, NOT run the target.
        use cdz_kernel::event_ast::{encode_shell_pipeline, ShellPipeline, ShellStage};
        let pipeline = ShellPipeline {
            stages: vec![ShellStage {
                program: "echo".into(),
                args: vec!["allowed-stage".into()],
            }],
        };
        // A sentinel target that WOULD write a file if it ran — proves refusal, not just a non-panic.
        let tmp = std::env::temp_dir().join("cdz_shell_pipeline_bypass_sentinel_should_not_exist");
        let _ = std::fs::remove_file(&tmp);
        let target = format!("touch {}", tmp.display());
        let req = EffectRequest::new(
            EffectKind::Shell,
            target,
            Some(Payload::Inline(encode_shell_pipeline(&pipeline).into())),
            Timeliness::Interactive,
        );
        let out = ShellExecutor::new().perform(&req, Hash::of(b"k")).await;
        match out {
            EffectOutcome::Err {
                retryability: Retryability::Permanent,
                ..
            } => {}
            other => panic!("a shell-pipeline payload must be refused PERMANENT, got {other:?}"),
        }
        assert!(
            !tmp.exists(),
            "the un-gated target must NOT have run (the sentinel file must not exist)"
        );
    }

    #[tokio::test]
    async fn a_command_runs_and_returns_its_stdout() {
        let mut exec = ShellExecutor::new();
        let out = exec.perform(&shell_req("echo hello"), Hash::of(b"k")).await;
        match out {
            EffectOutcome::Ok(Some(Payload::Inline(bytes))) => {
                assert_eq!(
                    String::from_utf8_lossy(&bytes).trim(),
                    "hello",
                    "the command's stdout is folded back"
                );
            }
            other => panic!("expected Ok(stdout), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn shell_metacharacters_are_literal_args_not_a_second_command() {
        // CWE-78 (irreducible mechanism, kept in the host): the `;` + `rm` are LITERAL args to echo (direct
        // exec, no sh -c), so nothing is deleted — echo prints the tokens. This is the safe-spawn property,
        // independent of any policy (policy lives in Cedar). Proves a `;`-injected command never runs.
        let mut exec = ShellExecutor::new();
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
    async fn a_non_zero_exit_is_a_permanent_error_with_the_stderr() {
        // `false` exits non-zero → a real command failure the reducer folds, PERMANENT (not a host fault).
        let mut exec = ShellExecutor::new();
        let out = exec.perform(&shell_req("false"), Hash::of(b"k")).await;
        assert!(
            matches!(&out, EffectOutcome::Err { message, retryability } if *retryability == Retryability::Permanent && message.contains("exit ")),
            "a non-zero exit is a PERMANENT command-failure, got {out:?}"
        );
    }

    #[tokio::test]
    async fn a_missing_program_is_a_permanent_spawn_error() {
        let mut exec = ShellExecutor::new();
        let out = exec
            .perform(
                &shell_req("this-program-does-not-exist-xyz"),
                Hash::of(b"k"),
            )
            .await;
        assert!(
            matches!(&out, EffectOutcome::Err { message, retryability } if *retryability == Retryability::Permanent && message.contains("spawn failed")),
            "a missing program is a PERMANENT spawn failure, got {out:?}"
        );
    }

    #[tokio::test]
    async fn a_wrong_family_is_a_permanent_error() {
        let mut exec = ShellExecutor::new();
        let req = EffectRequest::new(
            EffectKind::Http,
            "echo hi".to_string(),
            None,
            Timeliness::Interactive,
        );
        let out = exec.perform(&req, Hash::of(b"k")).await;
        assert!(
            matches!(&out, EffectOutcome::Err { message, retryability } if *retryability == Retryability::Permanent && message.contains("only handles")),
            "a non-Shell family is rejected PERMANENT, got {out:?}"
        );
        assert!(exec.handles_family(effect_ct::SHELL) && !exec.handles_family(effect_ct::HTTP));
    }

    #[tokio::test]
    async fn an_empty_command_target_is_a_permanent_error() {
        let mut exec = ShellExecutor::new();
        let out = exec.perform(&shell_req("   "), Hash::of(b"k")).await;
        assert!(
            matches!(&out, EffectOutcome::Err { message, .. } if message.contains("empty command")),
            "an empty/whitespace target is PERMANENT, got {out:?}"
        );
    }
}
