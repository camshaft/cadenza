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

use cdz_kernel::effect::{effect_ct, EffectId, EffectRequest, Payload};
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
    async fn perform(
        &mut self,
        _id: EffectId,
        req: &EffectRequest,
        _idempotency_key: Hash,
    ) -> EffectOutcome {
        // Family guard (seq-39 family-string source of truth, same as Clock/Http/Model). A wrong family is
        // structural → PERMANENT (§17: an observable Err, never a panic; a supervisor must not retry it).
        if !req.content_type.matches_family(effect_ct::SHELL) {
            return EffectOutcome::err(format!(
                "ShellExecutor only handles the {} family, got {}",
                effect_ct::SHELL,
                req.content_type.family
            ));
        }
        // PIPELINE PATH (shell-pipeline fan-out): if the payload decodes as a `(shell-pipeline …)`, run the
        // authorized STAGES — NOT `req.target`. The kernel fan-out (#2596) authorized each stage's program
        // (the SEC-F1 unit) before dispatch and does NOT gate `req.target` for a pipeline, so the executor
        // MUST key on the SAME "decodes as a pipeline" discriminant + run the stages — running `req.target`
        // here would exec an UN-GATED command (the authz-bypass the #2605 stopgap refused). This SUPERSEDES
        // that refuse-stopgap: now that the outcome codec (#2602) landed, run the decoded pipeline instead of
        // rejecting it. (The kernel #2606 gate — target AND stages — is belt-and-suspenders while we still
        // carry a `target`; a co-landed relax with v-agent-harness makes the pipeline `target` vestigial.)
        if let Some(Payload::Inline(bytes)) = &req.payload {
            if let Ok(pipeline) = cdz_kernel::event_ast::decode_shell_pipeline(bytes) {
                return run_pipeline(&pipeline);
            }
        }
        // Split the target into program + args on whitespace and exec DIRECTLY — no shell, so metacharacters
        // are literal args, not interpreted (CWE-78 / kernel PR#992 parity). This is the irreducible spawn
        // mechanism; the command itself was already authorized by the Cedar policy (SEC-F1) before dispatch.
        // The target is now opaque Arc<[u8]>; a command is UTF-8, so a non-UTF-8 target is malformed →
        // PERMANENT (fail-closed).
        let Ok(command) = req.target_str() else {
            return EffectOutcome::err("shell: target is not valid UTF-8 (expected a command)");
        };
        let mut parts = command.split_whitespace();
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

/// Run a `(shell-pipeline …)`: spawn each stage DIRECTLY (`Command::new(program).args(...)` — no `sh -c`,
/// CWE-78-safe like the single-command path), wiring each stage's stdout into the next stage's stdin, and
/// fold the outcome per Option A (a ran-but-nonzero exit is DATA in an `Ok`-frame, not a kernel `Err`). Every
/// stage's program was already authorized by the kernel's per-stage fan-out (SEC-F1) before dispatch, so this
/// is pure spawn+wire mechanism — no policy. Returns:
/// - all stages exit 0 → `Ok(Some(Inline(encode_shell_pipeline_outcome(Ok{final stdout}))))`,
/// - the FIRST stage with a nonzero exit → `Ok(Some(Inline(…Failed{stage_index, program, exit_code, stderr})))`,
///   pipefail deny-all with NO partial stdout (the reducer folds the failure + decides retry as policy),
/// - a genuine host fault (couldn't spawn a program / a pipe-wiring OS error — nothing produced a result) →
///   `EffectOutcome::err` (PERMANENT; a missing program / fork error won't succeed on a blind retry).
///
/// An EMPTY pipeline (no stages) is a host `Err` — the kernel rejects an empty `(shell-pipeline)` at
/// authz/dispatch, but guard here too so a stage-less pipeline is a clean error, never an index panic.
fn run_pipeline(pipeline: &cdz_kernel::event_ast::ShellPipeline) -> EffectOutcome {
    use cdz_kernel::event_ast::{encode_shell_pipeline_outcome, ShellPipelineOutcome};
    use std::io::{self, Read};
    use std::process::{Command, Stdio};

    if pipeline.stages.is_empty() {
        return EffectOutcome::err("shell-pipeline has no stages");
    }

    // Spawn all stages, wiring each stage's stdout into the next stage's stdin. The first stage inherits no
    // stdin (null); every stage's stdout + stderr are piped. `prev_stdout` carries the just-spawned stage's
    // stdout handle to the next iteration's stdin.
    let mut children = Vec::with_capacity(pipeline.stages.len());
    let mut prev_stdout: Option<std::process::ChildStdout> = None;
    let last_index = pipeline.stages.len() - 1;
    // The FINAL stage's stdout (the pipeline output) is drained on a DEDICATED THREAD, concurrently with
    // waiting all stages — see the deadlock note below. Held here so we can join it after the waits.
    let mut final_stdout_reader: Option<std::thread::JoinHandle<io::Result<Vec<u8>>>> = None;
    // Each stage's stderr is ALSO drained concurrently (threads), for the same reason: a stage that emits
    // >~64KB stderr would block on its full stderr pipe before we `wait` it, deadlocking the pipeline. We
    // collect all stderr readers + join them, so `Failed{stderr}` reports the failing stage's full stderr.
    let mut stderr_readers: Vec<Option<std::thread::JoinHandle<io::Result<Vec<u8>>>>> = Vec::new();
    for (index, stage) in pipeline.stages.iter().enumerate() {
        let stdin = match prev_stdout.take() {
            Some(out) => Stdio::from(out),
            None => Stdio::null(),
        };
        match Command::new(&stage.program)
            .args(&stage.args)
            .stdin(stdin)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(mut c) => {
                // Drain this stage's STDERR on a thread (concurrent with the pipeline running) so a large
                // stderr can't fill its pipe + block the stage + deadlock the pipeline.
                let stderr_handle = c.stderr.take().map(|mut e| {
                    std::thread::spawn(move || {
                        let mut buf = Vec::new();
                        e.read_to_end(&mut buf).map(|_| buf)
                    })
                });
                stderr_readers.push(stderr_handle);
                if index != last_index {
                    // Feed a NON-final stage's stdout into the NEXT stage's stdin.
                    prev_stdout = c.stdout.take();
                } else {
                    // DEADLOCK FIX (reviewer MED-HIGH DoS): the FINAL stage's stdout must be drained
                    // CONCURRENTLY with the waits, NOT read after them. If we waited stage 0..n in order and
                    // only read the final stdout at its (last) wait, a final stage emitting > one pipe buffer
                    // (~64KB) blocks on its full stdout pipe, stops reading stdin, upstream backs up on ITS
                    // stdout pipe, and the earlier `wait` on an upstream stage blocks FOREVER. So take the
                    // final stdout NOW + read it to EOF on a dedicated thread while the waits proceed.
                    final_stdout_reader = c.stdout.take().map(|mut o| {
                        std::thread::spawn(move || {
                            let mut buf = Vec::new();
                            o.read_to_end(&mut buf).map(|_| buf)
                        })
                    });
                }
                children.push(c);
            }
            // A spawn failure = a genuine host fault (program missing / fork error) → PERMANENT Err; nothing
            // produced a pipeline result. Prior spawned stages are dropped (their pipes close, they exit).
            Err(e) => {
                return EffectOutcome::err(format!(
                    "shell-pipeline: spawn of stage program {:?} failed: {e}",
                    stage.program
                ));
            }
        }
    }

    // Wait all stages for their EXIT STATUS (stdout/stderr are already being drained on threads above, so a
    // `wait` can't deadlock on a full pipe). Record the FIRST nonzero exit (deny-all pipefail). We still wait
    // ALL stages (even after a failure) so no child is left zombied.
    let mut first_failure: Option<(usize, i32)> = None;
    for (index, child) in children.iter_mut().enumerate() {
        match child.wait() {
            Ok(status) => {
                if !status.success() && first_failure.is_none() {
                    first_failure = Some((index, status.code().unwrap_or(-1)));
                }
            }
            Err(e) => {
                return EffectOutcome::err(format!(
                    "shell-pipeline: waiting on stage {index} failed: {e}"
                ));
            }
        }
    }

    // Join the stderr drain threads (indexed by stage) + the final-stdout drain thread. A thread panic or an
    // IO error draining a pipe is a host fault → Err.
    let join_reader =
        |h: Option<std::thread::JoinHandle<io::Result<Vec<u8>>>>| -> io::Result<Vec<u8>> {
            match h {
                Some(handle) => handle.join().map_err(|_| {
                    io::Error::other("shell-pipeline: a pipe-drain thread panicked")
                })?,
                None => Ok(Vec::new()),
            }
        };

    // First nonzero exit → Failed (deny-all pipefail, no partial stdout). Report that stage's stderr.
    if let Some((index, exit_code)) = first_failure {
        let stderr = match join_reader(stderr_readers.into_iter().nth(index).flatten()) {
            Ok(b) => b,
            Err(e) => return EffectOutcome::err(format!("shell-pipeline: {e}")),
        };
        let outcome = ShellPipelineOutcome::Failed {
            stage_index: index as u64,
            program: pipeline.stages[index].program.clone(),
            exit_code: i64::from(exit_code),
            stderr,
        };
        return EffectOutcome::Ok(Some(Payload::Inline(
            encode_shell_pipeline_outcome(&outcome).into(),
        )));
    }

    // All stages exited 0 → Ok with the final stage's fully-drained stdout.
    let final_stdout = match join_reader(final_stdout_reader) {
        Ok(b) => b,
        Err(e) => return EffectOutcome::err(format!("shell-pipeline: {e}")),
    };
    let outcome = ShellPipelineOutcome::Ok {
        stdout: final_stdout,
    };
    EffectOutcome::Ok(Some(Payload::Inline(
        encode_shell_pipeline_outcome(&outcome).into(),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_kernel::effect::{EffectKind, Timeliness};
    use cdz_kernel::event::Retryability;

    fn shell_req(target: &str) -> EffectRequest {
        EffectRequest::new(EffectKind::Shell, target, None, Timeliness::Interactive)
    }

    #[tokio::test]
    async fn a_two_stage_pipeline_wires_stdout_to_stdin_and_returns_the_final_stdout() {
        // `printf 'a\nb\nc\n' | grep b` — stage 0 stdout wired into stage 1 stdin; the Ok outcome carries the
        // FINAL stage's stdout. Proves the runner spawns the authorized STAGES + wires the pipe, NOT req.target
        // (which is set to a would-be-dangerous command here to prove it is not run). SECURITY: this is the
        // runner that SUPERSEDES the #2605 refuse-stopgap — it runs the Cedar-authorized stages, never target.
        use cdz_kernel::event_ast::{
            decode_shell_pipeline_outcome, encode_shell_pipeline, ShellPipeline,
            ShellPipelineOutcome, ShellStage,
        };
        let pipeline = ShellPipeline {
            stages: vec![
                ShellStage {
                    program: "printf".into(),
                    args: vec!["a\nb\nc\n".into()],
                },
                ShellStage {
                    program: "grep".into(),
                    args: vec!["b".into()],
                },
            ],
        };
        // A sentinel target that WOULD write a file if req.target were run — proves the runner runs the STAGES.
        let tmp = std::env::temp_dir().join("cdz_shell_pipeline_runner_sentinel_should_not_exist");
        let _ = std::fs::remove_file(&tmp);
        let req = EffectRequest::new(
            EffectKind::Shell,
            format!("touch {}", tmp.display()),
            Some(Payload::Inline(encode_shell_pipeline(&pipeline).into())),
            Timeliness::Interactive,
        );
        let out = ShellExecutor::new()
            .perform(EffectId(0), &req, Hash::of(b"k"))
            .await;
        match out {
            EffectOutcome::Ok(Some(Payload::Inline(bytes))) => {
                match decode_shell_pipeline_outcome(&bytes).expect("decodes as a pipeline outcome")
                {
                    ShellPipelineOutcome::Ok { stdout } => assert_eq!(
                        String::from_utf8_lossy(&stdout).trim(),
                        "b",
                        "the pipeline's final stdout is grep's match, not req.target's output"
                    ),
                    other => panic!("expected Ok pipeline outcome, got {other:?}"),
                }
            }
            other => panic!("expected an Ok pipeline-outcome frame, got {other:?}"),
        }
        assert!(
            !tmp.exists(),
            "req.target must NOT have run — the runner runs the authorized stages, not the bare target"
        );
    }

    #[tokio::test]
    async fn a_large_final_stdout_does_not_deadlock_the_pipeline() {
        // REGRESSION (reviewer MED-HIGH DoS/hang): a >=2-stage pipeline whose FINAL stage emits > one pipe
        // buffer (~64KB) used to deadlock — the forward-order waits didn't drain the final stdout until last,
        // so the full final-stdout pipe backed up the whole chain. Fix drains the final stdout on a thread
        // concurrently. Here: stage0 `yes` (unbounded) | stage1 `head -c 262144` (emits 256KB then closes) —
        // 256KB >> 64KB, so a deadlocked runner would hang; a correct one returns the 256KB promptly.
        use cdz_kernel::event_ast::{
            decode_shell_pipeline_outcome, encode_shell_pipeline, ShellPipeline,
            ShellPipelineOutcome, ShellStage,
        };
        let pipeline = ShellPipeline {
            stages: vec![
                ShellStage {
                    program: "yes".into(),
                    args: vec!["ABCDEFGH".into()],
                },
                ShellStage {
                    program: "head".into(),
                    args: vec!["-c".into(), "262144".into()],
                },
            ],
        };
        let req = EffectRequest::new(
            EffectKind::Shell,
            String::new(),
            Some(Payload::Inline(encode_shell_pipeline(&pipeline).into())),
            Timeliness::Interactive,
        );
        // A generous timeout: a correct runner finishes in ms; a deadlocked one never returns (the bug).
        let out = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            ShellExecutor::new().perform(EffectId(0), &req, Hash::of(b"k")),
        )
        .await
        .expect("pipeline must not deadlock on a large final stdout (>64KB)");
        match out {
            EffectOutcome::Ok(Some(Payload::Inline(bytes))) => {
                match decode_shell_pipeline_outcome(&bytes).expect("decodes") {
                    ShellPipelineOutcome::Ok { stdout } => assert_eq!(
                        stdout.len(),
                        262144,
                        "the full 256KB final stdout is captured, not truncated/hung"
                    ),
                    // `yes | head` can surface as `yes` exiting nonzero (SIGPIPE) on some platforms — that's a
                    // Failed outcome, still NOT a hang; the point of this test is no-deadlock, which the
                    // timeout above already proves. Accept either a clean Ok(256KB) or a Failed (no hang).
                    ShellPipelineOutcome::Failed { .. } => {}
                }
            }
            other => panic!("expected a pipeline-outcome frame (Ok or Failed), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_pipeline_stage_that_exits_nonzero_is_a_failed_outcome_deny_all_no_partial_stdout() {
        // `printf x | false` — stage 1 (`false`) exits nonzero → Failed{stage_index:1,...}, Option-A (a
        // ran-but-nonzero exit is Ok-frame DATA, not a kernel Err), pipefail deny-all (no partial stdout).
        use cdz_kernel::event_ast::{
            decode_shell_pipeline_outcome, encode_shell_pipeline, ShellPipeline,
            ShellPipelineOutcome, ShellStage,
        };
        let pipeline = ShellPipeline {
            stages: vec![
                ShellStage {
                    program: "printf".into(),
                    args: vec!["x".into()],
                },
                ShellStage {
                    program: "false".into(),
                    args: vec![],
                },
            ],
        };
        let req = EffectRequest::new(
            EffectKind::Shell,
            String::new(),
            Some(Payload::Inline(encode_shell_pipeline(&pipeline).into())),
            Timeliness::Interactive,
        );
        let out = ShellExecutor::new()
            .perform(EffectId(0), &req, Hash::of(b"k"))
            .await;
        match out {
            EffectOutcome::Ok(Some(Payload::Inline(bytes))) => {
                match decode_shell_pipeline_outcome(&bytes).expect("decodes") {
                    ShellPipelineOutcome::Failed {
                        stage_index,
                        program,
                        exit_code,
                        ..
                    } => {
                        assert_eq!(stage_index, 1, "the second stage (index 1) failed");
                        assert_eq!(program, "false");
                        assert_ne!(exit_code, 0, "a nonzero exit");
                    }
                    other => panic!("expected a Failed pipeline outcome, got {other:?}"),
                }
            }
            other => panic!("expected an Ok(Failed-frame), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_command_runs_and_returns_its_stdout() {
        let mut exec = ShellExecutor::new();
        let out = exec
            .perform(EffectId(0), &shell_req("echo hello"), Hash::of(b"k"))
            .await;
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
            .perform(EffectId(0), &shell_req("echo a ; rm -rf /"), Hash::of(b"k"))
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
        let out = exec
            .perform(EffectId(0), &shell_req("false"), Hash::of(b"k"))
            .await;
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
                EffectId(0),
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
        let req = EffectRequest::new(EffectKind::Http, "echo hi", None, Timeliness::Interactive);
        let out = exec.perform(EffectId(0), &req, Hash::of(b"k")).await;
        assert!(
            matches!(&out, EffectOutcome::Err { message, retryability } if *retryability == Retryability::Permanent && message.contains("only handles")),
            "a non-Shell family is rejected PERMANENT, got {out:?}"
        );
        assert!(exec.handles_family(effect_ct::SHELL) && !exec.handles_family(effect_ct::HTTP));
    }

    #[tokio::test]
    async fn an_empty_command_target_is_a_permanent_error() {
        let mut exec = ShellExecutor::new();
        let out = exec
            .perform(EffectId(0), &shell_req("   "), Hash::of(b"k"))
            .await;
        assert!(
            matches!(&out, EffectOutcome::Err { message, .. } if message.contains("empty command")),
            "an empty/whitespace target is PERMANENT, got {out:?}"
        );
    }

    // ---- an agent RUNS a shell TOOL-CALL loop end-to-end (converted from the deleted agent_tool_call_e2e
    // integration test, operator no-integration-tests mandate — hermetic: a Session + a Rust reducer + a
    // ModelExecutor over a SCRIPTED transport + the REAL ShellExecutor spawning `echo`, no network). This is
    // GAP-1, the self-hosting harness's first real agent-runs-a-shell-tool-call: model → shell tool-call →
    // model → end_turn, the whole loop through the real machinery (only the model transport is stubbed). ----
    use crate::model::{ModelExecutor, ModelTransport};
    use cdz_kernel::authz::Authorizer;
    use cdz_kernel::effect::{Capability, ResourcePredicate};
    use cdz_kernel::event::{ContentType, Event, EventBody};
    use cdz_kernel::event_ast::{
        encode_model_request, encode_model_response, ChatMessage, ContentBlock, ModelRequest,
        ModelResponse, ToolDef,
    };
    use cdz_kernel::executor::CompositeExecutor;
    use cdz_kernel::kernel::Session;
    use cdz_kernel::kv::Kv;
    use cdz_kernel::reducer::{FoldOutput, Reducer};
    use std::cell::Cell;

    const MODEL_ID: &str = "claude-test";

    /// A STUB Converse model transport scripting the two turns of the loop: the FIRST model call returns an M2
    /// `tool_use` (call the `shell` tool), the SECOND returns `end_turn` (the answer). Hermetic — the real
    /// Bedrock transport implements this same trait behind `live-net`. Turn-ordered, so it ignores the request
    /// bytes and just returns the M2 bytes the reducer folds.
    struct ScriptedConverse {
        calls: Cell<u32>,
    }
    #[async_trait::async_trait(?Send)]
    impl ModelTransport for ScriptedConverse {
        async fn invoke(
            &self,
            _model_id: &str,
            _body: &[u8],
            _key: Hash,
        ) -> Result<bytes::Bytes, EffectOutcome> {
            let n = self.calls.get();
            self.calls.set(n + 1);
            let resp = if n == 0 {
                ModelResponse {
                    stop_reason: "tool_use".to_string(),
                    content: vec![ContentBlock::ToolCall {
                        id: "call-1".to_string(),
                        name: "shell".to_string(),
                        input: br#"{"cmd":"echo built-green"}"#.to_vec(),
                    }],
                }
            } else {
                ModelResponse {
                    stop_reason: "end_turn".to_string(),
                    content: vec![ContentBlock::Text("done: built-green".to_string())],
                }
            };
            Ok(encode_model_response(&resp).into())
        }
    }

    /// The reference AGENT-LOOP reducer (M3 shape). Distinguishes a MODEL response (decodes as M2) from a TOOL
    /// result (raw shell stdout, doesn't) and routes: `tool_use` → dispatch the shell effect; `end_turn` →
    /// record the answer; a tool result → re-emit the next model turn carrying the ToolResult (call id
    /// round-trips so the loop closes). The tool→effect map is REDUCER-defined (policy in the reducer).
    struct ToolCallingAgent;
    impl ToolCallingAgent {
        fn model_effect(req: &ModelRequest) -> EffectRequest {
            EffectRequest::new_with_family(
                effect_ct::MODEL,
                MODEL_ID,
                Some(Payload::Inline(encode_model_request(req).into())),
                Timeliness::Interactive,
            )
        }
    }
    #[async_trait::async_trait(?Send)]
    impl Reducer for ToolCallingAgent {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound { .. } => {
                    let req = ModelRequest {
                        model: MODEL_ID.to_string(),
                        messages: vec![ChatMessage {
                            role: "user".to_string(),
                            content: vec![ContentBlock::Text("build the project".to_string())],
                        }],
                        tools: vec![ToolDef {
                            name: "shell".to_string(),
                            schema: br#"{"type":"object"}"#.to_vec(),
                        }],
                        max_tokens: Some(1024),
                    };
                    FoldOutput::with(vec![Self::model_effect(&req)])
                }
                EventBody::EffectResult {
                    result: EffectOutcome::Ok(Some(Payload::Inline(bytes))),
                    ..
                } => {
                    if let Ok(resp) = cdz_kernel::event_ast::decode_model_response(bytes) {
                        match resp.stop_reason.as_str() {
                            "tool_use" => {
                                let mut effects = Vec::new();
                                for blk in &resp.content {
                                    if let ContentBlock::ToolCall { name, .. } = blk {
                                        if name == "shell" {
                                            // Reducer-defined tool→effect map: `shell` tool → `shell` effect;
                                            // the target is the command the real ShellExecutor runs.
                                            effects.push(EffectRequest::new_with_family(
                                                effect_ct::SHELL,
                                                "echo built-green",
                                                None,
                                                Timeliness::Interactive,
                                            ));
                                        }
                                    }
                                }
                                FoldOutput::with(effects)
                            }
                            _ => {
                                // end_turn → record the answer, loop done.
                                let answer: String = resp
                                    .content
                                    .iter()
                                    .filter_map(|b| match b {
                                        ContentBlock::Text(t) => Some(t.as_str()),
                                        _ => None,
                                    })
                                    .collect();
                                kv.put(b"answer".to_vec(), answer.into_bytes());
                                FoldOutput::none()
                            }
                        }
                    } else {
                        // A TOOL (shell) result → re-emit the next model turn carrying the tool-result (call id
                        // round-trips so the model correlates it). Records the raw tool output for the assert.
                        kv.put(b"shell-out".to_vec(), bytes.to_vec());
                        let req = ModelRequest {
                            model: MODEL_ID.to_string(),
                            messages: vec![ChatMessage {
                                role: "tool".to_string(),
                                content: vec![ContentBlock::ToolResult {
                                    id: "call-1".to_string(),
                                    result: bytes.to_vec(),
                                }],
                            }],
                            tools: vec![],
                            max_tokens: Some(1024),
                        };
                        FoldOutput::with(vec![Self::model_effect(&req)])
                    }
                }
                _ => FoldOutput::none(),
            }
        }
    }

    fn tool_task() -> EventBody {
        EventBody::Inbound {
            content_type: ContentType {
                family: "message".into(),
                version: 1,
            },
            payload: Payload::Inline(b"go".to_vec().into()),
        }
    }

    /// Grant `model` (to the test model id) + `shell` (to the exact command) — deny-by-default, SEC-F1 scopes
    /// each target. This is where the COMMAND is authorized: the host ShellExecutor runs only what's permitted.
    fn agent_caps() -> Authorizer {
        Authorizer::new(vec![
            Capability {
                kind: EffectKind::Model,
                predicate: ResourcePredicate::Exact(MODEL_ID.into()),
            },
            Capability {
                kind: EffectKind::Shell,
                predicate: ResourcePredicate::Exact("echo built-green".into()),
            },
        ])
    }

    #[tokio::test]
    async fn agent_runs_a_shell_tool_call_end_to_end_model_tool_model_end_turn() {
        let mut reducer = ToolCallingAgent;
        let mut exec = CompositeExecutor::new()
            .with_effect(
                effect_ct::MODEL,
                Box::new(ModelExecutor::new(ScriptedConverse {
                    calls: Cell::new(0),
                })),
            )
            .with_effect(effect_ct::SHELL, Box::new(ShellExecutor::new()));
        let mut session =
            Session::genesis(Hash::of(b"tool-agent-v1"), Hash::of(b"tool-agent-nonce"));

        session
            .deliver(tool_task(), None, &mut reducer, &agent_caps(), &mut exec)
            .await
            .unwrap();

        // The loop ran model→shell→model→end_turn to quiescence and recorded the final answer.
        assert_eq!(
            session.kv().get(b"answer"),
            Some(&b"done: built-green"[..]),
            "the agent folded through model → shell tool-call → model → end_turn and recorded the answer"
        );
        // The REAL ShellExecutor ran `echo built-green` — its stdout is the tool result the reducer folded.
        let shell_out = session.kv().get(b"shell-out").expect("shell ran");
        assert_eq!(
            String::from_utf8_lossy(shell_out).trim(),
            "built-green",
            "the real ShellExecutor executed the tool-call command and its stdout folded back"
        );
        assert_eq!(
            session.open_effects(),
            0,
            "every effect in the loop settled"
        );

        // Replay-equivalence: the model + shell outcomes are in the log, so replay reconstructs the identical
        // KV without re-invoking the transport or re-running the command (a side-effecting tool runs once).
        let replayed = Session::replay(session.log().to_vec(), &mut reducer)
            .await
            .unwrap();
        assert_eq!(
            replayed.kv().get(b"answer"),
            Some(&b"done: built-green"[..])
        );
        assert_eq!(replayed.snapshot().kv_root, session.snapshot().kv_root);
    }
}
