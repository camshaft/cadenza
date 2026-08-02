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

/// Performs effects. Synchronous in v0 (the loop is single-threaded and deterministic to test); the
/// async/remote dispatch path is a later layer that preserves this contract.
pub trait Executor {
    /// Perform `req`. `idempotency_key` lets a side-effecting executor dedup a re-driven dispatch after
    /// a crash. Returns the outcome the kernel will fold back as an `EffectResult`.
    fn perform(&mut self, req: &EffectRequest, idempotency_key: Hash) -> EffectOutcome;
}

/// The ASYNC executor interface — the async counterpart of [`Executor`] (operator all-async directive).
///
/// A real executor performs I/O (a Bedrock model call, an HTTP request): to run it without blocking the
/// single-threaded kernel loop, `perform` must be awaitable. This is introduced ADDITIVELY alongside the
/// sync [`Executor`] (nothing in the kernel loop consumes it yet — the async drive loop switches to it in
/// a follow-up slice, once the host's executors are migrated to this trait). The sync path is removed once
/// every executor + caller is async (the operator's "no sync remains").
///
/// **Object-safe via `async-trait`.** The kernel routes effects through `&mut dyn Executor` (see
/// [`CompositeExecutor`], which holds `Box<dyn Executor>`), so the async trait MUST stay dyn-compatible —
/// native `async fn` in a trait is not, so this uses `#[async_trait(?Send)]`. `?Send` because the kernel
/// is single-threaded by design and a real transport future (holding a non-`Send` client / wasmtime store)
/// needn't cross threads.
///
/// **A sync [`Executor`] is driven async by wrapping it in [`SyncExecutorAsAsync`]** — NOT a blanket impl
/// (same coherence reason as [`crate::reducer::SyncAsAsync`]: a blanket would forbid a genuinely-async
/// executor, e.g. a real Bedrock/HTTP one, from writing its own `AsyncExecutor`).
#[async_trait::async_trait(?Send)]
pub trait AsyncExecutor {
    /// Perform `req` asynchronously — the async counterpart of [`Executor::perform`]. Same idempotency
    /// contract (`idempotency_key` dedups a re-driven dispatch); may `.await` real I/O internally.
    async fn perform_async(&mut self, req: &EffectRequest, idempotency_key: Hash) -> EffectOutcome;
}

/// Adapt any sync [`Executor`] into an [`AsyncExecutor`] (its `perform_async` runs the sync `perform` to
/// completion — no await point, correct for an in-memory/test executor that never blocks). The explicit
/// alternative to a blanket impl (so a genuinely-async executor writes its own [`AsyncExecutor`] without a
/// coherence collision). Wrap a sync executor — `SyncExecutorAsAsync(MyExecutor)` — to drive it through the
/// async kernel loop; a real I/O executor skips the wrapper and impls [`AsyncExecutor`] directly.
pub struct SyncExecutorAsAsync<E>(pub E);

#[async_trait::async_trait(?Send)]
impl<E: Executor> AsyncExecutor for SyncExecutorAsAsync<E> {
    async fn perform_async(&mut self, req: &EffectRequest, idempotency_key: Hash) -> EffectOutcome {
        self.0.perform(req, idempotency_key)
    }
}

/// Routes each effect to a per-KIND executor (§2 "the kernel routes, doesn't distinguish"). Until now
/// a session wired exactly ONE `&mut dyn Executor`, so a reducer that emits more than one effect kind
/// (e.g. an agent that does both `Http` and `Shell`) couldn't be served — a single-kind executor like
/// a single-kind executor errors on every other kind. This is the by-kind router the executor docs promised;
/// it's independent of the wasm host, so multi-kind sessions work now.
///
/// A request whose kind has no registered executor returns [`EffectOutcome::Err`] (an OBSERVABLE outcome
/// the reducer folds — the §9d anti-stuck contract: an unroutable effect is a normal failure event, not
/// a panic or a wedge), never a silent drop. The `idempotency_key` passes through unchanged so a routed
/// executor keeps its crash-dedup contract (§16c-S1).
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

    /// Register the executor for one effect kind. Builder-style so a session's executor set reads as one
    /// expression. Registering a kind twice replaces the prior executor (last wins) — a deliberate
    /// override, e.g. swapping a recording executor for a live one in a test.
    pub fn with(mut self, kind: EffectKind, executor: Box<dyn Executor>) -> Self {
        self.by_kind.insert(kind, executor);
        self
    }

    /// Is an executor registered for this kind? (Lets a driver check routability before dispatch.)
    pub fn handles(&self, kind: &EffectKind) -> bool {
        self.by_kind.contains_key(kind)
    }
}

impl Executor for CompositeExecutor {
    fn perform(&mut self, req: &EffectRequest, idempotency_key: Hash) -> EffectOutcome {
        match self.by_kind.get_mut(&req.kind) {
            Some(inner) => inner.perform(req, idempotency_key),
            None => EffectOutcome::Err(format!(
                "no executor registered for effect kind {:?} (target {:?})",
                req.kind, req.target
            )),
        }
    }
}

/// The ASYNC by-kind router — the async twin of [`CompositeExecutor`] (operator all-async directive).
/// Holds `Box<dyn AsyncExecutor>` per kind and routes `perform_async` to the registered executor, awaiting
/// it. This is what a session's async drive loop routes through once effects go async: a NATIVE-async
/// executor (a real Bedrock/HTTP one that awaits its transport) registers here directly; a sync executor
/// registers wrapped in [`SyncExecutorAsAsync`]. Separate from [`CompositeExecutor`] because an
/// `AsyncExecutor` can't be a sync `Executor` (perform can't await) — both coexist during the sync→async
/// migration; the sync one is removed once every caller is async.
///
/// Same routing contract as the sync router: an unroutable kind returns an OBSERVABLE [`EffectOutcome::Err`]
/// (the reducer folds it — §9d anti-stuck), never a panic or silent drop; `idempotency_key` passes through.
#[derive(Default)]
pub struct AsyncCompositeExecutor {
    by_kind: HashMap<EffectKind, Box<dyn AsyncExecutor>>,
}

impl AsyncCompositeExecutor {
    pub fn new() -> Self {
        AsyncCompositeExecutor {
            by_kind: HashMap::new(),
        }
    }

    /// Register the async executor for one effect kind (builder-style; last registration wins, mirroring
    /// [`CompositeExecutor::with`]).
    pub fn with(mut self, kind: EffectKind, executor: Box<dyn AsyncExecutor>) -> Self {
        self.by_kind.insert(kind, executor);
        self
    }

    /// Is an async executor registered for this kind?
    pub fn handles(&self, kind: &EffectKind) -> bool {
        self.by_kind.contains_key(kind)
    }
}

#[async_trait::async_trait(?Send)]
impl AsyncExecutor for AsyncCompositeExecutor {
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
/// registering it under `Shell` in a [`CompositeExecutor`]). **Idempotency (§16c-S1):** a command is NOT
/// generally idempotent; a real deployment dedups re-driven dispatches on `idempotency_key` (documented
/// as the dedup handle).
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

    // A test executor that tags its Ok payload so we can prove WHICH inner executor ran.
    struct TagExecutor(&'static [u8]);
    impl Executor for TagExecutor {
        fn perform(&mut self, _req: &EffectRequest, _key: Hash) -> EffectOutcome {
            EffectOutcome::Ok(Some(Payload::Inline(self.0.to_vec().into())))
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sync_executor_wrapped_in_adapter_performs_via_async_trait() {
        // A sync Executor wrapped in SyncExecutorAsAsync is drivable via the async path (perform_async runs
        // the sync perform). Exercise through &mut dyn AsyncExecutor so object-safety holds (the reason for
        // async-trait) — and via the ADAPTER, not a blanket impl.
        let mut adapted = SyncExecutorAsAsync(TagExecutor(b"async-ran"));
        let dyn_exec: &mut dyn AsyncExecutor = &mut adapted;
        let out = dyn_exec
            .perform_async(&req(EffectKind::Http, "https://ok/x"), Hash::of(b"k"))
            .await;
        assert_eq!(
            out,
            EffectOutcome::Ok(Some(Payload::Inline(b"async-ran".to_vec().into())))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn async_composite_routes_each_kind_and_errors_on_unroutable() {
        // The async router mirrors the sync one: route by kind (awaiting the inner async executor), and an
        // unroutable kind is an OBSERVABLE Err, not a panic/drop. Register sync TagExecutors via the
        // SyncExecutorAsAsync adapter (proving a sync executor plugs into the async router during transition).
        let mut exec = AsyncCompositeExecutor::new()
            .with(
                EffectKind::Http,
                Box::new(SyncExecutorAsAsync(TagExecutor(b"http-ran"))),
            )
            .with(
                EffectKind::Model,
                Box::new(SyncExecutorAsAsync(TagExecutor(b"model-ran"))),
            );
        assert!(exec.handles(&EffectKind::Http));
        assert_eq!(
            exec.perform_async(&req(EffectKind::Http, "https://ok/x"), Hash::of(b"k"))
                .await,
            EffectOutcome::Ok(Some(Payload::Inline(b"http-ran".to_vec().into())))
        );
        assert_eq!(
            exec.perform_async(&req(EffectKind::Model, "m"), Hash::of(b"k"))
                .await,
            EffectOutcome::Ok(Some(Payload::Inline(b"model-ran".to_vec().into())))
        );
        // Unroutable kind → observable Err (Shell wasn't registered).
        assert!(matches!(
            exec.perform_async(&req(EffectKind::Shell, "echo hi"), Hash::of(b"k"))
                .await,
            EffectOutcome::Err(_)
        ));
    }

    #[test]
    fn composite_routes_each_kind_to_its_executor() {
        let mut exec = CompositeExecutor::new()
            .with(EffectKind::Http, Box::new(TagExecutor(b"http-ran")))
            .with(EffectKind::Shell, Box::new(TagExecutor(b"shell-ran")));
        assert!(exec.handles(&EffectKind::Http));
        assert!(exec.handles(&EffectKind::Shell));

        // Each kind reaches its own executor — the multi-kind session the single-executor wiring couldn't serve.
        assert_eq!(
            exec.perform(&req(EffectKind::Http, "https://ok/x"), Hash::of(b"k")),
            EffectOutcome::Ok(Some(Payload::Inline(b"http-ran".to_vec().into())))
        );
        assert_eq!(
            exec.perform(&req(EffectKind::Shell, "echo hi"), Hash::of(b"k")),
            EffectOutcome::Ok(Some(Payload::Inline(b"shell-ran".to_vec().into())))
        );
    }

    #[test]
    fn composite_unroutable_kind_is_an_observable_err_not_a_drop() {
        // A kind with no registered executor → Err (an observable outcome the reducer folds, §9d), never
        // a silent drop or panic.
        let mut exec = CompositeExecutor::new().with(EffectKind::Http, Box::new(TagExecutor(b"h")));
        assert!(!exec.handles(&EffectKind::Model));
        match exec.perform(&req(EffectKind::Model, "gpt"), Hash::of(b"k")) {
            EffectOutcome::Err(msg) => {
                assert!(
                    msg.contains("Model"),
                    "err names the unroutable kind: {msg}"
                );
            }
            other => panic!("expected Err for an unroutable kind, got {other:?}"),
        }
    }

    #[test]
    fn composite_last_registration_wins() {
        // Registering a kind twice replaces the prior executor (deliberate override, e.g. swap recording
        // for live in a test).
        let mut exec = CompositeExecutor::new()
            .with(EffectKind::Http, Box::new(TagExecutor(b"first")))
            .with(EffectKind::Http, Box::new(TagExecutor(b"second")));
        assert_eq!(
            exec.perform(&req(EffectKind::Http, "x"), Hash::of(b"k")),
            EffectOutcome::Ok(Some(Payload::Inline(b"second".to_vec().into())))
        );
    }

    #[test]
    fn composite_idempotency_key_passes_through() {
        // The router must forward the key unchanged so a routed side-effecting executor keeps its
        // crash-dedup contract (§16c-S1).
        struct KeyEcho;
        impl Executor for KeyEcho {
            fn perform(&mut self, _req: &EffectRequest, key: Hash) -> EffectOutcome {
                EffectOutcome::Ok(Some(Payload::Inline(key.as_bytes().to_vec().into())))
            }
        }
        let mut exec = CompositeExecutor::new().with(EffectKind::Http, Box::new(KeyEcho));
        let key = Hash::of(b"the-key");
        assert_eq!(
            exec.perform(&req(EffectKind::Http, "x"), key),
            EffectOutcome::Ok(Some(Payload::Inline(key.as_bytes().to_vec().into())))
        );
    }
}
