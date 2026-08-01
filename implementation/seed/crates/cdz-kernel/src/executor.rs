//! Executors — the things that actually perform effects (§2, §12).
//!
//! The kernel authorizes an effect, appends a durable `Dispatched` record, then hands the request to
//! an executor. Executors are uniform (§2): local WASI (shell/http), model invocation, a peer inbox, a
//! remote node — the kernel doesn't distinguish, it just routes. In v0 this is a trait with in-memory
//! test impls; real WASI/model executors land next.
//!
//! Idempotency (§16c-S1/D): an executor receives the dispatch's `idempotency_key`. For a
//! side-effecting executor, re-driving the same key after a crash must not double-apply — the executor
//! dedups on it. Naturally-idempotent executors can ignore it.

use crate::effect::{EffectRequest, Payload};
use crate::event::EffectOutcome;
use crate::hash::Hash;

/// Performs effects. Synchronous in v0 (the loop is single-threaded and deterministic to test); the
/// async/remote dispatch path is a later layer that preserves this contract.
pub trait Executor {
    /// Perform `req`. `idempotency_key` lets a side-effecting executor dedup a re-driven dispatch after
    /// a crash. Returns the outcome the kernel will fold back as an `EffectResult`.
    fn perform(&mut self, req: &EffectRequest, idempotency_key: Hash) -> EffectOutcome;
}

/// A recording test executor: performs nothing real, returns a canned outcome, and logs what it saw
/// (including whether an idempotency key was replayed). Lets kernel-loop tests assert the crash /
/// no-double-fire behavior deterministically.
#[derive(Default)]
pub struct RecordingExecutor {
    pub seen: Vec<(EffectRequest, Hash)>,
}

impl RecordingExecutor {
    pub fn new() -> Self {
        RecordingExecutor { seen: Vec::new() }
    }

    /// How many times a given idempotency key was performed — the double-fire detector for tests.
    pub fn times_performed(&self, key: Hash) -> usize {
        self.seen.iter().filter(|(_, k)| *k == key).count()
    }
}

impl Executor for RecordingExecutor {
    fn perform(&mut self, req: &EffectRequest, idempotency_key: Hash) -> EffectOutcome {
        self.seen.push((req.clone(), idempotency_key));
        EffectOutcome::Ok(Some(Payload::Inline(b"ok".to_vec())))
    }
}

/// A REAL local command executor (feature `live-exec`, Unix only) — the first executor that touches the
/// world. Runs an `EffectKind::Shell` request's `target` as a program + args, executed **directly via
/// `Command::new(program).args(...)` — NO `sh -c`** (PR#992 security fix, CWE-78). Returns the outcome
/// the kernel folds back:
/// - exit 0 → `Ok(Some(stdout))`
/// - non-zero exit → `Err("exit <code>: <stderr>")`
/// - spawn failure / empty command → `Err`.
///
/// **No shell = no injection (PR#992 ⚠⚠ command injection):** the previous `sh -c <target>` let shell
/// metacharacters in the target (`;`, `|`, `&&`, `$()`, backtick) execute arbitrary commands, DEFEATING
/// the SEC-F1 `Prefix` allow-list — `echo ok; rm -rf /` passes `starts_with("echo ")` but `sh` ran the
/// `rm`. Direct exec makes every token a LITERAL argument: a `;` is an argument to the program, not a
/// separator. The target is split on whitespace into `program` + `args` (v0's minimal arg model — the
/// operator-directed structured `{program, args}` command model, §18b, is the fuller successor; this is
/// the injection-safe interim). Note this means the v0 target cannot contain quoted args with spaces —
/// acceptable for the allow-listed commands v0 runs; the structured model lifts that.
///
/// **Trust boundary:** this executor does NOT re-authorize; the kernel gated the resolved `target`
/// against a resource-scoped capability (SEC-F1) before dispatch. But note the fix's defense is
/// structural (no shell), NOT reliant on the allow-list being metachar-proof — so even an over-broad
/// grant can't yield injection, only the wrong (still-literal) program.
///
/// **Non-Shell effects** return an `Err` (v0 = one kind per executor; the by-kind composite router
/// lands with the wasm host). **Idempotency (§16c-S1):** a command is NOT generally idempotent; a real
/// deployment dedups re-driven dispatches on `idempotency_key` (documented as the dedup handle).
#[cfg(all(feature = "live-exec", unix))]
pub struct ShellExecutor;

#[cfg(all(feature = "live-exec", unix))]
impl Executor for ShellExecutor {
    fn perform(&mut self, req: &EffectRequest, _idempotency_key: Hash) -> EffectOutcome {
        use crate::effect::EffectKind;
        if req.kind != EffectKind::Shell {
            return EffectOutcome::Err(format!(
                "ShellExecutor only handles Shell effects, got {:?}",
                req.kind
            ));
        }
        // Split the target into program + args on whitespace and exec DIRECTLY — no shell, so
        // metacharacters are literal arguments, not interpreted (PR#992 CWE-78 fix).
        let mut parts = req.target.split_whitespace();
        let Some(program) = parts.next() else {
            return EffectOutcome::Err("empty command".to_string());
        };
        let args: Vec<&str> = parts.collect();
        let output = std::process::Command::new(program).args(&args).output();
        match output {
            Ok(out) if out.status.success() => EffectOutcome::Ok(Some(Payload::Inline(out.stdout))),
            Ok(out) => EffectOutcome::Err(format!(
                "exit {}: {}",
                out.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&out.stderr).trim()
            )),
            Err(e) => EffectOutcome::Err(format!("spawn failed: {e}")),
        }
    }
}
