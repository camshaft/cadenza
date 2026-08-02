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

use crate::effect::{EffectKind, EffectRequest, Payload};
use crate::event::EffectOutcome;
use crate::hash::Hash;
use std::collections::HashMap;

/// Performs effects — the ONE executor interface, async (operator ruling: "one async trait only"). A real
/// executor performs I/O (a Bedrock model call, an HTTP request) and `.await`s its transport so a long call
/// yields instead of blocking the single-threaded kernel loop; an in-memory/test executor just returns (an
/// `async fn` with no `.await`).
///
/// **Object-safe via `async-trait`.** The kernel routes effects through `&mut dyn Executor` (see
/// [`CompositeExecutor`], which holds `Box<dyn Executor>`), so the trait MUST stay dyn-compatible —
/// native `async fn` in a trait is not, so this uses `#[async_trait(?Send)]`. `?Send` because the kernel is
/// single-threaded by design and a real transport future (a non-`Send` client / wasmtime store) needn't
/// cross threads.
#[async_trait::async_trait(?Send)]
pub trait Executor {
    /// Perform `req`. `idempotency_key` lets a side-effecting executor dedup a re-driven dispatch after a
    /// crash. Returns the outcome the kernel folds back as an `EffectResult`. May `.await` real I/O.
    async fn perform_async(&mut self, req: &EffectRequest, idempotency_key: Hash) -> EffectOutcome;
}

/// The by-kind effect router (§2 "the kernel routes, doesn't distinguish"): holds `Box<dyn Executor>`
/// per kind and routes `perform_async` to the registered executor, awaiting it. A session that emits more
/// than one effect kind (an agent doing both `Http` and `Model`) wires one of these; a native-async
/// executor (a real Bedrock/HTTP one that awaits its transport) registers directly.
///
/// A request whose kind has no registered executor returns an OBSERVABLE [`EffectOutcome::Err`] (the
/// reducer folds it — §9d anti-stuck: an unroutable effect is a normal failure event, not a panic or a
/// wedge), never a silent drop; `idempotency_key` passes through so a routed executor keeps its crash-dedup
/// contract (§16c-S1).
#[derive(Default)]
pub struct CompositeExecutor {
    by_kind: HashMap<EffectKind, Box<dyn Executor>>,
}

impl CompositeExecutor {
    pub fn new() -> Self {
        CompositeExecutor {
            by_kind: HashMap::new(),
        }
    }

    /// Register the async executor for one effect kind (builder-style; last registration wins — a
    /// deliberate override, e.g. swapping a recording executor for a live one in a test).
    pub fn with(mut self, kind: EffectKind, executor: Box<dyn Executor>) -> Self {
        self.by_kind.insert(kind, executor);
        self
    }

    /// Is an async executor registered for this kind?
    pub fn handles(&self, kind: &EffectKind) -> bool {
        self.by_kind.contains_key(kind)
    }
}

#[async_trait::async_trait(?Send)]
impl Executor for CompositeExecutor {
    async fn perform_async(&mut self, req: &EffectRequest, idempotency_key: Hash) -> EffectOutcome {
        match self.by_kind.get_mut(&req.kind) {
            Some(inner) => inner.perform_async(req, idempotency_key).await,
            None => EffectOutcome::Err(format!(
                "no executor registered for effect kind {:?} (target {:?})",
                req.kind, req.target
            )),
        }
    }
}

// Transitional aliases for `Executor` / `CompositeExecutor` — the redundant `Async` prefix was dropped
// now there is one (async) executor trait each (operator directive 2026-08-02). `pub use` re-exports
// (NOT `type` aliases) so `impl AsyncExecutor for X`, `dyn AsyncExecutor`, and
// `AsyncCompositeExecutor::new()` in the downstream `cdz-agent-host` crate keep compiling verbatim across
// the rename (alias-bridge beat 1); removed once its impls migrate to the bare names (beat 3). Do not use
// in new code — write `Executor` / `CompositeExecutor`.
#[doc(hidden)]
pub use self::CompositeExecutor as AsyncCompositeExecutor;
#[doc(hidden)]
pub use self::Executor as AsyncExecutor;

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

#[async_trait::async_trait(?Send)]
impl Executor for RecordingExecutor {
    async fn perform_async(&mut self, req: &EffectRequest, idempotency_key: Hash) -> EffectOutcome {
        self.seen.push((req.clone(), idempotency_key));
        EffectOutcome::Ok(Some(Payload::Inline(b"ok".to_vec().into())))
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
/// **Non-Shell effects** return an `Err` (this is a single-KIND executor; route multiple kinds by
/// registering it under `Shell` in an [`CompositeExecutor`]). **Idempotency (§16c-S1):** a command is
/// NOT generally idempotent; a real deployment dedups re-driven dispatches on `idempotency_key` (documented
/// as the dedup handle). Native async (the subprocess spawn is sync `std::process` today — no `.await`; a
/// truly-async spawn is a later refinement, but the trait is async so it drops in without a signature
/// change).
#[cfg(all(feature = "live-exec", unix))]
pub struct ShellExecutor;

#[cfg(all(feature = "live-exec", unix))]
#[async_trait::async_trait(?Send)]
impl Executor for ShellExecutor {
    async fn perform_async(
        &mut self,
        req: &EffectRequest,
        _idempotency_key: Hash,
    ) -> EffectOutcome {
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
            Ok(out) if out.status.success() => {
                EffectOutcome::Ok(Some(Payload::Inline(out.stdout.into())))
            }
            Ok(out) => EffectOutcome::Err(format!(
                "exit {}: {}",
                out.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&out.stderr).trim()
            )),
            Err(e) => EffectOutcome::Err(format!("spawn failed: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::EffectKind;

    fn req(kind: EffectKind, target: &str) -> EffectRequest {
        EffectRequest {
            kind,
            target: target.to_string(),
            payload: None,
            timeliness: crate::effect::Timeliness::Interactive,
        }
    }

    // A test executor that tags its Ok payload so we can prove WHICH inner executor ran. Native async.
    struct TagExecutor(&'static [u8]);
    #[async_trait::async_trait(?Send)]
    impl Executor for TagExecutor {
        async fn perform_async(&mut self, _req: &EffectRequest, _key: Hash) -> EffectOutcome {
            EffectOutcome::Ok(Some(Payload::Inline(self.0.to_vec().into())))
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tag_executor_is_object_safe_as_dyn_executor() {
        // Drivable through &mut dyn Executor — object-safety (the reason for async-trait).
        let mut tagged = TagExecutor(b"async-ran");
        let dyn_exec: &mut dyn Executor = &mut tagged;
        let out = dyn_exec
            .perform_async(&req(EffectKind::Http, "https://ok/x"), Hash::of(b"k"))
            .await;
        assert_eq!(
            out,
            EffectOutcome::Ok(Some(Payload::Inline(b"async-ran".to_vec().into())))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn composite_routes_each_kind_to_its_executor() {
        let mut exec = CompositeExecutor::new()
            .with(EffectKind::Http, Box::new(TagExecutor(b"http-ran")))
            .with(EffectKind::Shell, Box::new(TagExecutor(b"shell-ran")));
        assert!(exec.handles(&EffectKind::Http));
        assert!(exec.handles(&EffectKind::Shell));

        // Each kind reaches its own executor — the multi-kind session a single executor couldn't serve.
        assert_eq!(
            exec.perform_async(&req(EffectKind::Http, "https://ok/x"), Hash::of(b"k"))
                .await,
            EffectOutcome::Ok(Some(Payload::Inline(b"http-ran".to_vec().into())))
        );
        assert_eq!(
            exec.perform_async(&req(EffectKind::Shell, "echo hi"), Hash::of(b"k"))
                .await,
            EffectOutcome::Ok(Some(Payload::Inline(b"shell-ran".to_vec().into())))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn composite_unroutable_kind_is_an_observable_err_not_a_drop() {
        // A kind with no registered executor → Err (an observable outcome the reducer folds, §9d), never
        // a silent drop or panic.
        let mut exec = CompositeExecutor::new().with(EffectKind::Http, Box::new(TagExecutor(b"h")));
        assert!(!exec.handles(&EffectKind::Model));
        match exec
            .perform_async(&req(EffectKind::Model, "gpt"), Hash::of(b"k"))
            .await
        {
            EffectOutcome::Err(msg) => {
                assert!(
                    msg.contains("Model"),
                    "err names the unroutable kind: {msg}"
                );
            }
            other => panic!("expected Err for an unroutable kind, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn composite_last_registration_wins() {
        // Registering a kind twice replaces the prior executor (deliberate override).
        let mut exec = CompositeExecutor::new()
            .with(EffectKind::Http, Box::new(TagExecutor(b"first")))
            .with(EffectKind::Http, Box::new(TagExecutor(b"second")));
        assert_eq!(
            exec.perform_async(&req(EffectKind::Http, "x"), Hash::of(b"k"))
                .await,
            EffectOutcome::Ok(Some(Payload::Inline(b"second".to_vec().into())))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn composite_idempotency_key_passes_through() {
        // The router must forward the key unchanged so a routed side-effecting executor keeps its
        // crash-dedup contract (§16c-S1).
        struct KeyEcho;
        #[async_trait::async_trait(?Send)]
        impl Executor for KeyEcho {
            async fn perform_async(&mut self, _req: &EffectRequest, key: Hash) -> EffectOutcome {
                EffectOutcome::Ok(Some(Payload::Inline(key.as_bytes().to_vec().into())))
            }
        }
        let mut exec = CompositeExecutor::new().with(EffectKind::Http, Box::new(KeyEcho));
        let key = Hash::of(b"the-key");
        assert_eq!(
            exec.perform_async(&req(EffectKind::Http, "x"), key).await,
            EffectOutcome::Ok(Some(Payload::Inline(key.as_bytes().to_vec().into())))
        );
    }
}
